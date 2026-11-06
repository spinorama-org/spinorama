//! `generate-radar`: metadata JSON to EQ spider (radar) figures.
//!
//! Rust replacement for Python `scripts/generate_radar.py`. Reads the
//! generated `metadata.json`/`eqdata.json` pair, computes the global
//! preference-score scale across speakers, and for each speaker with a
//! `pref_rating` and `estimates` writes a Plotly `scatterpolar` figure
//! to `dist-private/speakers/<Brand Model>/spider.json`, plus a hardened
//! public `dist/speakers/<Brand Model>/spider.webp` (unless
//! `SPINORAMA_EMIT_PUBLIC_IMAGES=0`). EQ traces recompute the preference
//! rating by applying `datas/eq/<Brand Model>/iir-<eq>.txt` at 48 kHz to
//! the measured H/V planes, mirroring Python `scores_apply_filter`.
//! Never spawns Python.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use serde_json::{Map, Value, json};
use spinorama_generate::compute::cea2034::compute_spin_data;
use spinorama_generate::compute::scores::pref_rating_dict_unrounded;
use spinorama_generate::filters::biquad::peq_response_db;
use spinorama_generate::generate_meta::parse_rew_eq;
use spinorama_generate::loaders::{LoadedMeasurement, load_measurement};
use spinorama_generate::measurements::symmetrise_hv;
use spinorama_generate::meta::sanitize_filename;
use spinorama_generate::render::webp;
use spinorama_generate::render::{DEFAULT_RENDER_IMAGES_BIN, render_pngs};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// EQ keys that get a spider trace, as in Python `generate_radar.py`.
const EQ_KEYS: [&str; 5] = [
    "autoeq",
    "autoeq_lw",
    "autoeq_score",
    "maiky76_lw",
    "maiky76_score",
];

/// Measurement formats scored from H/V planes (Python `scores_apply_filter`).
const SCORES_FORMATS: [&str; 3] = ["klippel", "spl_hv_txt", "gll_hv_txt"];

/// Sample rate of the REW EQ files, as in Python `parse_eq_iir_rews(..., 48000)`.
const EQ_SAMPLE_RATE_HZ: f64 = 48_000.0;

/// Pretty trace names for EQ keys (Python `pretty_name` in `print_radar`).
fn pretty_name(eq_key: &str) -> &str {
    match eq_key {
        "autoeq_lw" => "autoEQ LW",
        "autoeq_score" | "autoeq" => "autoEQ Score",
        "maiky76_lw" => "Maiky76 LW",
        "maiky76_score" => "Maiky76 Score",
        other => other,
    }
}

#[derive(Clone)]
struct Args {
    speaker: Option<String>,
    data_dir: PathBuf,
    metadata_json: Option<PathBuf>,
    eqdata_json: Option<PathBuf>,
    measurements_root: PathBuf,
    eq_dir: PathBuf,
    force: bool,
    jobs: usize,
    webp_quality: f32,
    webp_lossless: bool,
    font: Option<PathBuf>,
    render_images_bin: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        speaker: None,
        data_dir: PathBuf::from("."),
        metadata_json: None,
        eqdata_json: None,
        measurements_root: PathBuf::from("datas/measurements"),
        eq_dir: PathBuf::from("datas/eq"),
        force: false,
        jobs: 0,
        webp_quality: 82.0,
        webp_lossless: false,
        font: None,
        render_images_bin: std::env::var_os("SPINORAMA_RENDER_IMAGES")
            .map_or_else(|| PathBuf::from(DEFAULT_RENDER_IMAGES_BIN), PathBuf::from),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--speaker" => args.speaker = Some(value(&mut it)?),
            // Accepted for CLI parity with the Python script; the Python
            // `main` only ever filters on `--speaker`, so these are ignored.
            "--origin" | "--brand" | "--mversion" => {
                let _ = value(&mut it)?;
            }
            "--data-dir" => args.data_dir = PathBuf::from(value(&mut it)?),
            "--metadata-json" => args.metadata_json = Some(PathBuf::from(value(&mut it)?)),
            "--eqdata-json" => args.eqdata_json = Some(PathBuf::from(value(&mut it)?)),
            "--measurements-root" => args.measurements_root = PathBuf::from(value(&mut it)?),
            "--eq-dir" => args.eq_dir = PathBuf::from(value(&mut it)?),
            "--force" => args.force = true,
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--webp-quality" => {
                args.webp_quality = value(&mut it)?
                    .parse()
                    .context("--webp-quality needs a number in 0..=100")?;
            }
            "--webp-lossless" => args.webp_lossless = true,
            "--font" => args.font = Some(PathBuf::from(value(&mut it)?)),
            "--render-images-bin" => args.render_images_bin = PathBuf::from(value(&mut it)?),
            "--version" => {
                println!("generate-radar version 0.2");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "usage: generate-radar [--speaker NAME] [--data-dir DIR] \
                     [--metadata-json FILE] [--eqdata-json FILE] \
                     [--measurements-root DIR] [--eq-dir DIR] [--force] [--jobs N] \
                     [--webp-quality N] [--webp-lossless] [--font FILE] \
                     [--render-images-bin BIN]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(args)
}

