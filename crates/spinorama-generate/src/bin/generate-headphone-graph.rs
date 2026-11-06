//! `generate-headphone-graph`: headphone measurements to graph payloads.
//!
//! Rust replacement for the `--headphones` mode of Python
//! `scripts/generate_graphs.py` (`generate_headphone_graphs`), which is
//! invoked from `update_website.sh` as
//! `generate_graphs.py --headphones [--force]`.
//!
//! For every headphone in the catalogue it writes, mirroring Python:
//!
//! - `dist-private/headphones/<name>/<origin>/<version>/` — pristine
//!   Plotly JSON (`Frequency Response`, `Frequency Response Compensated`,
//!   `Target Deviation`) plus v2 trace bundles under `traces/`;
//! - `dist/headphones/<name>/<origin>/<version>/` — hardened public WebP
//!   (GPUI renderer + [`webp`]), unless `SPINORAMA_EMIT_PUBLIC_IMAGES=0`
//!   or `--no-public-images`;
//! - `<version>_eq_autoeq_score/` and `<version>_eq_autoeq_flat/`
//!   variants of both trees when `datas/headphone_eq/<name>/`
//!   `iir-autoeq-{score,flat}.txt` exists.
//!
//! The catalogue is read from `dist/json/headphone_metadata.json`
//! (written by `generate_headphone_meta.py`, which runs before graph
//! generation in `update_website.sh`); override with `--metadata-json`.
//! Never spawns Python.

// Rust guideline compliant 2026-10-09

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use serde_json::Value;
use spinorama_generate::figures::{Axis, Figure, Trace};
use spinorama_generate::filters::biquad::{Peq, peq_response_db};
use spinorama_generate::generate_meta::{RewFilter, parse_rew_eq, preamp_gain};
use spinorama_generate::loaders::curves::parse_headphone_csv;
use spinorama_generate::meta::source_label;
use spinorama_generate::render::webp;
use spinorama_generate::render::{DEFAULT_RENDER_IMAGES_BIN, render_pngs, write_figure};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Lower edge of the normalisation band (Python `MEAN_MIN`).
const MEAN_MIN_HZ: f64 = 300.0;
/// Upper edge of the normalisation band (Python `MEAN_MAX`).
const MEAN_MAX_HZ: f64 = 3_000.0;
/// Sample rate of the headphone EQ files (Python passes 48000).
const EQ_SAMPLE_RATE_HZ: f64 = 48_000.0;
/// Frequency-response file names tried per origin, in Python order.
const FR_CANDIDATES: [&str; 3] = ["frequency_response.csv", "freq_response.csv", "fr.csv"];
/// EQ variants: `(key, file stem, display name)`, in Python order.
const EQ_VARIANTS: [(&str, &str, &str); 2] = [
    ("autoeq_score", "iir-autoeq-score", "Harman Score EQ (IIR)"),
    ("autoeq_flat", "iir-autoeq-flat", "Flat Target EQ (IIR)"),
];

