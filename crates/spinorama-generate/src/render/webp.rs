//! Public WebP hardening for rendered graph PNGs.
//!
//! Rust port of Python `image_export.harden_png_to_webp`: a seeded fine
//! texture plus a smooth luminance drift, then a repeated diagonal
//! attribution watermark, then WebP encoding. The data geometry is
//! never distorted. One difference from Python: the RNG is a
//! splitmix64 seeded from the same salt/seed text (image bytes are not
//! part of the parity contract). Encoding is libwebp through the
//! `webp` crate (bundled libwebp sources, NEON on aarch64), lossy at a
//! configurable quality like Pillow's libwebp path, or lossless.

// Rust guideline compliant 2026-02-21

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fontdue::{Font, FontSettings};
use image::{RgbImage, RgbaImage};

/// Base watermark line burned into public pictures.
pub const WATERMARK_TEXT: &str = "spinorama.org  CC BY-NC-SA 4.0";

/// Returns the full watermark line for a measurement source label.
#[must_use]
pub fn watermark_text(source: &str) -> String {
    let source = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if source.is_empty() {
        WATERMARK_TEXT.to_owned()
    } else {
        format!("{WATERMARK_TEXT}  |  {source}")
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed_text: &str) -> Self {
        let salt = std::env::var("SPINORAMA_IMAGE_SALT").unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        format!("{salt}|{seed_text}").hash(&mut hasher);
        Self(hasher.finish())
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_u8(&mut self) -> u8 {
        (self.next_u64() & 0xFF) as u8
    }
}

fn load_font(explicit: Option<&Path>) -> Option<Font> {
    let env_font = std::env::var("SPINORAMA_FONT").unwrap_or_default();
    let candidates: Vec<PathBuf> = [
        explicit.map(Path::to_path_buf),
        (!env_font.is_empty()).then(|| PathBuf::from(env_font)),
        Some(PathBuf::from(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        )),
        Some(PathBuf::from("/usr/share/fonts/TTF/DejaVuSans.ttf")),
        Some(PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        )),
        Some(PathBuf::from("/opt/homebrew/share/fonts/DejaVuSans.ttf")),
    ]
    .into_iter()
    .flatten()
    .collect();
    for candidate in candidates {
        if candidate.is_file()
            && let Ok(bytes) = std::fs::read(&candidate)
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default())
        {
            return Some(font);
        }
    }
    None
}

fn text_width(font: &Font, text: &str, px: f32) -> i32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "text width fits in i32 pixels"
    )]
    let width = text
        .chars()
        .map(|c| f64::from(font.metrics(c, px).advance_width))
        .sum::<f64>() as i32;
    width
}

fn draw_text(overlay: &mut RgbaImage, font: &Font, text: &str, px: f32, x0: i32, y0: i32) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "font size fits in i32 pixels"
    )]
    let baseline = y0 + px as i32;
    let mut pen_x = f64::from(x0);
    for c in text.chars() {
        let (metrics, bitmap) = font.rasterize(c, px);
        #[expect(clippy::cast_possible_truncation, reason = "pen position fits in i32")]
        let gx = pen_x as i32 + metrics.xmin;
        let gy = baseline - metrics.height as i32 - metrics.ymin;
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + col];
                if coverage == 0 {
                    continue;
                }
                let px_x = gx + col as i32;
                let px_y = gy + row as i32;
                if px_x < 0 || px_y < 0 {
                    continue;
                }
                #[expect(clippy::cast_sign_loss, reason = "coordinates checked non-negative")]
                let (ux, uy) = (px_x as u32, px_y as u32);
                if ux >= overlay.width() || uy >= overlay.height() {
                    continue;
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "scaled coverage fits in u8"
                )]
                let alpha = (u32::from(coverage) * 30 / 255) as u8;
                let pixel = overlay.get_pixel_mut(ux, uy);
                if alpha > pixel[3] {
                    *pixel = image::Rgba([65, 65, 65, alpha]);
                }
            }
        }
        pen_x += f64::from(metrics.advance_width);
    }
}

