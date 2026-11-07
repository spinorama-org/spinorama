//! `generate-peq`: one-speaker PEQ computation on Rust AutoEQ.
//!
//! Thin argument parser over the `spinorama_generate` library. Inputs
//! are a Klippel version directory (`--speaker-dir`), a CEA2034 Plotly
//! JSON payload from `generate-graph` (`--cea2034-json`), or a single
//! frequency/SPL CSV (`--curve`). Optimization runs in Rust AutoEQ
//! (`perform_optimization`); outputs are the site APO file
//! (`iir-autoeq.txt`), a `conf-autoeq.json` (create-if-absent), and an
//! optional `filters_<curve>` figure (Plotly JSON + trace bundle).
//! With `--generate-images-only` no optimization runs: the figure is
//! rebuilt from the existing APO file. Never spawns Python.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use autoeq_workflow::LossType;
use autoeq_workflow::iir::Peq;
use mimalloc::MiMalloc;
use rayon::prelude::*;
use spinorama_generate::compute::cea2034::{Cea2034, compute_cea2034, estimated_inroom_from};
use spinorama_generate::compute::scores::{preference_score, preference_score_with_peq};
use spinorama_generate::figures::{
    Axis, BarTrace, Figure, FigureKind, Trace, cea_figure, regression_traces,
};
use spinorama_generate::filters::apo::{format_site_apo, parse_apo};
use spinorama_generate::filters::biquad::{apply_peq, peq_response_db};
use spinorama_generate::loaders::{LoadedMeasurement, load_measurement};
use spinorama_generate::measurements::symmetrise_hv;
use spinorama_generate::meta::{origin_dir_name, sanitize_filename};
use spinorama_generate::peq::grapheq::{graphic_eq, graphic_eq_names, optimize_grapheq};
use spinorama_generate::peq::optimize::{
    Optimisation, PeqOptions, fitness_from_name, loss_from_fitness, optimize_peq, rms_deviation,
};
use spinorama_generate::peq::target::{
    default_slope, interp_log, minus_3db_freq, relative_slope_line, slope_target,
};
use spinorama_generate::render::write_figure;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Curve names of a full CEA2034 set, in site order.
const CEA_CURVES: [&str; 5] = [
    "On Axis",
    "Listening Window",
    "Early Reflections",
    "Sound Power",
    "Estimated In-Room Response",
];

