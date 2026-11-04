//! Port of Python `spinorama.compute.estimates` (+ the two
//! `compute.misc` helpers it uses) for `generate-meta`.
//!
//! Produces the `estimates` metadata dictionary: reference band
//! fall-off frequencies, reference level/band, sensitivity delta,
//! Sound Power DI spread, directivity angles, and per-curve
//! slope/smoothness.

// Rust guideline compliant 2026-02-21

use serde_json::{Map, Value};

use crate::compute::cea2034::Cea2034;
use crate::measurements::SplPlane;

const MIDRANGE_MIN_FREQ: f64 = 300.0;
const MIDRANGE_MAX_FREQ: f64 = 5000.0;
const SENSITIVITY_MIN_FREQ: f64 = 300.0;
const SENSITIVITY_MAX_FREQ: f64 = 3000.0;
const DIRECTIVITY_MIN_FREQ: f64 = 1000.0;
const DIRECTIVITY_MAX_FREQ: f64 = 10000.0;
const SLOPE_MIN_FREQ: f64 = 100.0;
const SLOPE_MAX_FREQ: f64 = 12000.0;

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn band(freq: &[f64], values: &[f64], min: f64, max: f64, inclusive_max: bool) -> Vec<f64> {
    freq.iter()
        .zip(values)
        .filter(|(f, _)| **f >= min && (if inclusive_max { **f <= max } else { **f < max }))
        .map(|(_, v)| *v)
        .collect()
}

/// Least-squares regression of SPL against log10(freq), with the
/// VituixCAD-style normalized smoothness (port of
/// `compute_smoothness_regression`).
///
/// Returns `(slope, intercept, smoothness)` describing the original
/// response; smoothness is clamped to `[0, 1]`.
#[must_use]
pub fn smoothness_regression(freq: &[f64], spl: &[f64]) -> (f64, f64, f64) {
    let n = freq.len();
    if n == 0 || n != spl.len() {
        return (0.0, 0.0, 0.0);
    }
    let x: Vec<f64> = freq.iter().map(|f| f.log10()).collect();
    let x_mean = mean(&x);
    let y_mean = mean(spl);
    let ss_xx: f64 = x.iter().map(|v| (v - x_mean).powi(2)).sum();
    let ss_yy: f64 = spl.iter().map(|v| (v - y_mean).powi(2)).sum();
    if ss_xx == 0.0 || ss_yy == 0.0 {
        return (0.0, y_mean, 1.0);
    }
    let ss_xy: f64 = x
        .iter()
        .zip(spl)
        .map(|(xv, yv)| (xv - x_mean) * (yv - y_mean))
        .sum();
    let slope = ss_xy / ss_xx;
    let intercept = y_mean - slope * x_mean;
    // Reference slope of -1 against ln(f) is -ln(10) per decade.
    let reference = -std::f64::consts::LN_10;
    let normalized: Vec<f64> = x
        .iter()
        .zip(spl)
        .map(|(xv, yv)| yv + xv * (reference - slope))
        .collect();
    let ny_mean = mean(&normalized);
    let nss_yy: f64 = normalized.iter().map(|v| (v - ny_mean).powi(2)).sum();
    let nss_xy: f64 = x
        .iter()
        .zip(&normalized)
        .map(|(xv, yv)| (xv - x_mean) * (yv - ny_mean))
        .sum();
    let n_slope = nss_xy / ss_xx;
    let n_intercept = ny_mean - n_slope * x_mean;
    let ss_res: f64 = x
        .iter()
        .zip(&normalized)
        .map(|(xv, yv)| (yv - (n_intercept + n_slope * xv)).powi(2))
        .sum();
    let smoothness = if nss_yy == 0.0 {
        0.0
    } else {
        (1.0 - ss_res / nss_yy).clamp(0.0, 1.0)
    };
    (slope, intercept, smoothness)
}

/// Slope in dB/octave and smoothness over the slope band.
///
/// Ports `compute_slope_smoothness` (also used by the AutoEQ error
/// histograms for their title slopes).
#[must_use]
pub fn slope_smoothness(freq: &[f64], spl: &[f64]) -> (f64, f64) {
    let lo = SLOPE_MIN_FREQ.max(freq.first().copied().unwrap_or(0.0));
    let hi = SLOPE_MAX_FREQ.min(freq.last().copied().unwrap_or(0.0));
    let pairs: Vec<(f64, f64)> = freq
        .iter()
        .zip(spl)
        .filter(|(f, _)| **f >= lo && **f <= hi)
        .map(|(f, v)| (*f, *v))
        .collect();
    if pairs.len() < 2 {
        return (0.0, 0.0);
    }
    let f: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let s: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    let (slope, _, smoothness) = smoothness_regression(&f, &s);
    let first = f[0];
    let last = f[f.len() - 1];
    let octaves = (last / first).log2();
    let slope_db_oct = if octaves > 0.0 {
        slope * (last.log10() - first.log10()) / octaves
    } else {
        0.0
    };
    (slope_db_oct, smoothness)
}

fn ewm_mean_min(values: &[f64], span: f64) -> f64 {
    // pandas ewm(span, adjust=True).mean(), then min: at step i the
    // weights are (1-alpha)^(i-j) for j <= i.
    let alpha = 2.0 / (span + 1.0);
    let mut min = f64::INFINITY;
    let mut weighted = 0.0;
    let mut weights = 0.0;
    for value in values {
        weighted = weighted * (1.0 - alpha) + value;
        weights = weights * (1.0 - alpha) + 1.0;
        let current = weighted / weights;
        if current < min {
            min = current;
        }
    }
    min
}

