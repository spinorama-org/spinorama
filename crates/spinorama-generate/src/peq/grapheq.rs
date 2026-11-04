//! Graphic-EQ optimizer for fixed-band hardware equalizers.
//!
//! Port of Python `auto_geq.optim_grapheq`: center frequencies and Q are
//! fixed by the device, and a single scalar scales the interpolated
//! error curve into band gains (rounded to 0.25 dB, bounded by the
//! device gain range, gains below 1 dB dropped).

// Rust guideline compliant 2026-02-21

use anyhow::{Result, bail};
use autoeq_workflow::iir::{Biquad, BiquadFilterType, Peq};

use crate::compute::cea2034::Cea2034;
use crate::compute::scores::preference_score_with_peq;
use crate::filters::biquad::peq_response_db;
use crate::peq::optimize::{Fitness, PeqOptions, rms_deviation};

/// One graphic-EQ device definition.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicEq {
    /// Device name as in Python `datas/grapheq.py`.
    pub name: &'static str,
    /// Fixed band center frequencies in hertz.
    pub bands: &'static [f64],
    /// Fixed Q for every band.
    pub fixed_q: f64,
    /// Maximum boost in dB.
    pub gain_p: f64,
    /// Maximum cut in dB (negative).
    pub gain_m: f64,
}

fn bandwidth2q(bandwidth_oct: f64) -> f64 {
    let pbw = 2f64.powf(bandwidth_oct);
    pbw.sqrt() / (pbw - 1.0)
}

const BANDS_DENON: &[f64] = &[
    63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
const BANDS_13: &[f64] = &[
    25.0, 40.0, 100.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0, 10000.0,
    16000.0,
];
const BANDS_15: &[f64] = &[
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];
const BANDS_31: &[f64] = &[
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12000.0, 16000.0, 20000.0,
];

/// Returns the known graphic-EQ device names.
#[must_use]
pub fn graphic_eq_names() -> Vec<&'static str> {
    vec![
        "Denon AVR",
        "Altair EQ-215",
        "Subzero SZEQ-215",
        "Altair EQ-230",
        "Behringer FBQ3102HD",
        "DBX 1215",
        "DBX 1231",
        "DBX 2231",
        "DBX 321S",
        "KlarkTecknik DN360",
        "KlarkTecknik DN370",
    ]
}

/// Looks up a graphic-EQ device by name.
///
/// # Errors
///
/// Returns an error for an unknown device name.
pub fn graphic_eq(name: &str) -> Result<GraphicEq> {
    let (bands, bandwidth, gain_p, gain_m): (&[f64], f64, f64, f64) = match name {
        "Denon AVR" => (BANDS_DENON, 1.0, 6.0, -20.0),
        "Altair EQ-215" | "Subzero SZEQ-215" => (BANDS_13, 2.0 / 3.0, 6.0, -6.0),
        "DBX 1215" => (BANDS_15, 2.0 / 3.0, 6.0, -6.0),
        "Altair EQ-230"
        | "Behringer FBQ3102HD"
        | "DBX 1231"
        | "DBX 2231"
        | "DBX 321S"
        | "KlarkTecknik DN360"
        | "KlarkTecknik DN370" => (BANDS_31, 1.0 / 3.0, 6.0, -6.0),
        other => bail!(
            "unknown graphic EQ {other:?} (known: {})",
            graphic_eq_names().join(", ")
        ),
    };
    Ok(GraphicEq {
        name: graphic_eq_names()
            .into_iter()
            .find(|known| *known == name)
            .unwrap_or(""),
        bands,
        fixed_q: bandwidth2q(bandwidth),
        gain_p,
        gain_m,
    })
}

fn interp_linear(freq: &[f64], src_freq: &[f64], src_values: &[f64]) -> Vec<f64> {
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

/// Optimizes fixed-band gains for one graphic-EQ device.
///
/// `error` is the Python `peq_target` curve (limited measured curve
/// minus the relative slope line) on `freq`. The scalar search replaces
/// Python's Powell run, which is unreliable on the stepped gain grid.
///
/// # Errors
///
/// Returns an error on length mismatches or missing CEA data for the
/// score fitness.
pub fn optimize_grapheq(
    freq: &[f64],
    error: &[f64],
    cea: Option<&Cea2034>,
    measured_spl: &[f64],
    target_spl: &[f64],
    device: &GraphicEq,
    options: &PeqOptions,
) -> Result<Peq> {
    if freq.len() != error.len() || freq.is_empty() {
        bail!("freq/error length mismatch: {}/{}", freq.len(), error.len());
    }
    if matches!(options.fitness, Fitness::Score | Fitness::Combine) && cea.is_none() {
        bail!("graphic-EQ score fitness needs CEA2034 curves");
    }

    let fit = |param: f64| -> Peq {
        device
            .bands
            .iter()
            .map(|band| {
                let db = if *band < freq[0] || *band > freq[freq.len() - 1] {
                    0.0
                } else {
                    let raw = interp_linear(&[*band], freq, error)[0] * param;
                    let rounded = (raw * 4.0).round() / 4.0;
                    let bounded = if rounded > 0.0 {
                        rounded.min(device.gain_p)
                    } else {
                        rounded.max(device.gain_m)
                    };
                    if bounded.abs() < 1.0 { 0.0 } else { bounded }
                };
                (
                    1.0,
                    Biquad::new(
                        BiquadFilterType::Peak,
                        *band,
                        options.sample_rate,
                        device.fixed_q,
                        db,
                    ),
                )
            })
            .collect()
    };

    let loss = |param: f64| -> Result<f64> {
        let peq = fit(param);
        match options.fitness {
            Fitness::Flat | Fitness::FlatPir => {
                let at_bands = peq_response_db(device.bands, &peq);
                let expended = interp_linear(freq, device.bands, &at_bands);
                Ok(error
                    .iter()
                    .zip(&expended)
                    .map(|(e, p)| (e + p).powi(2))
                    .sum::<f64>()
                    .sqrt())
            }
            Fitness::Score => {
                let cea = cea.ok_or_else(|| anyhow::anyhow!("missing CEA2034"))?;
                Ok(-preference_score_with_peq(cea, &peq)?.pref_score)
            }
            Fitness::Combine => {
                let cea = cea.ok_or_else(|| anyhow::anyhow!("missing CEA2034"))?;
                let score = preference_score_with_peq(cea, &peq)?.pref_score;
                let corrected: Vec<f64> = measured_spl
                    .iter()
                    .zip(peq_response_db(freq, &peq))
                    .map(|(value, delta)| value + delta)
                    .collect();
                let rms = rms_deviation(
                    freq,
                    &corrected,
                    target_spl,
                    options.min_freq,
                    options.max_freq,
                )?;
                Ok(-score + rms / 20.0)
            }
        }
    };

    let (lo, hi) = (device.gain_m, device.gain_p);
    let mut best_param = 0.0;
    let mut best_loss = loss(0.0)?;
    let mut step = (hi - lo) / 240.0;
    for _pass in 0..3 {
        let mut param = if _pass == 0 {
            lo
        } else {
            best_param - step * 10.0
        };
        let end = if _pass == 0 {
            hi
        } else {
            best_param + step * 10.0
        };
        while param <= end {
            let value = loss(param.clamp(lo, hi))?;
            if value < best_loss {
                best_loss = value;
                best_param = param.clamp(lo, hi);
            }
            param += step;
        }
        step /= 10.0;
    }
    Ok(fit(best_param))
}
