//! `generate-eq-compare`: EQ comparison figure per speaker.
//!
//! Rust replacement for `scripts/generate_eq_compare.py`. Loads the
//! generated metadata JSON, parses every `datas/eq/<Brand Model>/*.txt`
//! file except `iir.txt` as a 48 kHz IIR/REW EQ, builds the
//! `plot_eqs` figure (200-point log grid, 20 Hz to 20 kHz), writes the
//! private Plotly JSON under `dist-private/speakers/<Brand Model>/`
//! and, unless `SPINORAMA_EMIT_PUBLIC_IMAGES=0`, a hardened public WebP
//! under `dist/speakers/<Brand Model>/` via the GPUI renderer.
//! Never spawns Python.

// Rust guideline compliant 2026-10-09

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use serde_json::{Value, json};
use spinorama_generate::filters::biquad::peq_response_db;
use spinorama_generate::generate_meta::parse_rew_eq;
use spinorama_generate::meta::sanitize_filename;
use spinorama_generate::render::webp;
use spinorama_generate::render::{DEFAULT_RENDER_IMAGES_BIN, render_pngs};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Figure version, mirroring Python `VERSION = 0.2`.
const VERSION: &str = "0.2";
/// Sample rate of the parsed EQ filters, as in Python (48 kHz).
const SAMPLE_RATE_HZ: f64 = 48_000.0;
/// Points on the comparison frequency grid (Python `plot_eqs` call).
const FREQ_POINTS: usize = 200;
/// Lowest grid frequency in hertz.
const FREQ_MIN_HZ: f64 = 20.0;
/// Highest grid frequency in hertz.
const FREQ_MAX_HZ: f64 = 20_000.0;
/// Band over which Python aligns multiple EQ curves, lower edge.
const ALIGN_MIN_HZ: f64 = 80.0;
/// Band over which Python aligns multiple EQ curves, upper edge.
const ALIGN_MAX_HZ: f64 = 3_000.0;
/// Source label burned into the public watermark (Python `write_public_webp`).
const WEBP_SOURCE: &str = "Generated EQ";
/// Lossy WebP quality, matching Python `harden_png_to_webp` default.
const WEBP_QUALITY: f32 = 82.0;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Error = 0,
    Warning = 1,
    Info = 2,
    Debug = 3,
}

#[derive(Clone)]
struct Args {
    force: bool,
    level: Level,
    data_dir: PathBuf,
    metadata_json: Option<PathBuf>,
    speaker: String,
    limit: Option<usize>,
    jobs: usize,
    font: Option<PathBuf>,
    render_images_bin: PathBuf,
}

