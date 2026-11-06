//! Integration test for the `generate-headphone-graph` binary.
//!
//! Builds a one-headphone scratch catalogue and checks the private
//! Plotly payloads (base + EQ variant) land in the Python layout.

// Rust guideline compliant 2026-10-09

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

#[test]
fn headphone_graph_writes_python_layout() {
    let root: PathBuf =
        std::env::temp_dir().join(format!("spinorama-hp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let hp = "Test HP";
    write(
        &root.join(format!("datas/headphones/{hp}/asr/frequency_response.csv")),
        "frequency,raw\n20,80\n100,85\n1000,90\n10000,85\n20000,80\n",
    );
    write(
        &root.join("datas/headphone_targets/harman_overear_2019.csv"),
        "frequency,spl\n20,70\n1000,75\n20000,70\n",
    );
    write(
        &root.join(format!("datas/headphone_eq/{hp}/iir-autoeq-score.txt")),
        "Filter  1: ON PK Fc 1000 Hz Gain -3.00 dB Q 1.00\n",
    );
    let metadata = serde_json::json!({
        hp: {
            "brand": "Test", "model": "HP", "shape": "over-ear",
            "default_measurement": "asr",
            "measurements": {"asr": {"origin": "ASR"}},
        }
    });
    write(
        &root.join("dist/json/headphone_metadata.json"),
        &metadata.to_string(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_generate-headphone-graph"))
        .args([
            "--headphones",
            "--force",
            "--no-public-images",
            "--data-dir",
        ])
        .arg(&root)
        .output()
        .expect("run generate-headphone-graph");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let base = root.join(format!("dist-private/headphones/{hp}/ASR/asr"));
    for name in [
        "Frequency Response",
        "Frequency Response Compensated",
        "Target Deviation",
    ] {
        let payload: Value = serde_json::from_str(
            &std::fs::read_to_string(base.join(format!("{name}.json"))).expect("json"),
        )
        .expect("parse");
        assert_eq!(
            payload["layout"]["title"]["text"],
            format!(
                "Test HP - {}",
                name.replace(" Compensated", " - vs Harman Target")
                    .replace("Frequency Response - vs Harman Target", "vs Harman Target")
            )
        );
        assert!(base.join("traces").join(format!("{name}.json")).is_file());
    }
    let eq = root.join(format!(
        "dist-private/headphones/{hp}/ASR/asr_eq_autoeq_score"
    ));
    let eq_payload: Value = serde_json::from_str(
        &std::fs::read_to_string(eq.join("Frequency Response.json")).expect("eq json"),
    )
    .expect("parse eq");
    assert_eq!(eq_payload["data"].as_array().expect("traces").len(), 3);
    let _ = std::fs::remove_dir_all(&root);
}
