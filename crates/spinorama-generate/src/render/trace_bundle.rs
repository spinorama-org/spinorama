//! v2 trace-bundle encoder (the numeric comparison contract).
//!
//! Port of `image_export.py::extract_trace_bundle` for scatter figures:
//! visible traces are sorted by x, resampled onto a 1024-point
//! log-frequency grid spanning all visible traces, linearly interpolated
//! in log-frequency, quantized to `int16` centi-dB, and base64-encoded
//! little-endian. Samples outside a trace's own x range use the sentinel.

// Rust guideline compliant 2026-02-21

use anyhow::{Result, bail};
use base64::Engine;
use serde_json::{Value, json};

use crate::figures::{Axis, Figure, Visibility};

/// Number of points on the bundle log-frequency grid.
const BUNDLE_POINTS: usize = 1024;
/// Quantization scale: values are stored as `round(dB * 100)` int16.
const CENTI_DB: f64 = 100.0;
/// Sentinel for non-finite samples, matching the Python encoder.
const MISSING: i16 = i16::MIN;

struct BundleTrace {
    name: String,
    color: String,
    dash: String,
    axis: Axis,
    x: Vec<f64>,
    y: Vec<f64>,
    polygon: Option<Vec<[f64; 2]>>,
    opacity: f64,
    show_legend: bool,
}

/// Encodes a figure as a v2 trace-bundle JSON string.
///
/// # Errors
///
/// Returns an error when no visible scatter trace has a positive x range
/// or when JSON serialization fails.
pub fn to_json_string(figure: &Figure) -> Result<String> {
    let mut traces_in: Vec<BundleTrace> = Vec::new();
    for trace in &figure.traces {
        if trace.visibility != Visibility::Visible {
            continue;
        }
        let xs = trace.x.as_ref().unwrap_or(&figure.freq);
        if xs.len() != trace.values.len() || xs.len() < 2 {
            continue;
        }
        let mut pairs: Vec<(f64, f64)> = xs
            .iter()
            .zip(&trace.values)
            .filter(|(x, y)| **x > 0.0 && x.is_finite() && y.is_finite())
            .map(|(x, y)| (*x, *y))
            .collect();
        if pairs.len() < 2 {
            continue;
        }
        pairs.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        traces_in.push(BundleTrace {
            name: trace.name.clone(),
            color: trace.color.clone().unwrap_or_default(),
            dash: trace.dash.clone().unwrap_or_else(|| "solid".to_owned()),
            axis: trace.axis,
            x: pairs.iter().map(|pair| pair.0).collect(),
            y: pairs.iter().map(|pair| pair.1).collect(),
            polygon: trace.fill.as_ref().map(|_| {
                xs.iter()
                    .zip(&trace.values)
                    .map(|(x, y)| [*x, *y])
                    .collect()
            }),
            opacity: trace.opacity.unwrap_or(1.0),
            show_legend: trace.show_legend,
        });
    }
    if traces_in.is_empty() {
        bail!("figure {} has no visible scatter traces", figure.name);
    }

    let freq_min = traces_in
        .iter()
        .filter_map(|trace| trace.x.first().copied())
        .fold(f64::INFINITY, f64::min);
    let freq_max = traces_in
        .iter()
        .filter_map(|trace| trace.x.last().copied())
        .fold(f64::NEG_INFINITY, f64::max);
    if !(freq_min > 0.0 && freq_max > freq_min) {
        bail!("figure {} has no positive frequency range", figure.name);
    }

    let grid: Vec<f64> = (0..BUNDLE_POINTS)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "grid index fits exactly in f64")]
            let step = i as f64 / (BUNDLE_POINTS - 1) as f64;
            // Python's closed-form grid; endpoint sentinel behaviour matters.
            freq_min * (freq_max / freq_min).powf(step)
        })
        .collect();

    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut traces: Vec<Value> = Vec::with_capacity(traces_in.len());
    for trace in &traces_in {
        let mut bytes = Vec::with_capacity(BUNDLE_POINTS * 2);
        for &target in &grid {
            let value = interp_log_freq(&trace.x, &trace.y, target);
            let quantized = if value.is_finite() {
                if trace.axis == Axis::Primary {
                    y_min = y_min.min(value);
                    y_max = y_max.max(value);
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value clamped to int16 range below"
                )]
                let q = (value * CENTI_DB).round().clamp(-32767.0, 32767.0) as i16;
                q
            } else {
                MISSING
            };
            bytes.extend_from_slice(&quantized.to_le_bytes());
        }
        traces.push(json!({
            "name": trace.name,
            "color": trace.color,
            "dash": trace.dash,
            "axis": trace.axis.id(),
            "polygon": trace.polygon,
            "opacity": trace.opacity,
            "showlegend": trace.show_legend,
            "y_int16_b64": base64::engine::general_purpose::STANDARD.encode(&bytes),
        }));
    }

    Ok(serde_json::to_string(&json!({
        "v": 2,
        "graph": figure.name,
        "title": figure.title,
        "points": BUNDLE_POINTS,
        "freq_min": freq_min,
        "freq_max": freq_max,
        "y_min": y_min,
        "y_max": y_max,
        "y_range": [-45.0, 5.0],
        "y2_range": figure.secondary_axis.as_ref().map_or([-10.0, 40.0], |axis| axis.range),
        "traces": traces,
        "annotation_requests": crate::figures::cea_overlays::requests(figure),
    }))?)
}

/// Linear interpolation in log-frequency, NaN outside the source range.
fn interp_log_freq(freq: &[f64], values: &[f64], target: f64) -> f64 {
    if freq.is_empty() || values.len() != freq.len() {
        return f64::NAN;
    }
    if target < freq[0] || target > freq[freq.len() - 1] {
        return f64::NAN;
    }
    let upper = freq.partition_point(|f| *f < target);
    if upper == 0 {
        return values[0];
    }
    if upper >= freq.len() {
        return values[values.len() - 1];
    }
    let (x0, x1) = (freq[upper - 1], freq[upper]);
    if x0 <= 0.0 || x1 <= x0 {
        return values[upper - 1];
    }
    let t = (target.ln() - x0.ln()) / (x1.ln() - x0.ln());
    values[upper - 1] + t * (values[upper] - values[upper - 1])
}
