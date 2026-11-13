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
    // Exact count on purpose: an accidental speaker drop or duplicate
    // in the catalogue must fail loudly. Bump when speakers are added.
    assert_eq!(report.speakers, 1169);
}

fn metadata_fixture(label: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("spinorama-meta-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let measurements = root.join("measurements");
    let speaker = measurements.join("Test_Speaker");
    std::fs::create_dir_all(speaker.join("asr")).unwrap();
    std::fs::write(
        root.join("origins.toml"),
        "[origins.ASR]\n\"min hz\" = 20\n\"max hz\" = 20000\n\"min dB\" = -40\n\"max dB\" = 10\n",
    )
    .unwrap();
    let path = speaker.join("meta.toml");
    std::fs::write(&path, "name = \"Test|Speaker\"\nbrand = \"Test\"\nmodel = \"Speaker\"\ntype = \"passive\"\nprice = \"1\"\nshape = \"bookshelves\"\ndefault_measurement = \"asr\"\n[measurements.asr]\norigin = \"ASR\"\nformat = \"rew_text_dump\"\n").unwrap();
    (measurements, path)
}

#[test]
fn metadata_consistency_rules_reject_bad_fields() {
    let (root, path) = metadata_fixture("names");
    let valid = std::fs::read_to_string(&path).unwrap();
    for (from, to, error) in [
        ("brand = \"Test\"", "brand = \"\"", "brand is empty"),
        ("brand = \"Test\"", "brand = \"Other\"", "start with brand"),
        ("model = \"Speaker\"", "model = \"\"", "model is empty"),
        ("model = \"Speaker\"", "model = \"Other\"", "end with model"),
        (
            "default_measurement = \"asr\"",
            "default_measurement = \"missing\"",
            "not in measurements",
        ),
        ("origin = \"ASR\"", "origin = \"Unknown\"", "unknown origin"),
        (
            "[measurements.asr]\norigin = \"ASR\"\nformat = \"rew_text_dump\"",
            "[measurements]",
            "at least one measurement",
        ),
    ] {
        std::fs::write(&path, valid.replace(from, to)).unwrap();
        let report = check_metadata(&root).unwrap();
        assert!(
            report.errors.iter().any(|message| message.contains(error)),
            "{:?}",
            report.errors
        );
    }
    std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

#[test]
fn check_files_loads_measurements_and_reports_broken_or_skipped_sources() {
    use std::process::Command;
    let (root, path) = metadata_fixture("files");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_generate-graph"))
            .arg("--metadata-dir")
            .arg(&root)
            .args(["--check-meta", "--check-files"])
            .output()
            .unwrap()
    };
    let broken = run();
    assert!(!broken.status.success());
    assert!(String::from_utf8_lossy(&broken.stderr).contains("could not be loaded"));
    let version = path.parent().unwrap().join("asr");
    for file in ["On Axis.txt", "ER.txt", "LW.txt", "SP.txt"] {
        std::fs::write(version.join(file), "20 85\n20000 85\n").unwrap();
    }
    let valid = run();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    std::fs::write(version.join("On Axis.txt"), "not measurement data\n").unwrap();
    let corrupt = run();
    assert!(!corrupt.status.success());
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("could not be loaded"));
    std::fs::remove_dir_all(&version).unwrap();
    let missing = run();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing directory"));
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("[measurements.asr]", "skip = true\n[measurements.asr]");
    std::fs::write(&path, text).unwrap();
    assert!(
        run().status.success(),
        "skipped speakers must not load data"
    );
    std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
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
