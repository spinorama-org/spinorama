//! Speaker graph CLI output completeness and incremental repair.

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn explicit_renderer_rebuild_invalidates_its_fingerprint() {
    let path = std::env::temp_dir().join(format!("renderer-fingerprint-{}", std::process::id()));
    std::fs::write(&path, b"renderer version one").unwrap();
    let before = spinorama_generate::render::renderer_spec(&path).unwrap();
    std::fs::write(&path, b"renderer version two").unwrap();
    let after = spinorama_generate::render::renderer_spec(&path).unwrap();
    assert_ne!(before, after);
    std::fs::remove_file(path).unwrap();
}

#[test]
#[cfg(unix)]
fn hv_measurements_emit_eq_details_and_repair_missing_images() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = repo.join("datas/measurements/Topping MA4/asr");
    if !source.is_dir() {
        eprintln!("skipping: repository measurement data is absent");
        return;
    }
    let root = std::env::temp_dir().join(format!("graph-outputs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pixel = root.join("pixel.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
        .save(&pixel)
        .unwrap();
    let renderer = root.join("renderer");
    std::fs::write(&renderer, format!(
        "#!/bin/sh\nset -eu\nmkdir -p \"$2\"\nfor input in \"$1\"/*.json; do\ncp '{}' \"$2/$(basename \"$input\" .json).png\"\ndone\n",
        pixel.display()
    )).unwrap();
    std::fs::set_permissions(&renderer, std::fs::Permissions::from_mode(0o755)).unwrap();
    let png = root.join("dist-gpui/speakers/Topping MA4/ASR/asr");
    let webp = root.join("dist/speakers/Topping MA4/ASR/asr");
    let run = || {
        let output = Command::new(env!("CARGO_BIN_EXE_generate-graph"))
            .current_dir(&repo)
            .args([
                "--speaker",
                "Topping MA4",
                "--origin",
                "ASR",
                "--mversion",
                "asr",
            ])
            .arg("--speaker-dir")
            .arg(&source)
            .arg("--data-dir")
            .arg(&root)
            .arg("--png-dir")
            .arg(&png)
            .arg("--webp-dir")
            .arg(&webp)
            .arg("--render-images-bin")
            .arg(&renderer)
            .args(["--render-png", "--render-webp"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run();
    let origin_png = png.parent().unwrap();
    let origin_webp = webp.parent().unwrap();
    for stem in ["filters_eq", "filters_eq_full", "filters_auto_eq"] {
        assert!(
            origin_webp.join(format!("{stem}.webp")).is_file(),
            "missing {stem}"
        );
    }
    let missing = [
        origin_webp.join("filters_eq_full.webp"),
        origin_png.join("filters/filters_eq_full.png"),
        webp.join("CEA2034.webp"),
        png.join("CEA2034.png"),
    ];
    for path in &missing {
        std::fs::remove_file(path).unwrap();
    }
    run();
    assert!(
        missing.iter().all(|path| Path::new(path).is_file()),
        "missing assets were not repaired"
    );
    std::fs::remove_dir_all(root).unwrap();
}
