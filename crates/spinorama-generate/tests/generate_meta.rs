//! Tests for the `generate-meta` assembly helpers.

use std::io::Write;
use std::path::PathBuf;

use spinorama_generate::generate_meta::{
    compute_near, derived_quality, dump_metadata, interpolate, near_freq_grid, parse_rew_eq,
    preamp_gain, round1, scaled_flatness, scaled_lfx_hz, scaled_pref_score, scaled_sm_pir,
    sensitivity_at_1m,
};
use spinorama_generate::metadata::load_metadata_dir;

#[test]
fn scaled_buckets_match_python() {
    assert_eq!(scaled_pref_score(7.1), 100.0);
    assert_eq!(scaled_pref_score(6.2), 80.0);
    assert_eq!(scaled_pref_score(0.5), 0.0);
    assert_eq!(scaled_flatness(1.9), 100.0);
    assert_eq!(scaled_flatness(2.6), 70.0);
    assert_eq!(scaled_flatness(6.5), 0.0);
    assert_eq!(scaled_lfx_hz(37.0), 70.0);
    assert_eq!(scaled_lfx_hz(300.0), 0.0);
    assert_eq!(scaled_sm_pir(0.96), 100.0);
    assert_eq!(scaled_sm_pir(0.55), 25.0);
    // Python bug kept on purpose: scaled_sm uses the LFX bucketer.
    assert_eq!(scaled_lfx_hz(0.807), 100.0);
}

#[test]
fn sensitivity_distance_table() {
    assert_eq!(sensitivity_at_1m(80.0, 1.0), 80.0);
    assert_eq!(sensitivity_at_1m(80.0, 2.0), 86.0);
    assert_eq!(sensitivity_at_1m(80.0, 3.0), 89.5);
    assert_eq!(sensitivity_at_1m(80.0, 10.0), 100.0);
    assert_eq!(sensitivity_at_1m(80.0, 1.7), 80.0);
}

#[test]
fn rew_parser_types_and_defaults() {
    let dir = std::env::temp_dir().join(format!("spinorama-meta-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("iir-test.txt");
    let mut file = std::fs::File::create(&path).expect("create");
    writeln!(file, "Filter  1: ON PK Fc   245 Hz Gain -0.46 dB Q 2.42").expect("write");
    writeln!(file, "Filter  2: ON LS Fc    69 Hz Gain +2.87 dB Q 0.66").expect("write");
    writeln!(file, "Filter  3: ON NO Fc  1000 Hz Gain 0.0 dB Q 1.00").expect("write");
    writeln!(file, "Filter  4: OFF PK Fc   500 Hz Gain +1.00 dB Q 1.00").expect("write");
    writeln!(file, "Filter  5: ON HP Fc    30 Hz Gain 0.0 dB").expect("write");
    writeln!(
        file,
        "Filter  6: ON PK Fc   800 Hz Gain +1.00 dB BW Oct 1.00"
    )
    .expect("write");
    drop(file);

    let filters = parse_rew_eq(&path, 48_000.0);
    assert_eq!(filters.len(), 6);
    assert_eq!(filters[0].type_code, 3); // PK
    assert_eq!(filters[1].type_code, 5); // LS
    assert_eq!(filters[2].type_code, 4); // NO
    assert_eq!(filters[2].q, 30.0, "notch Q is forced to 30");
    assert_eq!(filters[3].weight, 0.0, "OFF filter kept with weight 0");
    assert_eq!(filters[4].type_code, 1); // HP
    assert!(
        (filters[4].q - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12,
        "HP default Q"
    );
    // BW Oct 1.0 -> bw2q(1) = sqrt(2)/(2-1)
    assert!((filters[5].q - std::f64::consts::SQRT_2).abs() < 1e-9);
    let json = filters[0].to_json();
    assert_eq!(json["type"], 3);
    assert_eq!(json["freq"], 245.0);
    assert_eq!(json["srate"], 48_000.0);

    let missing = parse_rew_eq(&dir.join("nope.txt"), 48_000.0);
    assert!(missing.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preamp_gain_is_negative_peak() {
    let dir = std::env::temp_dir().join(format!("spinorama-meta-test2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("iir.txt");
    std::fs::write(&path, "Filter  1: ON PK Fc   245 Hz Gain -3.00 dB Q 1.00\n").expect("write");
    let filters = parse_rew_eq(&path, 48_000.0);
    let gain = preamp_gain(&filters);
    assert!(
        gain <= 0.0 && gain > -0.5,
        "pure cut needs ~0 preamp, got {gain}"
    );
    assert_eq!(preamp_gain(&[]), 0.0);
    assert_eq!(round1(-2.85), -2.8);
    assert_eq!(round1(-2.75), -2.8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interpolate_and_near() {
    let grid = near_freq_grid();
    assert_eq!(grid.len(), 100);
    assert!((grid[0] - 25.0).abs() < 1e-9);
    assert!((grid[99] - 16_000.0).abs() < 1e-6);

    let freq1 = vec![100.0, 200.0, 400.0];
    let data1 = vec![0.0, 10.0, 20.0];
    let out = interpolate(&[150.0, 300.0], &freq1, &data1);
    assert!((out[0] - 5.0).abs() < 1e-9);
    assert!((out[1] - 15.0).abs() < 1e-9);

    let a = (vec![1.0, 2.0], vec![0.0, 0.0], vec![3.0, 4.0]);
    let b = (vec![1.0, 2.0], vec![0.0, 0.0], vec![3.0, 4.0]);
    assert_eq!(compute_near(&a, &b), 0.0);
    let c = (vec![2.0, 2.0], vec![0.0, 0.0], vec![3.0, 4.0]);
    assert!(compute_near(&a, &c) > 0.0);
}

#[test]
fn derived_quality_rules() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../datas/measurements");
    let speakers = load_metadata_dir(&root).expect("catalogue loads");
    let kef = &speakers["KEF R3"];
    assert_eq!(derived_quality(kef, "asr"), "high");
    // Vendor webplotdigitizer from a non-Harman brand stays unknown.
    assert_eq!(derived_quality(kef, "vendor"), "unknown");
}

#[test]
fn dump_writes_head_and_year_chunks() {
    let dir = std::env::temp_dir().join(format!("spinorama-meta-test3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut meta = std::collections::BTreeMap::new();
    for i in 0..25 {
        let year = 2026 - i / 5;
        let record = serde_json::json!({
            "brand": "Test",
            "model": format!("S{i}"),
            "default_measurement": "asr",
            "measurements": {"asr": {"origin": "ASR", "format": "klippel",
                "review_published": format!("{year}0101")}},
            "eqs": {"autoeq": {"type": "peq", "peq": []}}
        });
        meta.insert(format!("Test S{i}"), record);
    }
    dump_metadata(&dir, &meta).expect("dump");
    for name in [
        "metadata.json",
        "metadata.json.zip",
        "metadata.json.bz2",
        "eqdata.json",
        "metadata-head.json",
        "metadata-2022.json",
    ] {
        assert!(dir.join(name).is_file(), "missing {name}");
    }
    let head: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("metadata-head.json")).unwrap())
            .unwrap();
    assert_eq!(head.as_object().map_or(0, |o| o.len()), 20);
    let full: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();
    // eqs are split out of the main metadata file.
    assert!(
        full["Test S0"].get("eqs").is_none(),
        "metadata.json must not carry eqs"
    );
    let eqdata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("eqdata.json")).unwrap()).unwrap();
    assert!(eqdata["Test S0"].get("eqs").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
