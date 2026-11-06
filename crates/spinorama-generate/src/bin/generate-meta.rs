//! `generate-meta`: TOML catalogue to `dist/json` metadata files.
//!
//! Rust replacement for Python `scripts/generate_meta.py`. Unlike the
//! Python version, which read the HDF5 cache written by
//! `generate_graphs.py`, this loads every measurement directly with
//! the crate loaders, so the output cannot go stale when graphs are
//! produced by `generate-graph`. Writes `metadata.json`, `eqdata.json`,
//! `metadata-head.json`, per-year chunks, and their zip/bz2 companions.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use serde_json::{Map, Value};
use spinorama_generate::compute::cea2034::{Cea2034, compute_spin_data};
use spinorama_generate::compute::estimates::{estimates, estimates_cea};
use spinorama_generate::compute::scores::pref_rating_dict;
use spinorama_generate::filters::apo::parse_apo;
use spinorama_generate::filters::biquad::{apply_peq, peq_response_db};
use spinorama_generate::generate_meta::{
    compute_near, derived_quality, dump_metadata, interpolate, is_partial, near_freq_grid,
    scaled_pref_rating, sensitivity_at_1m, speaker_eqs,
};
use spinorama_generate::loaders::{LoadedMeasurement, load_measurement};
use spinorama_generate::measurements::{HvSpl, symmetrise_hv};
use spinorama_generate::meta::sanitize_filename;
use spinorama_generate::metadata::{SpeakerMeta, SpeakerType, load_metadata_dir};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Formats that provide full spin data for similarity computation.
const SPIN_DATA_FORMATS: &[&str] = &["klippel", "gll_hv_txt", "spl_hv_txt", "rew_text_dump"];

struct Args {
    metadata_dir: PathBuf,
    measurements_root: PathBuf,
    datas_dir: PathBuf,
    output_dir: PathBuf,
    speaker: Option<String>,
    parse_max: Option<usize>,
    jobs: usize,
    skip_eq: bool,
    skip_near: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        metadata_dir: PathBuf::from("datas/measurements"),
        measurements_root: PathBuf::from("datas/measurements"),
        datas_dir: PathBuf::from("./datas"),
        output_dir: PathBuf::from("dist/json"),
        speaker: None,
        parse_max: None,
        jobs: 0,
        skip_eq: false,
        skip_near: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--metadata-dir" => args.metadata_dir = PathBuf::from(value(&mut it)?),
            "--measurements-root" => args.measurements_root = PathBuf::from(value(&mut it)?),
            "--datas-dir" => args.datas_dir = PathBuf::from(value(&mut it)?),
            "--output-dir" => args.output_dir = PathBuf::from(value(&mut it)?),
            "--speaker" => args.speaker = Some(value(&mut it)?),
            "--parse-max" => {
                args.parse_max = Some(
                    value(&mut it)?
                        .parse()
                        .context("--parse-max needs a count")?,
                );
            }
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--skip-eq" => args.skip_eq = true,
            "--skip-near" => args.skip_near = true,
            "--help" | "-h" => {
                println!(
                    "usage: generate-meta [--metadata-dir DIR] [--measurements-root DIR] \
                     [--datas-dir DIR] [--output-dir DIR] [--speaker NAME] \
                     [--parse-max N] [--jobs N] [--skip-eq] [--skip-near]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(args)
}

fn toml_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::from(s.as_str()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::from(*b),
        toml::Value::Datetime(dt) => Value::from(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let mut obj = Map::new();
            for (key, item) in table {
                obj.insert(key.clone(), toml_to_json(item));
            }
            Value::Object(obj)
        }
    }
}

fn load_generic_speaker(metadata_dir: &Path, speaker: &SpeakerMeta) -> Result<Value> {
    let path = metadata_dir
        .join(sanitize_filename(&speaker.name))
        .join("meta.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: toml::Value =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut json = toml_to_json(&parsed);
    if let Some(obj) = json.as_object_mut() {
        obj.remove("name");
    }
    Ok(json)
}

/// Curves and planes needed for one measurement version.
struct LoadedSpin {
    cea: Cea2034,
    planes: Option<(
        spinorama_generate::measurements::SplPlane,
        spinorama_generate::measurements::SplPlane,
    )>,
    /// Pre-shift mean on-axis SPL (sensitivity at measurement distance).
    sensitivity: Option<f64>,
}

fn load_spin(
    format: &str,
    speaker_dir: &Path,
    speaker_name: &str,
    symmetry: Option<&str>,
    inwall: bool,
) -> Result<Option<LoadedSpin>> {
    let loaded = load_measurement(format, speaker_dir, speaker_name)?;
    match loaded {
        LoadedMeasurement::Hv(hv) => {
            let mut hv = symmetrise_hv(&hv, symmetry)?;
            // Python klippel loader zeroes rear angles for in-wall
            // speakers (they are baffled by the wall).
            if inwall && format == "klippel" {
                for plane in [&mut hv.horizontal, &mut hv.vertical] {
                    for (name, values) in &mut plane.columns {
                        if spinorama_generate::measurements::angle_degrees(name)
                            .is_some_and(|angle| angle.abs() > 90)
                        {
                            values.fill(0.0);
                        }
                    }
                }
            }
            let data = compute_spin_data(&hv)?;
            let sensitivity = data.cea.sensitivity_db;
            Ok(Some(LoadedSpin {
                cea: data.cea,
                planes: Some((data.horizontal, data.vertical)),
                sensitivity: Some(sensitivity),
            }))
        }
        LoadedMeasurement::Cea(cea) => {
            let sensitivity = cea.sensitivity_db;
            Ok(Some(LoadedSpin {
                cea,
                planes: None,
                sensitivity: Some(sensitivity),
            }))
        }
        LoadedMeasurement::Curve { .. } => Ok(None),
    }
}

/// Sensitivity fallback for curve measurements whose CEA2034 set is
/// incomplete (Python `filter_graphs_partial` still derives a
/// sensitivity from the On Axis / Listening Window curves).
fn curve_sensitivity(format: &str, speaker_dir: &Path, speaker_name: &str) -> Option<f64> {
    use spinorama_generate::loaders::curves::{load_rew_curves, load_webplot_curves};
    let curves = match format {
        "rew_text_dump" => load_rew_curves(speaker_dir).ok()?,
        "webplotdigitizer" => load_webplot_curves(speaker_dir, speaker_name).ok()?,
        _ => return None,
    };
    let mean_band = |name: &str| -> Option<f64> {
        let (freq, spl) = curves.get(name)?;
        let values: Vec<f64> = freq
            .iter()
            .zip(spl)
            .filter(|(f, _)| **f > 300.0 && **f < 3000.0)
            .map(|(_, v)| *v)
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f64>() / values.len() as f64)
        }
    };
    // Python iterates On Axis then Listening Window, overwriting when
    // the value is meaningful: Listening Window wins when usable.
    if let Some(lw) = mean_band("Listening Window")
        && lw > 20.0
    {
        return Some(lw);
    }
    mean_band("On Axis")
}

