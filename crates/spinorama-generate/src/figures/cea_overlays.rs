//! CEA2034 overlays, matching the original Python spinorama plots.

use annotations_rust::RawRequest;
use serde_json::{Value, json};

use super::{Axis, Figure, Trace};
use crate::compute::estimates::smoothness_regression;

// Python SLOPE_MIN_FREQ / SLOPE_MAX_FREQ and VituixCAD zone coefficients.
const SLOPE_BAND: (f64, f64) = (100.0, 12_000.0);

fn fit(freq: &[f64], values: &[f64]) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let pairs: Vec<_> = freq
        .iter()
        .zip(values)
        .filter(|(f, y)| **f >= SLOPE_BAND.0 && **f <= SLOPE_BAND.1 && y.is_finite())
        .map(|(f, y)| (*f, *y))
        .collect();
    if pairs.len() < 2 {
        return None;
    }
    let freqs: Vec<_> = pairs.iter().map(|p| p.0).collect();
    let spl: Vec<_> = pairs.iter().map(|p| p.1).collect();
    let (slope, intercept, smoothness) = smoothness_regression(&freqs, &spl);
    let first = freqs[0];
    let last = freqs[freqs.len() - 1];
    Some((
        first,
        last,
        intercept + slope * first.log10(),
        intercept + slope * last.log10(),
        slope * 2f64.log10(),
        smoothness,
    ))
}

pub(super) fn add_overlays(figure: &mut Figure) {
    let curves = figure.traces.clone();
    let slope = |name: &str| {
        curves
            .iter()
            .find(|t| t.name == name)
            .and_then(|t| fit(&figure.freq, &t.values))
            .map_or(0.0, |f| f.4)
    };
    let slope_di = (slope("On Axis") - slope("Sound Power")) * std::f64::consts::LN_2;
    let limited = slope_di.clamp(0.0, 1.2);
    for trace in curves {
        let Some((first, last, start, end, _, _)) = fit(&figure.freq, &trace.values) else {
            continue;
        };
        if trace.name != "On Axis" {
            figure.traces.push(
                Trace::with_x(
                    format!("{} slope", trace.name),
                    vec![first, last],
                    vec![start, end],
                    trace.axis,
                )
                .dash("dash")
                .color(trace.color.as_deref().unwrap_or("black"))
                .no_legend(),
            );
        }
        let bounds = match trace.name.as_str() {
            "On Axis" => Some((0.0, 0.4167 * limited - 0.5, "ON")),
            "Listening Window" => Some((0.3333 * limited - 0.65, 0.0, "LW")),
            "Sound Power" => Some((-0.2083 * limited - 0.95, -0.0833 * limited - 0.80, "SP")),
            "Sound Power DI" => Some((
                (slope_di + 0.15).min(1.2).mul_add(1.0, -0.3).max(0.0),
                (slope_di + 0.15).min(1.2),
                "SPDI",
            )),
            _ => None,
        };
        if let Some((min, max, label)) = bounds {
            let octaves = (last / first).log2();
            figure.traces.push(
                Trace::with_x(
                    format!("recommended {label} zone"),
                    vec![first, last, last, first, first],
                    vec![
                        start - 1.0,
                        start - 1.0 + min * octaves,
                        start + 1.0 + max * octaves,
                        start + 1.0,
                        start - 1.0,
                    ],
                    trace.axis,
                )
                .fill("toself")
                .mode("none")
                .opacity(0.25)
                .color(trace.color.as_deref().unwrap_or("black"))
                .no_legend(),
            );
        }
    }
}

/// Annotation anchors and priorities from Python `plot_spinorama_annotation`.
pub(crate) fn requests(figure: &Figure) -> Vec<RawRequest> {
    if !matches!(figure.name.as_str(), "CEA2034" | "CEA2034 Normalized") {
        return Vec::new();
    }
    let params = [
        (
            "On Axis",
            2000.0,
            100,
            vec!["top", "upper", "middle"],
            "above",
        ),
        (
            "Listening Window",
            16000.0,
            95,
            vec!["top", "upper", "middle"],
            "above",
        ),
        (
            "Early Reflections",
            10000.0,
            80,
            vec!["middle", "upper", "lower"],
            "below",
        ),
        (
            "Sound Power",
            10000.0,
            75,
            vec!["upper", "middle", "lower"],
            "below",
        ),
        (
            "Early Reflections DI",
            10000.0,
            70,
            vec!["upper", "top", "middle", "lower", "bottom"],
            "below",
        ),
        (
            "Sound Power DI",
            10000.0,
            65,
            vec!["upper", "top", "middle", "lower", "bottom"],
            "above",
        ),
    ];
    params
        .into_iter()
        .filter_map(|(name, initial, priority, lanes, direction)| {
            let trace = figure.traces.iter().find(|t| t.name == name)?;
            let (_, _, _, _, slope, smoothness) = fit(&figure.freq, &trace.values)?;
            let peak_band = match name {
                "On Axis" => Some((1000.0, 5000.0)),
                "Listening Window" => Some((8000.0, 16000.0)),
                _ => None,
            };
            let peak = peak_band.and_then(|(lo, hi)| {
                figure
                    .freq
                    .iter()
                    .zip(&trace.values)
                    .filter(|(f, _)| **f >= lo && **f < hi)
                    .reduce(|a, b| if b.1 > a.1 { b } else { a })
            });
            let (anchor_freq, anchor_spl) = if let Some((f, y)) = peak {
                (*f, *y)
            } else {
                let index = figure
                    .freq
                    .partition_point(|f| *f < initial)
                    .min(trace.values.len().checked_sub(1)?);
                (initial, trace.values[index])
            };
            Some((
                name.to_owned(),
                anchor_freq.log10(),
                anchor_spl,
                trace.axis.id().to_owned(),
                format!("{slope:.2} db/oct sm {smoothness:.2}"),
                priority,
                lanes.into_iter().map(str::to_owned).collect(),
                Some(direction.to_owned()),
            ))
        })
        .collect()
}

