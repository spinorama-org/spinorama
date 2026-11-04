//! `generate-graph`: Klippel version directory to spinorama graph payloads.
//!
//! Thin argument parser over the `spinorama_generate` library. Computes
//! CEA2034 curves in Rust, writes Plotly JSON + v2 trace bundles under
//! `<data-dir>/dist-private/speakers/<speaker>/<origin>/<version>/`, and
//! optionally drives the gpui-toolkit `spinorama-render-images` binary
//! to rasterize PNGs. Never spawns Python.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use spinorama_generate::compute::cea2034::{Cea2034, compute_spin_data};
use spinorama_generate::figures::{
    Axis, Figure, Trace, apply_scan_style, common_figures, spin_figures,
};
use spinorama_generate::filters::apo::parse_apo;
use spinorama_generate::filters::biquad::apply_peq;
use spinorama_generate::loaders::curves::parse_headphone_csv;
use spinorama_generate::loaders::{LoadedMeasurement, load_measurement};
use spinorama_generate::measurements::{angle_degrees, symmetrise_hv};
use spinorama_generate::meta::{
    measured_by_label, origin_dir_name, sanitize_filename, source_label,
};
use spinorama_generate::render::webp;
use spinorama_generate::render::{DEFAULT_RENDER_IMAGES_BIN, render_pngs, write_figure};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Clone)]
struct Args {
    speaker_dir: PathBuf,
    metadata_json: Option<PathBuf>,
    metadata_dir: Option<PathBuf>,
    check_meta: bool,
    measurements_root: PathBuf,
    limit: Option<usize>,
    format: String,
    speaker: String,
    origin: String,
    version: String,
    data_dir: PathBuf,
    measured_by: Option<String>,
    symmetry: Option<String>,
    shape: Option<String>,
    min_valid_freq: Option<f64>,
    max_valid_freq: Option<f64>,
    targets_dir: PathBuf,
    eq_dir: PathBuf,
    with_eq: bool,
    render_png: bool,
    render_webp: bool,
    scan_hard: bool,
    webp_quality: f32,
    webp_lossless: bool,
    webp_dir: Option<PathBuf>,
    font: Option<PathBuf>,
    render_images_bin: PathBuf,
    png_dir: Option<PathBuf>,
    force: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        speaker_dir: PathBuf::new(),
        metadata_json: None,
        metadata_dir: None,
        check_meta: false,
        measurements_root: PathBuf::from("datas/measurements"),
        limit: None,
        format: "klippel".to_owned(),
        speaker: String::new(),
        origin: String::new(),
        version: String::new(),
        data_dir: PathBuf::from("."),
        measured_by: None,
        symmetry: None,
        shape: None,
        min_valid_freq: None,
        max_valid_freq: None,
        targets_dir: PathBuf::from("datas/measurements/_support/headphone_targets"),
        eq_dir: PathBuf::from("datas/eq"),
        with_eq: true,
        render_png: false,
        render_webp: false,
        scan_hard: false,
        webp_quality: 82.0,
        webp_lossless: false,
        webp_dir: None,
        font: None,
        render_images_bin: std::env::var_os("SPINORAMA_RENDER_IMAGES")
            .map_or_else(|| PathBuf::from(DEFAULT_RENDER_IMAGES_BIN), PathBuf::from),
        png_dir: None,
        force: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--speaker-dir" => args.speaker_dir = PathBuf::from(value(&mut it)?),
            "--metadata-json" => args.metadata_json = Some(PathBuf::from(value(&mut it)?)),
            "--metadata-dir" => args.metadata_dir = Some(PathBuf::from(value(&mut it)?)),
            "--check-meta" => args.check_meta = true,
            "--measurements-root" => {
                args.measurements_root = PathBuf::from(value(&mut it)?);
            }
            "--limit" => {
                args.limit = Some(value(&mut it)?.parse().context("--limit needs a count")?);
            }
            "--format" => args.format = value(&mut it)?,
            "--speaker" => args.speaker = value(&mut it)?,
            "--origin" => args.origin = value(&mut it)?,
            "--mversion" => args.version = value(&mut it)?,
            "--data-dir" => args.data_dir = PathBuf::from(value(&mut it)?),
            "--measured-by" => args.measured_by = Some(value(&mut it)?),
            "--symmetry" => args.symmetry = Some(value(&mut it)?),
            "--shape" => args.shape = Some(value(&mut it)?),
            "--min-valid-freq" => {
                args.min_valid_freq = Some(
                    value(&mut it)?
                        .parse()
                        .context("--min-valid-freq needs a number")?,
                );
            }
            "--max-valid-freq" => {
                args.max_valid_freq = Some(
                    value(&mut it)?
                        .parse()
                        .context("--max-valid-freq needs a number")?,
                );
            }
            "--targets-dir" => args.targets_dir = PathBuf::from(value(&mut it)?),
            "--eq-dir" => args.eq_dir = PathBuf::from(value(&mut it)?),
            "--no-eq" => args.with_eq = false,
            "--render-png" => args.render_png = true,
            "--render-webp" => args.render_webp = true,
            "--scan-hard" => args.scan_hard = true,
            "--webp-quality" => {
                args.webp_quality = value(&mut it)?
                    .parse()
                    .context("--webp-quality needs a number in 0..=100")?;
            }
            "--webp-lossless" => args.webp_lossless = true,
            "--webp-dir" => args.webp_dir = Some(PathBuf::from(value(&mut it)?)),
            "--font" => args.font = Some(PathBuf::from(value(&mut it)?)),
            "--render-images-bin" => args.render_images_bin = PathBuf::from(value(&mut it)?),
            "--png-dir" => args.png_dir = Some(PathBuf::from(value(&mut it)?)),
            "--force" => args.force = true,
            "--help" | "-h" => {
                println!(
                    "usage: generate-graph --speaker-dir DIR --speaker NAME --origin ORIGIN \
                     --mversion VER [--format klippel|spl_hv_txt|gll_hv_txt|princeton|rew_text_dump|webplotdigitizer|csv_freq_spl] \
                     [--symmetry coaxial|horizontal|vertical|none] [--shape NAME] \
                     [--min-valid-freq HZ] [--max-valid-freq HZ] [--data-dir DIR] [--force] \
                     [--render-png [--png-dir DIR] [--render-images-bin BIN]]\n\
                     or:    generate-graph (--metadata-dir DIR | --metadata-json FILE) [--measurements-root DIR] \
                     [--speaker NAME] [--mversion VER] [--limit N] [--data-dir DIR] [--force] \
                     [--render-png [--png-dir DIR] [--render-images-bin BIN]]\n\
                     or:    generate-graph --metadata-dir DIR --check-meta"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    if args.metadata_json.is_none() && args.metadata_dir.is_none() {
        if args.speaker_dir.as_os_str().is_empty() || args.speaker.is_empty() {
            bail!("--speaker-dir and --speaker are required (try --help)");
        }
        if args.origin.is_empty() || args.version.is_empty() {
            bail!("--origin and --mversion are required (try --help)");
        }
    }
    Ok(args)
}

fn valid_freq_range(args: &Args, freq_bounds: Option<(f64, f64)>) -> Result<(f64, f64)> {
    let mut min_freq = args.min_valid_freq.unwrap_or(20.0);
    let mut max_freq = args.max_valid_freq.unwrap_or(20_000.0);
    if let Some((data_min, data_max)) = freq_bounds {
        min_freq = min_freq.max(data_min);
        max_freq = max_freq.min(data_max);
    }
    min_freq = min_freq.max(20.0);
    max_freq = max_freq.min(20_000.0);
    if min_freq > max_freq {
        bail!("empty valid frequency range {min_freq}-{max_freq} Hz");
    }
    Ok((min_freq, max_freq))
}

fn sibling_eq_dir(dir: &Path) -> PathBuf {
    match (dir.parent(), dir.file_name()) {
        (Some(parent), Some(name)) => parent.join(format!("{}_eq", name.to_string_lossy())),
        _ => dir.join("eq"),
    }
}

fn write_styled(args: &Args, dir: &Path, figure: &Figure) -> Result<PathBuf> {
    if args.scan_hard {
        let mut styled = figure.clone();
        apply_scan_style(&mut styled);
        write_figure(dir, &styled)
    } else {
        write_figure(dir, figure)
    }
}

fn maybe_webp(
    args: &Args,
    png_dir: &Path,
    webp_dir_override: Option<&Path>,
    seed_prefix: &str,
) -> Result<()> {
    if !args.render_webp {
        return Ok(());
    }
    let webp_dir = webp_dir_override
        .map(Path::to_path_buf)
        .or_else(|| args.webp_dir.clone())
        .unwrap_or_else(|| args.data_dir.join("dist/speakers"));
    let watermark = webp::watermark_text(&source_label(&args.origin, &args.version));
    let quality = if args.webp_lossless {
        None
    } else {
        Some(args.webp_quality)
    };
    let written = webp::convert_png_tree(
        png_dir,
        &webp_dir,
        seed_prefix,
        &watermark,
        args.font.as_deref(),
        quality,
    )?;
    println!("wrote {written} hardened WebP under {}", webp_dir.display());
    Ok(())
}

fn corrected_cea(cea: &Cea2034, peq: &spinorama_generate::filters::biquad::Peq) -> Cea2034 {
    let on_axis = apply_peq(&cea.freq, &cea.on_axis, peq);
    let listening_window = apply_peq(&cea.freq, &cea.listening_window, peq);
    let early_reflections = apply_peq(&cea.freq, &cea.early_reflections, peq);
    let sound_power = apply_peq(&cea.freq, &cea.sound_power, peq);
    Cea2034 {
        freq: cea.freq.clone(),
        early_reflections_di: listening_window
            .iter()
            .zip(&early_reflections)
            .map(|(lw, er)| lw - er)
            .collect(),
        sound_power_di: listening_window
            .iter()
            .zip(&sound_power)
            .map(|(lw, sp)| lw - sp)
            .collect(),
        estimated_inroom: apply_peq(&cea.freq, &cea.estimated_inroom, peq),
        on_axis,
        listening_window,
        early_reflections,
        sound_power,
        sensitivity_db: cea.sensitivity_db,
    }
}

fn interp_clamp(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
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
            let t = (target - x0) / (x1 - x0);
            src_values[upper - 1] + t * (src_values[upper] - src_values[upper - 1])
        })
        .collect()
}

