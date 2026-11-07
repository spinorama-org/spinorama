//! Plotly JSON payload emission (the `dist-private` figure format).

// Rust guideline compliant 2026-02-21

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::figures::{Axis, Figure, FigureKind, Visibility};

/// Primary SPL axis range in dB, matching the Python spin layout.
const Y_RANGE_DB: [f64; 2] = [-45.0, 5.0];
/// Secondary directivity-index axis range in dB.
// DI axis aligned with the SPL axis (Python generate_yaxis_di default
// -5..45 for a -45..5 SPL range, so DI 0 sits on the SPL -40 line and
// the axis goes down to -5 dB).
const Y2_RANGE_DB: [f64; 2] = [-5.0, 45.0];

/// Encodes a figure as the `{data, layout}` Plotly JSON object.
///
/// Byte equality with Python `fig.to_json()` is explicitly not claimed;
/// the payload carries the same trace names, axes, and values so the
/// gpui-toolkit renderer and the trace-bundle contract see identical data.
///
/// # Errors
///
/// Returns an error when JSON serialization fails.
pub fn to_json_string(figure: &Figure) -> Result<String> {
    match &figure.kind {
        FigureKind::Scatter => scatter_json_string(figure),
        FigureKind::Contour(_) => contour_json_string(figure),
        FigureKind::Radar(_) => radar_json_string(figure),
        FigureKind::Bar(_) => bar_json_string(figure),
        FigureKind::Subplot(_) => subplot_json_string(figure),
    }
}

/// Serializes one scatter trace (shared by scatter and subplot figures).
fn trace_json(trace: &crate::figures::Trace, freq: &[f64]) -> Value {
    let mut item = json!({
        "type": "scatter",
        "mode": "lines",
        "name": trace.name,
        "x": trace.x.as_ref().unwrap_or(&freq.to_vec()),
        "y": trace.values,
        "yaxis": trace.axis.id(),
        "showlegend": trace.show_legend,
    });
    match trace.visibility {
        Visibility::Visible => {}
        Visibility::LegendOnly => item["visible"] = json!("legendonly"),
        Visibility::Hidden => item["visible"] = json!(false),
    }
    if trace.dash.is_some() || trace.color.is_some() {
        let mut line = json!({});
        if let Some(dash) = &trace.dash {
            line["dash"] = json!(dash);
        }
        if let Some(color) = &trace.color {
            line["color"] = json!(color);
        }
        item["line"] = line;
    }
    if let Some(fill) = &trace.fill {
        item["fill"] = json!(fill);
        if let Some(color) = &trace.color {
            item["fillcolor"] = json!(color);
        }
    }
    if let Some(opacity) = trace.opacity {
        item["opacity"] = json!(opacity);
    }
    if let Some(mode) = &trace.mode {
        item["mode"] = json!(mode);
    }
    if let Some(symbol) = &trace.marker_symbol {
        let mut marker = json!({"symbol": symbol});
        if let Some(size) = trace.marker_size {
            marker["size"] = json!(size);
        }
        if let Some(color) = &trace.color {
            marker["color"] = json!(color);
        }
        item["marker"] = marker;
    }
    item
}

fn subplot_json_string(figure: &Figure) -> Result<String> {
    let FigureKind::Subplot(cells) = &figure.kind else {
        bail!("figure {} is not a subplot figure", figure.name);
    };
    let cols = 2usize;
    let rows = cells.len().div_ceil(cols);
    let h_gap = 0.09;
    let v_gap = 0.045;
    let cell_w = (1.0 - h_gap * (cols as f64 - 1.0)) / cols as f64;
    let cell_h = (1.0 - v_gap * rows.saturating_sub(1) as f64) / rows as f64;
    let mut data: Vec<Value> = Vec::new();
    let mut layout = json!({
        "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
        "width": 1024,
        "height": 1680,
        "barmode": "group",
    });
    let mut annotations: Vec<Value> = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let row = index / cols;
        let col = index % cols;
        let x0 = col as f64 * (cell_w + h_gap);
        let y_top = 1.0 - row as f64 * (cell_h + v_gap);
        let suffix = if index == 0 {
            String::new()
        } else {
            (index + 1).to_string()
        };
        for trace in &cell.traces {
            let mut item = trace_json(trace, &figure.freq);
            item["xaxis"] = json!(format!("x{suffix}"));
            item["yaxis"] = json!(format!("y{suffix}"));
            data.push(item);
        }
        for bar in &cell.bars {
            let mut item = json!({
                "type": "bar",
                "x": bar.categories,
                "y": bar.values,
                "name": bar.name,
                "xaxis": format!("x{suffix}"),
                "yaxis": format!("y{suffix}"),
            });
            if let Some(color) = &bar.color {
                item["marker"] = json!({"color": color});
            }
            data.push(item);
        }
        let mut x_axis = json!({"domain": [x0, x0 + cell_w]});
        if cell.log_x {
            x_axis["type"] = json!("log");
            x_axis["title"] = json!({"text": "Frequency (Hz)"});
            x_axis["range"] = json!([20f64.log10(), 20000f64.log10()]);
            x_axis["autorange"] = json!(false);
        }
        layout[format!("xaxis{suffix}")] = x_axis;
        layout[format!("yaxis{suffix}")] = json!({"domain": [y_top - cell_h, y_top]});
        annotations.push(json!({
            "text": cell.title,
            "x": x0 + cell_w / 2.0,
            "y": y_top + 0.004,
            "xref": "paper",
            "yref": "paper",
            "showarrow": false,
        }));
    }
    layout["annotations"] = json!(annotations);
    Ok(serde_json::to_string(
        &json!({"data": data, "layout": layout}),
    )?)
}

