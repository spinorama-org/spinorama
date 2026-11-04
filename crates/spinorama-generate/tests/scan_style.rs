//! Scan-hardening style transform tests.

// Rust guideline compliant 2026-02-21

use spinorama_generate::figures::{Axis, Figure, Trace, apply_scan_style};
use spinorama_generate::render::plotly_json::to_json_string;

fn sample_figure() -> Figure {
    Figure::scatter(
        "CEA2034",
        "CEA2034 for Test",
        vec![20.0, 200.0, 2000.0, 20000.0],
        vec![
            Trace::new("On Axis", vec![0.0; 4], Axis::Primary),
            Trace::new("Listening Window", vec![-1.0; 4], Axis::Primary),
            Trace::with_x(
                "Band ±3dB",
                vec![20.0, 20000.0, 20000.0, 20.0],
                vec![3.0, 3.0, -3.0, -3.0],
                Axis::Primary,
            )
            .color("#E2F705")
            .fill("toself")
            .opacity(0.25),
        ],
    )
}

#[test]
fn scan_style_makes_curves_grey_with_distinct_markers() {
    let mut figure = sample_figure();
    apply_scan_style(&mut figure);
    let symbols: Vec<&str> = figure
        .traces
        .iter()
        .filter(|t| t.fill.is_none())
        .map(|t| {
            assert_eq!(t.color.as_deref(), Some("#7F7F7F"));
            assert_eq!(t.mode.as_deref(), Some("lines+markers"));
            t.marker_symbol.as_deref().expect("marker symbol set")
        })
        .collect();
    assert_eq!(symbols, ["circle", "square"]);
    let band = figure
        .traces
        .iter()
        .find(|t| t.fill.is_some())
        .expect("band trace");
    assert_eq!(band.color.as_deref(), Some("#CCCCCC"));
    assert!(band.marker_symbol.is_none());
}

#[test]
fn scan_style_adds_dense_grid_tickvals() {
    let mut figure = sample_figure();
    apply_scan_style(&mut figure);
    let y = figure.y_tickvals.expect("y tickvals");
    assert_eq!(y.len(), 51); // every 1 dB from -45 to +5
    assert_eq!(y[0], -45.0);
    assert_eq!(*y.last().expect("last"), 5.0);
    let x = figure.x_tickvals.expect("x tickvals");
    assert!(x.contains(&30.0) && x.contains(&9000.0));
    assert!(x.iter().all(|v| (20.0..=20000.0).contains(v)));
}

#[test]
fn scan_style_reaches_plotly_json() {
    let mut figure = sample_figure();
    apply_scan_style(&mut figure);
    let text = to_json_string(&figure).expect("json encodes");
    assert!(text.contains("\"symbol\":\"square\""));
    assert!(text.contains("\"tickvals\""));
    // The untransformed figure must not carry scan artifacts.
    let plain = to_json_string(&sample_figure()).expect("json encodes");
    assert!(!plain.contains("\"tickvals\""));
}