#[derive(Clone)]
struct Args {
    speaker_dir: Option<PathBuf>,
    metadata_json: Option<PathBuf>,
    metadata_dir: Option<PathBuf>,
    measurements_root: PathBuf,
    limit: Option<usize>,
    jobs: usize,
    symmetry: Option<String>,
    format: String,
    cea2034_json: Option<PathBuf>,
    curve_csv: Option<PathBuf>,
    target_csv: Option<PathBuf>,
    curve_name: String,
    fitness: Option<String>,
    optimisation: Option<String>,
    graphic_eq: Option<String>,
    generate_images_only: bool,
    slope: Option<f64>,
    target_min_freq: Option<f64>,
    target_max_freq: f64,
    output_dir: PathBuf,
    speaker: String,
    origin: String,
    version: String,
    data_dir: Option<PathBuf>,
    force: bool,
    options: PeqOptions,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        speaker_dir: None,
        metadata_json: None,
        metadata_dir: None,
        measurements_root: PathBuf::from("datas/measurements"),
        limit: None,
        jobs: 0,
        symmetry: None,
        format: "klippel".to_owned(),
        cea2034_json: None,
        curve_csv: None,
        target_csv: None,
        curve_name: "Estimated In-Room Response".to_owned(),
        fitness: None,
        optimisation: None,
        graphic_eq: None,
        generate_images_only: false,
        slope: None,
        target_min_freq: None,
        target_max_freq: 16_000.0,
        output_dir: PathBuf::from("."),
        speaker: String::new(),
        origin: String::new(),
        version: String::new(),
        data_dir: None,
        force: false,
        options: PeqOptions::default(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        let number = |it: &mut dyn Iterator<Item = String>| -> Result<f64> {
            value(it)?
                .parse()
                .with_context(|| format!("{flag} needs a number"))
        };
        match flag.as_str() {
            "--speaker-dir" => args.speaker_dir = Some(PathBuf::from(value(&mut it)?)),
            "--metadata-json" => args.metadata_json = Some(PathBuf::from(value(&mut it)?)),
            "--metadata-dir" => args.metadata_dir = Some(PathBuf::from(value(&mut it)?)),
            "--measurements-root" => {
                args.measurements_root = PathBuf::from(value(&mut it)?);
            }
            "--limit" => {
                args.limit = Some(value(&mut it)?.parse().context("--limit needs a count")?);
            }
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--symmetry" => args.symmetry = Some(value(&mut it)?),
            "--format" => args.format = value(&mut it)?,
            "--cea2034-json" => args.cea2034_json = Some(PathBuf::from(value(&mut it)?)),
            "--curve" => args.curve_csv = Some(PathBuf::from(value(&mut it)?)),
            "--target" => args.target_csv = Some(PathBuf::from(value(&mut it)?)),
            "--curve-name" | "--curves" => args.curve_name = value(&mut it)?,
            "--fitness" => args.fitness = Some(value(&mut it)?),
            "--optimisation" => args.optimisation = Some(value(&mut it)?),
            "--graphic-eq" => args.graphic_eq = Some(value(&mut it)?),
            "--graphic-eq-list" => {
                println!(
                    "INFO: The list of known graphical EQ is: {:?}",
                    graphic_eq_names()
                );
                std::process::exit(0);
            }
            "--peq-model" => {
                use std::str::FromStr;
                args.options.peq_model = autoeq_workflow::PeqModel::from_str(&value(&mut it)?)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            "--algo" => args.options.algo = value(&mut it)?,
            "--generate-images-only" => args.generate_images_only = true,
            "--slope" => args.slope = Some(number(&mut it)?),
            "--target-min-freq" => args.target_min_freq = Some(number(&mut it)?),
            "--target-max-freq" => args.target_max_freq = number(&mut it)?,
            "--output-dir" => args.output_dir = PathBuf::from(value(&mut it)?),
            "--speaker" => args.speaker = value(&mut it)?,
            "--origin" => args.origin = value(&mut it)?,
            "--mversion" => args.version = value(&mut it)?,
            "--data-dir" => args.data_dir = Some(PathBuf::from(value(&mut it)?)),
            "--force" => args.force = true,
            "--max-peq" => {
                args.options.num_filters =
                    value(&mut it)?.parse().context("--max-peq needs a count")?;
            }
            "--min-q" => args.options.min_q = number(&mut it)?,
            "--max-q" => args.options.max_q = number(&mut it)?,
            "--min-db" => args.options.min_db = number(&mut it)?,
            "--max-db" => args.options.max_db = number(&mut it)?,
            "--min-freq" => args.options.min_freq = number(&mut it)?,
            "--max-freq" => args.options.max_freq = number(&mut it)?,
            "--max-iter" => {
                args.options.maxeval = value(&mut it)?
                    .parse()
                    .context("--max-iter needs a count")?;
            }
            "--seed" => {
                args.options.seed = Some(value(&mut it)?.parse().context("--seed needs a u64")?);
            }
            "--help" | "-h" => {
                println!(
                    "usage: generate-peq (--speaker-dir DIR | --cea2034-json FILE | --curve CSV) \
                     --speaker NAME --output-dir DIR --fitness Flat|LeastSquare|Score|Combine|FlatPir \
                     (--optimisation global|greedy | --graphic-eq NAME) \
                     [--origin ORIGIN] [--curve-name NAME] [--target CSV | --slope DB] \
                     [--max-peq N] [--max-iter N] [--seed N] [--peq-model pk|ls-pk-hs|...] \
                     [--algo autoeq:de] [--force] [--data-dir DIR --mversion VER]\n\
                     or:    generate-peq <input> --speaker NAME --output-dir DIR \
                     --generate-images-only --data-dir DIR --origin ORIGIN --mversion VER\n\
                     or:    generate-peq --metadata-json FILE --output-dir EQROOT \
                     --fitness Flat --optimisation global [--speaker NAME] [--limit N] [--jobs N] [--data-dir DIR]\n\
                     or:    generate-peq --graphic-eq-list"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    if args.metadata_json.is_none() && args.metadata_dir.is_none() {
        let inputs = [
            args.speaker_dir.is_some(),
            args.cea2034_json.is_some(),
            args.curve_csv.is_some(),
        ]
        .iter()
        .filter(|present| **present)
        .count();
        if inputs != 1 {
            bail!("exactly one input is required: --speaker-dir, --cea2034-json, or --curve");
        }
        if args.speaker.is_empty() {
            bail!("--speaker is required (try --help)");
        }
    }
    // Python generate_peqs.py: --fitness/--optimisation are mandatory
    // unless --generate-images-only.
    if !args.generate_images_only {
        let fitness = args.fitness.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--fitness is required unless --generate-images-only")
        })?;
        args.options.fitness = fitness_from_name(fitness)?;
        args.options.loss = loss_from_fitness(fitness)?;
        if args.graphic_eq.is_none() {
            match args.optimisation.as_deref() {
                None => bail!("--optimisation is required unless --generate-images-only"),
                Some("global") => args.options.optimisation = Optimisation::Global,
                Some("greedy") => args.options.optimisation = Optimisation::Greedy,
                Some(other) => {
                    bail!("unsupported --optimisation {other} (supported: global, greedy)")
                }
            }
        }
    }
    Ok(args)
}

/// Measured input: one optimization curve plus, when available, the
/// full CEA2034 set for score loss and preference-score reporting.
struct Input {
    freq: Vec<f64>,
    measured: Vec<f64>,
    cea: Option<Cea2034>,
}

fn parse_csv_curve(path: &Path) -> Result<(Vec<f64>, Vec<f64>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (mut freq, mut spl) = (Vec::new(), Vec::new());
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let fields: Vec<&str> = line
            .split([',', '\t', ';', ' '])
            .filter(|f| !f.is_empty())
            .collect();
        if fields.len() < 2 {
            bail!("{} line {}: expected freq,spl", path.display(), index + 1);
        }
        let (Ok(f), Ok(s)) = (fields[0].parse::<f64>(), fields[1].parse::<f64>()) else {
            if freq.is_empty() {
                continue; // header row
            }
            bail!(
                "{} line {}: non-numeric freq/spl",
                path.display(),
                index + 1
            );
        };
        if f > 0.0 && f.is_finite() && s.is_finite() {
            freq.push(f);
            spl.push(s);
        }
    }
    if freq.len() < 4 {
        bail!("{}: only {} usable points", path.display(), freq.len());
    }
    Ok((freq, spl))
}

fn cea_from_curves(curves: &HashMap<String, (Vec<f64>, Vec<f64>)>) -> Result<Cea2034> {
    let get = |name: &str| -> Result<&(Vec<f64>, Vec<f64>)> {
        curves
            .get(name)
            .with_context(|| format!("CEA2034 input is missing trace {name}"))
    };
    let on = get("On Axis")?;
    let lw = get("Listening Window")?;
    let er = get("Early Reflections")?;
    let sp = get("Sound Power")?;
    // Emitted CEA2034 figures carry no EIR trace (separate figure);
    // rebuild it with the identical pressure mix when absent.
    let derived_eir;
    let eir = match curves.get("Estimated In-Room Response") {
        Some(eir) => eir,
        None => {
            derived_eir = (on.0.clone(), estimated_inroom_from(&lw.1, &er.1, &sp.1));
            &derived_eir
        }
    };
    let er_di: Vec<f64> = lw.1.iter().zip(&er.1).map(|(l, e)| l - e).collect();
    let sp_di: Vec<f64> = lw.1.iter().zip(&sp.1).map(|(l, s)| l - s).collect();
    Ok(Cea2034 {
        freq: on.0.clone(),
        on_axis: on.1.clone(),
        listening_window: lw.1.clone(),
        early_reflections: er.1.clone(),
        sound_power: sp.1.clone(),
        early_reflections_di: er_di,
        sound_power_di: sp_di,
        estimated_inroom: eir.1.clone(),
        sensitivity_db: 0.0,
    })
}

fn cea_curves(cea: &Cea2034) -> HashMap<String, (Vec<f64>, Vec<f64>)> {
    let pairs: [(&str, &Vec<f64>); 5] = [
        ("On Axis", &cea.on_axis),
        ("Listening Window", &cea.listening_window),
        ("Early Reflections", &cea.early_reflections),
        ("Sound Power", &cea.sound_power),
        ("Estimated In-Room Response", &cea.estimated_inroom),
    ];
    pairs
        .iter()
        .map(|(name, spl)| ((*name).to_owned(), (cea.freq.clone(), (*spl).clone())))
        .collect()
}

fn load_input(args: &Args) -> Result<Input> {
    if let Some(dir) = &args.speaker_dir {
        let cea = match load_measurement(&args.format, dir, &args.speaker)? {
            LoadedMeasurement::Hv(hv) => {
                let hv = symmetrise_hv(&hv, args.symmetry.as_deref())?;
                compute_cea2034(&hv)?
            }
            LoadedMeasurement::Cea(cea) => cea,
            LoadedMeasurement::Curve { .. } => {
                bail!("csv_freq_spl headphone input is not supported by generate-peq")
            }
        };
        let curves = cea_curves(&cea);
        let (_, measured) = curves
            .get(&args.curve_name)
            .with_context(|| format!("unknown --curve-name {}", args.curve_name))?;
        return Ok(Input {
            measured: measured.clone(),
            freq: cea.freq.clone(),
            cea: Some(cea),
        });
    }
    if let Some(path) = &args.cea2034_json {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let root: serde_json::Value =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let data = root["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("{}: no Plotly data array", path.display()))?;
        let mut curves = HashMap::new();
        for trace in data {
            let Some(name) = trace["name"].as_str() else {
                continue;
            };
            if !CEA_CURVES.contains(&name) {
                continue;
            }
            let numbers = |key: &str| -> Result<Vec<f64>> {
                trace[key]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("trace {name}: missing {key} array"))?
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .ok_or_else(|| anyhow::anyhow!("trace {name}: non-numeric {key} value"))
                    })
                    .collect()
            };
            curves.insert(name.to_owned(), (numbers("x")?, numbers("y")?));
        }
        let cea = cea_from_curves(&curves)?;
        let all = cea_curves(&cea);
        let (_, measured) = all
            .get(&args.curve_name)
            .with_context(|| format!("unknown --curve-name {}", args.curve_name))?;
        return Ok(Input {
            measured: measured.clone(),
            freq: cea.freq.clone(),
            cea: Some(cea),
        });
    }
    let path = args.curve_csv.as_deref().expect("validated one input");
    let (freq, measured) = parse_csv_curve(path)?;
    Ok(Input {
        freq,
        measured,
        cea: None,
    })
}