fn eq_spin(
    base_hv: Option<&HvSpl>,
    base: &LoadedSpin,
    eq_file: &Path,
) -> Result<Option<LoadedSpin>> {
    let text = std::fs::read_to_string(eq_file)
        .with_context(|| format!("reading {}", eq_file.display()))?;
    let (_preamp, peq) = parse_apo(&text, 48_000.0)?;
    if peq.is_empty() {
        return Ok(None);
    }
    if let Some(hv) = base_hv {
        let mut hv = hv.clone();
        for plane in [&mut hv.horizontal, &mut hv.vertical] {
            let response = peq_response_db(&plane.freq, &peq);
            for (_, values) in &mut plane.columns {
                for (value, delta) in values.iter_mut().zip(&response) {
                    *value += delta;
                }
            }
        }
        let data = compute_spin_data(&hv)?;
        return Ok(Some(LoadedSpin {
            sensitivity: Some(data.cea.sensitivity_db),
            cea: data.cea,
            planes: Some((data.horizontal, data.vertical)),
        }));
    }
    let cea = &base.cea;
    let corrected = Cea2034 {
        freq: cea.freq.clone(),
        on_axis: apply_peq(&cea.freq, &cea.on_axis, &peq),
        listening_window: apply_peq(&cea.freq, &cea.listening_window, &peq),
        early_reflections: apply_peq(&cea.freq, &cea.early_reflections, &peq),
        sound_power: apply_peq(&cea.freq, &cea.sound_power, &peq),
        early_reflections_di: cea.early_reflections_di.clone(),
        sound_power_di: cea.sound_power_di.clone(),
        estimated_inroom: apply_peq(&cea.freq, &cea.estimated_inroom, &peq),
        sensitivity_db: cea.sensitivity_db,
    };
    Ok(Some(LoadedSpin {
        cea: corrected,
        planes: None,
        sensitivity: base.sensitivity,
    }))
}

