//! Olive preference scores from CEA2034 curves.
//!
//! Delegates to Rust AutoEQ (`autoeq_workflow::cea2034::score`), which
//! implements the same formula as Python `compute/scores.py`:
//! `pref = 12.69 - 2.49·NBD_ON - 2.99·NBD_PIR - 4.31·LFX + 2.32·SM_PIR`.

// Rust guideline compliant 2026-02-21

use anyhow::{Result, bail};
use autoeq_workflow::cea2034::{ScoreMetrics, octave_intervals, score};
use autoeq_workflow::iir::Peq;
use ndarray::Array1;

use crate::compute::cea2034::Cea2034;
use crate::filters::biquad::apply_peq;

fn to_array(values: &[f64]) -> Array1<f64> {
    Array1::from_vec(values.to_vec())
}

/// Computes preference-score metrics for a CEA2034 curve set.
///
/// # Errors
///
/// Returns an error when the curve lengths disagree with the
/// frequency axis.
pub fn preference_score(cea: &Cea2034) -> Result<ScoreMetrics> {
    let n = cea.freq.len();
    for (name, curve) in [
        ("On Axis", &cea.on_axis),
        ("Listening Window", &cea.listening_window),
        ("Sound Power", &cea.sound_power),
        ("Estimated In-Room Response", &cea.estimated_inroom),
    ] {
        if curve.len() != n {
            bail!("{name} has {} points, frequency axis has {n}", curve.len());
        }
    }
    let freq = to_array(&cea.freq);
    // Two bands per octave, matching Python scores.py via AutoEQ's
    // own cea2034 metrics path (compute.rs).
    let intervals = octave_intervals(2, &freq);
    Ok(score(
        &freq,
        &intervals,
        &to_array(&cea.on_axis),
        &to_array(&cea.listening_window),
        &to_array(&cea.sound_power),
        &to_array(&cea.estimated_inroom),
    ))
}

/// `floor(x * 10^d) / 10^d` for positive values (Python `round_down`).
#[must_use]
pub fn round_down(x: f64, decimals: i32) -> f64 {
    if x > 0.0 && decimals <= 5 {
        let mul = 10f64.powi(decimals);
        (x * mul).floor() / mul
    } else {
        let mul = 10f64.powi(decimals);
        (x * mul).round() / mul
    }
}

fn octave_bands(count: i32) -> Vec<(f64, f64, f64)> {
    let reference = 1290.0;
    let p = 2f64.powf(1.0 / f64::from(count));
    let p_band = 2f64.powf(1.0 / (2.0 * f64::from(count)));
    let o_iter = (count * 10 + 1) / 2;
    let mut centers: Vec<f64> = (1..=o_iter).rev().map(|i| reference / p.powi(i)).collect();
    centers.push(reference);
    centers.extend((1..=o_iter).map(|i| reference * p.powi(i)));
    centers
        .into_iter()
        .map(|c| (c / p_band, c, c * p_band))
        .collect()
}

fn mean_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn band_values(freq: &[f64], curve: &[f64], min: f64, max: f64, max_inclusive: bool) -> Vec<f64> {
    freq.iter()
        .zip(curve)
        .filter(|(f, _)| **f >= min && (if max_inclusive { **f <= max } else { **f < max }))
        .map(|(_, v)| *v)
        .collect()
}

fn mad(values: &[f64]) -> f64 {
    let m = mean_of(values);
    mean_of(&values.iter().map(|v| (v - m).abs()).collect::<Vec<_>>())
}

fn nbd(freq: &[f64], curve: &[f64], min_freq: f64) -> Option<f64> {
    let band_min_freq = 100f64.max(min_freq);
    let mut sum = 0.0;
    let mut n = 0usize;
    for (band_min, band_center, band_max) in octave_bands(2) {
        if !(band_min_freq <= band_center && band_center <= 12000.0) {
            continue;
        }
        let values = band_values(freq, curve, band_min, band_max, true);
        if values.is_empty() {
            continue;
        }
        sum += mad(&values);
        n += 1;
    }
    if n == 0 { None } else { Some(sum / n as f64) }
}