fn scatter_json_string(figure: &Figure) -> Result<String> {
    let data: Vec<Value> = figure
        .traces
        .iter()
        .map(|trace| {
            let mut item = json!({
                "type": "scatter",
                "mode": "lines",
                "name": trace.name,
                "x": trace.x.as_ref().unwrap_or(&figure.freq),
                "y": trace.values,
                "yaxis": trace.axis.id(),
                "showlegend": trace.show_legend,
            });
            match trace.visibility {
                Visibility::Visible => {}
                Visibility::LegendOnly => item["visible"] = json!("legendonly"),
                Visibility::Hidden => item["visible"] = json!(false),
            }
            if trace.dash.is_some() || trace.color.is_some() {
                let mut line = json!({});
                if let Some(dash) = &trace.dash {
                    line["dash"] = json!(dash);
                }
                if let Some(color) = &trace.color {
                    line["color"] = json!(color);
                }
                item["line"] = line;
            }
            if let Some(fill) = &trace.fill {
                item["fill"] = json!(fill);
                if let Some(color) = &trace.color {
                    item["fillcolor"] = json!(color);
                }
            }
            if let Some(opacity) = trace.opacity {
                item["opacity"] = json!(opacity);
            }
            if let Some(mode) = &trace.mode {
                item["mode"] = json!(mode);
            }
            if let Some(symbol) = &trace.marker_symbol {
                let mut marker = json!({"symbol": symbol});
                if let Some(size) = trace.marker_size {
                    marker["size"] = json!(size);
                }
                if let Some(color) = &trace.color {
                    marker["color"] = json!(color);
                }
                item["marker"] = marker;
            }
            item
        })
        .collect();
    let has_secondary = figure.traces.iter().any(|t| t.axis == Axis::Secondary);
    let mut layout = json!({
        "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
        "width": 1200,
        "height": 800,
        "xaxis": {
            "type": "log",
            "title": {"text": "Frequency (Hz)"},
            "range": [20f64.log10(), 20000f64.log10()],
            "autorange": false,
        },
        "yaxis": {
            "title": {"text": figure.y_title.as_deref().unwrap_or("SPL (dB)")},
            "range": figure.y_range.unwrap_or(Y_RANGE_DB),
            "autorange": false,
        },
    });
    if !figure.show_zero_line {
        layout["hide_zero_line"] = json!(true);
    }
    if let Some(tickvals) = &figure.x_tickvals {
        layout["xaxis"]["tickvals"] = json!(tickvals);
    }
    if let Some(tickvals) = &figure.y_tickvals {
        layout["yaxis"]["tickvals"] = json!(tickvals);
    }
    if has_secondary {
        layout["yaxis2"] = json!({
            "title": {"text": "Directivity Index (dB)"},
            "range": Y2_RANGE_DB,
            "autorange": false,
            "overlaying": "y",
            "side": "right",
        });
    }
    Ok(serde_json::to_string(
        &json!({"data": data, "layout": layout}),
    )?)
}

const CONTOUR_COLORSCALE: &[(f64, &str)] = &[
    (0.0, "rgb(0,0,168)"),
    (0.1, "rgb(0,0,200)"),
    (0.2, "rgb(0,74,255)"),
    (0.3, "rgb(0,152,255)"),
    (0.4, "rgb(74,255,161)"),
    (0.5, "rgb(161,255,74)"),
    (0.6, "rgb(255,255,0)"),
    (0.7, "rgb(234,159,0)"),
    (0.8, "rgb(255,74,0)"),
    (0.9, "rgb(222,74,0)"),
    (1.0, "rgb(253,14,13)"),
];

