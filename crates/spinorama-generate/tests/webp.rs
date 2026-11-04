//! WebP hardening contract tests.

// Rust guideline compliant 2026-02-21

use spinorama_generate::render::webp::{harden_png_to_webp, watermark_text};

#[test]
fn harden_is_deterministic_and_produces_webp() {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([255, 255, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .expect("PNG encodes");
    let png_bytes = png.into_inner();

    let first = harden_png_to_webp(
        &png_bytes,
        "Speaker/Origin/v1/CEA2034",
        "test",
        None,
        Some(82.0),
    )
    .expect("harden succeeds");
    let second = harden_png_to_webp(
        &png_bytes,
        "Speaker/Origin/v1/CEA2034",
        "test",
        None,
        Some(82.0),
    )
    .expect("harden succeeds again");
    assert_eq!(first, second, "same seed must give identical bytes");
    assert_eq!(&first[0..4], b"RIFF");
    assert_eq!(&first[8..12], b"WEBP");
    let decoded = image::load_from_memory(&first).expect("WebP decodes");
    assert_eq!((decoded.width(), decoded.height()), (64, 48));
}

#[test]
fn watermark_line_includes_source() {
    assert_eq!(watermark_text(""), "spinorama.org  CC BY-NC-SA 4.0");
    assert_eq!(
        watermark_text("Audio Science Review"),
        "spinorama.org  CC BY-NC-SA 4.0  |  Audio Science Review"
    );
}
