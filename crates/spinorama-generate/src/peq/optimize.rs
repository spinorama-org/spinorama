//! PEQ optimization adapter over Rust AutoEQ (`autoeq-workflow`).
//!
//! Mirrors the AutoEQ CLI CSV path: build `Curve`s, call
//! `setup_objective_data`, then the synchronous
//! `perform_optimization`, then `x2peq`. Python `generate_peqs.py`
//! defaults override Rust AutoEQ defaults (design §4): 7 filters,
//! Q ∈ [0.25, 3], signed gain ∈ [−3, +3] dB (Python's 0.75 dB minimum
//! magnitude has no signed-bound equivalent and is a named gap),
//! filter frequencies 20–20000 Hz at 48 kHz.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use autoeq_workflow::iir::{self, Peq};
use autoeq_workflow::{
    Curve, LossType, OptimParams, OptimizationConfig, PeqModel, perform_optimization,
    setup_objective_data, x2peq,
};
use ndarray::Array1;

use crate::compute::cea2034::{Cea2034, estimated_inroom_from};
use crate::compute::scores::preference_score_with_peq;

/// Python `generate_peqs.py` defaults (design §4).
pub const DEFAULT_MAX_PEQ: usize = 7;
/// Python `MIN_Q`.
pub const DEFAULT_MIN_Q: f64 = 0.25;
/// Python `MAX_Q`.
pub const DEFAULT_MAX_Q: f64 = 3.0;
/// Python `MAX_DBGAIN` as a signed bound.
pub const DEFAULT_MAX_DB: f64 = 3.0;
/// Python sample rate (`fs`).
pub const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;

/// Named CEA2034 curves as `(freq, spl)` pairs on a shared axis.
pub type SpinCurves = HashMap<String, (Vec<f64>, Vec<f64>)>;

/// Python `--fitness` selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fitness {
    /// Minimize deviation from the slope target.
    Flat,
    /// Maximize the Olive preference score.
    Score,
    /// Preference score plus a flatness penalty.
    Combine,
    /// Flatten the estimated in-room response.
    FlatPir,
}

/// Python `--optimisation` selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optimisation {
    /// Optimize all filters jointly.
    Global,
    /// Add one filter at a time against the residual.
    Greedy,
}

/// Knobs for one PEQ optimization run.
#[derive(Debug, Clone)]
pub struct PeqOptions {
    /// Number of peaking filters.
    pub num_filters: usize,
    /// Python fitness selection.
    pub fitness: Fitness,
    /// Optimisation strategy.
    pub optimisation: Optimisation,
    /// Rust AutoEQ algorithm name (for example `autoeq:de`).
    pub algo: String,
    /// Rust AutoEQ PEQ model (filter-type structure).
    pub peq_model: PeqModel,
    /// Loss selection passed to Rust AutoEQ for global runs.
    pub loss: LossType,
    /// Minimum filter Q.
    pub min_q: f64,
    /// Maximum filter Q.
    pub max_q: f64,
    /// Minimum signed filter gain in dB.
    pub min_db: f64,
    /// Maximum signed filter gain in dB.
    pub max_db: f64,
    /// Minimum filter frequency in hertz.
    pub min_freq: f64,
    /// Maximum filter frequency in hertz.
    pub max_freq: f64,
    /// Sample rate in hertz.
    pub sample_rate: f64,
    /// Optimizer evaluation budget.
    pub maxeval: usize,
    /// DE population size.
    pub population: usize,
    /// Stochastic seed (fixed by default for reproducibility).
    pub seed: Option<u64>,
}

impl Default for PeqOptions {
    fn default() -> Self {
        Self {
            num_filters: DEFAULT_MAX_PEQ,
            fitness: Fitness::Flat,
            optimisation: Optimisation::Global,
            algo: "autoeq:de".to_owned(),
            peq_model: PeqModel::Pk,
            loss: LossType::SpeakerFlat,
            min_q: DEFAULT_MIN_Q,
            max_q: DEFAULT_MAX_Q,
            min_db: -DEFAULT_MAX_DB,
            max_db: DEFAULT_MAX_DB,
            min_freq: 20.0,
            max_freq: 20_000.0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            maxeval: 10_000,
            population: 100,
            seed: Some(42),
        }
    }
}

/// Maps a Python `--fitness` value to a [`Fitness`].
///
/// # Errors
///
/// Returns an error for unmapped fitness names.
pub fn fitness_from_name(fitness: &str) -> Result<Fitness> {
    Ok(match fitness.to_lowercase().as_str() {
        "flat" | "leastsquare" | "least_square" | "leastsquares" | "flat_loss"
        | "leastsquare_loss" | "speaker-flat" => Fitness::Flat,
        "score" | "score_loss" | "speaker-score" => Fitness::Score,
        "combine" | "combine_loss" => Fitness::Combine,
        "flatpir" | "flat_pir" | "flat-pir" => Fitness::FlatPir,
        other => bail!(
            "unsupported --fitness {other} (supported: Flat, LeastSquare, Score, Combine, FlatPir)"
        ),
    })
}