fn aad(freq: &[f64], curve: &[f64], min_freq: f64) -> f64 {
    let y_ref = mean_of(&band_values(freq, curve, 200.0, 400.0, true));
    let band_min_freq = 100f64.max(min_freq);
    let mut sum = 0.0;
    let mut n = 0usize;
    for (band_min, _, band_max) in octave_bands(20) {
        if band_min < band_min_freq || band_max > 16000.0 {
            continue;
        }
        // Python filters on band_min_freq (not band_min) here.
        let mean = mean_of(&band_values(freq, curve, band_min_freq, band_max, false));
        sum += (y_ref - mean).abs();
        n += 1;
    }
    if n == 0 { -1.0 } else { sum / n as f64 }
}

fn lfx(freq: &[f64], lw: &[f64], sp: &[f64]) -> f64 {
    let lw_ref = mean_of(&band_values(freq, lw, 300.0, 10000.0, true)) - 6.0;
    let candidates: Vec<usize> = freq
        .iter()
        .zip(sp)
        .enumerate()
        .filter(|(_, (f, v))| **f <= 300.0 && **v <= lw_ref)
        .map(|(i, _)| i)
        .collect();
    let Some(first) = candidates.first() else {
        return freq.first().copied().unwrap_or(300.0).log10();
    };
    // First consecutive group, take its last index, then the next freq.
    let mut last = *first;
    for idx in &candidates[1..] {
        if *idx == last + 1 {
            last = *idx;
        } else {
            break;
        }
    }
    match freq.get(last + 1) {
        Some(hz) => hz.log10(),
        None => 300f64.log10(),
    }
}

fn lfq(freq: &[f64], lw: &[f64], sp: &[f64], lfx_log: f64) -> f64 {
    let val_lfx = 10f64.powf(lfx_log);
    let mut sum = 0.0;
    let mut n = 0usize;
    for (band_min, _, band_max) in octave_bands(2) {
        if band_min < val_lfx || band_max > 300.0 {
            continue;
        }
        let lw_band = band_values(freq, lw, band_min, band_max, false);
        let sp_band = band_values(freq, sp, band_min, band_max, false);
        if !lw_band.is_empty() && !sp_band.is_empty() {
            sum += (mean_of(&lw_band) - mean_of(&sp_band)).abs();
            n += 1;
        }
    }
    if n == 0 { 10.0 } else { sum / n as f64 }
}

fn sm(freq: &[f64], curve: &[f64]) -> f64 {
    let pairs: Vec<(f64, f64)> = freq
        .iter()
        .zip(curve)
        .filter(|(f, _)| (100.0..=16000.0).contains(*f))
        .map(|(f, v)| (*f, *v))
        .collect();
    let f: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let s: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    crate::compute::estimates::smoothness_regression(&f, &s).2
}

fn pref_rating(nbd_on: f64, nbd_pir: f64, lf_x: f64, sm_pir: f64) -> f64 {
    12.69 - 2.49 * nbd_on - 2.99 * nbd_pir - 4.31 * lf_x + 2.32 * sm_pir
}

/// Raw (unrounded) preference-rating components.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PrefValues {
    nbd_on: f64,
    nbd_lw: f64,
    nbd_sp: f64,
    nbd_pir: f64,
    aad_on: f64,
    lfx_log: f64,
    lfq_db: f64,
    sm_sp: f64,
    sm_pir: f64,
    pref_wsub: f64,
    pref: f64,
}

fn pref_values(cea: &Cea2034) -> Option<PrefValues> {
    let freq = &cea.freq;
    if freq.is_empty() || cea.estimated_inroom.is_empty() || freq[0] >= 40.0 {
        return None;
    }
    let min_freq = freq[0];
    let nbd_on = nbd(freq, &cea.on_axis, min_freq)?;
    let nbd_lw = nbd(freq, &cea.listening_window, min_freq)?;
    let nbd_sp = nbd(freq, &cea.sound_power, min_freq)?;
    let nbd_pir = nbd(freq, &cea.estimated_inroom, min_freq)?;
    let aad_on = aad(freq, &cea.on_axis, min_freq);
    let lfx_log = lfx(freq, &cea.listening_window, &cea.sound_power);
    let lfq_db = lfq(freq, &cea.listening_window, &cea.sound_power, lfx_log);
    let sm_sp = sm(freq, &cea.sound_power);
    let sm_pir = sm(freq, &cea.estimated_inroom);
    Some(PrefValues {
        nbd_on,
        nbd_lw,
        nbd_sp,
        nbd_pir,
        aad_on,
        lfx_log,
        lfq_db,
        sm_sp,
        sm_pir,
        pref_wsub: pref_rating(nbd_on, nbd_pir, 14.5f64.log10(), sm_pir),
        pref: pref_rating(nbd_on, nbd_pir, lfx_log, sm_pir),
    })
}