fn mean_in_band_inclusive(freq: &[f64], spl: &[f64], min_hz: f64, max_hz: f64) -> f64 {
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

fn run_headphone(args: &Args, freq: &[f64], spl: &[f64], started: Instant) -> Result<()> {
    let shape = args.shape.as_deref().unwrap_or("over-ear");
    let mut figures = vec![Figure::scatter(
        "Frequency Response",
        format!("{} - Frequency Response", args.speaker),
        freq.to_vec(),
        vec![Trace::new("Frequency Response", spl.to_vec(), Axis::Primary).color("#1f77b4")],
    )];

    let target_file = if matches!(shape, "in-ear" | "earbud") {
        "harman_inear_2019.csv"
    } else {
        "harman_overear_2019.csv"
    };
    let target_path = args.targets_dir.join(target_file);
    if target_path.is_file() {
        let text = std::fs::read_to_string(&target_path)
            .with_context(|| format!("reading {}", target_path.display()))?;
        let (t_freq, t_spl) = parse_headphone_csv(&text)?;
        let t_interp = interp_clamp(freq, &t_freq, &t_spl);
        let mean_spl = mean_in_band_inclusive(freq, spl, 300.0, 3000.0);
        let mean_target = mean_in_band_inclusive(freq, &t_interp, 300.0, 3000.0);
        let spl_norm: Vec<f64> = spl.iter().map(|v| v - mean_spl).collect();
        let target_norm: Vec<f64> = t_interp.iter().map(|v| v - mean_target).collect();
        figures.push(Figure::scatter(
            "Frequency Response Compensated",
            format!("{} - vs Harman Target", args.speaker),
            freq.to_vec(),
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
        ));
        let mut deviation: Vec<f64> = spl_norm
            .iter()
            .zip(&target_norm)
            .map(|(a, b)| a - b)
            .collect();
        let mean_deviation = mean_in_band_inclusive(freq, &deviation, 300.0, 3000.0);
        for value in &mut deviation {
            *value -= mean_deviation;
        }
        figures.push(Figure::scatter(
            "Target Deviation",
            format!("{} - Target Deviation", args.speaker),
            freq.to_vec(),
            vec![
                Trace::new("Deviation from Target", deviation, Axis::Primary).color("#d62728"),
                Trace::with_x("Zero", vec![20.0, 20_000.0], vec![0.0, 0.0], Axis::Primary)
                    .color("#888888")
                    .dash("dot")
                    .no_legend(),
            ],
        ));
    }

    let out_dir = args
        .data_dir
        .join("dist-private/headphones")
        .join(sanitize_filename(&args.speaker))
        .join(origin_dir_name(&args.origin))
        .join(&args.version);
    let mut wrote_any = false;
    for figure in &figures {
        let target = out_dir.join(format!("{}.json", figure.name));
        if !args.force && target.is_file() {
            println!("skip (exists, use --force): {}", target.display());
            continue;
        }
        let written = write_styled(args, &out_dir, figure)?;
        wrote_any = true;
        println!("wrote {}", written.display());
    }
    println!(
        "headphone: {} figures in {:?}",
        figures.len(),
        started.elapsed()
    );
    if args.render_png {
        let png_dir = args
            .png_dir
            .clone()
            .unwrap_or_else(|| args.data_dir.join("dist-gpui/headphones"));
        render_if_stale(
            args,
            &out_dir,
            &png_dir,
            None,
            &format!("{}/{}/{}", args.speaker, args.origin, args.version),
            wrote_any,
        )?;
    }
    Ok(())
}

fn dir_has_files(dir: &Path, extension: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| {
        entries.any(|entry| {
            entry.is_ok_and(|e| e.path().extension().is_some_and(|ext| ext == extension))
        })
    })
}