/// Maps a Python `--fitness` value to the Rust AutoEQ global loss.
///
/// `Combine` uses the score objective globally (its flatness term is
/// applied by the greedy/grapheq selectors); `FlatPir` uses the flat
/// objective on the EIR curve selected by the caller.
///
/// # Errors
///
/// Returns an error for unmapped fitness names.
pub fn loss_from_fitness(fitness: &str) -> Result<LossType> {
    Ok(match fitness_from_name(fitness)? {
        Fitness::Flat | Fitness::FlatPir => LossType::SpeakerFlat,
        Fitness::Score | Fitness::Combine => LossType::SpeakerScore,
    })
}

fn make_curve(freq: &[f64], spl: &[f64]) -> Curve {
    Curve {
        freq: Array1::from_vec(freq.to_vec()),
        spl: Array1::from_vec(spl.to_vec()),
        phase: None,
        ..Default::default()
    }
}

fn optimize_global(
    freq: &[f64],
    measured_spl: &[f64],
    target_spl: &[f64],
    spin: Option<&SpinCurves>,
    options: &PeqOptions,
) -> Result<Peq> {
    if freq.len() != measured_spl.len() || freq.len() != target_spl.len() || freq.is_empty() {
        bail!(
            "freq/measured/target length mismatch: {}/{}/{}",
            freq.len(),
            measured_spl.len(),
            target_spl.len()
        );
    }
    if options.loss == LossType::SpeakerScore && spin.is_none() {
        bail!("--fitness score needs CEA2034 curves (use --cea2034-json or --speaker-dir)");
    }

    let mut config = OptimizationConfig::speaker_defaults();
    config.num_filters = options.num_filters;
    config.loss = options.loss;
    config.min_q = options.min_q;
    config.max_q = options.max_q;
    config.min_db = options.min_db;
    config.max_db = options.max_db;
    config.min_freq = options.min_freq;
    config.max_freq = options.max_freq;
    config.sample_rate = options.sample_rate;
    config.maxeval = options.maxeval;
    config.population = options.population;
    config.seed = options.seed;
    config.algo = options.algo.clone();
    config.peq_model = options.peq_model;
    let params = OptimParams::from(&config);

    let input = make_curve(freq, measured_spl);
    let target = make_curve(freq, target_spl);
    let deviation_spl: Vec<f64> = target_spl
        .iter()
        .zip(measured_spl)
        .map(|(t, m)| t - m)
        .collect();
    let deviation = make_curve(freq, &deviation_spl);
    let spin_curves: Option<HashMap<String, Curve>> = spin.map(|set| {
        set.iter()
            .map(|(name, (f, s))| (name.clone(), make_curve(f, s)))
            .collect()
    });

    let (objective_data, _use_cea) =
        setup_objective_data(&params, &input, &target, &deviation, &spin_curves)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("setting up AutoEQ objective data")?;
    let x = perform_optimization(&params, &objective_data)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("running AutoEQ optimization")?;
    Ok(x2peq(&x, options.sample_rate, options.peq_model))
}

fn peq_response(freq: &[f64], peq: &Peq) -> Vec<f64> {
    iir::peq_spl(&Array1::from_vec(freq.to_vec()), peq).to_vec()
}

fn cea_from_spin(spin: &SpinCurves, freq: &[f64]) -> Result<Cea2034> {
    let get = |name: &str| -> Result<Vec<f64>> {
        spin.get(name)
            .map(|(_, spl)| spl.clone())
            .ok_or_else(|| anyhow::anyhow!("spin curves are missing {name}"))
    };
    let on_axis = get("On Axis")?;
    let listening_window = get("Listening Window")?;
    let early_reflections = get("Early Reflections")?;
    let sound_power = get("Sound Power")?;
    let estimated_inroom = match spin.get("Estimated In-Room Response") {
        Some((_, spl)) => spl.clone(),
        None => estimated_inroom_from(&listening_window, &early_reflections, &sound_power),
    };
    Ok(Cea2034 {
        freq: freq.to_vec(),
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
        on_axis,
        listening_window,
        early_reflections,
        sound_power,
        estimated_inroom,
        sensitivity_db: 0.0,
    })
}

fn corrected_spin(spin: &SpinCurves, freq: &[f64], peq: &Peq) -> SpinCurves {
    let response = peq_response(freq, peq);
    spin.iter()
        .map(|(name, (curve_freq, spl))| {
            let corrected: Vec<f64> = spl
                .iter()
                .zip(&response)
                .map(|(value, delta)| value + delta)
                .collect();
            (name.clone(), (curve_freq.clone(), corrected))
        })
        .collect()
}