/// Uses the original collision solver, including its short-leader fallback.
pub(crate) fn layout_annotations(figure: &Figure) -> Vec<Value> {
    let requests = requests(figure);
    if requests.is_empty() {
        return Vec::new();
    }
    // Use the plot's logical dimensions; image export scales both axes and labels.
    let (width, height) = (1000.0, 600.0);
    let y_range = figure.y_range.unwrap_or([-45.0, 5.0]);
    let y2_range = figure
        .secondary_axis
        .as_ref()
        .map_or([-5.0, 45.0], |axis| axis.range);
    let xr = (20f64.log10(), 20000f64.log10());
    let x_pixel = |x: f64| (x - xr.0) / (xr.1 - xr.0) * width;
    let y_pixel = |y: f64, axis: Axis| {
        let range = if axis == Axis::Secondary {
            y2_range
        } else {
            y_range
        };
        (range[1] - y) / (range[1] - range[0]) * height
    };
    let segments = figure
        .traces
        .iter()
        .flat_map(|t| {
            let points: Vec<_> =
                t.x.as_ref()
                    .unwrap_or(&figure.freq)
                    .iter()
                    .zip(&t.values)
                    .filter(|(f, y)| **f > 0.0 && y.is_finite())
                    .map(|(f, y)| (x_pixel(f.log10()), y_pixel(*y, t.axis)))
                    .collect();
            points
                .windows(2)
                .map(|p| {
                    (
                        p[0].0,
                        p[0].1,
                        p[1].0,
                        p[1].1,
                        t.axis.id().to_owned(),
                        Some(t.name.clone()),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let placements = annotations_rust::place_annotations(
        requests.clone(),
        width,
        height,
        (0.0, 0.0, 0.0, 0.0),
        xr,
        vec![
            ("y".into(), y_range[0], y_range[1]),
            ("y2".into(), y2_range[0], y2_range[1]),
        ],
        true,
        10.0,
        5.0,
        (0.0, 1.0),
        (0.0, 1.0),
        (1..=4)
            .flat_map(|decade| (1..=9).map(move |m| (f64::from(m) * 10f64.powi(decade)).log10()))
            .filter(|v| *v >= xr.0 && *v <= xr.1)
            .collect(),
        vec![
            ("y".into(), (-45..=5).step_by(5).map(f64::from).collect()),
            ("y2".into(), (-5..=45).step_by(5).map(f64::from).collect()),
        ],
        Vec::new(),
        Vec::new(),
        segments,
    );
    let fallback = requests
        .iter()
        .zip(&placements)
        .any(|(r, (center, hidden))| {
            let axis = if r.3 == "y2" {
                Axis::Secondary
            } else {
                Axis::Primary
            };
            *hidden
                || center
                    .is_none_or(|c| (c.0 - x_pixel(r.1)).hypot(c.1 - y_pixel(r.2, axis)) > 260.0)
        });
    requests
        .iter()
        .zip(placements)
        .map(|(r, (center, _))| {
            let range = if r.3 == "y2" { y2_range } else { y_range };
            let axis = if r.3 == "y2" {
                Axis::Secondary
            } else {
                Axis::Primary
            };
            let size = (r.4.chars().count() as f64 * 10.0 * 0.62 + 12.0, 25.5);
            let center = if fallback {
                let direction = if r.7.as_deref() == Some("above") {
                    -1.0
                } else {
                    1.0
                };
                (
                    x_pixel(r.1) - size.0 / 2.0,
                    y_pixel(r.2, axis) + direction * (20.0 + size.1 / 2.0),
                )
            } else {
                center.expect("complete placement checked")
            };
            let center = (
                center.0.clamp(size.0 / 2.0, width - size.0 / 2.0),
                center.1.clamp(size.1 / 2.0, height - size.1 / 2.0),
            );
            let color = figure
                .traces
                .iter()
                .find(|t| t.name == r.0)
                .and_then(|t| t.color.as_deref())
                .unwrap_or("black");
            json!({"text":r.4,"x":r.1,"y":r.2,"xref":"x","yref":r.3,
            "ax":xr.0 + center.0 / width * (xr.1 - xr.0),
            "ay":range[1] - center.1 / height * (range[1] - range[0]),
            "axref":"x","ayref":r.3,"showarrow":true,"font":{"size":10,"color":color},
            "bordercolor":color,"bgcolor":"rgba(255,255,255,0.86)","visible":true})
        })
        .collect()
}