/// Adds estimates/pref/scaled (and optionally sensitivity) for one
/// spin, mirroring Python `add_measurement`.
fn add_scores(
    speaker: &SpeakerMeta,
    version: &str,
    spin: &LoadedSpin,
    eq_tag: &str,
    out_measurement: &mut Map<String, Value>,
) {
    let measurement = &speaker.measurements[version];
    let is_default = version == speaker.default_measurement;

    if eq_tag.is_empty()
        && is_default
        && speaker.kind == SpeakerType::Passive
        && let Some(spl) = spin.sensitivity
        && spl > 20.0
    {
        let distance = measurement
            .data_acquisition
            .as_ref()
            .and_then(|da| da.distance)
            .unwrap_or(1.0);
        let mut sens = Map::new();
        sens.insert("computed".to_owned(), Value::from(spl));
        sens.insert("distance".to_owned(), Value::from(distance));
        sens.insert(
            "sensitivity_1m".to_owned(),
            Value::from(sensitivity_at_1m(spl, distance)),
        );
        out_measurement.insert("computed_sensitivity".to_owned(), Value::Object(sens));
    }

    let est = match &spin.planes {
        Some((h, v)) => estimates(&spin.cea, h, v),
        None => estimates_cea(&spin.cea),
    };
    let flatness = est.get("ref_band").and_then(Value::as_f64);
    out_measurement.insert(format!("estimates{eq_tag}"), Value::Object(est));

    if is_partial(speaker, version) {
        if let Some(flat) = flatness
            && flat.is_finite()
        {
            let mut scaled = Map::new();
            scaled.insert(
                "scaled_flatness".to_owned(),
                Value::from(spinorama_generate::generate_meta::scaled_flatness(flat)),
            );
            out_measurement.insert(format!("scaled_pref_rating{eq_tag}"), Value::Object(scaled));
        }
        return;
    }

    let Some(mut pref) = pref_rating_dict(&spin.cea) else {
        return;
    };
    let penalty = measurement
        .extras
        .as_ref()
        .and_then(|extras| extras.score_penalty)
        .unwrap_or(0.0);
    if penalty != 0.0
        && let Some(score) = pref.get_mut("pref_score")
        && let Some(value) = score.as_f64()
    {
        *score = Value::from(value + penalty);
    }
    let scaled = scaled_pref_rating(flatness.filter(|f| f.is_finite()), &pref);
    out_measurement.insert(format!("pref_rating{eq_tag}"), Value::Object(pref));
    out_measurement.insert(format!("scaled_pref_rating{eq_tag}"), Value::Object(scaled));
}

/// Interpolated (LW, ER, SP) curves on the nearest-neighbour grid.
type SpinCurves = (Vec<f64>, Vec<f64>, Vec<f64>);

struct SpeakerResult {
    name: String,
    json: Value,
    spin: Option<SpinCurves>,
}

fn process_speaker(args: &Args, speaker: &SpeakerMeta) -> SpeakerResult {
    let name = speaker.name.clone();
    let mut json = match load_generic_speaker(&args.metadata_dir, speaker) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("generate-meta: {name}: {error:#}");
            return SpeakerResult {
                name,
                json: Value::Object(Map::new()),
                spin: None,
            };
        }
    };
    let sanitized = sanitize_filename(&name);
    let mut default_spin: Option<SpinCurves> = None;

    // Raw HV of each version, kept for the _eq recompute.
    for (version, measurement) in &speaker.measurements {
        let format = measurement.format.as_str();
        let speaker_dir = args.measurements_root.join(&sanitized).join(version);
        let symmetry = measurement.symmetry.map(|s| s.as_str());

        // Quality is always present in the Python output.
        if let Some(obj) = json
            .get_mut("measurements")
            .and_then(|m| m.get_mut(version))
            .and_then(Value::as_object_mut)
        {
            obj.insert(
                "quality".to_owned(),
                Value::from(derived_quality(speaker, version)),
            );
        }

        let inwall = speaker.shape.as_str() == "inwall";
        let spin = match load_spin(format, &speaker_dir, &name, symmetry, inwall) {
            Ok(Some(spin)) => spin,
            Ok(None) => continue,
            Err(error) => {
                eprintln!("generate-meta: {name} {version}: {error:#}");
                // Incomplete curve sets can still yield a sensitivity.
                if version == &speaker.default_measurement
                    && speaker.kind == SpeakerType::Passive
                    && let Some(spl) = curve_sensitivity(format, &speaker_dir, &name)
                    && spl > 20.0
                    && let Some(obj) = json
                        .get_mut("measurements")
                        .and_then(|m| m.get_mut(version))
                        .and_then(Value::as_object_mut)
                {
                    let distance = measurement
                        .data_acquisition
                        .as_ref()
                        .and_then(|da| da.distance)
                        .unwrap_or(1.0);
                    let mut sens = Map::new();
                    sens.insert("computed".to_owned(), Value::from(spl));
                    sens.insert("distance".to_owned(), Value::from(distance));
                    sens.insert(
                        "sensitivity_1m".to_owned(),
                        Value::from(sensitivity_at_1m(spl, distance)),
                    );
                    obj.insert("computed_sensitivity".to_owned(), Value::Object(sens));
                }
                continue;
            }
        };

        if version == &speaker.default_measurement && SPIN_DATA_FORMATS.contains(&format) {
            let grid = near_freq_grid();
            default_spin = Some((
                interpolate(&grid, &spin.cea.freq, &spin.cea.listening_window),
                interpolate(&grid, &spin.cea.freq, &spin.cea.early_reflections),
                interpolate(&grid, &spin.cea.freq, &spin.cea.sound_power),
            ));
        }

        if let Some(obj) = json
            .get_mut("measurements")
            .and_then(|m| m.get_mut(version))
            .and_then(Value::as_object_mut)
        {
            add_scores(speaker, version, &spin, "", obj);
        }

        // _eq variant from datas/eq/<speaker>/iir.txt.
        let eq_file = args.datas_dir.join("eq").join(&sanitized).join("iir.txt");
        if eq_file.is_file() {
            // Re-load raw HV for the plane-level recompute when needed.
            let raw_hv = if spin.planes.is_some() {
                match load_measurement(format, &speaker_dir, &name) {
                    Ok(LoadedMeasurement::Hv(hv)) => symmetrise_hv(&hv, symmetry).ok(),
                    _ => None,
                }
            } else {
                None
            };
            match eq_spin(raw_hv.as_ref(), &spin, &eq_file) {
                Ok(Some(eq)) => {
                    if let Some(obj) = json
                        .get_mut("measurements")
                        .and_then(|m| m.get_mut(version))
                        .and_then(Value::as_object_mut)
                    {
                        add_scores(speaker, version, &eq, "_eq", obj);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("generate-meta: {name} {version} _eq: {error:#}");
                }
            }
        }
    }

    if !args.skip_eq {
        let (default_eq, eqs) = speaker_eqs(&args.datas_dir, &name);
        if let Some(obj) = json.as_object_mut() {
            if let Some(default_eq) = default_eq {
                obj.insert("default_eq".to_owned(), Value::from(default_eq));
            }
            // Python add_eq always initialises "eqs" (possibly empty);
            // consumers such as generate_radar.py rely on its presence.
            obj.insert("eqs".to_owned(), Value::Object(eqs));
        }
    }

    SpeakerResult {
        name,
        json,
        spin: default_spin,
    }
}

