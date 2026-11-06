//! `generate-peq-headphone`: headphone PEQ computation on Rust AutoEQ.
//!
//! Rust replacement for `scripts/headphone_eqs_compute.sh` and
//! `scripts/headphone_scores_compute.sh`, which shell out to the
//! external `autoeq` binary. Those scripts are not Python, but no bin
//! in this crate covered the headphone step: `generate-peq` only maps
//! speaker losses (`SpeakerFlat`/`SpeakerScore`) and rejects the
//! single-curve headphone workflow. This bin mirrors the scripts:
//! for one headphone (or, with `--batch`, every directory under
//! `datas/headphones`) it runs two 7-filter PK optimizations
//! (`HeadphoneScore` and `HeadphoneFlat`) over 20-6000 Hz against the
//! Harman 2019 target selected by shape, and writes the files
//! `scripts/generate_headphone_meta.py` consumes:
//! `iir-autoeq-score.txt`, `iir-autoeq-flat.txt`, `autoeq_score.json`.

// Rust guideline compliant 2026-10-09

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use autoeq_workflow::iir::{self, Peq};
use autoeq_workflow::loss::{HeadphoneLossData, headphone_loss_with_target};
use autoeq_workflow::read::read_curve_from_csv;
use autoeq_workflow::{
    Curve, LossType, OptimParams, OptimizationConfig, PeqModel, optimize_headphone,
};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use spinorama_generate::filters::apo::parse_apo;
use spinorama_generate::filters::biquad::apply_peq;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Scripts use 7 PK filters over 20-6000 Hz (`headphone_eqs_compute.sh`).
const DEFAULT_NUM_FILTERS: usize = 7;
const DEFAULT_MIN_FREQ: f64 = 20.0;
const DEFAULT_MAX_FREQ: f64 = 6_000.0;

#[derive(Clone)]
struct Args {
    curve: Option<PathBuf>,
    target: Option<PathBuf>,
    targets_dir: PathBuf,
    shape: String,
    name: String,
    output_dir: PathBuf,
    batch: bool,
    headphones_root: PathBuf,
    eq_root: PathBuf,
    metadata_py: PathBuf,
    only: Option<String>,
    limit: Option<usize>,
    jobs: usize,
    num_filters: usize,
    min_freq: f64,
    max_freq: f64,
    algo: String,
    maxeval: Option<usize>,
    seed: Option<u64>,
    force: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            curve: None,
            target: None,
            targets_dir: PathBuf::from("datas/headphone_targets"),
            shape: "over-ear".to_owned(),
            name: String::new(),
            output_dir: PathBuf::from("."),
            batch: false,
            headphones_root: PathBuf::from("datas/headphones"),
            eq_root: PathBuf::from("datas/headphone_eq"),
            metadata_py: PathBuf::from("datas/headphones.py"),
            only: None,
            limit: None,
            jobs: 0,
            num_filters: DEFAULT_NUM_FILTERS,
            min_freq: DEFAULT_MIN_FREQ,
            max_freq: DEFAULT_MAX_FREQ,
            algo: "autoeq:cmaes".to_owned(),
            maxeval: None,
            seed: Some(42),
            force: false,
        }
    }
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
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
            "--curve" => args.curve = Some(PathBuf::from(value(&mut it)?)),
            "--target" => args.target = Some(PathBuf::from(value(&mut it)?)),
            "--targets-dir" => args.targets_dir = PathBuf::from(value(&mut it)?),
            "--shape" => args.shape = value(&mut it)?,
            "--name" | "--speaker" => args.name = value(&mut it)?,
            "--output-dir" => args.output_dir = PathBuf::from(value(&mut it)?),
            "--batch" => args.batch = true,
            "--headphones-root" => args.headphones_root = PathBuf::from(value(&mut it)?),
            "--eq-root" => args.eq_root = PathBuf::from(value(&mut it)?),
            "--metadata-py" => args.metadata_py = PathBuf::from(value(&mut it)?),
            "--only" => args.only = Some(value(&mut it)?),
            "--limit" => {
                args.limit = Some(value(&mut it)?.parse().context("--limit needs a count")?)
            }
            "--jobs" => {
                args.jobs = value(&mut it)?.parse().context("--jobs needs a count")?;
            }
            "--max-peq" | "-n" => {
                args.num_filters = value(&mut it)?.parse().context("--max-peq needs a count")?;
            }
            "--min-freq" => args.min_freq = number(&mut it)?,
            "--max-freq" => args.max_freq = number(&mut it)?,
            "--algo" => args.algo = value(&mut it)?,
            "--max-iter" | "--maxeval" => {
                args.maxeval = Some(value(&mut it)?.parse().context("--maxeval needs a count")?);
            }
            "--seed" => args.seed = Some(value(&mut it)?.parse().context("--seed needs a u64")?),
            "--force" => args.force = true,
            "--help" | "-h" => {
                println!(
                    "usage: generate-peq-headphone --curve CSV [--target CSV] [--shape over-ear|in-ear|earbud] \\\n\
                     --name NAME --output-dir DIR [--max-peq 7] [--min-freq 20] [--max-freq 6000] \\\n\
                     [--algo autoeq:cmaes] [--seed 42] [--force]\n\
                     or:    generate-peq-headphone --batch [--headphones-root datas/headphones] \\\n\
                     [--eq-root datas/headphone_eq] [--targets-dir datas/headphone_targets] \\\n\
                     [--metadata-py datas/headphones.py] [--only NAME] [--limit N] [--jobs N] [--force]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    if !args.batch {
        if args.curve.is_none() {
            bail!("--curve is required unless --batch (try --help)");
        }
        if args.name.is_empty() {
            bail!("--name is required unless --batch (try --help)");
        }
    }
    Ok(args)
}

