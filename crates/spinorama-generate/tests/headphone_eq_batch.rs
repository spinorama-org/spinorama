//! Batch EQ must select the target from the split headphone catalogue.

use std::process::Command;

#[test]
fn batch_in_ear_uses_in_ear_target_from_catalogue_shard() {
    let root = std::env::temp_dir().join(format!("headphone-eq-shard-{}", std::process::id()));
    let datas = root.join("datas");
    let curve = datas.join("headphones/Test IEM/asr");
    let targets = datas.join("headphone_targets");
    std::fs::create_dir_all(&curve).unwrap();
    std::fs::create_dir_all(&targets).unwrap();
    std::fs::write(
        datas.join("headphones.py"),
        "from .headphone_t import headphones_info_t\nheadphones_info = headphones_info_t\n",
    )
    .unwrap();
    std::fs::write(
        datas.join("headphone_t.py"),
        "headphones_info_t = {'Test IEM': {'shape': 'in-ear'}}\n",
    )
    .unwrap();
    let csv = "frequency,spl\n20,70\n100,70\n500,70\n1000,70\n3000,70\n6000,70\n20000,70\n";
    std::fs::write(curve.join("frequency_response.csv"), csv).unwrap();
    // An over-ear fallback must fail because only the correct target exists.
    std::fs::write(targets.join("harman_inear_2019.csv"), csv).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_generate-peq-headphone"))
        .args([
            "--batch",
            "--only",
            "Test IEM",
            "--maxeval",
            "1",
            "--max-peq",
            "1",
            "--jobs",
            "1",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for file in [
        "iir-autoeq-score.txt",
        "iir-autoeq-flat.txt",
        "autoeq_score.json",
    ] {
        assert!(datas.join("headphone_eq/Test IEM").join(file).is_file());
    }
    std::fs::remove_dir_all(root).unwrap();
}
