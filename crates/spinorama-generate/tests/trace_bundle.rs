//! Trace-bundle v2 encoding contract tests.

// Rust guideline compliant 2026-02-21

use base64::Engine;
use spinorama_generate::figures::{Axis, Figure, Trace};
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