fn build_target(args: &Args, input: &Input) -> Result<Vec<f64>> {
    if let Some(path) = &args.target_csv {
        let (target_freq, target_spl) = parse_csv_curve(path)?;
        return interp_log(&input.freq, &target_freq, &target_spl);
    }
    let slope = match args.slope {
        Some(slope) => slope,
        None => default_slope(&args.curve_name).unwrap_or(0.0),
    };
    let target_min = match args.target_min_freq {
        Some(freq) => freq,
        None => minus_3db_freq(&input.freq, &input.measured, 100.0, 300.0)?,
    };
    slope_target(
        &input.freq,
        &input.measured,
        slope,
        target_min,
        args.target_max_freq,
        100.0,
        300.0,
    )
}

/// Preference score with `peq` applied, when CEA2034 curves exist.
fn score_with(input: &Input, peq: &Peq) -> Result<Option<f64>> {
    match &input.cea {
        Some(cea) => Ok(Some(preference_score_with_peq(cea, peq)?.pref_score)),
        None => Ok(None),
    }
}

fn filters_figure(args: &Args, input: &Input, target: Option<&[f64]>, peq: &Peq) -> Figure {
    let corrected = apply_peq(&input.freq, &input.measured, peq);
    let eq = peq_response_db(&input.freq, peq);
    let mut traces = vec![
        Trace::new(
            args.curve_name.clone(),
            input.measured.clone(),
            Axis::Primary,
        ),
        Trace::new(
            format!("{} with EQ", args.curve_name),
            corrected,
            Axis::Primary,
        ),
        Trace::new("EQ", eq, Axis::Primary),
    ];
    if let Some(target) = target {
        traces.push(Trace::new("Target", target.to_vec(), Axis::Primary));
    }
    let origin = origin_dir_name(&args.origin);
    Figure::scatter(
        format!("filters_{}", args.curve_name),
        format!("EQ for {} ({origin})", args.speaker),
        input.freq.clone(),
        traces,
    )
}

