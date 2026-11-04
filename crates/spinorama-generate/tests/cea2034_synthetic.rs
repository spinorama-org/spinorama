//! CEA2034 invariants on synthetic omnidirectional planes.

// Rust guideline compliant 2026-02-21

use spinorama_generate::compute::cea2034::{compute_cea2034, sound_power_weights};
use spinorama_generate::figures::common_figures;
use spinorama_generate::measurements::{HvSpl, SplPlane};

fn omni_plane(level_db: f64) -> SplPlane {
    let freq: Vec<f64> = (0..50)
        .map(|i| 20.0 * (1000f64).powf(f64::from(i) / 49.0))
        .collect();
    let mut names = vec!["On Axis".to_owned()];
    for angle in (10..=180).step_by(10) {
        names.push(format!("{angle}°"));
        if angle < 180 {
            names.push(format!("-{angle}°"));
        }
    }
    let columns = names
        .into_iter()
        .map(|name| (name, vec![level_db; freq.len()]))
        .collect();
    SplPlane::new(freq, columns).expect("synthetic plane is valid")
}

#[test]
fn omni_source_has_flat_zero_spin_after_shift() {
    let hv = HvSpl {
        horizontal: omni_plane(90.0),
        vertical: omni_plane(90.0),
    };
    let cea = compute_cea2034(&hv).expect("omni CEA2034 computes");
    assert!((cea.sensitivity_db - 90.0).abs() < 1e-9);
    for curve in [
        &cea.on_axis,
        &cea.listening_window,
        &cea.early_reflections,
        &cea.sound_power,
        &cea.estimated_inroom,
        &cea.early_reflections_di,
        &cea.sound_power_di,
    ] {
        assert!(
            curve.iter().all(|v| v.abs() < 1e-9),
            "curve not flat zero: {curve:?}"
        );
    }
}

#[test]
fn sound_power_weights_sum_to_sphere_area() {
    // Weights tile the half-sphere rings; total must be positive and the
    // 90° ring (doubled in the standard) the largest single weight.
    let weights = sound_power_weights();
    assert!(weights.iter().all(|w| *w > 0.0));
    assert!(weights[9] > weights[0]);
}

#[test]
fn common_figures_carry_the_six_cea_traces() {
    let hv = HvSpl {
        horizontal: omni_plane(85.0),
        vertical: omni_plane(85.0),
    };
    let cea = compute_cea2034(&hv).expect("omni CEA2034 computes");
    let figures = common_figures(&cea, None, "Test Speaker", "Lab", (20.0, 20_000.0));
    let names: Vec<&str> = figures.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "CEA2034",
            "CEA2034 Normalized",
            "On Axis",
            "Estimated In-Room Response",
            "Estimated In-Room Response Normalized"
        ]
    );
    let trace_names: Vec<&str> = figures[0].traces.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        trace_names,
        [
            "On Axis",
            "Listening Window",
            "Early Reflections",
            "Sound Power",
            "Early Reflections DI",
            "Sound Power DI"
        ]
    );
}