fn rotate_bilinear(img: &RgbaImage, degrees: f64) -> RgbaImage {
    let (width, height) = (img.width(), img.height());
    let cx = f64::from(width) / 2.0;
    let cy = f64::from(height) / 2.0;
    let theta = degrees.to_radians();
    let (sin, cos) = theta.sin_cos();
    let mut out = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let dx = f64::from(x) - cx;
            let dy = f64::from(y) - cy;
            // PIL rotate() turns counter-clockwise; sample the inverse.
            let sx = dx * cos - dy * sin + cx;
            let sy = dx * sin + dy * cos + cy;
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(width - 1) || sy >= f64::from(height - 1) {
                continue;
            }
            #[expect(clippy::cast_sign_loss, reason = "sample coordinates checked in range")]
            let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
            let (fx, fy) = (sx - f64::from(x0), sy - f64::from(y0));
            let mut channels = [0f64; 4];
            for (weight, xx, yy) in [
                ((1.0 - fx) * (1.0 - fy), x0, y0),
                (fx * (1.0 - fy), x0 + 1, y0),
                ((1.0 - fx) * fy, x0, y0 + 1),
                (fx * fy, x0 + 1, y0 + 1),
            ] {
                let pixel = img.get_pixel(xx, yy);
                for (channel, value) in channels.iter_mut().zip(pixel.0.iter()) {
                    *channel += weight * f64::from(*value);
                }
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "channel values are in 0..=255"
            )]
            let pixel = image::Rgba(channels.map(|v| v.round().clamp(0.0, 255.0) as u8));
            out.put_pixel(x, y, pixel);
        }
    }
    out
}

/// Applies anti-scraping hardening to PNG bytes and encodes as WebP.
///
/// Deterministic for a given `seed_text` (usually
/// `Speaker/Origin/Version/Graph`). When no font file is available the
/// texture layers still apply and the text watermark is skipped.
/// `quality` selects libwebp lossy encoding (0–100, method 4,
/// multithreaded); `None` encodes losslessly.
///
/// # Errors
///
/// Returns an error when the PNG cannot be decoded or WebP encoding fails.
pub fn harden_png_to_webp(
    png_bytes: &[u8],
    seed_text: &str,
    watermark: &str,
    font_path: Option<&Path>,
    quality: Option<f32>,
) -> Result<Vec<u8>> {
    let font = load_font(font_path);
    harden_impl(png_bytes, seed_text, watermark, font.as_ref(), quality)
}

fn harden_impl(
    png_bytes: &[u8],
    seed_text: &str,
    watermark: &str,
    font: Option<&Font>,
    quality: Option<f32>,
) -> Result<Vec<u8>> {
    let mut img: RgbImage = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
        .context("decoding rendered PNG")?
        .to_rgb8();
    let (width, height) = (img.width(), img.height());
    let mut rng = Rng::new(seed_text);

    for pixel in img.pixels_mut() {
        let noise = f64::from(rng.next_u8());
        for channel in pixel.0.iter_mut() {
            #[expect(clippy::cast_possible_truncation, reason = "blend stays in 0..=255")]
            let blended = (f64::from(*channel) * 0.965 + noise * 0.035).round() as u8;
            *channel = blended;
        }
    }
    let (small_w, small_h) = ((width / 16).max(8), (height / 16).max(8));
    let mut small = RgbImage::new(small_w, small_h);
    for pixel in small.pixels_mut() {
        let value = rng.next_u8();
        *pixel = image::Rgb([value, value, value]);
    }
    let blotch = image::imageops::resize(
        &small,
        width,
        height,
        image::imageops::FilterType::CatmullRom,
    );
    for (pixel, blotch_pixel) in img.pixels_mut().zip(blotch.pixels()) {
        for (channel, blotch_channel) in pixel.0.iter_mut().zip(blotch_pixel.0.iter()) {
            #[expect(clippy::cast_possible_truncation, reason = "blend stays in 0..=255")]
            let blended =
                (f64::from(*channel) * 0.97 + f64::from(*blotch_channel) * 0.03).round() as u8;
            *channel = blended;
        }
    }

    if let Some(font) = font {
        #[expect(clippy::cast_precision_loss, reason = "image dimensions fit in f32")]
        let font_px = (width / 44).max(20) as f32;
        #[expect(clippy::cast_possible_wrap, reason = "image dimensions fit in i32")]
        let step_y = (height as i32 / 5).max(120);
        #[expect(clippy::cast_possible_wrap, reason = "image dimensions fit in i32")]
        let step_x = ((width as i32 / 2).max(360)).max(text_width(font, watermark, font_px) + 80);
        let mut overlay = RgbaImage::new(width, height);
        let mut row = 0i32;
        #[expect(clippy::cast_possible_wrap, reason = "image dimensions fit in i32")]
        let height_i = height as i32;
        #[expect(clippy::cast_possible_wrap, reason = "image dimensions fit in i32")]
        let width_i = width as i32;
        let mut y = -step_y;
        while y < height_i + step_y {
            let x_offset = (row % 2) * (step_x / 2) - step_x / 3;
            let mut x = x_offset;
            while x < width_i + step_x {
                draw_text(&mut overlay, font, watermark, font_px, x, y);
                x += step_x;
            }
            y += step_y;
            row += 1;
        }
        let overlay = rotate_bilinear(&overlay, 24.0);
        let mut rgba = image::DynamicImage::ImageRgb8(img).to_rgba8();
        image::imageops::overlay(&mut rgba, &overlay, 0, 0);
        img = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    }

    encode_webp(&img, quality)
}

