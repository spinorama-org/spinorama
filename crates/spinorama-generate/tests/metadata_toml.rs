//! TOML metadata loading and validation tests.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;

use spinorama_generate::metadata::{catalogue_from_dir, check_metadata, load_metadata_dir};

fn repo_measurements() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../datas/measurements");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipping: {} not found", dir.display());
        None
    }
}

#[test]
fn catalogue_loads_from_toml() {
    let Some(root) = repo_measurements() else {
        return;
    };
    let speakers = load_metadata_dir(&root).expect("catalogue parses");
    assert!(
        speakers.len() > 1000,
        "expected the full catalogue, got {}",
        speakers.len()
    );
    let kef = speakers.get("KEF R3").expect("KEF R3 present");
    assert_eq!(kef.default_measurement, "asr");
    assert!(kef.measurements.contains_key("eac"));
}

#[test]
fn catalogue_entries_skip_flagged_speakers() {
    let Some(root) = repo_measurements() else {
        return;
    };
    let entries = catalogue_from_dir(&root).expect("entries build");
    assert!(entries.len() > 1400, "got {}", entries.len());
    assert!(
        entries.iter().all(|e| e.speaker != "Dynaudio Contour 20i"),
        "skip=true speakers must not produce entries"
    );
    let defaults = entries.iter().filter(|e| e.is_default).count();
    assert!(defaults > 1000, "got {defaults} defaults");
}

#[test]
fn metadata_check_passes_on_repository() {
    let Some(root) = repo_measurements() else {
        return;
    };
    let report = check_metadata(&root).expect("check runs");
    assert!(
        report.errors.is_empty(),
        "metadata errors: {:?}",
        &report.errors[..report.errors.len().min(5)]
    );
    assert_eq!(report.speakers, 1167);
}

#[test]
fn unknown_key_is_a_hard_error() {
    let dir = std::env::temp_dir().join("spinorama-meta-bad");
    let speaker_dir = dir.join("Test Speaker");
    std::fs::create_dir_all(&speaker_dir).expect("mkdir");
    std::fs::write(
        speaker_dir.join("meta.toml"),
        "name = \"Test Speaker\"\nbrand = \"Test\"\nmodel = \"T1\"\ntype = \"passive\"\nprice = \"1\"\nshape = \"bookshelves\"\ndefault_measurement = \"asr\"\ntypo_key = 1\n",
    )
    .expect("write meta");
    let error = load_metadata_dir(&dir).expect_err("unknown key must fail");
    assert!(format!("{error:#}").contains("typo_key") || format!("{error:#}").contains("unknown"));
}