fn run(args: &Args) -> Result<()> {
    let started = Instant::now();
    let speakers = load_metadata_dir(&args.metadata_dir)?;
    let mut work: Vec<&SpeakerMeta> = speakers
        .values()
        .filter(|speaker| !speaker.skip)
        .filter(|speaker| {
            args.speaker
                .as_deref()
                .is_none_or(|want| want == speaker.name)
        })
        .collect();
    if let Some(max) = args.parse_max {
        work.truncate(max);
    }
    println!("generate-meta: {} speakers", work.len());

    // Speakers are independent; rayon preserves catalogue order.
    // num_threads(0) lets rayon pick one worker per CPU core.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs)
        .build()
        .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
    let mut results: Vec<SpeakerResult> = pool.install(|| {
        work.par_iter()
            .map(|speaker| process_speaker(args, speaker))
            .collect()
    });
    results.sort_by(|a, b| a.name.cmp(&b.name));

    if !args.skip_near {
        let spins: BTreeMap<String, SpinCurves> = results
            .iter()
            .filter_map(|r| r.spin.clone().map(|spin| (r.name.clone(), spin)))
            .collect();
        let names: Vec<String> = spins.keys().cloned().collect();
        // The all-pairs nearest search is O(n^2); run it on the pool.
        let nearest: Vec<Option<Vec<Value>>> = pool.install(|| {
            results
                .par_iter()
                .map(|result| {
                    let mine = result.spin.clone()?;
                    let mut deltas: Vec<(f64, String)> = Vec::new();
                    for other in &names {
                        if *other == result.name {
                            continue;
                        }
                        deltas.push((compute_near(&mine, &spins[other]), other.clone()));
                    }
                    deltas
                        .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    Some(
                        deltas
                            .into_iter()
                            .take(10)
                            .map(|(dist, other)| {
                                Value::Array(vec![Value::from(dist), Value::from(other.as_str())])
                            })
                            .collect(),
                    )
                })
                .collect()
        });
        for (result, nearest) in results.iter_mut().zip(nearest) {
            if let Some(nearest) = nearest
                && let Some(obj) = result.json.as_object_mut()
            {
                obj.insert("nearest".to_owned(), Value::Array(nearest));
            }
        }
    }

    let meta: BTreeMap<String, Value> = results
        .into_iter()
        .map(|result| (result.name, result.json))
        .collect();
    dump_metadata(&args.output_dir, &meta)?;
    println!(
        "generate-meta: wrote {} speakers to {} in {:?}",
        meta.len(),
        args.output_dir.display(),
        started.elapsed()
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-meta: {error:#}");
            ExitCode::FAILURE
        }
    }
}