fn short_curve_name(name: &str) -> &'static str {
    match name {
        "Listening Window" => "LW",
        "Estimated In-Room Response" => "PIR",
        "On Axis" => "ON",
        _ => "curve",
    }
}

fn full_freq(freq: &[f64]) -> Vec<f64> {
    let n = freq.len().max(200);
    (0..n)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "grid index fits exactly in f64")]
            let step = i as f64 / (n - 1) as f64;
            20.0 * 1000f64.powf(step)
        })
        .collect()
}

fn interp_linear_clamp(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
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

fn corrected_cea(cea: &Cea2034, peq: &Peq) -> Cea2034 {
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

fn histogram_counts(
    freq: &[f64],
    values: &[f64],
    slope: f64,
    intercept: f64,
    min_freq: f64,
    max_freq: f64,
) -> Vec<f64> {
    let edges = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 5.0];
    let mut counts = vec![0.0; edges.len() - 1];
    for (f, value) in freq.iter().zip(values) {
        if *f < min_freq || *f > max_freq {
            continue;
        }
        let dist = (value - (slope * f.log10() + intercept)).abs();
        for bin in 0..edges.len() - 1 {
            let in_bin = if bin + 1 == edges.len() - 1 {
                dist >= edges[bin] && dist <= edges[bin + 1]
            } else {
                dist >= edges[bin] && dist < edges[bin + 1]
            };
            if in_bin {
                counts[bin] += 1.0;
                break;
            }
        }
    }
    counts
}

fn regression_line(freq: &[f64], values: &[f64], min_freq: f64, max_freq: f64) -> (f64, f64) {
    let points: Vec<(f64, f64)> = freq
        .iter()
        .zip(values)
        .filter(|(f, _)| **f > min_freq && **f < max_freq)
        .map(|(f, v)| (f.log10(), *v))
        .collect();
    if points.len() < 2 {
        return (0.0, 0.0);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "point counts fit exactly in f64"
    )]
    let n = points.len() as f64;
    let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let denominator: f64 = points.iter().map(|p| (p.0 - mean_x).powi(2)).sum();
    if denominator == 0.0 {
        return (0.0, mean_y);
    }
    let slope = points
        .iter()
        .map(|p| (p.0 - mean_x) * (p.1 - mean_y))
        .sum::<f64>()
        / denominator;
    (slope, mean_y - slope * mean_x)
}