/// Renders PNGs and converts WebP only when something is stale.
///
/// A rerun whose figure JSONs were all skipped and whose PNG/WebP
/// outputs already exist does no rendering work, which keeps the
/// batch mode of `update_website.sh` incremental.
fn render_if_stale(
    args: &Args,
    json_dir: &Path,
    png_dir: &Path,
    webp_dir_override: Option<&Path>,
    seed_prefix: &str,
    wrote_any: bool,
) -> Result<()> {
    let webp_dir = webp_dir_override
        .map(Path::to_path_buf)
        .or_else(|| args.webp_dir.clone())
        .unwrap_or_else(|| args.data_dir.join("dist/speakers"));
    let pngs_exist = dir_has_files(png_dir, "png");
    let webps_exist = !args.render_webp || dir_has_files(&webp_dir, "webp");
    if args.force || wrote_any || !pngs_exist {
        render_pngs(&args.render_images_bin, json_dir, png_dir)?;
        println!("rendered PNGs under {}", png_dir.display());
    }
    if args.force || wrote_any || !webps_exist {
        maybe_webp(args, png_dir, webp_dir_override, seed_prefix)?;
    }
    Ok(())
}

/// Contour stems that `spin_figures` may skip for degenerate planes.
///
/// Stale JSON/PNG/WebP outputs for skipped stems are removed so the
/// renderer never sees a contour it cannot draw.
const CONTOUR_STEMS: [&str; 8] = [
    "SPL Horizontal Contour",
    "SPL Vertical Contour",
    "SPL Horizontal Contour Normalized",
    "SPL Vertical Contour Normalized",
    "SPL Horizontal Contour 3D",
    "SPL Vertical Contour 3D",
    "SPL Horizontal Contour Normalized 3D",
    "SPL Vertical Contour Normalized 3D",
];