/// Computes the rounded `pref_rating` metadata dictionary exactly as
/// Python `speaker_pref_rating(..., rounded=True)`.
///
/// Returns `None` when the curves cannot be scored (empty PIR, or the
/// frequency axis starts at or above 40 Hz).
#[must_use]
pub fn pref_rating_dict(cea: &Cea2034) -> Option<serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;
    let v = pref_values(cea)?;
    let mut out = serde_json::Map::new();
    for (key, value) in [
        ("nbd_on_axis", round_down(v.nbd_on, 3)),
        ("nbd_listening_window", round_down(v.nbd_lw, 3)),
        ("nbd_sound_power", round_down(v.nbd_sp, 3)),
        ("nbd_pred_in_room", round_down(v.nbd_pir, 3)),
        ("sm_pred_in_room", round_down(v.sm_pir, 3)),
        ("sm_sound_power", round_down(v.sm_sp, 3)),
        ("pref_score_wsub", round_down(v.pref_wsub, 2)),
    ] {
        out.insert(key.to_owned(), Value::from(value));
    }
    out.insert(
        "lfx_hz".to_owned(),
        Value::from(10f64.powf(v.lfx_log) as i64),
    );
    out.insert("lfq".to_owned(), Value::from(round_down(v.lfq_db, 3)));
    out.insert("pref_score".to_owned(), Value::from(round_down(v.pref, 2)));
    if v.aad_on != 0.0 && v.aad_on != -1.0 {
        out.insert(
            "aad_on_axis".to_owned(),
            Value::from(round_down(v.aad_on, 3)),
        );
    }
    Some(out)
}

/// Computes the unrounded `pref_rating` dictionary exactly as Python
/// `speaker_pref_rating(..., rounded=False)`, as used by
/// `scores_apply_filter` for EQ traces in `generate_radar.py`.
///
/// Returns `None` under the same conditions as [`pref_rating_dict`].
#[must_use]
pub fn pref_rating_dict_unrounded(
    cea: &Cea2034,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;
    let v = pref_values(cea)?;
    let mut out = serde_json::Map::new();
    for (key, value) in [
        ("nbd_on_axis", v.nbd_on),
        ("nbd_listening_window", v.nbd_lw),
        ("nbd_sound_power", v.nbd_sp),
        ("nbd_pred_in_room", v.nbd_pir),
        ("sm_pred_in_room", v.sm_pir),
        ("sm_sound_power", v.sm_sp),
        ("pref_score_wsub", v.pref_wsub),
        ("pref_score", v.pref),
        ("lfx_hz", 10f64.powf(v.lfx_log)),
        ("lfq", v.lfq_db),
    ] {
        out.insert(key.to_owned(), Value::from(value));
    }
    // Python rounds `aad_on_axis` even in the unrounded branch.
    if v.aad_on != 0.0 && v.aad_on != -1.0 {
        out.insert(
            "aad_on_axis".to_owned(),
            Value::from(round_down(v.aad_on, 3)),
        );
    }
    Some(out)
}

/// Computes preference-score metrics after applying `peq` to every
/// CEA2034 curve (the same filter shifts all curves, as in Python).
///
/// # Errors
///
/// Propagates the length checks of [`preference_score`].
pub fn preference_score_with_peq(cea: &Cea2034, peq: &Peq) -> Result<ScoreMetrics> {
    let shifted = Cea2034 {
        freq: cea.freq.clone(),
        on_axis: apply_peq(&cea.freq, &cea.on_axis, peq),
        listening_window: apply_peq(&cea.freq, &cea.listening_window, peq),
        early_reflections: apply_peq(&cea.freq, &cea.early_reflections, peq),
        sound_power: apply_peq(&cea.freq, &cea.sound_power, peq),
        early_reflections_di: cea.early_reflections_di.clone(),
        sound_power_di: cea.sound_power_di.clone(),
        estimated_inroom: apply_peq(&cea.freq, &cea.estimated_inroom, peq),
        sensitivity_db: cea.sensitivity_db,
    };
    preference_score(&shifted)
}