fn directivity_deg(plane: &SplPlane) -> Option<(f64, f64, f64)> {
    let on_axis = plane.column("On Axis")?;
    let mean_on = mean(&band(
        &plane.freq,
        on_axis,
        DIRECTIVITY_MIN_FREQ,
        DIRECTIVITY_MAX_FREQ,
        false,
    ));
    // Python indexes spl[key] directly: a missing angle column aborts
    // the whole orientation (KeyError caught by the caller).
    let side = |sign: f64| -> Option<i32> {
        let mut angle = 10.0 * sign;
        while angle.abs() < 180.0 {
            let key = format!("{}°", angle as i32);
            let curve = plane.column(&key)?;
            let db: Vec<f64> = plane
                .freq
                .iter()
                .zip(curve)
                .filter(|(f, _)| **f >= DIRECTIVITY_MIN_FREQ && **f <= DIRECTIVITY_MAX_FREQ)
                .map(|(_, v)| v - mean_on)
                .collect();
            if !db.is_empty() && ewm_mean_min(&db, 10.0) < -6.0 {
                return Some(angle as i32);
            }
            angle += 10.0 * sign;
        }
        Some(0)
    };
    let pos = side(1.0)?;
    let neg = side(-1.0)?;
    Some((f64::from(pos), f64::from(neg), f64::from(pos - neg) / 2.0))
}

/// Computes the full `estimates` dictionary for one spin.
#[must_use]
pub fn estimates(cea: &Cea2034, horizontal: &SplPlane, vertical: &SplPlane) -> Map<String, Value> {
    estimates_with_planes(cea, Some((horizontal, vertical)))
}

/// Computes `estimates` for curve-only measurements (no SPL planes).
///
/// Matches Python `estimates(spin, None, None)`: reference values,
/// sensitivity delta, SPDI spread and slopes, but no directivity
/// angles.
#[must_use]
pub fn estimates_cea(cea: &Cea2034) -> Map<String, Value> {
    estimates_with_planes(cea, None)
}

fn estimates_with_planes(
    cea: &Cea2034,
    planes: Option<(&SplPlane, &SplPlane)>,
) -> Map<String, Value> {
    let mut est = Map::new();
    let freq = &cea.freq;
    let on_axis = &cea.on_axis;

    // Reference levels from the on-axis curve.
    let midrange: Vec<f64> = band(freq, on_axis, MIDRANGE_MIN_FREQ, MIDRANGE_MAX_FREQ, true);
    let y_ref = mean(&midrange);
    let restricted: Vec<(f64, f64)> = freq
        .iter()
        .zip(on_axis)
        .filter(|(f, _)| **f < MIDRANGE_MIN_FREQ)
        .map(|(f, v)| (*f, *v))
        .collect();
    for (name, delta) in [
        ("ref_3dB", 3.0),
        ("ref_6dB", 6.0),
        ("ref_9dB", 9.0),
        ("ref_12dB", 12.0),
    ] {
        if !restricted.is_empty() {
            // Python uses np.argmin over the boolean array
            // `dB < y_ref - delta`; argmin returns the first False,
            // i.e. the first frequency at/above the threshold, and 0
            // when the array is uniform.
            let idx = restricted
                .iter()
                .position(|(_, v)| *v >= y_ref - delta)
                .unwrap_or(0);
            let value = restricted[idx].0;
            est.insert(name.to_owned(), Value::from((value * 10.0).round() / 10.0));
        }
    }
    if !midrange.is_empty() {
        let up = midrange.iter().copied().fold(f64::MIN, f64::max);
        let down = midrange.iter().copied().fold(f64::MAX, f64::min);
        let band_db = (up - y_ref).abs().max((y_ref - down).abs());
        est.insert("ref_from".to_owned(), Value::from(MIDRANGE_MIN_FREQ));
        est.insert("ref_to".to_owned(), Value::from(MIDRANGE_MAX_FREQ));
        est.insert(
            "ref_level".to_owned(),
            Value::from((y_ref * 10.0).round() / 10.0),
        );
        est.insert(
            "ref_band".to_owned(),
            Value::from((band_db * 10.0).round() / 10.0),
        );
    }
    let sensitivity = band(
        freq,
        on_axis,
        SENSITIVITY_MIN_FREQ,
        SENSITIVITY_MAX_FREQ,
        true,
    );
    if !sensitivity.is_empty() {
        est.insert(
            "sensitivity_delta".to_owned(),
            Value::from(mean(&sensitivity)),
        );
    }
    // Sound Power DI spread (pandas std, ddof=1).
    let spdi = &cea.sound_power_di;
    if spdi.len() > 1 {
        let m = mean(spdi);
        let var: f64 = spdi.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (spdi.len() - 1) as f64;
        est.insert(
            "directivity_spdi_stddev".to_owned(),
            Value::from(var.sqrt()),
        );
    }
    if let Some((horizontal, vertical)) = planes {
        for (label, plane) in [("horizontal", horizontal), ("vertical", vertical)] {
            if let Some((pos, neg, avg)) = directivity_deg(plane) {
                est.insert(format!("directivity_{label}_pos"), Value::from(pos));
                est.insert(format!("directivity_{label}_neg"), Value::from(neg));
                est.insert(format!("directivity_{label}_avg"), Value::from(avg));
            }
        }
    }
    for (key, curve) in [
        ("on_axis", &cea.on_axis),
        ("listening_window", &cea.listening_window),
        ("sound_power", &cea.sound_power),
        ("early_reflections", &cea.early_reflections),
        ("sound_power_di", &cea.sound_power_di),
        ("early_reflections_di", &cea.early_reflections_di),
    ] {
        let (slope, smoothness) = slope_smoothness(freq, curve);
        est.insert(format!("slope_{key}"), Value::from(slope));
        est.insert(format!("smoothness_{key}"), Value::from(smoothness));
    }
    est
}
