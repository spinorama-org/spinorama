//! Slope targets, ported from Python `auto_target.get_target`.
//!
//! Python builds a relative line that is flat below `target_min_freq`
//! and drops by `slope` dB in total between `target_min_freq` and
//! `target_max_freq` (the configured slope is divided by the log10
//! span). Rust AutoEQ optimizes against an absolute target curve, so
//! the line is level-aligned to the measured curve over the Python
//! mean band (100–300 Hz by default).

// Rust guideline compliant 2026-02-21

use anyhow::{Result, bail};

/// Default per-curve slopes from Python `generate_peqs.py`.
///
/// Total dB drop across the target band, per curve name.
pub const DEFAULT_SLOPES: &[(&str, f64)] = &[
    ("On Axis", 0.0),
    ("Listening Window", -0.5),
    ("Early Reflections", -4.0),
    ("Estimated In-Room Response", -6.5),
    ("Sound Power", -9.0),
];

/// Returns the default slope for a CEA2034 curve name.
///
/// # Errors
///
/// Returns an error for unknown curve names (no silent default).
pub fn default_slope(curve_name: &str) -> Result<f64> {
    DEFAULT_SLOPES
        .iter()
        .find(|(name, _)| *name == curve_name)
        .map(|(_, slope)| *slope)
        .ok_or_else(|| anyhow::anyhow!("unknown curve name for slope target: {curve_name}"))
}

/// Finds the −3 dB frequency of a curve relative to its mean-band level.
///
/// The reference is the mean SPL over `[mean_min, mean_max]`; the
/// result is the lowest frequency at or above which the curve first
/// reaches `reference − 3 dB` when scanning upward. Used as the
/// default `target_min_freq`, mirroring Python `use_3dB_target`.
///
/// # Errors
///
/// Returns an error when the mean band contains no points.
pub fn minus_3db_freq(freq: &[f64], spl: &[f64], mean_min: f64, mean_max: f64) -> Result<f64> {
    let band: Vec<f64> = freq
        .iter()
        .zip(spl)
        .filter(|(f, _)| **f >= mean_min && **f <= mean_max)
        .map(|(_, s)| *s)
        .collect();
    if band.is_empty() {
        bail!("no points in mean band {mean_min}-{mean_max} Hz");
    }
    #[expect(clippy::cast_precision_loss, reason = "band sizes fit exactly in f64")]
    let reference = band.iter().sum::<f64>() / band.len() as f64;
    let threshold = reference - 3.0;
    for (f, s) in freq.iter().zip(spl) {
        if *s >= threshold {
            return Ok(*f);
        }
    }
    Ok(*freq.last().unwrap_or(&mean_min))
}

/// Builds Python's relative slope line on the `freq` axis.
///
/// The line is 0 dB at and below `target_min_freq` and drops by
/// `slope_db` in total across the target band. This is the raw
/// `auto_target.get_target` output before level alignment.
///
/// # Errors
///
/// Returns an error when the target band contains no usable frequencies.
pub fn relative_slope_line(
    freq: &[f64],
    slope_db: f64,
    target_min_freq: f64,
    target_max_freq: f64,
) -> Result<Vec<f64>> {
    if freq.is_empty() {
        bail!("empty frequency axis for slope target");
    }
    let first = freq
        .iter()
        .position(|f| *f >= target_min_freq)
        .ok_or_else(|| anyhow::anyhow!("no frequency >= target_min_freq {target_min_freq}"))?;
    let last = freq
        .iter()
        .rposition(|f| *f <= target_max_freq)
        .ok_or_else(|| anyhow::anyhow!("no frequency <= target_max_freq {target_max_freq}"))?;
    if last <= first {
        bail!("empty target band {target_min_freq}-{target_max_freq} Hz");
    }
    let per_decade = slope_db / (freq[last].log10() - freq[first].log10());
    Ok(freq
        .iter()
        .enumerate()
        .map(|(i, f)| {
            if i < first {
                0.0
            } else {
                per_decade * (f.log10() - freq[first].log10())
            }
        })
        .collect())
}

/// Builds an absolute slope target for `spl` on the `freq` axis.
///
/// # Errors
///
/// Returns an error when the target or mean band contains no points,
/// or when `freq` and `spl` lengths differ.
pub fn slope_target(
    freq: &[f64],
    spl: &[f64],
    slope_db: f64,
    target_min_freq: f64,
    target_max_freq: f64,
    mean_min: f64,
    mean_max: f64,
) -> Result<Vec<f64>> {
    if freq.len() != spl.len() || freq.is_empty() {
        bail!("freq/spl length mismatch: {} vs {}", freq.len(), spl.len());
    }
    let line = relative_slope_line(freq, slope_db, target_min_freq, target_max_freq)?;
    // Level-align over the mean band: mean(curve) - mean(line).
    let mut curve_sum = 0.0;
    let mut line_sum = 0.0;
    let mut count = 0usize;
    for ((f, s), l) in freq.iter().zip(spl).zip(&line) {
        if *f >= mean_min && *f <= mean_max {
            curve_sum += s;
            line_sum += l;
            count += 1;
        }
    }
    if count == 0 {
        bail!("no points in mean band {mean_min}-{mean_max} Hz");
    }
    #[expect(clippy::cast_precision_loss, reason = "band sizes fit exactly in f64")]
    let offset = curve_sum / count as f64 - line_sum / count as f64;
    Ok(line.iter().map(|l| l + offset).collect())
}

/// Log-frequency linear interpolation of `(src_freq, src_spl)` at `freq`.
///
/// Out-of-range frequencies clamp to the edge values.
///
/// # Errors
///
/// Returns an error on empty or mismatched source arrays.
pub fn interp_log(freq: &[f64], src_freq: &[f64], src_spl: &[f64]) -> Result<Vec<f64>> {
    if src_freq.is_empty() || src_freq.len() != src_spl.len() {
        bail!(
            "bad interpolation source: {} freq, {} spl",
            src_freq.len(),
            src_spl.len()
        );
    }
    Ok(freq
        .iter()
        .map(|f| {
            if *f <= src_freq[0] {
                return src_spl[0];
            }
            if *f >= *src_freq.last().expect("non-empty checked above") {
                return *src_spl.last().expect("non-empty checked above");
            }
            let upper = src_freq.partition_point(|sf| sf < f);
            let (f0, f1) = (src_freq[upper - 1], src_freq[upper]);
            let (s0, s1) = (src_spl[upper - 1], src_spl[upper]);
            let t = (f.log10() - f0.log10()) / (f1.log10() - f0.log10());
            s0 + t * (s1 - s0)
        })
        .collect())
}