fn remove_stale_contours(figures: &[Figure], dirs: &[&Path]) {
    let produced: std::collections::HashSet<&str> =
        figures.iter().map(|figure| figure.name.as_str()).collect();
    for stem in CONTOUR_STEMS {
        if produced.contains(stem) {
            continue;
        }
        for dir in dirs {
            for ext in ["json", "png", "webp"] {
                let stale = dir.join(format!("{stem}.{ext}"));
                if stale.is_file() {
                    let _ = std::fs::remove_file(&stale);
                }
            }
        }
    }
}

fn run_one(args: &Args) -> Result<()> {
    let started = Instant::now();
    let measured_by = args
        .measured_by
        .clone()
        .unwrap_or_else(|| measured_by_label(&args.origin, &args.version));
    let loaded = load_measurement(&args.format, &args.speaker_dir, &args.speaker)?;
    if let LoadedMeasurement::Curve { freq, spl } = &loaded {
        return run_headphone(args, freq, spl, started);
    }
    let (figures, sensitivity_db, valid_range, eq_hv, eq_cea) = match loaded {
        LoadedMeasurement::Hv(hv) => {
            let mut hv = symmetrise_hv(&hv, args.symmetry.as_deref())?;
            if args.shape.as_deref() == Some("inwall") {
                for plane in [&mut hv.horizontal, &mut hv.vertical] {
                    for (name, values) in &mut plane.columns {
                        if angle_degrees(name).is_some_and(|angle| angle.abs() > 90) {
                            values.fill(0.0);
                        }
                    }
                }
            }
            let bounds = (
                hv.horizontal
                    .freq
                    .first()
                    .copied()
                    .unwrap_or(20.0)
                    .max(hv.vertical.freq.first().copied().unwrap_or(20.0)),
                hv.horizontal
                    .freq
                    .last()
                    .copied()
                    .unwrap_or(20_000.0)
                    .min(hv.vertical.freq.last().copied().unwrap_or(20_000.0)),
            );
            let valid_range = valid_freq_range(args, Some(bounds))?;
            let eq_source = hv.clone();
            let data = compute_spin_data(&hv)?;
            let sensitivity_db = data.cea.sensitivity_db;
            (
                spin_figures(&data, &args.speaker, &measured_by, valid_range),
                sensitivity_db,
                valid_range,
                Some(eq_source),
                None,
            )
        }
        LoadedMeasurement::Cea(cea) => {
            let sensitivity_db = cea.sensitivity_db;
            let valid_range = valid_freq_range(args, None)?;
            let figures = common_figures(&cea, None, &args.speaker, &measured_by, valid_range);
            (figures, sensitivity_db, valid_range, None, Some(cea))
        }
        LoadedMeasurement::Curve { .. } => {
            return Err(anyhow::anyhow!("headphone curve handled above"));
        }
    };

    let out_dir = args
        .data_dir
        .join("dist-private/speakers")
        .join(sanitize_filename(&args.speaker))
        .join(origin_dir_name(&args.origin))
        .join(&args.version);
    let mut wrote_any = false;
    for figure in &figures {
        let target = out_dir.join(format!("{}.json", figure.name));
        if !args.force && target.is_file() {
            println!("skip (exists, use --force): {}", target.display());
            continue;
        }
        let written = write_styled(args, &out_dir, figure)?;
        wrote_any = true;
        println!("wrote {}", written.display());
    }
    remove_stale_contours(&figures, &[&out_dir]);
    println!(
        "sensitivity {:.2} dB (300-3000 Hz mean, pre-shift); {} figures in {:?}",
        sensitivity_db,
        figures.len(),
        started.elapsed()
    );

    // EQ appendix: same figure set recomputed with datas/eq/<speaker>/iir.txt
    // applied to the raw planes/curves (Python parse_eq_speaker). The PEQ
    // response is applied on the measurement grid only; phase is untouched
    // and the APO preamp is not added to the curves, matching Python.
    let eq_file = args
        .eq_dir
        .join(sanitize_filename(&args.speaker))
        .join("iir.txt");
    if args.with_eq && eq_file.is_file() {
        let text = std::fs::read_to_string(&eq_file)
            .with_context(|| format!("reading {}", eq_file.display()))?;
        let (_preamp, peq) = parse_apo(&text, 48_000.0)?;
        if !peq.is_empty() {
            let eq_figures = if let Some(mut hv) = eq_hv {
                for plane in [&mut hv.horizontal, &mut hv.vertical] {
                    let response =
                        spinorama_generate::filters::biquad::peq_response_db(&plane.freq, &peq);
                    for (_, values) in &mut plane.columns {
                        for (value, delta) in values.iter_mut().zip(&response) {
                            *value += delta;
                        }
                    }
                }
                let data = compute_spin_data(&hv)?;
                spin_figures(&data, &args.speaker, &measured_by, valid_range)
            } else if let Some(cea) = &eq_cea {
                common_figures(
                    &corrected_cea(cea, &peq),
                    None,
                    &args.speaker,
                    &measured_by,
                    valid_range,
                )
            } else {
                Vec::new()
            };
            // EQ filter figures (AutoEQ parity: filters_eq, EA overlays,
            // histograms and the combined sheet). CEA 2034 only; HV-only
            // speakers skip them like Python does. The frontend loads
            // filters_*.webp from the origin directory (next to the
            // speaker page), not from the <version>_eq appendix, so the
            // JSON/PNG/WebP for this family go to the origin dirs.
            let mut filter_figures = Vec::new();
            if let Some(cea) = &eq_cea {
                let cea_eq = corrected_cea(cea, &peq);
                filter_figures = spinorama_generate::figures::eq_filter_figures(
                    cea,
                    &cea_eq,
                    &peq,
                    &args.speaker,
                );
            }
            if !filter_figures.is_empty() {
                let origin_private = out_dir.parent().unwrap_or(&out_dir).to_path_buf();
                let mut wrote_filters = false;
                for figure in &filter_figures {
                    let target = origin_private.join(format!("{}.json", figure.name));
                    if !args.force && target.is_file() {
                        continue;
                    }
                    write_styled(args, &origin_private, figure)?;
                    wrote_filters = true;
                }
                println!(
                    "{} EQ filter figures from {}",
                    filter_figures.len(),
                    eq_file.display()
                );
                if args.render_png {
                    // Per-measurement PNG/WebP dirs end in <version>; the
                    // filters family lives one level up, in the origin dir.
                    let png_origin = match &args.png_dir {
                        Some(dir) => dir.parent().map_or_else(|| dir.clone(), Path::to_path_buf),
                        None => args.data_dir.join("dist-gpui/speakers"),
                    };
                    let webp_origin = args
                        .webp_dir
                        .as_ref()
                        .map(|dir| dir.parent().map_or_else(|| dir.clone(), Path::to_path_buf));
                    render_if_stale(
                        args,
                        &origin_private,
                        &png_origin,
                        webp_origin.as_deref(),
                        &format!("{}/{}/filters", args.speaker, args.origin),
                        wrote_filters,
                    )?;
                }
            }
            if !eq_figures.is_empty() {
                let eq_dir = out_dir
                    .parent()
                    .map(|parent| parent.join(format!("{}_eq", args.version)))
                    .unwrap_or_else(|| out_dir.clone());
                let mut wrote_eq = false;
                for figure in &eq_figures {
                    let target = eq_dir.join(format!("{}.json", figure.name));
                    if !args.force && target.is_file() {
                        println!("skip (exists, use --force): {}", target.display());
                        continue;
                    }
                    let written = write_styled(args, &eq_dir, figure)?;
                    wrote_eq = true;
                    println!("wrote {}", written.display());
                }
                println!("{} EQ figures from {}", eq_figures.len(), eq_file.display());
                if args.render_png {
                    let png_dir = sibling_eq_dir(
                        &args
                            .png_dir
                            .clone()
                            .unwrap_or_else(|| args.data_dir.join("dist-gpui/speakers")),
                    );
                    let webp_eq = args.webp_dir.clone().map(|dir| sibling_eq_dir(&dir));
                    render_if_stale(
                        args,
                        &eq_dir,
                        &png_dir,
                        webp_eq.as_deref(),
                        &format!("{}/{}/{}_eq", args.speaker, args.origin, args.version),
                        wrote_eq,
                    )?;
                }
            }
        }
    }

    if args.render_png {
        let png_dir = args
            .png_dir
            .clone()
            .unwrap_or_else(|| args.data_dir.join("dist-gpui/speakers"));
        let webp_dir = args
            .webp_dir
            .clone()
            .unwrap_or_else(|| args.data_dir.join("dist/speakers"));
        remove_stale_contours(&figures, &[&png_dir, &webp_dir]);
        render_if_stale(
            args,
            &out_dir,
            &png_dir,
            None,
            &format!("{}/{}/{}", args.speaker, args.origin, args.version),
            wrote_any,
        )?;
    }
    Ok(())
}