/// Locates one generated JSON file, mirroring Python `find_metadata_file`.
///
/// The unhashed `dist/json/<stem>.json` (what `generate-meta` writes and
/// Python uses while `flags_ADD_HASH` is false) wins; otherwise the
/// hashed `dist/json/<stem>-<5 hex>.json` variant is accepted.
fn find_json(data_dir: &Path, stem: &str) -> Option<PathBuf> {
    let plain = data_dir.join("dist/json").join(format!("{stem}.json"));
    if plain.is_file() {
        return Some(plain);
    }
    let dir = data_dir.join("dist/json");
    let prefix = format!("{stem}-");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name.strip_prefix(&prefix).is_some_and(|rest| {
                rest.strip_suffix(".json").is_some_and(|hash| {
                    hash.len() == 5 && hash.chars().all(|c| c.is_ascii_hexdigit())
                })
            })
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn load_json_map(path: &Path) -> Result<Map<String, Value>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    match serde_json::from_str::<Value>(&text)
        .with_context(|| format!("parsing {}", path.display()))?
    {
        Value::Object(map) => Ok(map),
        _ => bail!("{} does not contain a JSON object", path.display()),
    }
}

fn get_f64(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

/// Global `(min, max)` per score variable, exactly as Python `compute_scale`.
///
/// Only the default measurement of each speaker counts, and only when it
/// carries both `pref_rating` and `estimates`; both `pref_rating` and
/// `pref_rating_eq` dictionaries feed the scale.
fn compute_scale(speakers: &Map<String, Value>) -> BTreeMap<String, (f64, f64)> {
    let mut scale: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for speaker in speakers.values() {
        let Some(def) = speaker.get("default_measurement").and_then(Value::as_str) else {
            continue;
        };
        let Some(measurement) = speaker
            .get("measurements")
            .and_then(|m| m.get(def))
            .and_then(Value::as_object)
        else {
            continue;
        };
        if !measurement.contains_key("pref_rating") || !measurement.contains_key("estimates") {
            continue;
        }
        for key_score in ["pref_rating", "pref_rating_eq"] {
            let Some(rating) = measurement.get(key_score).and_then(Value::as_object) else {
                continue;
            };
            for (key_variable, value) in rating {
                let Some(value) = value.as_f64() else {
                    continue;
                };
                // Python seeds a missing variable with min=100, max=-100.
                let entry = scale.entry(key_variable.clone()).or_insert((100.0, -100.0));
                entry.0 = entry.0.min(value);
                entry.1 = entry.1.max(value);
            }
        }
    }
    scale
}

fn scale_higher_is_better(
    key: &str,
    val: f64,
    scale: &BTreeMap<String, (f64, f64)>,
) -> Option<f64> {
    let (v_min, v_max) = scale.get(key)?;
    if v_max == v_min {
        // Python raises ZeroDivisionError; a degenerate scale axis is 0.
        return Some(0.0);
    }
    Some((val - v_min) / (v_max - v_min))
}

fn scale_lower_is_better(key: &str, val: f64, scale: &BTreeMap<String, (f64, f64)>) -> Option<f64> {
    Some(1.0 - scale_higher_is_better(key, val, scale)?)
}

/// One `scatterpolar` trace, mirroring Python `build_scatterplot`.
///
/// Returns `None` when a required score or scale entry is missing (Python
/// raises `KeyError` there; the Rust port skips the trace instead).
fn build_scatterplot(
    pref_rating: &Map<String, Value>,
    scale: &BTreeMap<String, (f64, f64)>,
    name: &str,
) -> Option<Value> {
    let r = vec![
        scale_higher_is_better("pref_score", get_f64(pref_rating, "pref_score")?, scale)?,
        scale_higher_is_better(
            "pref_score_wsub",
            get_f64(pref_rating, "pref_score_wsub")?,
            scale,
        )?,
        scale_lower_is_better("lfx_hz", get_f64(pref_rating, "lfx_hz")?, scale)?,
        scale_lower_is_better("nbd_on_axis", get_f64(pref_rating, "nbd_on_axis")?, scale)?,
        scale_lower_is_better(
            "nbd_pred_in_room",
            get_f64(pref_rating, "nbd_pred_in_room")?,
            scale,
        )?,
        scale_higher_is_better(
            "sm_pred_in_room",
            get_f64(pref_rating, "sm_pred_in_room")?,
            scale,
        )?,
    ];
    Some(json!({
        "type": "scatterpolar",
        "r": r,
        "theta": ["Score", "Score w/sub", "LFX", "NBD ON", "NBD PIR", "SM PIR"],
        "fill": "toself",
        "opacity": if name == "Reference" { 1.0 } else { 0.5 },
        "name": name,
    }))
}

/// Whether the Python skip rule drops `autoeq_score` for this EQ set.
fn skip_eq_key(eq_key: &str, eqs: &Map<String, Value>) -> bool {
    eq_key == "autoeq_score" && eqs.contains_key("autoeq") && eqs.contains_key("autoeq_score")
}

/// Recomputes the unrounded EQ preference rating for one speaker.
///
/// Mirrors Python `scores_apply_filter` on H/V planes: the PEQ response
/// shifts every measured angle, CEA2034 is recomputed, and the rating is
/// scored unrounded. Returns `None` for formats Python routes to
/// `noscore_apply_filter`, whose third return value is the filtered
/// on-axis frame (not a rating), so Python never adds a trace for them.
fn eq_pref_rating(
    args: &Args,
    speaker_name: &str,
    version: &str,
    format: &str,
    symmetry: Option<&str>,
    eq_file: &Path,
) -> Option<Map<String, Value>> {
    if !SCORES_FORMATS.contains(&format) {
        return None;
    }
    // Python `parse_eq_iir_rews` on a missing file yields an empty PEQ.
    let peq: spinorama_generate::filters::biquad::Peq = parse_rew_eq(eq_file, EQ_SAMPLE_RATE_HZ)
        .iter()
        .filter(|filter| filter.weight != 0.0)
        .map(spinorama_generate::generate_meta::RewFilter::to_biquad)
        .collect();
    let speaker_dir = args
        .measurements_root
        .join(sanitize_filename(speaker_name))
        .join(version);
    let loaded = match load_measurement(format, &speaker_dir, speaker_name) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("generate-radar: {speaker_name}: {error:#}");
            return None;
        }
    };
    let LoadedMeasurement::Hv(hv) = loaded else {
        return None;
    };
    let mut hv = match symmetrise_hv(&hv, symmetry) {
        Ok(hv) => hv,
        Err(error) => {
            eprintln!("generate-radar: {speaker_name}: {error:#}");
            return None;
        }
    };
    for plane in [&mut hv.horizontal, &mut hv.vertical] {
        let response = peq_response_db(&plane.freq, &peq);
        for (_, values) in &mut plane.columns {
            for (value, delta) in values.iter_mut().zip(&response) {
                *value += delta;
            }
        }
    }
    let data = match compute_spin_data(&hv) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("generate-radar: {speaker_name}: {error:#}");
            return None;
        }
    };
    pref_rating_dict_unrounded(&data.cea)
}