fn histogram_figure(
    name: String,
    title: String,
    freq: &[f64],
    raw: &[f64],
    corrected: &[f64],
    args: &Args,
    midrange: bool,
) -> Figure {
    let (hist_min, hist_max) = if midrange {
        (300.0, 5_000.0)
    } else {
        (args.target_min_freq.unwrap_or(20.0), args.target_max_freq)
    };
    let target_min = args.target_min_freq.unwrap_or(20.0);
    let (raw_slope, raw_intercept) = regression_line(freq, raw, target_min, args.target_max_freq);
    let (auto_slope, auto_intercept) =
        regression_line(freq, corrected, target_min, args.target_max_freq);
    let labels = vec![
        "0.0-0.5".to_owned(),
        "0.5-1.0".to_owned(),
        "1.0-1.5".to_owned(),
        "1.5-2.0".to_owned(),
        "2.0-2.5".to_owned(),
        "2.5-3.0".to_owned(),
        "3.0-5.0".to_owned(),
        "5.0+".to_owned(),
    ];
    Figure {
        name,
        title,
        freq: freq.to_vec(),
        traces: Vec::new(),
        x_tickvals: None,
        y_tickvals: None,
        y_range: None,
        y_title: None,
        show_zero_line: true,
        kind: FigureKind::Bar(vec![
            BarTrace {
                name: "noEQ".to_owned(),
                categories: labels.clone(),
                values: histogram_counts(freq, raw, raw_slope, raw_intercept, hist_min, hist_max),
                color: Some("#5c77a5".to_owned()),
            },
            BarTrace {
                name: "autoEQ".to_owned(),
                categories: labels,
                values: histogram_counts(
                    freq,
                    corrected,
                    auto_slope,
                    auto_intercept,
                    hist_min,
                    hist_max,
                ),
                color: Some("#dc842a".to_owned()),
            },
        ]),
    }
}

