//! Trace-bundle v2 encoding contract tests.

// Rust guideline compliant 2026-02-21

use base64::Engine;
use spinorama_generate::figures::{Axis, Figure, SecondaryAxis, Trace};
use spinorama_generate::loaders::load_hv;
use spinorama_generate::render::trace_bundle::to_json_string;

#[test]
fn bundle_quantizes_flat_trace_to_centi_db() {
    let figure = Figure::scatter(
        "On Axis",
        "On Axis for Test",
        vec![20.0, 200.0, 2000.0, 20000.0],
        vec![Trace::new("On Axis", vec![-3.456; 4], Axis::Primary)],
    );
    let bundle: serde_json::Value =
        serde_json::from_str(&to_json_string(&figure).expect("bundle encodes"))
            .expect("bundle is JSON");
    assert_eq!(bundle["v"], 2);
    assert_eq!(bundle["points"], 1024);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(
            bundle["traces"][0]["y_int16_b64"]
                .as_str()
                .expect("b64 field"),
        )
        .expect("valid base64");
    assert_eq!(raw.len(), 2048);
    let first = i16::from_le_bytes([raw[0], raw[1]]);
    assert_eq!(first, -346);
}

#[test]
fn unknown_format_is_a_hard_error() {
    let error = load_hv("no-such-format", std::path::Path::new("."), "test")
        .expect_err("unknown formats must not fall back to a default loader");
    assert!(error.to_string().contains("unknown measurement format"));
}

#[test]
fn phase_bundle_preserves_degree_range_and_connects_wraps() {
    let mut figure = Figure::scatter(
        "On Axis",
        "On Axis for Test",
        vec![100.0, 200.0, 400.0, 800.0],
        vec![
            Trace::new("On Axis", vec![0.0; 4], Axis::Primary),
            Trace::new(
                "Phase (deg)",
                vec![-170.0, 170.0, 160.0, 150.0],
                Axis::Secondary,
            ),
        ],
    );
    figure.secondary_axis = Some(SecondaryAxis {
        range: [-180.0, 360.0],
        title: "Phase (deg)".to_owned(),
    });
    let bundle: serde_json::Value =
        serde_json::from_str(&to_json_string(&figure).unwrap()).unwrap();
    assert_eq!(bundle["y2_range"], serde_json::json!([-180.0, 360.0]));
    let raw = base64::engine::general_purpose::STANDARD
        .decode(bundle["traces"][1]["y_int16_b64"].as_str().unwrap())
        .unwrap();
    let values: Vec<_> = raw
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert!(values.iter().all(|&value| value != i16::MIN));
    assert_eq!(values[0], -17000);
    assert!(values[170].abs() < 100, "wrap connector crosses zero");
    assert!(
        values[342..]
            .iter()
            .all(|&value| (15000..=17000).contains(&value))
    );
}