/// Builds the full Plotly figure payload for one speaker.
fn build_figure(
    args: &Args,
    speaker_name: &str,
    meta: &Map<String, Value>,
    scale: &BTreeMap<String, (f64, f64)>,
) -> Option<Value> {
    let def = meta.get("default_measurement").and_then(Value::as_str)?;
    let measurement = meta
        .get("measurements")
        .and_then(|m| m.get(def))
        .and_then(Value::as_object)?;
    let pref_rating = measurement.get("pref_rating").and_then(Value::as_object)?;
    measurement.get("estimates")?;
    let mut graph_data = vec![build_scatterplot(pref_rating, scale, "Reference")?];

    let empty = Map::new();
    let eqs = meta.get("eqs").and_then(Value::as_object).unwrap_or(&empty);
    let brand = meta.get("brand").and_then(Value::as_str).unwrap_or("");
    let model = meta.get("model").and_then(Value::as_str).unwrap_or("");
    let format = measurement
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("");
    let symmetry = measurement.get("symmetry").and_then(Value::as_str);
    for eq_key in eqs.keys() {
        if !EQ_KEYS.contains(&eq_key.as_str()) || skip_eq_key(eq_key, eqs) {
            continue;
        }
        let eq_file = args
            .eq_dir
            .join(format!(
                "{} {}",
                sanitize_filename(brand),
                sanitize_filename(model)
            ))
            .join(format!("iir-{}.txt", eq_key.replace('_', "-")));
        let Some(pref_eq) = eq_pref_rating(args, speaker_name, def, format, symmetry, &eq_file)
        else {
            continue;
        };
        // Python adds the trace only when both ratings carry `pref_score`.
        if !pref_rating.contains_key("pref_score") || !pref_eq.contains_key("pref_score") {
            continue;
        }
        if let Some(trace) = build_scatterplot(&pref_eq, scale, pretty_name(eq_key)) {
            graph_data.push(trace);
        }
    }
    if graph_data.is_empty() {
        return None;
    }
    Some(json!({
        "data": graph_data,
        "layout": {
            "width": 800,
            "height": 600,
            "polar": {"radialaxis": {"visible": true, "range": [0, 1]}},
            "showlegend": true,
            "legend": {"orientation": "h", "x": 0.5, "xanchor": "center"},
            "margin": {"t": 20, "l": 20, "r": 20, "b": 20},
            "title": {"text": "Improvements made by EQ"},
        },
    }))
}

