//! Phase preservation in generated On Axis plots.

use spinorama_generate::compute::cea2034::compute_spin_data;
use spinorama_generate::figures::{apply_black_marker_style, spin_figures};
use spinorama_generate::measurements::{HvSpl, SplPlane};
use spinorama_generate::render::plotly_json::to_json_string;

fn figure(freq: Vec<f64>, phase: Option<Vec<f64>>) -> serde_json::Value {
    let columns = (-180..=180)
        .step_by(10)
        .map(|angle| {
            (
                if angle == 0 {
                    "On Axis".to_owned()
                } else {
                    format!("{angle}°")
                },
                vec![85.0; freq.len()],
            )
        })
        .collect();
    let mut plane = SplPlane::new(freq, columns).unwrap();
    plane.phase_on_axis = phase;
    let data = compute_spin_data(&HvSpl {
        horizontal: plane.clone(),
        vertical: plane,
    })
    .unwrap();
    let mut onaxis = spin_figures(&data, "Test", "Test", (20.0, 20000.0))
        .into_iter()
        .find(|figure| figure.name == "On Axis")
        .unwrap();
    apply_black_marker_style(&mut onaxis);
    serde_json::from_str(&to_json_string(&onaxis).unwrap()).unwrap()
}

#[test]
fn onaxis_phase_has_degree_axis_and_connects_wraps() {
    let plot = figure(
        vec![100.0, 500.0, 1000.0, 2000.0, 5000.0],
        Some(vec![-170.0, -190.0, -350.0, -530.0, -550.0]),
    );
    let trace = plot["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|trace| trace["name"] == "Phase (deg)")
        .expect("phase trace");
    assert_eq!(trace["yaxis"], "y2");
    assert_eq!(
        trace["x"],
        serde_json::json!([100.0, 500.0, 1000.0, 2000.0, 5000.0])
    );
    assert_eq!(
        trace["y"],
        serde_json::json!([-170.0, 170.0, 10.0, -170.0, 170.0])
    );
    assert_eq!(
        plot["layout"]["yaxis2"]["range"],
        serde_json::json!([-180.0, 360.0])
    );
    assert!(
        plot["layout"]["yaxis2"]["title"]["text"]
            .as_str()
            .unwrap()
            .contains("Phase (deg")
    );
    assert!(
        trace.get("marker").is_none(),
        "phase is a line, not a measurement marker series"
    );
}

#[test]
fn onaxis_phase_removes_high_frequency_delay() {
    let freq = vec![
        100.0, 500.0, 1000.0, 12000.0, 12500.0, 13000.0, 13500.0, 14000.0, 14500.0, 15000.0,
        15500.0, 16000.0,
    ];
    for wrapped in [false, true] {
        let phase = freq
            .iter()
            .map(|f| {
                let value: f64 = 37.0 - 360.0 * f * 0.0002;
                if wrapped {
                    (value + 180.0).rem_euclid(360.0) - 180.0
                } else {
                    value
                }
            })
            .collect();
        let plot = figure(freq.clone(), Some(phase));
        let trace = plot["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|trace| trace["name"] == "Phase (deg)")
            .expect("phase trace");
        assert!(
            trace["y"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| (v.as_f64().unwrap() - 37.0).abs() < 1e-8)
        );
        assert!(
            plot["layout"]["yaxis2"]["title"]["text"]
                .as_str()
                .unwrap()
                .contains("0.20 ms delay removed")
        );
    }
}

#[test]
fn onaxis_without_measured_phase_has_no_phase_axis() {
    let plot = figure(vec![100.0, 500.0, 1000.0, 2000.0, 5000.0], None);
    assert!(plot["layout"].get("yaxis2").is_none());
}