fn contour_json_string(figure: &Figure) -> Result<String> {
    let FigureKind::Contour(data) = &figure.kind else {
        bail!("figure {} is not a contour figure", figure.name);
    };
    let colorscale: Vec<Value> = CONTOUR_COLORSCALE
        .iter()
        .map(|(stop, color)| json!([stop, color]))
        .collect();
    let colorbar = json!({
        "dtick": 3,
        "len": 0.5,
        "lenmode": "fraction",
        "thickness": 15,
        "thicknessmode": "pixels",
        "title": {"text": "dB (SPL)"},
    });
    let (trace, layout) = if data.surface {
        (
            json!({
                "type": "surface",
                "x": figure.freq,
                "y": data.angles,
                "z": data.z,
                "surfacecolor": data.surface_color,
                "colorscale": colorscale,
                "cmin": -30.0,
                "cmax": 3.0,
                "colorbar": colorbar,
                "contours": {
                    "z": {
                        "show": true,
                        "start": -30.0,
                        "end": 3.0,
                        "size": 3.0,
                        "color": "black",
                        "width": 1,
                    }
                },
            }),
            json!({
                "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
                "width": 1200,
                "height": 800,
                "autosize": false,
                "scene": {
                    "camera": {"eye": {"x": -1.2, "y": -1.4, "z": 0.6}},
                    "xaxis": {"title": "Freq (Hz)", "type": "log"},
                    "yaxis": {"title": "Angle"},
                    "zaxis": {"title": "SPL (dB)", "range": [-50.0, 0.0]},
                },
            }),
        )
    } else {
        (
            json!({
                "type": "contour",
                "x": figure.freq,
                "y": data.angles,
                "z": data.z,
                "colorscale": colorscale,
                "zauto": false,
                "zmin": -30.0,
                "zmid": -18.0,
                "zmax": 3.0,
                "colorbar": colorbar,
                "contours": {"coloring": "heatmap", "showlabels": false},
                "line": {"smoothing": 0.65},
                "ncontours": 30,
            }),
            json!({
                "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
                "width": 1200,
                "height": 800,
                "xaxis": {
                    "type": "log",
                    "title": {"text": "Frequency (Hz)"},
                    "range": [100f64.log10(), 20000f64.log10()],
                    "autorange": false,
                },
                "yaxis": {"title": {"text": "Angle"}, "range": [-180.0, 180.0], "autorange": false},
            }),
        )
    };
    Ok(serde_json::to_string(
        &json!({"data": [trace], "layout": layout}),
    )?)
}

fn radar_json_string(figure: &Figure) -> Result<String> {
    let FigureKind::Radar(traces) = &figure.kind else {
        bail!("figure {} is not a radar figure", figure.name);
    };
    let data: Vec<Value> = traces
        .iter()
        .map(|trace| {
            let subplot = match (trace.row, trace.col) {
                (1, 1) => "polar",
                (1, 2) => "polar2",
                (2, 1) => "polar3",
                _ => "polar4",
            };
            let mut item = json!({
                "type": "scatterpolar",
                "mode": "lines",
                "r": trace.r,
                "theta": trace.theta,
                "name": trace.name,
                "subplot": subplot,
            });
            if let Some(color) = &trace.color {
                item["marker"] = json!({"color": color});
            }
            item
        })
        .collect();
    let polar = |domain_x: [f64; 2], domain_y: [f64; 2]| {
        json!({
            "domain": {"x": domain_x, "y": domain_y},
            "radialaxis": {"visible": true, "range": [-40.0, 0.0], "tickvals": [-30, -20, -10]},
            "angularaxis": {"direction": "clockwise", "rotation": 90},
        })
    };
    let mut layout = json!({
        "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
        "width": 1000,
        "height": 1200,
        "showlegend": true,
        "annotations": [
            {"text": "LOW (100Hz-200Hz)", "showarrow": false, "x": 0.225, "y": 1.0, "xref": "paper", "yref": "paper"},
            {"text": "MIDHIGH (1600Hz-3150Hz)", "showarrow": false, "x": 0.775, "y": 1.0, "xref": "paper", "yref": "paper"},
            {"text": "LOWMID (250Hz-500Hz)", "showarrow": false, "x": 0.225, "y": 0.475, "xref": "paper", "yref": "paper"},
            {"text": "HIGH (4000Hz-8000Hz)", "showarrow": false, "x": 0.775, "y": 0.475, "xref": "paper", "yref": "paper"},
        ],
    });
    layout["polar"] = polar([0.0, 0.45], [0.55, 1.0]);
    layout["polar2"] = polar([0.55, 1.0], [0.55, 1.0]);
    layout["polar3"] = polar([0.0, 0.45], [0.0, 0.45]);
    layout["polar4"] = polar([0.55, 1.0], [0.0, 0.45]);
    Ok(serde_json::to_string(
        &json!({"data": data, "layout": layout}),
    )?)
}

fn bar_json_string(figure: &Figure) -> Result<String> {
    let FigureKind::Bar(traces) = &figure.kind else {
        bail!("figure {} is not a bar figure", figure.name);
    };
    let data: Vec<Value> = traces
        .iter()
        .map(|trace| {
            let mut item = json!({
                "type": "bar",
                "x": trace.categories,
                "y": trace.values,
                "name": trace.name,
            });
            if let Some(color) = &trace.color {
                item["marker"] = json!({"color": color});
            }
            item
        })
        .collect();
    let layout = json!({
        "title": {"text": figure.title, "font": {"size": 16, "family": "Arial"}, "x": 0.5},
        "width": 600,
        "height": 300,
        "yaxis": {"title": {"text": "Count"}, "nticks": 5},
        "barmode": "group",
    });
    Ok(serde_json::to_string(
        &json!({"data": data, "layout": layout}),
    )?)
}