/// Writes `content` only when it differs; returns whether it wrote.
fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Renders and hardens the public WebP for one speaker, when stale.
///
/// Same pattern as `generate-graph`: the GPUI renderer turns the private
/// Plotly JSON into a PNG, then `render::webp` hardens it. Failures are
/// reported and swallowed, as Python `write_public_webp` does.
fn emit_webp(
    args: &Args,
    private_dir: &Path,
    speaker_dir_name: &str,
    seed: &str,
    wrote_json: bool,
) {
    let webp_path = args
        .data_dir
        .join("dist/speakers")
        .join(speaker_dir_name)
        .join("spider.webp");
    let png_dir = args
        .data_dir
        .join("dist-gpui/speakers")
        .join(speaker_dir_name);
    let png_path = png_dir.join("spider.png");
    if !args.force && !wrote_json && png_path.is_file() && webp_path.is_file() {
        return;
    }
    if let Err(error) = render_pngs(&args.render_images_bin, private_dir, &png_dir) {
        eprintln!("generate-radar: rendering {seed}: {error:#}");
        return;
    }
    let result = (|| -> Result<()> {
        let png =
            std::fs::read(&png_path).with_context(|| format!("reading {}", png_path.display()))?;
        let quality = if args.webp_lossless {
            None
        } else {
            Some(args.webp_quality)
        };
        let bytes = webp::harden_png_to_webp(
            &png,
            seed,
            &webp::watermark_text("Generated EQ"),
            args.font.as_deref(),
            quality,
        )?;
        if let Some(parent) = webp_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = webp_path.with_extension("webp.tmp");
        std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &webp_path)
            .with_context(|| format!("renaming to {}", webp_path.display()))?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("generate-radar: WebP {seed}: {error:#}");
    }
}