fn selection_metric(
    freq: &[f64],
    measured_spl: &[f64],
    target_spl: &[f64],
    spin: Option<&SpinCurves>,
    peq: &Peq,
    options: &PeqOptions,
) -> Result<f64> {
    let response = peq_response(freq, peq);
    let corrected: Vec<f64> = measured_spl
        .iter()
        .zip(&response)
        .map(|(value, delta)| value + delta)
        .collect();
    let rms = rms_deviation(
        freq,
        &corrected,
        target_spl,
        options.min_freq,
        options.max_freq,
    )?;
    match options.fitness {
        Fitness::Flat | Fitness::FlatPir => Ok(rms),
        Fitness::Score | Fitness::Combine => {
            let Some(spin) = spin else {
                return Ok(rms);
            };
            let cea = cea_from_spin(spin, freq)?;
            let score = preference_score_with_peq(&cea, peq)?.pref_score;
            if options.fitness == Fitness::Combine {
                Ok(-score + rms / 20.0)
            } else {
                Ok(-score)
            }
        }
    }
}

fn optimize_greedy(
    freq: &[f64],
    measured_spl: &[f64],
    target_spl: &[f64],
    spin: Option<&SpinCurves>,
    options: &PeqOptions,
) -> Result<Peq> {
    let mut peq: Peq = Vec::new();
    let mut best_peq: Peq = Vec::new();
    let mut best_metric = selection_metric(freq, measured_spl, target_spl, spin, &peq, options)?;
    for _round in 0..options.num_filters {
        let response = peq_response(freq, &peq);
        let corrected: Vec<f64> = measured_spl
            .iter()
            .zip(&response)
            .map(|(value, delta)| value + delta)
            .collect();
        let corrected_spin_map;
        let spin_arg = match spin {
            Some(set) => {
                corrected_spin_map = corrected_spin(set, freq, &peq);
                Some(&corrected_spin_map)
            }
            None => None,
        };
        let mut one = options.clone();
        one.num_filters = 1;
        one.optimisation = Optimisation::Global;
        one.maxeval = (options.maxeval / options.num_filters.max(1)).max(500);
        if matches!(options.fitness, Fitness::Combine | Fitness::FlatPir) {
            one.loss = LossType::SpeakerFlat;
        }
        let candidate = optimize_global(freq, &corrected, target_spl, spin_arg, &one)?;
        if candidate.is_empty() {
            break;
        }
        let mut next = peq.clone();
        next.extend(candidate);
        let metric = selection_metric(freq, measured_spl, target_spl, spin, &next, options)?;
        let flat_metric = selection_metric(
            freq,
            measured_spl,
            target_spl,
            spin,
            &next,
            &PeqOptions {
                fitness: Fitness::Flat,
                ..options.clone()
            },
        )?;
        let previous_flat = selection_metric(
            freq,
            measured_spl,
            target_spl,
            spin,
            &peq,
            &PeqOptions {
                fitness: Fitness::Flat,
                ..options.clone()
            },
        )?;
        peq = next;
        if metric < best_metric {
            best_metric = metric;
            best_peq.clone_from(&peq);
        }
        if options.fitness == Fitness::Flat && flat_metric >= previous_flat - 1e-4 {
            break;
        }
    }
    if matches!(options.fitness, Fitness::Score | Fitness::Combine) && spin.is_some() {
        Ok(best_peq)
    } else {
        Ok(peq)
    }
}

/// Optimizes a PEQ for one curve against an absolute target.
///
/// `spin` carries the full CEA2034 set (keys `On Axis`,
/// `Listening Window`, `Early Reflections`, `Sound Power`,
/// `Estimated In-Room Response`) and is required for `SpeakerScore`;
/// pass `None` for flat losses on a single curve.
///
/// # Errors
///
/// Returns an error on length mismatches, missing spin data for the
/// score loss, or optimizer failure.
pub fn optimize_peq(
    freq: &[f64],
    measured_spl: &[f64],
    target_spl: &[f64],
    spin: Option<&SpinCurves>,
    options: &PeqOptions,
) -> Result<Peq> {
    match options.optimisation {
        Optimisation::Global => optimize_global(freq, measured_spl, target_spl, spin, options),
        Optimisation::Greedy => optimize_greedy(freq, measured_spl, target_spl, spin, options),
    }
}

/// Returns the RMS deviation (dB) between two curves over a band.
///
/// Used as the acceptance metric for optimized PEQs: Python's DE is
/// unseeded, so bit-identical filters are never claimed (design §8).
///
/// # Errors
///
/// Returns an error when the band contains no points.
pub fn rms_deviation(
    freq: &[f64],
    a: &[f64],
    b: &[f64],
    min_freq: f64,
    max_freq: f64,
) -> Result<f64> {
    let mut sum_sq = 0.0;
    let mut count = 0usize;
    for ((f, x), y) in freq.iter().zip(a).zip(b) {
        if *f >= min_freq && *f <= max_freq {
            sum_sq += (x - y) * (x - y);
            count += 1;
        }
    }
    if count == 0 {
        bail!("no points in band {min_freq}-{max_freq} Hz");
    }
    #[expect(clippy::cast_precision_loss, reason = "band sizes fit exactly in f64")]
    Ok((sum_sq / count as f64).sqrt())
}