fn filters_figures(args: &Args, input: &Input, target: Option<&[f64]>, peq: &Peq) -> Vec<Figure> {
    let Some(cea) = &input.cea else {
        return vec![filters_figure(args, input, target, peq)];
    };
    let origin = origin_dir_name(&args.origin);
    let corrected = corrected_cea(cea, peq);
    let full = full_freq(&input.freq);
    let mut figures = Vec::new();

    let auto_eq_traces: Vec<Trace> = peq
        .iter()
        .enumerate()
        .map(|(index, single)| {
            Trace::new(
                format!(
                    "{}{index}: {:.0}Hz",
                    single.1.filter_type.short_name(),
                    single.1.freq
                ),
                peq_response_db(&full, &vec![single.clone()]),
                Axis::Primary,
            )
        })
        .collect();
    figures.push(Figure::scatter(
        "filters_auto_eq",
        format!("EQ for {} ({origin})", args.speaker),
        full.clone(),
        auto_eq_traces,
    ));

    if target.is_some()
        && let Ok(slope) = args
            .slope
            .map_or_else(|| default_slope(&args.curve_name), Ok)
    {
        let target_min = args.target_min_freq.unwrap_or_else(|| {
            minus_3db_freq(&input.freq, &input.measured, 100.0, 300.0).unwrap_or(20.0)
        });
        if let Ok(line) = relative_slope_line(&input.freq, slope, target_min, args.target_max_freq)
        {
            let short = short_curve_name(&args.curve_name);
            let i_min = input
                .freq
                .iter()
                .position(|f| *f >= target_min)
                .unwrap_or(0);
            let first_value = input.measured.get(i_min).copied().unwrap_or(0.0);
            let db_cut = first_value.signum() * args.options.max_db.min(first_value.abs());
            let error: Vec<f64> = input
                .measured
                .iter()
                .enumerate()
                .zip(&line)
                .map(|((index, measured), ideal)| {
                    let limited = if index > i_min { *measured } else { db_cut };
                    (-(limited - ideal)).min(args.options.max_db)
                })
                .collect();
            figures.push(Figure::scatter(
                "filters_eq_full",
                format!("EQ v.s. Target for {} ({origin})", args.speaker),
                full.clone(),
                vec![
                    Trace::new("autoEQ", peq_response_db(&full, peq), Axis::Primary),
                    Trace::new(
                        format!("error {short}"),
                        interp_linear_clamp(&full, &input.freq, &error),
                        Axis::Primary,
                    ),
                    Trace::new(
                        format!("ideal {short}"),
                        interp_linear_clamp(&full, &input.freq, &line),
                        Axis::Primary,
                    ),
                ],
            ));
        }
    }

    figures.push(cea_figure(
        "filters_spin_noeq",
        format!("Spin for {} ({origin})", args.speaker),
        cea,
    ));
    figures.push(cea_figure(
        "filters_spin_auto",
        format!("Spin w/autoEQ for {} ({origin})", args.speaker),
        &corrected,
    ));

    for (stem, curve_name, raw, auto) in [
        ("on", "On Axis", &cea.on_axis, &corrected.on_axis),
        (
            "lw",
            "Listening Window",
            &cea.listening_window,
            &corrected.listening_window,
        ),
        (
            "pir",
            "Estimated In-Room Response",
            &cea.estimated_inroom,
            &corrected.estimated_inroom,
        ),
    ] {
        figures.push(Figure::scatter(
            format!("filters_{stem}_noeq"),
            format!("{curve_name} for {} ({origin})", args.speaker),
            cea.freq.clone(),
            regression_traces(&cea.freq, raw, curve_name, Some("Midrange ±3dB")),
        ));
        figures.push(Figure::scatter(
            format!("filters_{stem}_auto"),
            format!("{curve_name} with EQ for {} ({origin})", args.speaker),
            cea.freq.clone(),
            regression_traces(&cea.freq, auto, curve_name, Some("Midrange ±3dB")),
        ));
        figures.push(histogram_figure(
            format!("filters_{stem}_hist_fullrange"),
            format!("Histogram of errors (fullrange) for {curve_name}"),
            &cea.freq,
            raw,
            auto,
            args,
            false,
        ));
        figures.push(histogram_figure(
            format!("filters_{stem}_hist_midrange"),
            format!("Histogram of errors (midrange) for {curve_name}"),
            &cea.freq,
            raw,
            auto,
            args,
            true,
        ));
    }
    figures
}

fn write_filters_figures(
    args: &Args,
    input: &Input,
    target: Option<&[f64]>,
    peq: &Peq,
) -> Result<()> {
    let Some(data_dir) = &args.data_dir else {
        return Ok(());
    };
    if args.origin.is_empty() {
        bail!("--data-dir needs --origin for the site layout");
    }
    let figures = filters_figures(args, input, target, peq);
    let base = data_dir
        .join("dist-private/speakers")
        .join(sanitize_filename(&args.speaker))
        .join(origin_dir_name(&args.origin));
    let out_dir = if input.cea.is_some() {
        base
    } else {
        if args.version.is_empty() {
            bail!("--data-dir needs --mversion when no CEA2034 curves are available");
        }
        base.join(&args.version)
    };
    for figure in &figures {
        let (written, _) = write_figure(&out_dir, figure)?;
        println!("wrote {}", written.display());
    }
    Ok(())
}