/// Target file for a shape, mirroring `target_for_shape` in the scripts.
fn target_for_shape(targets_dir: &Path, shape: &str) -> PathBuf {
    match shape {
        "in-ear" | "earbud" => targets_dir.join("harman_inear_2019.csv"),
        _ => targets_dir.join("harman_overear_2019.csv"),
    }
}

/// Extract `"shape"` for `name` from generated `datas/headphones.py`.
///
/// The file is generated Python (`'Name': { "shape": 'over-ear', ...}`);
/// a bounded window after the name avoids a Python dependency.
fn shape_from_metadata(text: &str, name: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let needle = format!("{quote}{name}{quote}:");
        if let Some(start) = text.find(&needle) {
            let window = &text[start..(start + 800).min(text.len())];
            if let Some(pos) = window.find("shape") {
                let rest = &window[pos..];
                if let Some(colon) = rest.find(':') {
                    let value: String = rest[colon + 1..]
                        .trim_start()
                        .trim_start_matches(['\'', '"'])
                        .chars()
                        .take_while(|c| *c != '\'' && *c != '"' && *c != ',')
                        .collect();
                    let value = value.trim().to_owned();
                    if !value.is_empty() {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

fn config_for(args: &Args, loss: LossType) -> OptimParams {
    let mut config = OptimizationConfig::headphone_defaults();
    config.num_filters = args.num_filters;
    config.loss = loss;
    config.peq_model = PeqModel::Pk;
    config.min_freq = args.min_freq;
    config.max_freq = args.max_freq;
    config.algo = args.algo.clone();
    config.seed = args.seed;
    if let Some(maxeval) = args.maxeval {
        config.maxeval = maxeval;
    }
    OptimParams::from(&config)
}

fn headphone_score(response: &Curve, target: &Curve, peq: Option<&Peq>) -> f64 {
    let corrected = match peq {
        Some(peq) => Curve {
            freq: response.freq.clone(),
            spl: ndarray::Array1::from_vec(apply_peq(
                &response.freq.to_vec(),
                &response.spl.to_vec(),
                peq,
            )),
            ..Default::default()
        },
        None => response.clone(),
    };
    // Same smoothing the optimizer config carries (speaker defaults: 1/1 oct).
    headphone_loss_with_target(&HeadphoneLossData::new(true, 1), &corrected, target)
}

struct OneResult {
    pre_score: f64,
    post_score: f64,
    score_filters: usize,
    flat_filters: usize,
}

fn run_one(
    args: &Args,
    curve: &Path,
    target_path: &Path,
    name: &str,
    output_dir: &Path,
) -> Result<OneResult> {
    if !args.force && output_dir.join("iir-autoeq-score.txt").is_file() {
        bail!("EQ already computed for {name} (use --force)");
    }
    let target = read_curve_from_csv(&target_path.to_path_buf())
        .map_err(|e| anyhow::anyhow!("reading target {}: {e}", target_path.display()))?;
    let response = read_curve_from_csv(&curve.to_path_buf())
        .map_err(|e| anyhow::anyhow!("reading curve {}: {e}", curve.display()))?;

    let pre_score = headphone_score(&response, &target, None);

    let mut out = OneResult {
        pre_score,
        post_score: pre_score,
        score_filters: 0,
        flat_filters: 0,
    };
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("creating {}", output_dir.display()))?;

    for (loss, stem, label) in [
        (
            LossType::HeadphoneScore,
            "iir-autoeq-score",
            "HeadphoneScore",
        ),
        (LossType::HeadphoneFlat, "iir-autoeq-flat", "HeadphoneFlat"),
    ] {
        let params = config_for(args, loss);
        let result = optimize_headphone::<
            fn(&autoeq_workflow::ProgressUpdate) -> autoeq_workflow::de::CallbackAction,
        >(&curve.to_path_buf(), &target, &params, None, None)
        .map_err(|e| anyhow::anyhow!("optimizing {name} ({label}): {e}"))?;
        let peq: Peq = result.biquads.iter().map(|b| (1.0, b.clone())).collect();
        if loss == LossType::HeadphoneScore {
            out.post_score = headphone_score(&response, &target, Some(&peq));
            out.score_filters = peq.len();
        } else {
            out.flat_filters = peq.len();
        }
        let mut rounded = peq.clone();
        for (_, filter) in &mut rounded {
            filter.freq = filter.freq.round();
        }
        let header = format!(
            "# AutoEQ Parametric Equalizer Settings\n# Speaker: {name}\n# Loss Type: {label}\n# Filters: {}\n# Generated by spinorama-generate generate-peq-headphone (Rust AutoEQ)",
            rounded.len()
        );
        let apo = iir::peq_format_apo(&header, &rounded);
        let path = output_dir.join(format!("{stem}.txt"));
        std::fs::write(&path, &apo).with_context(|| format!("writing {}", path.display()))?;
        // Round-trip check: the meta reader must parse what we wrote.
        let (_, parsed) = parse_apo(&apo, 48_000.0)?;
        if parsed.len() != rounded.len() {
            bail!("APO round-trip for {} lost filters", path.display());
        }
        println!("wrote {} ({} filters)", path.display(), parsed.len());
    }

    let score_json = format!(
        "{{\"pre_score\": {:.3}, \"post_score\": {:.3}}}\n",
        out.pre_score, out.post_score
    );
    let score_path = output_dir.join("autoeq_score.json");
    std::fs::write(&score_path, score_json)
        .with_context(|| format!("writing {}", score_path.display()))?;
    println!(
        "{name}: headphone score {:.3} -> {:.3} with EQ ({} score filters, {} flat filters)",
        out.pre_score, out.post_score, out.score_filters, out.flat_filters
    );
    Ok(out)
}

fn find_curve(dir: &Path) -> Option<PathBuf> {
    let mut origins: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut names: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        names.sort();
        origins.append(&mut names);
    }
    // Prefer `asr`, as the scripts do via default_measurement then asr.
    origins.sort_by_key(|p| {
        if p.file_name().is_some_and(|n| n == "asr") {
            0
        } else {
            1
        }
    });
    for origin in &origins {
        for file in ["frequency_response.csv", "freq_response.csv", "fr.csv"] {
            let candidate = origin.join(file);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn run_batch(args: &Args) -> Result<()> {
    let metadata = std::fs::read_to_string(&args.metadata_py).unwrap_or_default();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&args.headphones_root)
        .with_context(|| format!("reading {}", args.headphones_root.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    names.sort();
    let (mut done, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    // Build the work list serially: skips are cheap and ordered.
    struct Job {
        name: String,
        curve: PathBuf,
        target: PathBuf,
        output_dir: PathBuf,
    }
    let mut work: Vec<Job> = Vec::new();
    for dir in names {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if args.only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        let Some(curve) = find_curve(&dir) else {
            println!("SKIP {name}: no frequency response CSV found");
            skipped += 1;
            continue;
        };
        let shape = shape_from_metadata(&metadata, &name).unwrap_or_else(|| "over-ear".to_owned());
        let target = target_for_shape(&args.targets_dir, &shape);
        let output_dir = args.eq_root.join(&name);
        work.push(Job {
            name,
            curve,
            target,
            output_dir,
        });
    }
    #[derive(Clone, Copy)]
    enum Outcome {
        Done,
        Skipped,
        Failed,
    }
    let run_job =
        |job: &Job| match run_one(args, &job.curve, &job.target, &job.name, &job.output_dir) {
            Ok(_) => Outcome::Done,
            Err(error) if error.to_string().contains("already computed") => {
                println!("SKIP {}: EQ already computed", job.name);
                Outcome::Skipped
            }
            Err(error) => {
                eprintln!("FAIL {}: {error:#}", job.name);
                Outcome::Failed
            }
        };
    let outcomes: Vec<Outcome> = if let Some(limit) = args.limit {
        // --limit counts completed/failed headphones in order, so
        // this path stays serial to preserve the stopping point.
        let mut out = Vec::new();
        for job in &work {
            if done + failed >= limit {
                break;
            }
            let outcome = run_job(job);
            match outcome {
                Outcome::Done => done += 1,
                Outcome::Skipped => skipped += 1,
                Outcome::Failed => failed += 1,
            }
            out.push(outcome);
        }
        out
    } else {
        // Headphones are independent: each writes only its own EQ dir.
        // num_threads(0) lets rayon pick one worker per CPU core.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(args.jobs)
            .build()
            .map_err(|error| anyhow::anyhow!("building thread pool: {error}"))?;
        pool.install(|| work.par_iter().map(&run_job).collect())
    };
    if args.limit.is_none() {
        for outcome in outcomes {
            match outcome {
                Outcome::Done => done += 1,
                Outcome::Skipped => skipped += 1,
                Outcome::Failed => failed += 1,
            }
        }
    }
    println!("batch: {done} computed, {skipped} skipped, {failed} failed");
    if failed > 0 {
        bail!("{failed} headphone(s) failed");
    }
    Ok(())
}

fn run(args: &Args) -> Result<()> {
    if args.batch {
        return run_batch(args);
    }
    let curve = args.curve.as_deref().expect("validated --curve");
    let target = match &args.target {
        Some(target) => target.clone(),
        None => target_for_shape(&args.targets_dir, &args.shape),
    };
    run_one(args, curve, &target, &args.name, &args.output_dir)?;
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-peq-headphone: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_lookup_reads_generated_python() {
        let text =
            "'AKG K371': {\n \"shape\": 'over-ear',\n},\n'1More P50': {\n \"shape\": 'in-ear',\n}";
        assert_eq!(
            shape_from_metadata(text, "AKG K371").as_deref(),
            Some("over-ear")
        );
        assert_eq!(
            shape_from_metadata(text, "1More P50").as_deref(),
            Some("in-ear")
        );
        assert_eq!(shape_from_metadata(text, "Missing"), None);
    }

    #[test]
    fn target_selection_mirrors_scripts() {
        let dir = Path::new("datas/headphone_targets");
        assert!(target_for_shape(dir, "in-ear").ends_with("harman_inear_2019.csv"));
        assert!(target_for_shape(dir, "earbud").ends_with("harman_inear_2019.csv"));
        assert!(target_for_shape(dir, "over-ear").ends_with("harman_overear_2019.csv"));
        assert!(target_for_shape(dir, "on-ear").ends_with("harman_overear_2019.csv"));
    }
}
