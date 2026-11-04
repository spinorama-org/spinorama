//! Real-data parity tests against Python-generated golden bundles.
//!
//! The fixtures under `tests/fixtures/` are v2 trace bundles produced
//! by the Python pipeline (`scripts/generate_graphs.py --speaker ...
//! --force`, Nov 2026) for the same measurements. The Rust loader +
//! CEA2034 computation must reproduce every golden trace within one
//! quantization step (0.01 dB) plus interpolation slack.
//!
//! These tests read the real measurement data in `datas/measurements`
//! (repository root, two levels above this crate). When the data tree
//! is absent (crate copied out of the repository) they skip.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine;
use spinorama_generate::compute::cea2034::compute_spin_data;
use spinorama_generate::compute::scores::preference_score;
use spinorama_generate::loaders::{LoadedMeasurement, load_measurement};
use spinorama_generate::measurements::symmetrise_hv;
use spinorama_generate::render::trace_bundle::to_json_string;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn measurements_dir() -> Option<PathBuf> {
    let dir = repo_root().join("datas/measurements");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipping: {} not found", dir.display());
        None
    }
}

fn decode_bundle(text: &str) -> HashMap<String, Vec<f64>> {
    let bundle: serde_json::Value = serde_json::from_str(text).expect("bundle is JSON");
    let mut out = HashMap::new();
    for trace in bundle["traces"].as_array().expect("traces array") {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(trace["y_int16_b64"].as_str().expect("b64 field"))
            .expect("valid base64");
        let values: Vec<f64> = raw
            .chunks(2)
            .map(|c| f64::from(i16::from_le_bytes([c[0], c[1]])) / 100.0)
            .collect();
        out.insert(
            trace["name"].as_str().expect("trace name").to_owned(),
            values,
        );
    }
    out
}

fn assert_bundle_parity(figure_name: &str, rust_text: &str, golden_text: &str) {
    let rust_bundle: serde_json::Value =
        serde_json::from_str(rust_text).expect("rust bundle is JSON");
    eprintln!(
        "{figure_name}: rust freq_min={} freq_max={} points={}",
        rust_bundle["freq_min"], rust_bundle["freq_max"], rust_bundle["points"]
    );
    let rust = decode_bundle(rust_text);
    let golden = decode_bundle(golden_text);
    assert!(!golden.is_empty(), "golden bundle has no traces");
    for (name, golden_values) in &golden {
        let rust_values = rust
            .get(name)
            .unwrap_or_else(|| panic!("{figure_name}: trace {name} missing in Rust bundle"));
        assert_eq!(
            rust_values.len(),
            golden_values.len(),
            "{figure_name}/{name}: grid length differs"
        );
        let (max_diff, argmax) = rust_values
            .iter()
            .zip(golden_values)
            .enumerate()
            .map(|(i, (a, b))| ((a - b).abs(), i))
            .fold(
                (0.0_f64, 0usize),
                |acc, x| if x.0 > acc.0 { x } else { acc },
            );
        assert!(
            max_diff <= 0.011,
            "{figure_name}/{name}: max |dB| diff {max_diff:.4} at index {argmax} \
             (rust {} vs golden {}) exceeds 0.011",
            rust_values[argmax],
            golden_values[argmax]
        );
    }
}

fn load_spin(
    root: &std::path::Path,
    speaker: &str,
    version: &str,
    format: &str,
    symmetry: Option<&str>,
) -> (spinorama_generate::compute::cea2034::SpinData, (f64, f64)) {
    let loaded = load_measurement(format, &root.join(speaker).join(version), speaker)
        .expect("measurement loads");
    let LoadedMeasurement::Hv(hv) = loaded else {
        panic!("{format} must load as HV planes");
    };
    let hv = symmetrise_hv(&hv, symmetry).expect("symmetry applies");
    // Same valid-range derivation as generate-graph: data bounds
    // clamped to 20 Hz–20 kHz (drives the N/A zone traces).
    let data_min = hv
        .horizontal
        .freq
        .first()
        .copied()
        .unwrap_or(20.0)
        .max(hv.vertical.freq.first().copied().unwrap_or(20.0));
    let data_max = hv
        .horizontal
        .freq
        .last()
        .copied()
        .unwrap_or(20_000.0)
        .min(hv.vertical.freq.last().copied().unwrap_or(20_000.0));
    let valid_range = (data_min.max(20.0), data_max.min(20_000.0));
    (compute_spin_data(&hv).expect("spin computes"), valid_range)
}

#[test]
fn kef_r3_klippel_bundle_matches_python() {
    let Some(root) = measurements_dir() else {
        return;
    };
    let (data, valid_range) = load_spin(&root, "KEF R3", "asr", "klippel", None);
    let figures = spinorama_generate::figures::spin_figures(
        &data,
        "KEF R3",
        "Audio Science Review",
        valid_range,
    );
    for (stem, fixture) in [
        ("CEA2034", "kef-r3-asr-cea2034.bundle.json"),
        ("On Axis", "kef-r3-asr-on-axis.bundle.json"),
    ] {
        let figure = figures
            .iter()
            .find(|f| f.name == stem)
            .unwrap_or_else(|| panic!("figure {stem} generated"));
        let golden = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
        )
        .expect("golden fixture readable");
        assert_bundle_parity(stem, &to_json_string(figure).expect("bundle"), &golden);
    }
}

#[test]
fn genelec_8351a_princeton_bundle_matches_python() {
    let Some(root) = measurements_dir() else {
        return;
    };
    let (data, valid_range) = load_spin(
        &root,
        "Genelec 8351A",
        "princeton",
        "princeton",
        Some("horizontal"),
    );
    let figures = spinorama_generate::figures::spin_figures(
        &data,
        "Genelec 8351A",
        "Princeton 3D3A",
        valid_range,
    );
    let figure = figures
        .iter()
        .find(|f| f.name == "CEA2034")
        .expect("CEA2034 generated");
    let golden = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/genelec-8351a-princeton-cea2034.bundle.json"),
    )
    .expect("golden fixture readable");
    assert_bundle_parity("CEA2034", &to_json_string(figure).expect("bundle"), &golden);
}

#[test]
fn neumann_kh80_preference_score_matches_python() {
    let Some(root) = measurements_dir() else {
        return;
    };
    let (data, _valid_range) =
        load_spin(&root, "Neumann KH 80", "asr-v3-20200711", "klippel", None);
    let metrics = preference_score(&data.cea).expect("score computes");
    // Python speaker_pref_rating on the same data (Nov 2026):
    // pref_score 5.8919, pref_score_wsub 8.2559. The Rust (AutoEQ)
    // score deviates from Python in both directions depending on the
    // speaker (KH 80: Rust +0.16; KEF R3: Rust -0.11 vs the Python
    // APO header), so this is a regression guard at 0.2, not exact
    // parity — closing the gap means aligning the NBD/SM/LFX
    // component implementations with Python.
    assert!(
        (metrics.pref_score - 5.8919).abs() <= 0.2,
        "pref_score {} vs Python 5.8919",
        metrics.pref_score
    );
}