fn log(level: Level, threshold: Level, message: &str) {
    if level <= threshold {
        eprintln!("{message}");
    }
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        force: false,
        level: Level::Warning,
        data_dir: PathBuf::from("."),
        metadata_json: None,
        speaker: String::new(),
        limit: None,
        jobs: 0,
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
            "--force" => args.force = true,
            "--log-level" => {
                args.level = match value(&mut it)?.as_str() {
                    "DEBUG" => Level::Debug,
                    "INFO" => Level::Info,
                    "WARNING" => Level::Warning,
                    "ERROR" => Level::Error,
                    other => {
                        bail!("--log-level must be DEBUG, INFO, WARNING or ERROR, got {other}")
                    }
                };
            }
            // Accepted for CLI parity with the Python script, which also
            // ignores it: rendering is one staged GPUI call per speaker,
            // so there is no cross-figure batch to size.
            "--batch-size" => {
                let _ignored: usize = value(&mut it)?
                    .parse()
                    .context("--batch-size needs a count")?;
            }
            "--data-dir" => args.data_dir = PathBuf::from(value(&mut it)?),
            "--metadata-json" => args.metadata_json = Some(PathBuf::from(value(&mut it)?)),
            "--speaker" => args.speaker = value(&mut it)?,
            "--limit" => {
                args.limit = Some(value(&mut it)?.parse().context("--limit needs a count")?);
            }
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--font" => args.font = Some(PathBuf::from(value(&mut it)?)),
            "--render-images-bin" => args.render_images_bin = PathBuf::from(value(&mut it)?),
            "--version" => {
                println!("generate-eq-compare version {VERSION}");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "usage: generate-eq-compare [--force] [--log-level DEBUG|INFO|WARNING|ERROR] \
                     [--batch-size N] [--data-dir DIR] [--metadata-json FILE] \
                     [--speaker NAME] [--limit N] [--jobs N] [--font FILE] [--render-images-bin BIN]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(args)
}

/// Locates the generated metadata JSON like Python `find_metadata_file`.
///
/// With `flags_ADD_HASH = False` that is `dist/json/metadata.json`; the
/// hashed `metadata-<5 hex>.json` variant is accepted as a fallback so
/// hashed builds still resolve.
///
/// # Errors
///
/// Returns an error when no candidate file exists.
fn find_metadata_file(args: &Args) -> Result<PathBuf> {
    if let Some(path) = &args.metadata_json {
        return Ok(path.clone());
    }
    let json_dir = args.data_dir.join("dist").join("json");
    let plain = json_dir.join("metadata.json");
    if plain.is_file() {
        return Ok(plain);
    }
    let mut hashed: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&json_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_hash = name.strip_prefix("metadata-").is_some_and(|rest| {
                rest.strip_suffix(".json").is_some_and(|hash| {
                    hash.len() == 5 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                })
            });
            if is_hash {
                hashed.push(path);
            }
        }
    }
    hashed.sort();
    hashed
        .into_iter()
        .next()
        .context("Cannot find metadata file!")
}

/// Returns the 200-point log frequency grid of the Python caller.
///
/// `np.logspace(log10(2) + 1, log10(2) + 4, 200)` is 20 Hz to 20 kHz.
#[must_use]
fn freq_grid() -> Vec<f64> {
    (0..FREQ_POINTS)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "grid indices fit exactly in f64"
            )]
            let step = i as f64 / (FREQ_POINTS - 1) as f64;
            FREQ_MIN_HZ * (FREQ_MAX_HZ / FREQ_MIN_HZ).powf(step)
        })
        .collect()
}

/// Aligns curves like Python `plot_eqs` when more than one EQ is shown.
///
/// Each curve is shifted so its mean over 80 Hz to 3 kHz matches the
/// first curve's mean over the same band.
fn align_curves(freq: &[f64], curves: &mut [Vec<f64>]) {
    if curves.len() <= 1 {
        return;
    }
    // bisect_right(freq, 80) / bisect_left(freq, 3000) in Python.
    let lo = freq.partition_point(|f| *f <= ALIGN_MIN_HZ);
    let hi = freq.partition_point(|f| *f < ALIGN_MAX_HZ);
    if lo >= hi {
        return;
    }
    let means: Vec<f64> = curves
        .iter()
        .map(|curve| {
            let slice = &curve[lo..hi];
            #[expect(
                clippy::cast_precision_loss,
                reason = "point counts fit exactly in f64"
            )]
            let n = slice.len() as f64;
            slice.iter().sum::<f64>() / n
        })
        .collect();
    let reference = means[0];
    for (curve, mean) in curves.iter_mut().zip(&means) {
        let shift = mean - reference;
        for value in curve.iter_mut() {
            *value -= shift;
        }
    }
}

/// Rounds half to even, matching Python `round` on floats.
fn round_half_even(value: f64) -> f64 {
    value.round_ties_even()
}