#[derive(Clone)]
struct Args {
    data_dir: PathBuf,
    metadata_json: Option<PathBuf>,
    targets_dir: Option<PathBuf>,
    eq_dir: Option<PathBuf>,
    speaker: Option<String>,
    jobs: usize,
    force: bool,
    emit_public_images: bool,
    webp_quality: f32,
    webp_lossless: bool,
    webp_dir: Option<PathBuf>,
    font: Option<PathBuf>,
    render_images_bin: PathBuf,
    png_root: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        data_dir: PathBuf::from("."),
        metadata_json: None,
        targets_dir: None,
        eq_dir: None,
        speaker: None,
        jobs: 0,
        force: false,
        emit_public_images: std::env::var("SPINORAMA_EMIT_PUBLIC_IMAGES").as_deref() != Ok("0"),
        webp_quality: 82.0,
        webp_lossless: false,
        webp_dir: None,
        font: None,
        render_images_bin: std::env::var_os("SPINORAMA_RENDER_IMAGES")
            .map_or_else(|| PathBuf::from(DEFAULT_RENDER_IMAGES_BIN), PathBuf::from),
        png_root: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            // Python `--headphones` selects this mode; accepted for CLI parity.
            "--headphones" => {}
            "--force" => args.force = true,
            "--data-dir" => args.data_dir = PathBuf::from(value(&mut it)?),
            "--speaker" => args.speaker = Some(value(&mut it)?),
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--metadata-json" => args.metadata_json = Some(PathBuf::from(value(&mut it)?)),
            "--targets-dir" => args.targets_dir = Some(PathBuf::from(value(&mut it)?)),
            "--eq-dir" => args.eq_dir = Some(PathBuf::from(value(&mut it)?)),
            "--no-public-images" => args.emit_public_images = false,
            "--render-png" | "--render-webp" => args.emit_public_images = true,
            "--webp-quality" => {
                args.webp_quality = value(&mut it)?
                    .parse()
                    .context("--webp-quality needs a number in 0..=100")?;
            }
            "--webp-lossless" => args.webp_lossless = true,
            "--webp-dir" => args.webp_dir = Some(PathBuf::from(value(&mut it)?)),
            "--font" => args.font = Some(PathBuf::from(value(&mut it)?)),
            "--render-images-bin" => args.render_images_bin = PathBuf::from(value(&mut it)?),
            "--png-dir" => args.png_root = Some(PathBuf::from(value(&mut it)?)),
            "--help" | "-h" => {
                println!(
                    "usage: generate-headphone-graph [--headphones] [--force] \
                     [--data-dir DIR] [--speaker NAME] [--jobs N] [--metadata-json FILE] \
                     [--targets-dir DIR] [--eq-dir DIR] [--no-public-images] \
                     [--render-images-bin BIN] [--font FILE]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(args)
}

fn mean_in_band(freq: &[f64], spl: &[f64], min_hz: f64, max_hz: f64) -> f64 {
    let (mut sum, mut count) = (0.0, 0u64);
    for (f, v) in freq.iter().zip(spl) {
        if *f >= min_hz && *f <= max_hz {
            sum += v;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        #[expect(
            clippy::cast_precision_loss,
            reason = "point counts fit exactly in f64"
        )]
        let n = count as f64;
        sum / n
    }
}

/// Linear interpolation with end clamping, matching `numpy.interp`.
fn interp_clamp(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
    if src_freq.is_empty() {
        return vec![0.0; freq.len()];
    }
    freq.iter()
        .map(|target| {
            if *target <= src_freq[0] {
                return src_values[0];
            }
            if *target >= src_freq[src_freq.len() - 1] {
                return src_values[src_values.len() - 1];
            }
            let upper = src_freq.partition_point(|f| *f < *target);
            let (x0, x1) = (src_freq[upper - 1], src_freq[upper]);
            if x1 == x0 {
                return src_values[upper];
            }
            let t = (target - x0) / (x1 - x0);
            src_values[upper - 1] + t * (src_values[upper] - src_values[upper - 1])
        })
        .collect()
}

fn load_csv_curve(path: &Path) -> Result<(Vec<f64>, Vec<f64>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_headphone_csv(&text).with_context(|| format!("parsing {}", path.display()))
}

fn target_key_for_shape(shape: &str) -> &'static str {
    match shape {
        "in-ear" | "earbud" => "harman_inear_2019",
        _ => "harman_overear_2019",
    }
}