fn encode_webp(img: &RgbImage, quality: Option<f32>) -> Result<Vec<u8>> {
    let encoder = webp::Encoder::from_rgb(img.as_raw(), img.width(), img.height());
    let memory = match quality {
        Some(quality) => {
            let mut config = webp::WebPConfig::new()
                .map_err(|()| anyhow::anyhow!("libwebp config init failed"))?;
            config.lossless = 0;
            config.quality = quality.clamp(0.0, 100.0);
            config.method = 4;
            config.thread_level = 1;
            encoder
                .encode_advanced(&config)
                .map_err(|error| anyhow::anyhow!("libwebp encode failed: {error:?}"))?
        }
        None => encoder.encode_lossless(),
    };
    Ok(memory.to_vec())
}

/// Converts every PNG under `png_root` to a hardened WebP under
/// `webp_root`, mirroring relative paths.
///
/// Files are converted in parallel across CPU cores (the font is loaded
/// once). Returns the number of images written. Existing WebP files
/// are overwritten; the write is atomic per file
/// (`<name>.webp.tmp` rename).
///
/// # Errors
///
/// Returns an error when a PNG cannot be read, hardened, or written.
pub fn convert_png_tree(
    png_root: &Path,
    webp_root: &Path,
    seed_prefix: &str,
    watermark: &str,
    font_path: Option<&Path>,
    quality: Option<f32>,
) -> Result<usize> {
    let mut pngs = Vec::new();
    collect_pngs(png_root, &mut pngs)?;
    pngs.sort();
    if pngs.is_empty() {
        return Ok(0);
    }
    let font = load_font(font_path);
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(8);
    let chunk = pngs.len().div_ceil(threads).max(1);
    let results: Vec<Result<usize>> = std::thread::scope(|scope| {
        let handles: Vec<_> = pngs
            .chunks(chunk)
            .map(|chunk| {
                let font = &font;
                scope.spawn(move || -> Result<usize> {
                    let mut written = 0usize;
                    for png in chunk {
                        let relative = png.strip_prefix(png_root).unwrap_or(png);
                        let stem = relative.with_extension("");
                        let seed = format!("{seed_prefix}/{}", stem.to_string_lossy());
                        let bytes = std::fs::read(png)
                            .with_context(|| format!("reading {}", png.display()))?;
                        let webp = harden_impl(&bytes, &seed, watermark, font.as_ref(), quality)?;
                        let target = webp_root.join(&stem).with_extension("webp");
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent)
                                .with_context(|| format!("creating {}", parent.display()))?;
                        }
                        let tmp = target.with_extension("webp.tmp");
                        std::fs::write(&tmp, webp)
                            .with_context(|| format!("writing {}", tmp.display()))?;
                        std::fs::rename(&tmp, &target)
                            .with_context(|| format!("renaming to {}", target.display()))?;
                        written += 1;
                    }
                    Ok(written)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("WebP worker thread panicked")))
            })
            .collect()
    });
    let mut written = 0usize;
    for result in results {
        written += result?;
    }
    Ok(written)
}

fn collect_pngs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_pngs(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "png") {
            out.push(path);
        }
    }
    Ok(())
}
