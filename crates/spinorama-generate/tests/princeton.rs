//! Princeton MAT v4 loader tests with a synthetic impulse IR.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;

use spinorama_generate::loaders::load_hv;

fn write_mat_v4(path: &std::path::Path, suffix: &str) {
    // fs = 96000 (1x1), IR = 72 x 2048 with impulses on rows 0 and 2.
    let mut bytes: Vec<u8> = Vec::new();
    let mut variable = |name: &str, rows: usize, cols: usize, data: &[f64]| {
        bytes.extend_from_slice(&1000i32.to_be_bytes());
        #[expect(clippy::cast_possible_truncation, reason = "test sizes fit in i32")]
        bytes.extend_from_slice(&(rows as i32).to_be_bytes());
        #[expect(clippy::cast_possible_truncation, reason = "test sizes fit in i32")]
        bytes.extend_from_slice(&(cols as i32).to_be_bytes());
        bytes.extend_from_slice(&0i32.to_be_bytes());
        #[expect(clippy::cast_possible_truncation, reason = "name fits in i32")]
        bytes.extend_from_slice(&((name.len() + 1) as i32).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        // Column-major storage.
        for col in 0..cols {
            for row in 0..rows {
                bytes.extend_from_slice(&data[row * cols + col].to_be_bytes());
            }
        }
    };
    variable(&format!("fs_{suffix}"), 1, 1, &[96_000.0]);
    let mut ir = vec![0.0f64; 72 * 2048];
    ir[0] = 1.0; // row 0 (On Axis), sample 0
    ir[2 * 2048] = 0.5; // row 2 (10 degrees), sample 0
    variable(&format!("IR_{suffix}"), 72, 2048, &ir);
    std::fs::write(path, bytes).expect("write mat file");
}

#[test]
fn princeton_impulse_gives_flat_105_db() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("spinorama-princeton-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    write_mat_v4(&dir.join("Test_H_IR.mat"), "H");
    write_mat_v4(&dir.join("Test_V_IR.mat"), "V");

    let hv = load_hv("princeton", &dir, "test").expect("princeton loads");
    let on_axis = hv.horizontal.column("On Axis").expect("On Axis present");
    assert!(
        on_axis.iter().all(|v| (v - 105.0).abs() < 0.01),
        "unit impulse must give flat 105 dB, got {on_axis:?}"
    );
    let ten = hv.horizontal.column("10°").expect("10 degree present");
    assert!(
        ten.iter().all(|v| (v - 98.98).abs() < 0.01),
        "half impulse must give 105 - 6.02 dB, got {ten:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