fn is_fresh(output: &Path, input: &Path, force: bool) -> bool {
    if force {
        return false;
    }
    let modified = |path: &Path| -> Option<SystemTime> {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    match (modified(output), modified(input)) {
        (Some(out), Some(inp)) => out > inp,
        _ => false,
    }
}

struct PendingImage {
    private_dir: PathBuf,
    public_dir: PathBuf,
    seed_prefix: String,
    source: String,
}

fn write_graph(
    figure: &Figure,
    public_dir: &Path,
    private_dir: &Path,
    seed_prefix: &str,
    source: &str,
    pending: &mut Vec<PendingImage>,
) -> Result<()> {
    std::fs::create_dir_all(public_dir)
        .with_context(|| format!("creating {}", public_dir.display()))?;
    write_figure(private_dir, figure)?;
    pending.push(PendingImage {
        private_dir: private_dir.to_path_buf(),
        public_dir: public_dir.to_path_buf(),
        seed_prefix: seed_prefix.to_owned(),
        source: source.to_owned(),
    });
    Ok(())
}

fn emit_public_images(args: &Args, pending: &[PendingImage]) -> usize {
    if !args.emit_public_images || pending.is_empty() {
        return 0;
    }
    // One render pass per output directory (base and each EQ variant).
    let mut dirs: Vec<&PendingImage> = Vec::new();
    for item in pending {
        if !dirs.iter().any(|d| d.private_dir == item.private_dir) {
            dirs.push(item);
        }
    }
    // One render pass per directory; directories are independent.
    let written: usize = dirs
        .par_iter()
        .map(|item| {
            let item = *item;
            let png_dir = args.png_root.clone().unwrap_or_else(|| {
                args.data_dir.join("dist-gpui/headphones").join(
                    item.private_dir
                        .strip_prefix(args.data_dir.join("dist-private/headphones"))
                        .unwrap_or(&item.private_dir),
                )
            });
            let result = render_pngs(&args.render_images_bin, &item.private_dir, &png_dir)
                .and_then(|()| {
                    let watermark = webp::watermark_text(&item.source);
                    let quality = if args.webp_lossless {
                        None
                    } else {
                        Some(args.webp_quality)
                    };
                    webp::convert_png_tree(
                        &png_dir,
                        &item.public_dir,
                        &item.seed_prefix,
                        &watermark,
                        args.font.as_deref(),
                        quality,
                    )
                });
            match result {
                Ok(count) => count,
                // Python logs a failed image export and carries on.
                Err(error) => {
                    eprintln!(
                        "warning: public image export failed for {}: {error:#}",
                        item.private_dir.display()
                    );
                    0
                }
            }
        })
        .sum();
    written
}

fn run(args: &Args) -> Result<usize> {
    let started = Instant::now();
    let hp_dir = args.data_dir.join("datas/headphones");
    if !hp_dir.is_dir() {
        println!("no headphone measurements directory, skipping");
        return Ok(0);
    }
    let metadata_path = args
        .metadata_json
        .clone()
        .unwrap_or_else(|| args.data_dir.join("dist/json/headphone_metadata.json"));
    let metadata_text = std::fs::read_to_string(&metadata_path)
        .with_context(|| format!("reading headphone metadata {}", metadata_path.display()))?;
    let catalogue: BTreeMap<String, Value> = serde_json::from_str(&metadata_text)
        .with_context(|| format!("parsing {}", metadata_path.display()))?;

    let targets_dir = args
        .targets_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("datas/headphone_targets"));
    let eq_root = args
        .eq_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("datas/headphone_eq"));
    let mut targets: BTreeMap<&str, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for (key, file) in [
        ("harman_overear_2019", "harman_overear_2019.csv"),
        ("harman_inear_2019", "harman_inear_2019.csv"),
    ] {
        let path = targets_dir.join(file);
        if path.is_file() {
            targets.insert(key, load_csv_curve(&path)?);
        }
    }

    let dist_dir = args.data_dir.join("dist/headphones");
    let private_root = args.data_dir.join("dist-private/headphones");

    // Headphones are independent: each writes only its own directories.
    // num_threads(0) lets rayon pick one worker per CPU core.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs)
        .build()
        .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
    let items: Vec<(&String, &Value)> = catalogue.iter().collect();
    let results: Vec<Result<Option<Vec<PendingImage>>>> = pool.install(|| {
        items
            .par_iter()
            .map(|(hp_name, hp_info)| {
                process_headphone(
                    args,
                    &hp_dir,
                    &dist_dir,
                    &private_root,
                    &eq_root,
                    &targets,
                    hp_name,
                    hp_info,
                )
            })
            .collect()
    });
    let mut count = 0usize;
    let mut pending: Vec<PendingImage> = Vec::new();
    for result in results {
        if let Some(mut images) = result? {
            count += 1;
            pending.append(&mut images);
        }
    }

    let images = pool.install(|| emit_public_images(args, &pending));
    println!(
        "generated headphone graphs for {count} headphones ({images} public images) in {:?}",
        started.elapsed()
    );
    Ok(count)
}