fn catalogue_entries(args: &Args) -> Result<Vec<spinorama_generate::metadata::CatalogueEntry>> {
    use spinorama_generate::metadata::{CatalogueEntry, catalogue_from_dir};
    if let Some(dir) = &args.metadata_dir {
        return catalogue_from_dir(dir);
    }
    let path = args
        .metadata_json
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--metadata-json or --metadata-dir is required"))?;
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let root: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let speakers = root
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{}: top level is not an object", path.display()))?;
    let mut speaker_names: Vec<&String> = speakers.keys().collect();
    speaker_names.sort();
    let mut entries = Vec::new();
    for speaker_name in speaker_names {
        let speaker = &speakers[speaker_name];
        if speaker["skip"].as_bool() == Some(true) {
            continue;
        }
        let Some(measurements) = speaker["measurements"].as_object() else {
            continue;
        };
        let default = speaker["default_measurement"].as_str().unwrap_or_default();
        let mut versions: Vec<&String> = measurements.keys().collect();
        versions.sort();
        for version in versions {
            let measurement = &measurements[version];
            entries.push(CatalogueEntry {
                speaker: speaker_name.clone(),
                version: version.clone(),
                origin: measurement["origin"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                format: measurement["format"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                symmetry: measurement["symmetry"].as_str().map(str::to_owned),
                speaker_dir: args.measurements_root.join(speaker_name).join(version),
                is_default: version == default,
                shape: speaker["shape"].as_str().map(str::to_owned),
                min_valid_freq: measurement["data_acquisition"]["min_valid_freq"].as_f64(),
                max_valid_freq: measurement["data_acquisition"]["max_valid_freq"].as_f64(),
            });
        }
    }
    Ok(entries)
}

fn run_batch(args: &Args) -> Result<()> {
    let entries = catalogue_entries(args)?;
    let (mut done, mut failed) = (0usize, 0usize);
    for entry in &entries {
        if !args.speaker.is_empty() && args.speaker != entry.speaker {
            continue;
        }
        if args.limit.is_some_and(|limit| done + failed >= limit) {
            println!("batch: {done} generated, {failed} failed (limit reached)");
            report_batch_failures(failed, done);
            return Ok(());
        }
        if !args.version.is_empty() && args.version != entry.version {
            continue;
        }
        let speaker_name = &entry.speaker;
        let version = &entry.version;
        {
            let mut one = args.clone();
            one.metadata_json = None;
            one.metadata_dir = None;
            one.speaker.clone_from(speaker_name);
            one.version.clone_from(version);
            one.speaker_dir = entry.speaker_dir.clone();
            one.origin.clone_from(&entry.origin);
            one.format.clone_from(&entry.format);
            one.symmetry.clone_from(&entry.symmetry);
            one.shape.clone_from(&entry.shape);
            one.min_valid_freq = entry.min_valid_freq;
            one.max_valid_freq = entry.max_valid_freq;
            one.measured_by = None;
            // Renderer output is flat stems; give each measurement its own
            // PNG/WebP directory in site layout so batch runs cannot
            // overwrite another speaker's pictures.
            if args.render_png && one.png_dir.is_none() {
                one.png_dir = Some(
                    args.data_dir
                        .join("dist-gpui/speakers")
                        .join(sanitize_filename(speaker_name))
                        .join(origin_dir_name(&one.origin))
                        .join(version),
                );
            }
            if args.render_webp && one.webp_dir.is_none() {
                one.webp_dir = Some(
                    args.data_dir
                        .join("dist/speakers")
                        .join(sanitize_filename(speaker_name))
                        .join(origin_dir_name(&one.origin))
                        .join(version),
                );
            }
            match run_one(&one) {
                Ok(()) => done += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("batch: FAILED {speaker_name} {version}: {error:#}");
                }
            }
        }
    }
    println!("batch: {done} generated, {failed} failed");
    report_batch_failures(failed, done);
    Ok(())
}

/// Reports batch failures without aborting the pipeline.
///
/// Matches the Python `generate_graphs.py` contract: per-measurement
/// failures (incomplete source data) are counted and logged, the run
/// still succeeds; only a total failure is an error.
fn report_batch_failures(failed: usize, done: usize) {
    if failed > 0 && done == 0 {
        eprintln!("generate-graph: all {failed} measurement(s) failed");
        std::process::exit(1);
    }
    if failed > 0 {
        eprintln!(
            "generate-graph: warning: {failed} measurement(s) failed (see FAILED lines above)"
        );
    }
}

fn run_check_meta(args: &Args) -> Result<()> {
    let dir = args
        .metadata_dir
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--check-meta needs --metadata-dir"))?;
    let report = spinorama_generate::metadata::check_metadata(dir)?;
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    for error in &report.errors {
        eprintln!("error: {error}");
    }
    println!(
        "meta-check: {} speakers, {} errors, {} warnings",
        report.speakers,
        report.errors.len(),
        report.warnings.len()
    );
    if report.errors.is_empty() {
        Ok(())
    } else {
        bail!("{} metadata error(s)", report.errors.len())
    }
}

fn run(args: &Args) -> Result<()> {
    if args.check_meta {
        run_check_meta(args)
    } else if args.metadata_json.is_some() || args.metadata_dir.is_some() {
        run_batch(args)
    } else {
        run_one(args)
    }
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-graph: {error:#}");
            ExitCode::FAILURE
        }
    }
}