/// Computes the y axis range and tick of Python `plot_eqs`.
///
/// The range derives from the raw (unaligned) responses, quantized
/// outward to 5 dB steps and clamped to -10 dB / +15 dB.
fn y_axis(raw_curves: &[Vec<f64>]) -> (f64, f64, f64) {
    let mut spl_min = -5.0;
    let mut spl_max = 5.0;
    if !raw_curves.is_empty() {
        let raw_min = raw_curves
            .iter()
            .flat_map(|curve| curve.iter().copied())
            .fold(f64::INFINITY, f64::min);
        let raw_max = raw_curves
            .iter()
            .flat_map(|curve| curve.iter().copied())
            .fold(f64::NEG_INFINITY, f64::max);
        if raw_min.is_finite() && raw_min < -5.0 {
            spl_min = (-10.0f64).max(-5.0 * round_half_even(-raw_min / 5.0));
        }
        if raw_max.is_finite() && raw_max > 5.0 {
            spl_max = 15.0f64.min(5.0 * round_half_even(raw_max / 5.0) + 5.0);
        }
    }
    let range = spl_max - spl_min;
    let dtick = if range <= 6.0 {
        1.0
    } else if range <= 12.0 {
        2.0
    } else {
        5.0
    };
    (spl_min, spl_max, dtick)
}

/// Builds the `plot_eqs` figure payload for one speaker.
///
/// Trace names are the EQ file basenames; the layout title is the
/// caller override `EQs for <Brand> <Model>` anchored centre/bottom.
fn build_figure_json(freq: &[f64], curves: &[Vec<f64>], names: &[String], title: &str) -> String {
    let (spl_min, spl_max, y_dtick) = y_axis(curves);
    let mut aligned = curves.to_vec();
    align_curves(freq, &mut aligned);
    let data: Vec<Value> = aligned
        .iter()
        .zip(names)
        .map(|(spl, name)| {
            json!({
                "type": "scatter",
                "mode": "lines",
                "name": name,
                "x": freq,
                "y": spl,
                "hovertemplate": "Freq: %{x:.0f}Hz<br>SPL: %{y:.1f}dB<br>",
            })
        })
        .collect();
    let figure = json!({
        "data": data,
        "layout": {
            "title": {
                "text": title,
                "x": 0.5,
                "y": 0.1,
                "xanchor": "center",
                "yanchor": "bottom",
            },
            "width": 600,
            "height": 450,
            "legend": {
                "orientation": "h",
                "yanchor": "bottom",
                "xanchor": "center",
                "y": 1.1,
                "x": 0.5,
                "title": {},
                "font": {"size": 12, "family": "Arial"},
            },
            "xaxis": {
                "title": {"text": "Frequency (Hz)"},
                "type": "log",
                "range": [FREQ_MIN_HZ.log10(), FREQ_MAX_HZ.log10()],
                "autorange": false,
                "showline": true,
                "dtick": "D1",
            },
            "yaxis": {
                "title": {"text": "SPL (dB)"},
                "range": [spl_min, spl_max],
                "autorange": false,
                "showline": true,
                "dtick": y_dtick,
            },
        },
    });
    serde_json::to_string(&figure).unwrap_or_default()
}