/// Generates every graph for one headphone.
///
/// Returns `None` when the headphone is skipped (filtered out, marked
/// skipped, incomplete catalogue entry, no measurement, or fresh).
#[expect(clippy::too_many_arguments, reason = "mirrors the run() context")]
fn process_headphone(
    args: &Args,
    hp_dir: &Path,
    dist_dir: &Path,
    private_root: &Path,
    eq_root: &Path,
    targets: &BTreeMap<&str, (Vec<f64>, Vec<f64>)>,
    hp_name: &str,
    hp_info: &Value,
) -> Result<Option<Vec<PendingImage>>> {
    let mut pending: Vec<PendingImage> = Vec::new();
    {
        if let Some(wanted) = &args.speaker
            && wanted != hp_name
        {
            return Ok(None);
        }
        if hp_info.get("skip").and_then(Value::as_bool) == Some(true) {
            return Ok(None);
        }
        let (Some(brand), Some(model)) = (
            hp_info.get("brand").and_then(Value::as_str),
            hp_info.get("model").and_then(Value::as_str),
        ) else {
            return Ok(None);
        };
        let shape = hp_info
            .get("shape")
            .and_then(Value::as_str)
            .unwrap_or("over-ear");
        let default_m = hp_info
            .get("default_measurement")
            .and_then(Value::as_str)
            .unwrap_or("asr");
        let hp_m_dir = hp_dir.join(hp_name);
        if !hp_m_dir.is_dir() {
            return Ok(None);
        }
        let mut fr_file: Option<PathBuf> = None;
        'search: for origin_dir in [default_m, "asr"] {
            for candidate in FR_CANDIDATES {
                let path = hp_m_dir.join(origin_dir).join(candidate);
                if path.is_file() {
                    fr_file = Some(path);
                    break 'search;
                }
            }
        }
        let Some(fr_file) = fr_file else {
            return Ok(None);
        };
        let origin = hp_info
            .pointer(&format!("/measurements/{default_m}/origin"))
            .and_then(Value::as_str)
            .unwrap_or("ASR");
        let out_dir = dist_dir.join(hp_name).join(origin).join(default_m);
        let private_out = private_root.join(hp_name).join(origin).join(default_m);
        if is_fresh(
            &private_out.join("Frequency Response.json"),
            &fr_file,
            args.force,
        ) {
            return Ok(None);
        }
        let (freq, spl) = load_csv_curve(&fr_file)?;
        if freq.is_empty() {
            eprintln!("warning: empty frequency response for {hp_name}");
            return Ok(None);
        }
        let title_prefix = format!("{brand} {model}");
        let source = source_label(origin, default_m);
        let fig_fr = Figure::scatter(
            "Frequency Response",
            format!("{title_prefix} - Frequency Response"),
            freq.clone(),
            vec![Trace::new("Frequency Response", spl.clone(), Axis::Primary).color("#1f77b4")],
        );
        write_graph(
            &fig_fr,
            &out_dir,
            &private_out,
            hp_name,
            &source,
            &mut pending,
        )?;

        let target_key = target_key_for_shape(shape);
        if let Some((t_freq, t_spl)) = targets.get(target_key) {
            let t_interp = interp_clamp(&freq, t_freq, t_spl);
            let mean_spl = mean_in_band(&freq, &spl, MEAN_MIN_HZ, MEAN_MAX_HZ);
            let mean_target = mean_in_band(&freq, &t_interp, MEAN_MIN_HZ, MEAN_MAX_HZ);
            let spl_norm: Vec<f64> = spl.iter().map(|v| v - mean_spl).collect();
            let target_norm: Vec<f64> = t_interp.iter().map(|v| v - mean_target).collect();
            let fig_comp = Figure::scatter(
                "Frequency Response Compensated",
                format!("{title_prefix} - vs Harman Target"),
                freq.clone(),
                vec![
                    Trace::new("Measurement", spl_norm.clone(), Axis::Primary).color("#1f77b4"),
                    Trace::new(
                        format!("Harman Target ({shape})"),
                        target_norm.clone(),
                        Axis::Primary,
                    )
                    .color("#ff7f0e")
                    .dash("dash"),
                ],
            );
            write_graph(
                &fig_comp,
                &out_dir,
                &private_out,
                hp_name,
                &source,
                &mut pending,
            )?;

            let deviation_raw: Vec<f64> = spl_norm
                .iter()
                .zip(&target_norm)
                .map(|(a, b)| a - b)
                .collect();
            let mean_dev = mean_in_band(&freq, &deviation_raw, MEAN_MIN_HZ, MEAN_MAX_HZ);
            let deviation: Vec<f64> = deviation_raw.iter().map(|v| v - mean_dev).collect();
            let fig_dev = Figure::scatter(
                "Target Deviation",
                format!("{title_prefix} - Target Deviation"),
                freq.clone(),
                vec![
                    Trace::new("Deviation from Target", deviation.clone(), Axis::Primary)
                        .color("#d62728"),
                    Trace::with_x("Zero", vec![20.0, 20_000.0], vec![0.0, 0.0], Axis::Primary)
                        .color("#888888")
                        .dash("dot")
                        .no_legend(),
                ],
            );
            write_graph(
                &fig_dev,
                &out_dir,
                &private_out,
                hp_name,
                &source,
                &mut pending,
            )?;

            // EQ variants (Python: only when a target and the EQ dir exist).
            let eq_dir = eq_root.join(hp_name);
            if eq_dir.is_dir() {
                for (eq_key, file_stem, display) in EQ_VARIANTS {
                    let eq_file = eq_dir.join(format!("{file_stem}.txt"));
                    if !eq_file.is_file() {
                        continue;
                    }
                    let filters: Vec<RewFilter> = parse_rew_eq(&eq_file, EQ_SAMPLE_RATE_HZ)
                        .into_iter()
                        .filter(|f| f.weight != 0.0)
                        .collect();
                    if filters.is_empty() {
                        continue;
                    }
                    let eq_out = PathBuf::from(format!("{}_eq_{eq_key}", out_dir.display()));
                    let eq_private =
                        PathBuf::from(format!("{}_eq_{eq_key}", private_out.display()));
                    if is_fresh(
                        &eq_private.join("Frequency Response.json"),
                        &eq_file,
                        args.force,
                    ) {
                        continue;
                    }
                    let peq: Peq = filters.iter().map(RewFilter::to_biquad).collect();
                    let preamp = preamp_gain(&filters);
                    let response = peq_response_db(&freq, &peq);
                    let spl_eq: Vec<f64> = spl
                        .iter()
                        .zip(&response)
                        .map(|(s, r)| s + r + preamp)
                        .collect();
                    let mean_eq = mean_in_band(&freq, &spl_eq, MEAN_MIN_HZ, MEAN_MAX_HZ);
                    let spl_eq_norm: Vec<f64> = spl_eq.iter().map(|v| v - mean_eq).collect();
                    let seed = format!("{hp_name}/{eq_key}");
                    let fig_eq = Figure::scatter(
                        "Frequency Response",
                        format!("{title_prefix} - Frequency Response ({display})"),
                        freq.clone(),
                        vec![
                            Trace::new("Frequency Response", spl_norm.clone(), Axis::Primary)
                                .color("#1f77b4"),
                            Trace::new(
                                format!("Harman Target ({shape})"),
                                target_norm.clone(),
                                Axis::Primary,
                            )
                            .color("#ff7f0e")
                            .dash("dash"),
                            Trace::new("With EQ", spl_eq_norm.clone(), Axis::Primary)
                                .color("#2ca02c"),
                        ],
                    );
                    write_graph(&fig_eq, &eq_out, &eq_private, &seed, &source, &mut pending)?;
                    let dev_eq_raw: Vec<f64> = spl_eq_norm
                        .iter()
                        .zip(&target_norm)
                        .map(|(a, b)| a - b)
                        .collect();
                    let mean_dev_eq = mean_in_band(&freq, &dev_eq_raw, MEAN_MIN_HZ, MEAN_MAX_HZ);
                    let dev_eq: Vec<f64> = dev_eq_raw.iter().map(|v| v - mean_dev_eq).collect();
                    let fig_dev_eq = Figure::scatter(
                        "Target Deviation",
                        format!("{title_prefix} - Target Deviation ({display})"),
                        freq.clone(),
                        vec![
                            Trace::new("Deviation", deviation.clone(), Axis::Primary)
                                .color("#d62728"),
                            Trace::new("With EQ", dev_eq, Axis::Primary).color("#2ca02c"),
                            Trace::with_x(
                                "Zero",
                                vec![20.0, 20_000.0],
                                vec![0.0, 0.0],
                                Axis::Primary,
                            )
                            .color("#888888")
                            .dash("dot")
                            .no_legend(),
                        ],
                    );
                    write_graph(
                        &fig_dev_eq,
                        &eq_out,
                        &eq_private,
                        &seed,
                        &source,
                        &mut pending,
                    )?;
                    println!("generated EQ graphs for {hp_name} {eq_key}");
                }
            }
        }
        println!("generated graphs for {hp_name}");
    }
    Ok(Some(pending))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