fn run(args: &Args) -> Result<()> {
    let started = Instant::now();
    let metadata_path = args
        .metadata_json
        .clone()
        .or_else(|| find_json(&args.data_dir, "metadata"))
        .context("cannot find metadata file, did you run generate-meta?")?;
    let eqdata_path = args
        .eqdata_json
        .clone()
        .or_else(|| find_json(&args.data_dir, "eqdata"));
    let mut jsmeta = load_json_map(&metadata_path)?;
    if let Some(eqdata_path) = &eqdata_path {
        let eqmeta = load_json_map(eqdata_path)?;
        for (key, speaker) in &mut jsmeta {
            if let Some(eqs) = eqmeta.get(key).and_then(|e| e.get("eqs"))
                && let Some(obj) = speaker.as_object_mut()
            {
                obj.insert("eqs".to_owned(), eqs.clone());
            }
        }
    }
    println!(
        "generate-radar: data {} loaded ({} speakers)",
        metadata_path.display(),
        jsmeta.len()
    );
    let scale = compute_scale(&jsmeta);
    let emit_public = std::env::var("SPINORAMA_EMIT_PUBLIC_IMAGES").map_or(true, |v| v != "0");
    // Speakers are independent: each writes only its own directory.
    let work: Vec<&Map<String, Value>> = jsmeta
        .values()
        .filter_map(Value::as_object)
        .filter(|meta| {
            let brand = meta.get("brand").and_then(Value::as_str).unwrap_or("");
            let model = meta.get("model").and_then(Value::as_str).unwrap_or("");
            args.speaker
                .as_deref()
                .is_none_or(|want| want == format!("{brand} {model}"))
        })
        .collect();
    // num_threads(0) lets rayon pick one worker per CPU core.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs)
        .build()
        .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
    let results: Vec<Result<Option<String>>> = pool.install(|| {
        work.par_iter()
            .map(|meta| {
                let brand = meta.get("brand").and_then(Value::as_str).unwrap_or("");
                let model = meta.get("model").and_then(Value::as_str).unwrap_or("");
                let speaker_name = format!("{brand} {model}");
                let Some(figure) = build_figure(args, &speaker_name, meta, &scale) else {
                    return Ok(None);
                };
                let dir_name = format!("{} {}", sanitize_filename(brand), sanitize_filename(model));
                let private_dir = args.data_dir.join("dist-private/speakers").join(&dir_name);
                let content =
                    serde_json::to_string(&figure).context("serializing spider figure")?;
                let wrote = write_if_changed(&private_dir.join("spider.json"), &content)?;
                if emit_public {
                    emit_webp(
                        args,
                        &private_dir,
                        &dir_name,
                        &format!("{speaker_name}/spider"),
                        wrote,
                    );
                }
                Ok(Some(speaker_name))
            })
            .collect()
    });
    let mut written = 0usize;
    for result in results {
        if let Some(speaker_name) = result? {
            written += 1;
            println!("generate-radar: wrote {speaker_name}/spider");
        }
    }
    println!(
        "generate-radar: {written} spiders in {:?}",
        started.elapsed()
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-radar: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rating(pref: f64, wsub: f64, lfx: f64, nbd_on: f64, nbd_pir: f64, sm: f64) -> Value {
        json!({
            "pref_score": pref, "pref_score_wsub": wsub, "lfx_hz": lfx,
            "nbd_on_axis": nbd_on, "nbd_pred_in_room": nbd_pir, "sm_pred_in_room": sm,
        })
    }

    fn speaker(pref: Value, pref_eq: Option<Value>) -> Value {
        let mut measurement = json!({
            "format": "klippel", "pref_rating": pref, "estimates": {},
        });
        if let Some(pref_eq) = pref_eq {
            measurement["pref_rating_eq"] = pref_eq;
        }
        json!({
            "brand": "B", "model": "M", "default_measurement": "v1",
            "measurements": {"v1": measurement},
        })
    }

    #[test]
    fn scale_spans_pref_and_pref_eq() {
        let mut speakers = Map::new();
        speakers.insert(
            "B M".to_owned(),
            speaker(
                rating(5.0, 6.0, 50.0, 0.2, 0.3, 0.8),
                Some(rating(7.0, 8.0, 30.0, 0.1, 0.2, 0.9)),
            ),
        );
        let scale = compute_scale(&speakers);
        assert_eq!(scale["pref_score"], (5.0, 7.0));
        assert_eq!(scale["lfx_hz"], (30.0, 50.0));
    }

    #[test]
    fn scale_skips_speakers_without_estimates() {
        let mut meta = speaker(rating(5.0, 6.0, 50.0, 0.2, 0.3, 0.8), None);
        meta["measurements"]["v1"]
            .as_object_mut()
            .expect("object")
            .remove("estimates");
        let mut speakers = Map::new();
        speakers.insert("B M".to_owned(), meta);
        assert!(compute_scale(&speakers).is_empty());
    }

    #[test]
    fn scatterplot_scales_and_opacity() {
        let mut scale = BTreeMap::new();
        for (key, range) in [
            ("pref_score", (0.0, 10.0)),
            ("pref_score_wsub", (0.0, 10.0)),
            ("lfx_hz", (20.0, 100.0)),
            ("nbd_on_axis", (0.0, 1.0)),
            ("nbd_pred_in_room", (0.0, 1.0)),
            ("sm_pred_in_room", (0.0, 1.0)),
        ] {
            scale.insert(key.to_owned(), range);
        }
        let pref = rating(10.0, 0.0, 20.0, 0.0, 1.0, 0.5);
        let obj = pref.as_object().expect("object");
        let trace = build_scatterplot(obj, &scale, "Reference").expect("trace");
        assert_eq!(trace["opacity"], json!(1.0));
        assert_eq!(trace["r"], json!([1.0, 0.0, 1.0, 1.0, 0.0, 0.5]));
        let eq = build_scatterplot(obj, &scale, "autoEQ Score").expect("trace");
        assert_eq!(eq["opacity"], json!(0.5));
    }

    #[test]
    fn autoeq_score_skip_rule() {
        let mut eqs = Map::new();
        eqs.insert("autoeq".to_owned(), json!({}));
        assert!(!skip_eq_key("autoeq_score", &eqs));
        eqs.insert("autoeq_score".to_owned(), json!({}));
        assert!(skip_eq_key("autoeq_score", &eqs));
        assert!(!skip_eq_key("autoeq", &eqs));
    }
}
