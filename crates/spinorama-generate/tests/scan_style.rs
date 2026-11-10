//! Black-curve / marker speaker style and scan-hardening tests.

// Rust guideline compliant 2026-02-21

use spinorama_generate::figures::{
    Axis, Figure, Trace, apply_black_marker_style, apply_scan_style, marker_symbol_for,
};
use spinorama_generate::render::plotly_json::to_json_string;

fn sample_figure() -> Figure {
    Figure::scatter(
        "CEA2034",
        "CEA2034 for Test",
        vec![20.0, 200.0, 2000.0, 20000.0],
        vec![
            Trace::new("On Axis", vec![0.0; 4], Axis::Primary),
            Trace::new("Listening Window", vec![-1.0; 4], Axis::Primary),
            Trace::new("Trend line", vec![0.0; 4], Axis::Primary)
                .dash("dot")
                .no_legend(),
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
fn black_marker_style_makes_curves_black_with_named_markers() {
    let mut figure = sample_figure();
    apply_black_marker_style(&mut figure);
    let symbols: Vec<&str> = figure
        .traces
        .iter()
        .filter(|t| t.fill.is_none() && t.marker_symbol.is_some())
        .map(|t| {
            assert_eq!(t.color.as_deref(), Some("#000000"));
            assert_eq!(t.mode.as_deref(), Some("lines+markers"));
            t.marker_symbol.as_deref().expect("marker symbol set")
        })
        .collect();
    assert_eq!(symbols, ["circle", "square"]);
    // No dense grid: that stays a --scan-hard extra.
    assert!(figure.x_tickvals.is_none());
    assert!(figure.y_tickvals.is_none());
}

#[test]
fn helper_traces_stay_black_without_markers_and_fills_turn_grey() {
    let mut figure = sample_figure();
    apply_black_marker_style(&mut figure);
    let trend = figure
        .traces
        .iter()
        .find(|t| t.name == "Trend line")
        .expect("trend trace");
    assert_eq!(trend.color.as_deref(), Some("#000000"));
    assert_eq!(trend.dash.as_deref(), Some("dot"));
    assert!(trend.marker_symbol.is_none());
    assert_ne!(trend.mode.as_deref(), Some("lines+markers"));
    let band = figure
        .traces
        .iter()
        .find(|t| t.fill.is_some())
        .expect("band trace");
    assert_eq!(band.color.as_deref(), Some("#CCCCCC"));
    assert!(band.marker_symbol.is_none());
}

#[test]
fn marker_symbols_are_keyed_by_measurement_name() {
    // The same measurement gets the same symbol in every view and
    // for every speaker; helpers get none.
    assert_eq!(marker_symbol_for("On Axis"), Some("circle"));
    assert_eq!(marker_symbol_for("ON"), marker_symbol_for("On Axis"));
    assert_eq!(marker_symbol_for("Listening Window"), Some("square"));
    assert_eq!(marker_symbol_for("Sound Power DI"), Some("x"));
    assert_eq!(marker_symbol_for("30°"), marker_symbol_for("-30°"));
    assert_eq!(marker_symbol_for("Band ±3dB"), None);
    assert_eq!(marker_symbol_for("Midrange Band -3dB"), None);
    assert_eq!(marker_symbol_for("Listening Window slope"), None);
    assert_eq!(marker_symbol_for("recommended SP zone"), None);
    assert_eq!(marker_symbol_for("N/A"), None);
    // Unknown measurements pick a stable symbol.
    assert_eq!(
        marker_symbol_for("Some Custom Curve"),
        marker_symbol_for("Some Custom Curve")
    );
    assert!(marker_symbol_for("Some Custom Curve").is_some());
}

#[test]
fn scan_style_makes_curves_black_with_distinct_markers() {
    let mut figure = sample_figure();
    apply_scan_style(&mut figure);
    let symbols: Vec<&str> = figure
        .traces
        .iter()
        .filter(|t| t.fill.is_none() && t.marker_symbol.is_some())
        .map(|t| {
            assert_eq!(t.color.as_deref(), Some("#000000"));
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
    assert!(text.contains("\"#000000\""));
    assert!(text.contains("\"tickvals\""));
    // The untransformed figure must not carry scan artifacts.
    let plain = to_json_string(&sample_figure()).expect("json encodes");
    assert!(!plain.contains("\"tickvals\""));
    assert!(!plain.contains("\"symbol\""));
}

#[test]
fn styled_write_keeps_the_trace_bundle_pristine() {
    use spinorama_generate::figures::apply_black_marker_style;
    use spinorama_generate::render::write_figure_with_bundle;

    let pristine = Figure::scatter(
        "CEA2034",
        "CEA2034 for Test",
        vec![20.0, 200.0, 2000.0, 20000.0],
        vec![
            Trace::new("On Axis", vec![0.0; 4], Axis::Primary).color("#1f77b4"),
            Trace::new("Listening Window", vec![-1.0; 4], Axis::Primary).color("#ff7f0e"),
        ],
    );
    let mut styled = pristine.clone();
    apply_black_marker_style(&mut styled);

    let dir = std::env::temp_dir().join(format!("styled-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_figure_with_bundle(&dir, &styled, &pristine).expect("write");
    let json = std::fs::read_to_string(dir.join("CEA2034.json")).expect("json");
    assert!(json.contains("\"#000000\""), "display json is black");
    assert!(
        json.contains("\"symbol\":\"circle\""),
        "display json has markers"
    );
    let bundle = std::fs::read_to_string(dir.join("traces/CEA2034.json")).expect("bundle");
    assert!(bundle.contains("#1f77b4"), "bundle keeps pristine colours");
    assert!(!bundle.contains("#000000"), "bundle is not styled");
    let _ = std::fs::remove_dir_all(&dir);
}