fn write_conf_if_absent(args: &Args) -> Result<()> {
    let path = args.output_dir.join("conf-autoeq.json");
    if path.is_file() {
        return Ok(());
    }
    let conf = serde_json::json!({
        "generator": "spinorama-generate generate-peq (Rust AutoEQ)",
        "fitness": format!("{:?}", args.options.loss),
        "curve_name": args.curve_name,
        "num_filters": args.options.num_filters,
        "min_q": args.options.min_q,
        "max_q": args.options.max_q,
        "min_db": args.options.min_db,
        "max_db": args.options.max_db,
        "min_freq": args.options.min_freq,
        "max_freq": args.options.max_freq,
        "sample_rate": args.options.sample_rate,
        "maxeval": args.options.maxeval,
        "seed": args.options.seed,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&conf)?)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn apo_path(args: &Args) -> PathBuf {
    match &args.graphic_eq {
        Some(name) => args.output_dir.join(format!(
            "iir-autoeq-{}.txt",
            name.to_lowercase().replace(' ', "-")
        )),
        None => args.output_dir.join("iir-autoeq.txt"),
    }
}

fn run_images_only(args: &Args, input: &Input) -> Result<()> {
    let apo_path = apo_path(args);
    let text = std::fs::read_to_string(&apo_path)
        .with_context(|| format!("--generate-images-only needs {}", apo_path.display()))?;
    let (preamp_db, peq) = parse_apo(&text, args.options.sample_rate)?;
    println!(
        "loaded {} filters (preamp {preamp_db:.1} dB) from {}",
        peq.len(),
        apo_path.display()
    );
    if let (Some(before), Some(after)) = (
        input.cea.as_ref().map(preference_score).transpose()?,
        score_with(input, &peq)?,
    ) {
        println!(
            "preference score {:.2} -> {after:.2} with EQ",
            before.pref_score
        );
    }
    let target = build_target(args, input)?;
    write_filters_figures(args, input, Some(&target), &peq)
}

fn run_optimize(args: &Args, input: &Input) -> Result<()> {
    let started = Instant::now();
    let target = build_target(args, input)?;
    let spin = input.cea.as_ref().map(cea_curves);
    let peq = if let Some(device_name) = &args.graphic_eq {
        let device = graphic_eq(device_name)?;
        let cea = input
            .cea
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--graphic-eq needs CEA2034 curves"))?;
        let slope = args
            .slope
            .map_or_else(|| default_slope(&args.curve_name), Ok)?;
        let target_min = args.target_min_freq.unwrap_or_else(|| {
            minus_3db_freq(&input.freq, &input.measured, 100.0, 300.0).unwrap_or(20.0)
        });
        let line = relative_slope_line(&input.freq, slope, target_min, args.target_max_freq)?;
        let i_min = input
            .freq
            .iter()
            .position(|f| *f >= target_min)
            .unwrap_or(0);
        let first_value = input.measured.get(i_min).copied().unwrap_or(0.0);
        let db_cut = first_value.signum() * args.options.max_db.min(first_value.abs());
        let error: Vec<f64> = input
            .measured
            .iter()
            .enumerate()
            .zip(&line)
            .map(|((index, measured), ideal)| {
                let limited = if index > i_min { *measured } else { db_cut };
                limited - ideal
            })
            .collect();
        optimize_grapheq(
            &input.freq,
            &error,
            Some(cea),
            &input.measured,
            &target,
            &device,
            &args.options,
        )?
    } else {
        optimize_peq(
            &input.freq,
            &input.measured,
            &target,
            spin.as_ref(),
            &args.options,
        )?
    };
    println!(
        "optimized {} filters in {:?} (algo {}, optimisation {:?}, maxeval {}, seed {:?})",
        peq.len(),
        started.elapsed(),
        args.graphic_eq.as_ref().map_or_else(
            || args.options.algo.clone(),
            |name| format!("grapheq:{name}")
        ),
        args.options.optimisation,
        args.options.maxeval,
        args.options.seed
    );

    let corrected = apply_peq(&input.freq, &input.measured, &peq);
    let band = (args.options.min_freq.max(20.0), args.target_max_freq);
    let rms_before = rms_deviation(&input.freq, &input.measured, &target, band.0, band.1)?;
    let rms_after = rms_deviation(&input.freq, &corrected, &target, band.0, band.1)?;
    println!("RMS deviation to target: {rms_before:.3} dB -> {rms_after:.3} dB");

    let score_before = input
        .cea
        .as_ref()
        .map(preference_score)
        .transpose()?
        .map(|m| m.pref_score);
    let score_after = score_with(input, &peq)?;
    if let (Some(before), Some(after)) = (score_before, score_after) {
        println!("preference score {before:.2} -> {after:.2} with EQ");
    }

    // Previous-score gating (Python optim_save_peq): never overwrite a
    // better existing iir-autoeq.txt unless --force. Without CEA2034
    // curves the gate compares RMS deviation to the same target.
    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating {}", args.output_dir.display()))?;
    let apo_path = apo_path(args);
    if apo_path.is_file() && !args.force {
        let previous_text = std::fs::read_to_string(&apo_path)?;
        let (_, previous_peq) = parse_apo(&previous_text, args.options.sample_rate)?;
        let keep_previous = match (score_with(input, &previous_peq)?, score_after) {
            (Some(previous), Some(new)) => {
                println!("previous score {previous:.2} vs new {new:.2}");
                new <= previous
            }
            _ => {
                let previous_corrected = apply_peq(&input.freq, &input.measured, &previous_peq);
                let previous_rms =
                    rms_deviation(&input.freq, &previous_corrected, &target, band.0, band.1)?;
                println!("previous RMS {previous_rms:.3} dB vs new {rms_after:.3} dB");
                rms_after >= previous_rms
            }
        };
        if keep_previous {
            println!(
                "keeping existing {} (use --force to overwrite)",
                apo_path.display()
            );
            return write_filters_figures(args, input, Some(&target), &previous_peq);
        }
    }

    let origin = if args.origin.is_empty() {
        "unknown"
    } else {
        origin_dir_name(&args.origin)
    };
    let apo = format_site_apo(
        &args.speaker,
        origin,
        score_before.unwrap_or(0.0),
        score_after.unwrap_or(0.0),
        &peq,
    );
    std::fs::write(&apo_path, apo).with_context(|| format!("writing {}", apo_path.display()))?;
    println!("wrote {}", apo_path.display());
    // The optimizer only emits speaker-flat/score losses in this slice.
    debug_assert!(matches!(
        args.options.loss,
        LossType::SpeakerFlat | LossType::SpeakerScore
    ));
    write_conf_if_absent(args)?;
    write_filters_figures(args, input, Some(&target), &peq)
}

fn run_one(args: &Args) -> Result<()> {
    let input = load_input(args)?;
    if args.generate_images_only {
        run_images_only(args, &input)
    } else {
        run_optimize(args, &input)
    }
}

/// Runs per-speaker PEQ jobs, in parallel unless `--limit` is set.
///
/// Each job writes only its own speaker output directory. `--limit`
/// counts completed/failed speakers in order, so that path stays
/// serial to preserve the exact stopping point. `jobs = 0` lets rayon
/// pick one worker per CPU core.
fn run_batch_work(work: &[Args], limit: Option<usize>, jobs: usize) -> Result<()> {
    let (mut done, mut failed) = (0usize, 0usize);
    if let Some(limit) = limit {
        for one in work {
            if done + failed >= limit {
                break;
            }
            match run_one(one) {
                Ok(()) => done += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("batch: FAILED {} {}: {error:#}", one.speaker, one.version);
                }
            }
        }
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
        let outcomes: Vec<bool> = pool.install(|| {
            work.par_iter()
                .map(|one| match run_one(one) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("batch: FAILED {} {}: {error:#}", one.speaker, one.version);
                        false
                    }
                })
                .collect()
        });
        done = outcomes.iter().filter(|ok| **ok).count();
        failed = outcomes.len() - done;
    }
    println!("batch: {done} generated, {failed} failed");
    if failed > 0 {
        bail!("{failed} speaker(s) failed");
    }
    Ok(())
}