/// Lists the EQ files of one speaker, excluding `iir.txt`.
///
/// Sorted by file name for determinism; Python `glob` order is
/// filesystem-dependent, so trace order may differ from a Python run.
fn eq_files(eq_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(eq_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "txt")
                && path.file_name().is_some_and(|name| name != "iir.txt")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Mirrors Python `eq_compare_needs_update` against one output file.
fn needs_update(output: &Path, dependencies: &[PathBuf], force: bool) -> bool {
    if force {
        return true;
    }
    let Some(output_mtime) = mtime(output) else {
        return true;
    };
    if std::fs::metadata(output).is_ok_and(|meta| meta.len() == 0) {
        return true;
    }
    dependencies
        .iter()
        .any(|dep| mtime(dep).is_some_and(|dep_mtime| dep_mtime > output_mtime))
}

/// Writes bytes atomically via a `.tmp` sibling rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

struct SpeakerJob {
    brand: String,
    model: String,
    dir_name: String,
}

fn process_speaker(args: &Args, job: &SpeakerJob, metadata_json: &Path) -> Result<bool> {
    let eq_dir = args.data_dir.join("datas").join("eq").join(&job.dir_name);
    let files = eq_files(&eq_dir);
    let private_path = args
        .data_dir
        .join("dist-private")
        .join("speakers")
        .join(&job.dir_name)
        .join("eq_compare.json");
    let public_path = args
        .data_dir
        .join("dist")
        .join("speakers")
        .join(&job.dir_name)
        .join("eq_compare.webp");
    let emit_public = std::env::var("SPINORAMA_EMIT_PUBLIC_IMAGES").map_or(true, |v| v != "0");
    // Python keys staleness off the public WebP; with public images
    // disabled the private JSON is the only output to key off.
    let cache_output = if emit_public {
        &public_path
    } else {
        &private_path
    };
    let mut dependencies = files.clone();
    dependencies.push(metadata_json.to_path_buf());
    if !needs_update(cache_output, &dependencies, args.force) {
        log(
            Level::Info,
            args.level,
            &format!("skip (up to date): {}", cache_output.display()),
        );
        return Ok(false);
    }

    let freq = freq_grid();
    let mut names: Vec<String> = Vec::new();
    let mut curves: Vec<Vec<f64>> = Vec::new();
    for file in &files {
        // Python `parse_eq_iir_rews`: REW parsing skips malformed,
        // out-of-range and OFF lines instead of failing the file, and
        // the APO `Preamp:` line never enters the response.
        let filters = parse_rew_eq(file, SAMPLE_RATE_HZ);
        let peq: spinorama_generate::filters::biquad::Peq =
            filters.iter().map(|f| f.to_biquad()).collect();
        curves.push(peq_response_db(&freq, &peq));
        names.push(
            file.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    let title = format!("EQs for {} {}", job.brand, job.model);
    let figure_json = build_figure_json(&freq, &curves, &names, &title);
    write_atomic(&private_path, figure_json.as_bytes())?;
    log(
        Level::Info,
        args.level,
        &format!("wrote {}", private_path.display()),
    );

    if emit_public {
        // Stage only this figure so the batch GPUI renderer does not
        // re-render every other figure of the speaker.
        let stage_json_dir = args
            .data_dir
            .join("build")
            .join("eq-compare-stage")
            .join(&job.dir_name);
        std::fs::create_dir_all(&stage_json_dir)
            .with_context(|| format!("creating {}", stage_json_dir.display()))?;
        let stage_json = stage_json_dir.join("eq_compare.json");
        std::fs::write(&stage_json, &figure_json)
            .with_context(|| format!("writing {}", stage_json.display()))?;
        let stage_png_dir = args
            .data_dir
            .join("dist-gpui")
            .join("speakers")
            .join(&job.dir_name)
            .join("eq_compare");
        render_pngs(&args.render_images_bin, &stage_json_dir, &stage_png_dir)?;
        let png_path = stage_png_dir.join("eq_compare.png");
        let png_bytes = std::fs::read(&png_path)
            .with_context(|| format!("reading rendered {}", png_path.display()))?;
        let seed = format!("{} {}/eq_compare", job.brand, job.model);
        let webp_bytes = webp::harden_png_to_webp(
            &png_bytes,
            &seed,
            &webp::watermark_text(WEBP_SOURCE),
            args.font.as_deref(),
            Some(WEBP_QUALITY),
        )?;
        write_atomic(&public_path, &webp_bytes)?;
        log(
            Level::Info,
            args.level,
            &format!("wrote {}", public_path.display()),
        );
    }
    Ok(true)
}

fn run(args: &Args) -> Result<()> {
    let metadata_json = find_metadata_file(args)?;
    let text = std::fs::read_to_string(&metadata_json)
        .with_context(|| format!("reading {}", metadata_json.display()))?;
    let root: Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", metadata_json.display()))?;
    let speakers = root.as_object().ok_or_else(|| {
        anyhow::anyhow!("{}: top level is not an object", metadata_json.display())
    })?;
    log(
        Level::Info,
        args.level,
        &format!(
            "Data {} loaded ({} speakers!",
            metadata_json.display(),
            speakers.len()
        ),
    );

    let mut jobs: Vec<SpeakerJob> = Vec::new();
    let mut names: Vec<&String> = speakers.keys().collect();
    names.sort();
    for name in names {
        let data = &speakers[name];
        let (Some(brand), Some(model)) = (data["brand"].as_str(), data["model"].as_str()) else {
            log(
                Level::Warning,
                args.level,
                &format!("skip {name}: missing brand/model in metadata"),
            );
            continue;
        };
        if !args.speaker.is_empty() && args.speaker != *name {
            continue;
        }
        jobs.push(SpeakerJob {
            brand: brand.to_owned(),
            model: model.to_owned(),
            dir_name: format!("{} {}", sanitize_filename(brand), sanitize_filename(model)),
        });
    }

    let (mut done, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    if let Some(limit) = args.limit {
        // --limit counts completed/failed speakers in order, so this
        // path stays serial to preserve the exact stopping point.
        for job in &jobs {
            if done + failed >= limit {
                break;
            }
            match process_speaker(args, job, &metadata_json) {
                Ok(true) => done += 1,
                Ok(false) => skipped += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("FAILED {} {}: {error:#}", job.brand, job.model);
                }
            }
        }
    } else {
        // Speakers are independent: each writes only its own directory.
        // num_threads(0) lets rayon pick one worker per CPU core.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(args.jobs)
            .build()
            .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
        #[derive(Clone, Copy)]
        enum Outcome {
            Done,
            Skipped,
            Failed,
        }
        let outcomes: Vec<Outcome> = pool.install(|| {
            jobs.par_iter()
                .map(|job| match process_speaker(args, job, &metadata_json) {
                    Ok(true) => Outcome::Done,
                    Ok(false) => Outcome::Skipped,
                    Err(error) => {
                        eprintln!("FAILED {} {}: {error:#}", job.brand, job.model);
                        Outcome::Failed
                    }
                })
                .collect()
        });
        for outcome in outcomes {
            match outcome {
                Outcome::Done => done += 1,
                Outcome::Skipped => skipped += 1,
                Outcome::Failed => failed += 1,
            }
        }
    }
    println!("eq-compare: {done} generated, {skipped} skipped, {failed} failed");
    if failed > 0 && done + skipped == 0 {
        bail!("all {failed} speaker(s) failed");
    }
    if failed > 0 {
        eprintln!("generate-eq-compare: warning: {failed} speaker(s) failed");
    }
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-eq-compare: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_endpoints_and_len() {
        let freq = freq_grid();
        assert_eq!(freq.len(), 200);
        assert!((freq[0] - 20.0).abs() < 1e-9);
        assert!((freq[199] - 20_000.0).abs() < 1e-6);
        assert!(freq.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn align_matches_first_curve_mean() {
        let freq = freq_grid();
        let mut curves = vec![vec![0.0; 200], vec![3.0; 200]];
        align_curves(&freq, &mut curves);
        assert!(curves[1].iter().all(|v| v.abs() < 1e-9));
    }

    #[test]
    fn y_axis_defaults_and_quantization() {
        assert_eq!(y_axis(&[]), (-5.0, 5.0, 2.0));
        assert_eq!(y_axis(&[vec![-7.0, 8.0]]), (-5.0, 15.0, 5.0));
        assert_eq!(y_axis(&[vec![-12.0, 0.0]]), (-10.0, 5.0, 5.0));
    }

    #[test]
    fn figure_json_has_python_layout() {
        let freq = freq_grid();
        let json_text = build_figure_json(
            &freq,
            &[vec![0.0; 200]],
            &["iir-autoeq.txt".to_owned()],
            "EQs for Brand Model",
        );
        let value: Value = serde_json::from_str(&json_text).expect("valid json");
        assert_eq!(value["layout"]["width"], 600);
        assert_eq!(value["layout"]["height"], 450);
        assert_eq!(value["layout"]["title"]["text"], "EQs for Brand Model");
        assert_eq!(value["layout"]["title"]["y"], 0.1);
        assert_eq!(value["data"][0]["name"], "iir-autoeq.txt");
        assert_eq!(
            value["data"][0]["hovertemplate"],
            "Freq: %{x:.0f}Hz<br>SPL: %{y:.1f}dB<br>"
        );
    }
}