fn run_batch_toml(args: &Args, dir: &std::path::Path) -> Result<()> {
    let entries = spinorama_generate::metadata::catalogue_from_dir(dir)?;
    let mut work: Vec<Args> = Vec::new();
    for entry in entries.iter().filter(|e| e.is_default) {
        if !args.speaker.is_empty() && args.speaker != entry.speaker {
            continue;
        }
        let mut one = args.clone();
        one.metadata_json = None;
        one.metadata_dir = None;
        one.speaker.clone_from(&entry.speaker);
        one.version.clone_from(&entry.version);
        one.speaker_dir = Some(entry.speaker_dir.clone());
        one.cea2034_json = None;
        one.curve_csv = None;
        one.origin.clone_from(&entry.origin);
        one.format.clone_from(&entry.format);
        one.symmetry.clone_from(&entry.symmetry);
        one.output_dir = args.output_dir.join(sanitize_filename(&entry.speaker));
        work.push(one);
    }
    run_batch_work(&work, args.limit, args.jobs)
}

fn run_batch(args: &Args) -> Result<()> {
    if let Some(dir) = &args.metadata_dir {
        return run_batch_toml(args, dir);
    }
    let path = args
        .metadata_json
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--metadata-json is required for batch mode"))?;
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let root: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let speakers = root
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{}: top level is not an object", path.display()))?;
    let mut speaker_names: Vec<&String> = speakers.keys().collect();
    speaker_names.sort();

    let mut work: Vec<Args> = Vec::new();
    for speaker_name in speaker_names {
        if !args.speaker.is_empty() && &args.speaker != speaker_name {
            continue;
        }
        let speaker = &speakers[speaker_name];
        let Some(version) = speaker["default_measurement"].as_str() else {
            continue;
        };
        let measurement = &speaker["measurements"][version];
        if measurement.is_null() {
            continue;
        }
        let mut one = args.clone();
        one.metadata_json = None;
        one.speaker.clone_from(speaker_name);
        one.version = version.to_owned();
        one.speaker_dir = Some(args.measurements_root.join(speaker_name).join(version));
        one.cea2034_json = None;
        one.curve_csv = None;
        one.origin = measurement["origin"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        one.format = measurement["format"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        one.symmetry = measurement["symmetry"].as_str().map(str::to_owned);
        one.output_dir = args.output_dir.join(sanitize_filename(speaker_name));
        work.push(one);
    }
    run_batch_work(&work, args.limit, args.jobs)
}

fn run(args: &Args) -> Result<()> {
    if args.metadata_json.is_some() || args.metadata_dir.is_some() {
        run_batch(args)
    } else {
        run_one(args)
    }
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-peq: {error:#}");
            ExitCode::FAILURE
        }
    }
}
