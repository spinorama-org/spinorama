//! `GET /compare.webp`: two-speaker overlay pictures, rendered on the fly.
//!
//! Rust replacement for the Python `scripts/compare_render_server.py` +
//! `spinorama/compare_render.py` pair. The overlay (speaker A solid,
//! speaker B dashed) is drawn from the pristine trace bundles under
//! `<spin_files>/<speaker>/<origin>/<version>/traces/<measurement>.json`,
//! hardened (seeded texture + diagonal watermark + lossy WebP) so the
//! underlying data is never served, and cached on disk by content hash:
//! repeat requests are plain file reads. Renders are serialised and
//! budgeted per minute, mirroring the Python service; budget exhaustion
//! answers 503 with `Retry-After` so the frontend can fall back to the
//! side-by-side pictures.

// Rust guideline compliant 2026-10-09

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::Engine;
use fontdue::{Font, FontSettings};
use image::{Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Cache namespace; bump when the rendered picture changes shape.
pub(crate) const COMPARE_CACHE_VERSION: &str = "compare-v3-rust";

/// Measurements the comparison view may overlay (Python whitelist).
pub(crate) const ALLOWED_MEASUREMENTS: [&str; 14] = [
    "CEA2034",
    "CEA2034 Normalized",
    "On Axis",
    "Estimated In-Room Response",
    "Estimated In-Room Response Normalized",
    "Early Reflections",
    "Horizontal Reflections",
    "Vertical Reflections",
    "SPL Horizontal",
    "SPL Horizontal Normalized",
    "SPL Vertical",
    "SPL Vertical Normalized",
    "Horizontal Directivity",
    "Vertical Directivity",
];

/// Width buckets the frontend may request (Python whitelist).
pub(crate) const ALLOWED_WIDTHS: [u32; 2] = [900, 1400];

/// Attribution line burned into every public picture.
const WATERMARK_TEXT: &str = "spinorama.org  CC BY-NC-SA 4.0";

/// Supersampling factor for anti-aliased drawing (Python `_SS`).
const SS: u32 = 2;

/// Bundle value marking "no data" in the int16 payloads.
const SENTINEL: i16 = -32768;

/// Trace colours when a bundle trace carries none (Python palette).
const FALLBACK_COLORS: [&str; 10] = [
    "#5c77a5", "#dc842a", "#c85857", "#89b5b1", "#71a152", "#bab0ac", "#e15759", "#b07aa1",
    "#76b7b2", "#ff9da7",
];

const GRID: Rgb<u8> = Rgb([205, 205, 205]);
const TEXT: Rgb<u8> = Rgb([35, 35, 35]);
const AXIS: Rgb<u8> = Rgb([120, 120, 120]);

/// Shared compare-render state: cache directory, render budget, and the
/// one-render-at-a-time lock.
pub(crate) struct CompareState {
    cache_dir: PathBuf,
    renders: Mutex<VecDeque<Instant>>,
    render_lock: Mutex<()>,
}

impl CompareState {
    pub(crate) fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            renders: Mutex::new(VecDeque::new()),
            render_lock: Mutex::new(()),
        }
    }

    fn budget_available(&self, now: Instant) -> bool {
        let limit = std::env::var("SPINORAMA_COMPARE_MAX_RENDERS_PER_MIN")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .map_or(20, |value| value.max(1));
        let mut renders = self.renders.lock().unwrap_or_else(|e| e.into_inner());
        while renders
            .front()
            .is_some_and(|first| now.duration_since(*first) > Duration::from_secs(60))
        {
            renders.pop_front();
        }
        if renders.len() >= limit {
            return false;
        }
        renders.push_back(now);
        true
    }
}

/// A rendered (or cached) comparison picture.
pub(crate) struct ComparePicture {
    pub body: Vec<u8>,
    pub cache_hit: bool,
}

/// A client-visible compare failure.
#[derive(Debug)]
pub(crate) struct CompareFailure {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<u32>,
}

impl CompareFailure {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after: None,
        }
    }
}

type CompareResult<T> = std::result::Result<T, CompareFailure>;

fn fail<T>(status: u16, message: impl Into<String>) -> CompareResult<T> {
    Err(CompareFailure::new(status, message))
}

/// One decoded bundle trace.
struct Trace {
    name: String,
    axis: String,
    color: String,
    values: Vec<f64>,
}

/// One decoded trace bundle.
struct Bundle {
    freq_min: f64,
    freq_max: f64,
    points: usize,
    y_range: Option<(f64, f64)>,
    y2_range: Option<(f64, f64)>,
    traces: Vec<Trace>,
}

fn decode_values(trace: &Value) -> Vec<f64> {
    let Some(encoded) = trace.get("y_int16_b64").and_then(Value::as_str) else {
        return Vec::new();
    };
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return Vec::new();
    };
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let value = i16::from_le_bytes(*pair);
            if value == SENTINEL {
                f64::NAN
            } else {
                f64::from(value) / 100.0
            }
        })
        .collect()
}

fn range_of(bundle: &Value, key: &str) -> Option<(f64, f64)> {
    let items = bundle.get(key)?.as_array()?;
    if items.len() != 2 {
        return None;
    }
    Some((items[0].as_f64()?, items[1].as_f64()?))
}

fn parse_bundle(text: &str) -> Result<Bundle> {
    let value: Value = serde_json::from_str(text).context("parsing trace bundle")?;
    let traces = value
        .get("traces")
        .and_then(Value::as_array)
        .map(|traces| {
            traces
                .iter()
                .map(|trace| Trace {
                    name: trace
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    axis: trace
                        .get("axis")
                        .and_then(Value::as_str)
                        .unwrap_or("y")
                        .to_owned(),
                    color: trace
                        .get("color")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    values: decode_values(trace),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Bundle {
        freq_min: value.get("freq_min").and_then(Value::as_f64).unwrap_or(0.0),
        freq_max: value.get("freq_max").and_then(Value::as_f64).unwrap_or(0.0),
        points: value
            .get("points")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(0),
        y_range: range_of(&value, "y_range"),
        y2_range: range_of(&value, "y2_range"),
        traces,
    })
}

impl Bundle {
    fn freqs(&self) -> Vec<f64> {
        if self.points < 2 || self.freq_min <= 0.0 || self.freq_max <= self.freq_min {
            return Vec::new();
        }
        let ratio = self.freq_max / self.freq_min;
        (0..self.points)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "point counts fit in f64")]
                let t = i as f64 / (self.points - 1) as f64;
                self.freq_min * ratio.powf(t)
            })
            .collect()
    }

    fn visible_traces(&self) -> Vec<&Trace> {
        // Drop recommended zones and slope helpers (Python rule).
        self.traces
            .iter()
            .filter(|trace| {
                let name = trace.name.to_lowercase();
                !name.starts_with("recommended")
                    && !name.ends_with(" slope")
                    && !name.ends_with(" zone")
            })
            .collect()
    }
}

/// Union of the bundles' layout ranges for one axis, else data range.
fn axis_range(bundles: &[&Bundle], axis: &str) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (None::<f64>, None::<f64>);
    for bundle in bundles {
        let range = if axis == "y" {
            bundle.y_range
        } else {
            bundle.y2_range
        };
        if let Some((r_lo, r_hi)) = range {
            lo = Some(lo.map_or(r_lo, |v: f64| v.min(r_lo)));
            hi = Some(hi.map_or(r_hi, |v: f64| v.max(r_hi)));
        }
    }
    if let (Some(lo), Some(hi)) = (lo, hi)
        && hi > lo
    {
        return Some((lo, hi));
    }
    let mut values: Vec<f64> = Vec::new();
    for bundle in bundles {
        for trace in &bundle.traces {
            if trace.axis == axis {
                values.extend(trace.values.iter().copied().filter(|v| v.is_finite()));
            }
        }
    }
    if values.is_empty() {
        return None;
    }
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let pad = ((hi - lo) * 0.08).max(1.0);
    Some((lo - pad, hi + pad))
}

fn parse_color(text: &str, fallback_idx: usize) -> Rgb<u8> {
    let text = text.trim();
    if text.len() == 7
        && text.starts_with('#')
        && let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&text[1..3], 16),
            u8::from_str_radix(&text[3..5], 16),
            u8::from_str_radix(&text[5..7], 16),
        )
    {
        return Rgb([r, g, b]);
    }
    if let Some(inner) = text.strip_prefix("rgb(").and_then(|t| t.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() == 3 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "colour channels clamp to u8"
            )]
            #[expect(clippy::cast_sign_loss, reason = "colour channels clamp to u8")]
            let ch = |v: f64| v.round().clamp(0.0, 255.0) as u8;
            return Rgb([ch(parts[0]), ch(parts[1]), ch(parts[2])]);
        }
    }
    parse_color(FALLBACK_COLORS[fallback_idx % FALLBACK_COLORS.len()], 0)
}

/// Formats a tick like Python's `%g` (no trailing zeros).
fn fmt_tick(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "checked integral and bounded"
        )]
        let int = value as i64;
        format!("{int}")
    } else {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn nice_ticks(lo: f64, hi: f64) -> Vec<f64> {
    let span = hi - lo;
    let mut step = 50.0;
    for candidate in [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0] {
        step = candidate;
        if span / step <= 10.0 {
            break;
        }
    }
    let mut ticks = Vec::new();
    let mut value = (lo / step).ceil() * step;
    while value <= hi + 1e-9 {
        ticks.push(value);
        value += step;
    }
    ticks
}

/// Loads the first usable font from the candidate paths.
fn load_font() -> Option<Font> {
    let env_font = std::env::var("SPINORAMA_FONT").unwrap_or_default();
    let candidates = [
        (!env_font.is_empty()).then(|| PathBuf::from(env_font.clone())),
        Some(PathBuf::from(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        )),
        Some(PathBuf::from("/usr/share/fonts/TTF/DejaVuSans.ttf")),
        Some(PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        )),
        Some(PathBuf::from("/opt/homebrew/share/fonts/DejaVuSans.ttf")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Ok(bytes) = std::fs::read(&candidate)
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default())
        {
            return Some(font);
        }
    }
    None
}

fn text_width(font: &Font, text: &str, px: f32) -> f64 {
    text.chars()
        .map(|c| f64::from(font.metrics(c, px).advance_width))
        .sum()
}

/// Draws text with its top-left corner at (`x`, `y`), alpha-blended.
fn draw_text(img: &mut RgbImage, font: &Font, text: &str, px: f32, x: f64, y: f64, color: Rgb<u8>) {
    let mut cursor = x;
    for c in text.chars() {
        let (metrics, bitmap) = font.rasterize(c, px);
        for (i, &coverage) in bitmap.iter().enumerate() {
            if coverage == 0 || metrics.width == 0 {
                continue;
            }
            #[expect(clippy::cast_possible_truncation, reason = "bitmap fits in i32")]
            #[expect(clippy::cast_possible_wrap, reason = "bitmap fits in i32")]
            let bx = (i % metrics.width) as i32;
            #[expect(clippy::cast_possible_truncation, reason = "bitmap fits in i32")]
            #[expect(clippy::cast_possible_wrap, reason = "bitmap fits in i32")]
            let by = (i / metrics.width) as i32;
            let px_x = cursor.round() as i32 + metrics.xmin + bx;
            let px_y = y.round() as i32 - metrics.ymin - metrics.height as i32 + by;
            if px_x < 0 || px_y < 0 {
                continue;
            }
            if let (Ok(ux), Ok(uy)) = (u32::try_from(px_x), u32::try_from(px_y))
                && let Some(pixel) = img.get_pixel_mut_checked(ux, uy)
            {
                let alpha = f64::from(coverage) / 255.0;
                for (channel, target) in pixel.0.iter_mut().zip(color.0.iter()) {
                    #[expect(clippy::cast_possible_truncation, reason = "blend stays in 0..=255")]
                    let blended = (f64::from(*channel) * (1.0 - alpha) + f64::from(*target) * alpha)
                        .round() as u8;
                    *channel = blended;
                }
            }
        }
        cursor += f64::from(metrics.advance_width);
    }
}

/// Deterministic byte RNG seeded like Python `_seed_bytes`
/// (SHA-256 of `"<salt>|<seed>"`), expanded with xorshift64*.
struct Rng(u64);

impl Rng {
    fn new(seed_text: &str) -> Self {
        let salt = std::env::var("SPINORAMA_IMAGE_SALT").unwrap_or_default();
        let digest = Sha256::digest(format!("{salt}|{seed_text}").as_bytes());
        Self(u64::from_le_bytes(
            digest[..8].try_into().unwrap_or([0x9e; 8]),
        ))
    }

    fn next_u8(&mut self) -> u8 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8
    }
}

/// Rotates an RGBA overlay by `degrees` around its centre (bilinear).
fn rotate_overlay(src: &RgbaImage, degrees: f64) -> RgbaImage {
    let (w, h) = (f64::from(src.width()), f64::from(src.height()));
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut out = RgbaImage::new(src.width(), src.height());
    for y in 0..src.height() {
        for x in 0..src.width() {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let (sx, sy) = (cx + dx * cos + dy * sin, cy - dx * sin + dy * cos);
            if sx < 0.0 || sy < 0.0 || sx >= w - 1.0 || sy >= h - 1.0 {
                continue;
            }
            let (x0, y0) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - x0, sy - y0);
            let mut acc = [0.0f64; 4];
            for (ox, oy, weight) in [
                (0.0, 0.0, (1.0 - tx) * (1.0 - ty)),
                (1.0, 0.0, tx * (1.0 - ty)),
                (0.0, 1.0, (1.0 - tx) * ty),
                (1.0, 1.0, tx * ty),
            ] {
                #[expect(clippy::cast_possible_truncation, reason = "coords checked in range")]
                #[expect(clippy::cast_sign_loss, reason = "coords checked in range")]
                let pixel = src.get_pixel((x0 + ox) as u32, (y0 + oy) as u32);
                for (a, &v) in acc.iter_mut().zip(pixel.0.iter()) {
                    *a += f64::from(v) * weight;
                }
            }
            #[expect(clippy::cast_possible_truncation, reason = "blend stays in 0..=255")]
            let px = acc.map(|v| v.round() as u8);
            out.put_pixel(x, y, Rgba(px));
        }
    }
    out
}

/// Applies the anti-scraping hardening and encodes lossy WebP.
///
/// Same layers as Python `harden_png_to_webp`: fine seeded texture, a
/// smooth luminance blotch, the repeated diagonal attribution
/// watermark, then WebP at quality 82. Geometry is never distorted.
fn harden(
    img: &RgbImage,
    seed_text: &str,
    watermark: &str,
    font: Option<&Font>,
) -> Result<Vec<u8>> {
    let (width, height) = (img.width(), img.height());
    let mut rng = Rng::new(seed_text);
    let mut out = img.clone();
    for pixel in out.pixels_mut() {
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
        *pixel = Rgb([value, value, value]);
    }
    let blotch =
        image::imageops::resize(&small, width, height, image::imageops::FilterType::Triangle);
    for (pixel, blotch_pixel) in out.pixels_mut().zip(blotch.pixels()) {
        for (channel, blotch_channel) in pixel.0.iter_mut().zip(blotch_pixel.0.iter()) {
            #[expect(clippy::cast_possible_truncation, reason = "blend stays in 0..=255")]
            let blended =
                (f64::from(*channel) * 0.97 + f64::from(*blotch_channel) * 0.03).round() as u8;
            *channel = blended;
        }
    }

    if let Some(font) = font {
        #[expect(clippy::cast_precision_loss, reason = "dimensions fit in f32")]
        let font_px = (width / 44).max(20) as f32;
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let step_y = ((height / 5).max(120)) as i32;
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let step_x =
            ((width / 2).max(360) as i32).max(text_width(font, watermark, font_px) as i32 + 80);
        let mut overlay = RgbaImage::new(width, height);
        let mut row = 0i32;
        let mut y = -step_y;
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let height_i = height as i32;
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let width_i = width as i32;
        while y < height_i + step_y {
            let x_offset = (row % 2) * (step_x / 2) - step_x / 3;
            let mut x = x_offset;
            while x < width_i + step_x {
                // (65,65,65) at alpha 30/255, like the Python overlay.
                let mut cursor = f64::from(x);
                for c in watermark.chars() {
                    let (metrics, bitmap) = font.rasterize(c, font_px);
                    for (i, &coverage) in bitmap.iter().enumerate() {
                        if coverage == 0 || metrics.width == 0 {
                            continue;
                        }
                        #[expect(clippy::cast_possible_truncation, reason = "bitmap fits in i32")]
                        #[expect(clippy::cast_possible_wrap, reason = "bitmap fits in i32")]
                        let bx = (i % metrics.width) as i32;
                        #[expect(clippy::cast_possible_truncation, reason = "bitmap fits in i32")]
                        #[expect(clippy::cast_possible_wrap, reason = "bitmap fits in i32")]
                        let by = (i / metrics.width) as i32;
                        let px_x = cursor.round() as i32 + metrics.xmin + bx;
                        let px_y = y - metrics.ymin - metrics.height as i32 + by;
                        if px_x < 0 || px_y < 0 {
                            continue;
                        }
                        if let (Ok(ux), Ok(uy)) = (u32::try_from(px_x), u32::try_from(px_y))
                            && let Some(pixel) = overlay.get_pixel_mut_checked(ux, uy)
                        {
                            let alpha = (u16::from(coverage) * 30 / 255).min(255);
                            let alpha = alpha as u8;
                            if alpha > pixel.0[3] {
                                *pixel = Rgba([65, 65, 65, alpha]);
                            }
                        }
                    }
                    cursor += f64::from(metrics.advance_width);
                }
                x += step_x;
            }
            y += step_y;
            row += 1;
        }
        let overlay = rotate_overlay(&overlay, 24.0);
        let mut rgba = image::DynamicImage::ImageRgb8(out).to_rgba8();
        image::imageops::overlay(&mut rgba, &overlay, 0, 0);
        out = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    }

    let encoder = webp::Encoder::from_rgb(out.as_raw(), width, height);
    let mut config =
        webp::WebPConfig::new().map_err(|()| anyhow::anyhow!("libwebp config init failed"))?;
    config.lossless = 0;
    config.quality = 82.0;
    config.method = 4;
    let memory = encoder
        .encode_advanced(&config)
        .map_err(|e| anyhow::anyhow!("webp encode failed: {e:?}"))?;
    Ok(memory.to_vec())
}

/// Serves one `/compare.webp` request.
///
/// `spin_files` is the pristine speakers tree; bundles are read from
/// `traces/` subdirectories and never exposed directly.
pub(crate) fn serve(
    spin_files: &Path,
    state: &CompareState,
    query: &HashMap<String, String>,
) -> CompareResult<ComparePicture> {
    let measurement = query.get("measurement").map_or("", String::as_str);
    if !ALLOWED_MEASUREMENTS.contains(&measurement) {
        return fail(400, "measurement not allowed for comparison");
    }
    let width: u32 = query.get("w").map_or(1400, |w| w.parse().unwrap_or(0));
    if !ALLOWED_WIDTHS.contains(&width) {
        return fail(400, "width must be one of 900, 1400");
    }
    let param = |key: &str| query.get(key).map_or("", String::as_str);
    let safe = |value: &str| {
        !value.is_empty() && !value.contains('/') && !value.contains('\\') && !value.contains("..")
    };
    let mut sides: Vec<(String, String, String)> = Vec::new();
    for prefix in ["a", "b"] {
        let (speaker, origin, version) = (
            param(&format!("{prefix}_speaker")),
            param(&format!("{prefix}_origin")),
            param(&format!("{prefix}_version")),
        );
        if !safe(speaker) || !safe(origin) || !safe(version) {
            return fail(400, "invalid speaker, origin, or version");
        }
        sides.push((
            speaker.to_owned(),
            origin.strip_prefix("Vendors-").unwrap_or(origin).to_owned(),
            version.to_owned(),
        ));
    }

    let mut paths: Vec<PathBuf> = Vec::new();
    for (speaker, origin, version) in &sides {
        let Some(path) = crate::safe_path(
            &spin_files
                .join(speaker)
                .join(origin)
                .join(version)
                .join("traces"),
            &[&format!("{measurement}.json")],
        ) else {
            return fail(400, "invalid speaker, origin, or version");
        };
        paths.push(path);
    }
    if paths.iter().any(|p| !p.is_file()) {
        return fail(404, "comparison data not available");
    }
    let bundle_bytes: Vec<Vec<u8>> = paths
        .iter()
        .map(|p| std::fs::read(p).unwrap_or_default())
        .collect();

    let mut digest = Sha256::new();
    digest.update(format!("{COMPARE_CACHE_VERSION}|{measurement}|{width}|").as_bytes());
    for bytes in &bundle_bytes {
        digest.update(bytes);
    }
    let cache_file = state
        .cache_dir
        .join(format!("{:x}.webp", digest.finalize()));
    if let Ok(body) = std::fs::read(&cache_file)
        && !body.is_empty()
    {
        return Ok(ComparePicture {
            body,
            cache_hit: true,
        });
    }

    let Ok(_guard) = state.render_lock.try_lock() else {
        let mut failure = CompareFailure::new(503, "renderer busy");
        failure.retry_after = Some(5);
        return Err(failure);
    };
    if !state.budget_available(Instant::now()) {
        let mut failure = CompareFailure::new(503, "render budget exhausted");
        failure.retry_after = Some(30);
        return Err(failure);
    }
    // Re-check under the lock: another request may have rendered it.
    if let Ok(body) = std::fs::read(&cache_file)
        && !body.is_empty()
    {
        return Ok(ComparePicture {
            body,
            cache_hit: true,
        });
    }

    let render = (|| -> Result<Vec<u8>> {
        let bundle_a = parse_bundle(std::str::from_utf8(&bundle_bytes[0])?)?;
        let bundle_b = parse_bundle(std::str::from_utf8(&bundle_bytes[1])?)?;
        render_compare(
            &bundle_a,
            &bundle_b,
            &format!("{} ({})", sides[0].0, sides[0].1),
            &format!("{} ({})", sides[1].0, sides[1].1),
            measurement,
            width,
            &source_label(&sides[0].1, &sides[0].2),
            &source_label(&sides[1].1, &sides[1].2),
        )
    })();
    let body = match render {
        Ok(body) => body,
        Err(error) => return fail(500, format!("render failed: {error:#}")),
    };
    if let Some(parent) = cache_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = cache_file.with_extension("webp.tmp");
    if std::fs::write(&tmp, &body).is_ok() {
        let _ = std::fs::rename(&tmp, &cache_file);
    }
    Ok(ComparePicture {
        body,
        cache_hit: false,
    })
}

/// Stamps a filled disc of `radius` at (`x`, `y`).
fn stamp(img: &mut RgbImage, x: f64, y: f64, radius: f64, color: Rgb<u8>) {
    let r = radius.ceil() as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            if f64::from(dx * dx + dy * dy) > radius * radius + radius {
                continue;
            }
            let (px_x, px_y) = (x.round() as i32 + dx, y.round() as i32 + dy);
            if px_x < 0 || px_y < 0 {
                continue;
            }
            if let (Ok(ux), Ok(uy)) = (u32::try_from(px_x), u32::try_from(px_y))
                && let Some(pixel) = img.get_pixel_mut_checked(ux, uy)
            {
                *pixel = color;
            }
        }
    }
}

fn draw_segment(img: &mut RgbImage, a: (f64, f64), b: (f64, f64), color: Rgb<u8>, width: f64) {
    let dist = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let steps = (dist / 0.5).ceil().max(1.0);
    for i in 0..=steps as i32 {
        let t = f64::from(i) / steps;
        stamp(
            img,
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            width / 2.0,
            color,
        );
    }
}

/// Display name of a measurement source (Python `source_label` table,
/// common cases; unknown vendors fall back to the vendor name).
fn source_label(origin: &str, version: &str) -> String {
    let labels: &[(&str, &str)] = &[
        ("asr", "Audio Science Review"),
        ("audiosciencereview", "Audio Science Review"),
        ("eac", "Erin's Audio Corner"),
        ("erinsaudiocorner", "Erin's Audio Corner"),
        ("princeton", "Princeton 3D3A"),
        ("3d3a", "Princeton 3D3A"),
    ];
    let key = |text: &str| {
        text.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    for candidate in [origin, version] {
        if let Some((_, label)) = labels.iter().find(|(k, _)| *k == key(candidate)) {
            return (*label).to_owned();
        }
    }
    origin
        .strip_prefix("Vendors-")
        .map_or_else(|| origin.to_owned(), ToOwned::to_owned)
}

fn watermark_text(source: &str) -> String {
    let source = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if source.is_empty() {
        WATERMARK_TEXT.to_owned()
    } else {
        format!("{WATERMARK_TEXT}  |  {source}")
    }
}

/// Renders the A/B overlay at `width`, then hardens it to WebP bytes.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the Python render_compare signature"
)]
fn render_compare(
    bundle_a: &Bundle,
    bundle_b: &Bundle,
    name_a: &str,
    name_b: &str,
    graph_name: &str,
    width: u32,
    source_a: &str,
    source_b: &str,
) -> Result<Vec<u8>> {
    let bundles: [&Bundle; 2] = [bundle_a, bundle_b];
    let has_y2 = bundles
        .iter()
        .any(|b| b.traces.iter().any(|t| t.axis == "y2"));
    let y_range = axis_range(&bundles, "y").unwrap_or((-40.0, 10.0));
    let y2_range = if has_y2 {
        axis_range(&bundles, "y2")
    } else {
        None
    };
    let n_traces = bundles
        .iter()
        .map(|b| b.visible_traces().len())
        .max()
        .unwrap_or(0);
    #[expect(clippy::cast_precision_loss, reason = "trace counts fit in f64")]
    let legend_rows = ((n_traces as f64 / 3.0).ceil() as u32).max(1) * 2 + 5;
    #[expect(clippy::cast_possible_truncation, reason = "height fits in u32")]
    #[expect(clippy::cast_sign_loss, reason = "height is positive")]
    let height = (f64::from(width) * 2.0 / 3.0) as u32 + legend_rows * 18;

    let s = |v: f64| v * f64::from(SS);
    let (w_px, h_px) = (width * SS, height * SS);
    let mut img = RgbImage::from_pixel(w_px, h_px, Rgb([255, 255, 255]));
    let font = load_font();
    #[expect(clippy::cast_precision_loss, reason = "width fits in f32")]
    let (font_px, small_px, title_px) = (
        (width / 64).max(18) as f32 * SS as f32,
        (width / 80).max(15) as f32 * SS as f32,
        (width / 48).max(22) as f32 * SS as f32,
    );

    let margin_l = 64.0;
    let margin_r = if has_y2 { 64.0 } else { 18.0 };
    let margin_t = 64.0;
    let margin_b = 52.0 + f64::from(legend_rows) * 18.0;
    let (plot_x0, plot_y0) = (s(margin_l), s(margin_t));
    let (plot_x1, plot_y1) = (
        s(f64::from(width) - margin_r),
        s(f64::from(height) - margin_b),
    );
    let (plot_w, plot_h) = (plot_x1 - plot_x0, plot_y1 - plot_y0);

    let freq_lo = bundles
        .iter()
        .map(|b| b.freq_min)
        .fold(f64::INFINITY, f64::min);
    let freq_hi = bundles
        .iter()
        .map(|b| b.freq_max)
        .fold(f64::NEG_INFINITY, f64::max);
    let (log_lo, log_hi) = (freq_lo.max(1.0).log10(), freq_hi.log10());
    let x_of = |freq: f64| plot_x0 + (freq.log10() - log_lo) / (log_hi - log_lo) * plot_w;
    let y_of =
        |value: f64, range: (f64, f64)| plot_y1 - (value - range.0) / (range.1 - range.0) * plot_h;

    if let Some(font) = &font {
        draw_text(
            &mut img,
            font,
            &format!("{graph_name}: {name_a} vs {name_b}"),
            title_px,
            plot_x0,
            s(14.0) + f64::from(title_px),
            TEXT,
        );
    }
    for freq in [
        20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
    ] {
        if freq_lo <= freq && freq <= freq_hi {
            let x = x_of(freq);
            draw_segment(&mut img, (x, plot_y0), (x, plot_y1), GRID, f64::from(SS));
            if let Some(font) = &font {
                let label = if freq >= 1_000.0 {
                    format!("{}k", (freq / 1_000.0) as i64)
                } else {
                    format!("{freq:.0}")
                };
                draw_text(
                    &mut img,
                    font,
                    &label,
                    small_px,
                    x - s(10.0),
                    plot_y1 + s(6.0) + f64::from(small_px),
                    TEXT,
                );
            }
        }
    }
    for tick in nice_ticks(y_range.0, y_range.1) {
        let y = y_of(tick, y_range);
        draw_segment(&mut img, (plot_x0, y), (plot_x1, y), GRID, f64::from(SS));
        if let Some(font) = &font {
            draw_text(
                &mut img,
                font,
                &fmt_tick(tick),
                small_px,
                s(8.0),
                y + s(2.0),
                TEXT,
            );
        }
    }
    if let (Some(range), Some(font)) = (y2_range, &font) {
        for tick in nice_ticks(range.0, range.1) {
            let y = y_of(tick, range);
            draw_text(
                &mut img,
                font,
                &fmt_tick(tick),
                small_px,
                plot_x1 + s(8.0),
                y + s(2.0),
                TEXT,
            );
        }
    }
    for (a, b) in [
        ((plot_x0, plot_y0), (plot_x1, plot_y0)),
        ((plot_x1, plot_y0), (plot_x1, plot_y1)),
        ((plot_x1, plot_y1), (plot_x0, plot_y1)),
        ((plot_x0, plot_y1), (plot_x0, plot_y0)),
    ] {
        draw_segment(&mut img, a, b, AXIS, f64::from(SS));
    }
    if let Some(font) = &font {
        draw_text(
            &mut img,
            font,
            "Frequency (Hz)",
            small_px,
            plot_x0 + plot_w / 2.0 - s(40.0),
            s(f64::from(height) - margin_b + 28.0) + f64::from(small_px),
            TEXT,
        );
    }

    for (speaker_idx, bundle) in bundles.iter().enumerate() {
        let freqs = bundle.freqs();
        if freqs.is_empty() {
            continue;
        }
        let dashed = speaker_idx == 1;
        for (trace_idx, trace) in bundle.visible_traces().iter().enumerate() {
            let range = if trace.axis == "y2" {
                y2_range.unwrap_or(y_range)
            } else {
                y_range
            };
            let color = parse_color(&trace.color, trace_idx);
            let points: Vec<(f64, f64)> = freqs
                .iter()
                .zip(&trace.values)
                .map(|(f, v)| {
                    if v.is_finite() {
                        (x_of(*f), y_of(*v, range))
                    } else {
                        (f64::NAN, f64::NAN)
                    }
                })
                .collect();
            draw_polyline(&mut img, &points, color, dashed, 2.0 * f64::from(SS));
        }
    }

    if let Some(font) = &font {
        let mut legend_y = plot_y1 + s(34.0);
        for (speaker_idx, bundle) in bundles.iter().enumerate() {
            let name = [name_a, name_b][speaker_idx];
            let label = if speaker_idx == 1 {
                format!("{name} (dashed)")
            } else {
                name.to_owned()
            };
            draw_text(
                &mut img,
                font,
                &label,
                font_px,
                plot_x0,
                legend_y + f64::from(font_px),
                TEXT,
            );
            legend_y += s(20.0);
            let mut x = plot_x0;
            for (trace_idx, trace) in bundle.visible_traces().iter().enumerate() {
                let color = parse_color(&trace.color, trace_idx);
                draw_polyline(
                    &mut img,
                    &[(x, legend_y + s(7.0)), (x + s(26.0), legend_y + s(7.0))],
                    color,
                    speaker_idx == 1,
                    2.0 * f64::from(SS),
                );
                draw_text(
                    &mut img,
                    font,
                    &trace.name,
                    small_px,
                    x + s(32.0),
                    legend_y + f64::from(small_px),
                    TEXT,
                );
                x += s(38.0) + text_width(font, &trace.name, small_px) + s(18.0);
                if (trace_idx + 1) % 3 == 0 {
                    x = plot_x0;
                    legend_y += s(18.0);
                }
            }
            legend_y += s(22.0);
        }
    }

    let img = image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3);
    let sources = [source_a, source_b]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" + ");
    let seed = format!("compare/{graph_name}/{name_a}/{name_b}");
    harden(&img, &seed, &watermark_text(&sources), font.as_ref())
}

/// Draws a polyline; NaN points break it. `dashed` walks the same
/// dash/gap pattern as the Python renderer.
fn draw_polyline(
    img: &mut RgbImage,
    points: &[(f64, f64)],
    color: Rgb<u8>,
    dashed: bool,
    width: f64,
) {
    let dash = 14.0 * f64::from(SS);
    let gap = 9.0 * f64::from(SS);
    let mut remaining = dash;
    let mut drawing = true;
    let mut prev: Option<(f64, f64)> = None;
    for &point in points {
        if !(point.0.is_finite() && point.1.is_finite()) {
            prev = None;
            continue;
        }
        if let Some(prev_point) = prev {
            let dist = ((point.0 - prev_point.0).powi(2) + (point.1 - prev_point.1).powi(2)).sqrt();
            if !dashed {
                draw_segment(img, prev_point, point, color, width);
            } else {
                let mut travelled = 0.0;
                while travelled < dist {
                    let step = remaining.min(dist - travelled);
                    if drawing && step > 0.0 && dist > 0.0 {
                        let a = (
                            prev_point.0 + (point.0 - prev_point.0) * (travelled / dist),
                            prev_point.1 + (point.1 - prev_point.1) * (travelled / dist),
                        );
                        let b = (
                            prev_point.0 + (point.0 - prev_point.0) * ((travelled + step) / dist),
                            prev_point.1 + (point.1 - prev_point.1) * ((travelled + step) / dist),
                        );
                        draw_segment(img, a, b, color, width);
                    }
                    travelled += step;
                    remaining -= step;
                    if remaining <= 0.0 {
                        drawing = !drawing;
                        remaining = if drawing { dash } else { gap };
                    }
                }
            }
        }
        prev = Some(point);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_json(values_a: &[i16], values_b: &[i16]) -> String {
        let encode = |values: &[i16]| {
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            base64::engine::general_purpose::STANDARD.encode(bytes)
        };
        format!(
            r##"{{"freq_min":20.0,"freq_max":20000.0,"points":{},"y_range":[-45,5],"traces":[
              {{"name":"On Axis","axis":"y","color":"#ff0000","y_int16_b64":"{}"}},
              {{"name":"Recommended Zone","axis":"y","color":"","y_int16_b64":"{}"}}
            ]}}"##,
            values_a.len(),
            encode(values_a),
            encode(values_b),
        )
    }

    #[test]
    fn decode_values_maps_sentinel_to_nan() {
        let bundle = parse_bundle(&bundle_json(&[0, 100, SENTINEL], &[0, 0, 0])).expect("bundle");
        let values = &bundle.traces[0].values;
        assert_eq!(values[1], 1.0);
        assert!(values[2].is_nan());
        // Recommended zones are dropped from the overlay.
        assert_eq!(bundle.visible_traces().len(), 1);
        assert_eq!(bundle.freqs().len(), 3);
    }

    #[test]
    fn render_produces_webp_bytes() {
        let bundle_a =
            parse_bundle(&bundle_json(&[0, -100, -200, -300], &[0, 0, 0, 0])).expect("bundle a");
        let bundle_b =
            parse_bundle(&bundle_json(&[-50, -150, -250, -350], &[0, 0, 0, 0])).expect("bundle b");
        let body = render_compare(
            &bundle_a,
            &bundle_b,
            "A (ASR)",
            "B (ASR)",
            "CEA2034",
            900,
            "Audio Science Review",
            "Audio Science Review",
        )
        .expect("render");
        assert_eq!(&body[..4], b"RIFF");
        assert_eq!(&body[8..12], b"WEBP");
    }

    #[test]
    fn serve_validates_params_and_caches() {
        let root = std::env::temp_dir().join(format!("compare-test-{}", std::process::id()));
        let traces = root.join("files/Spk A/ASR/asr/traces");
        let traces_b = root.join("files/Spk B/ASR/asr/traces");
        std::fs::create_dir_all(&traces).expect("mkdir");
        std::fs::create_dir_all(&traces_b).expect("mkdir");
        let json = bundle_json(&[0, -100, -200, -300], &[0, 0, 0, 0]);
        std::fs::write(traces.join("CEA2034.json"), &json).expect("write a");
        std::fs::write(traces_b.join("CEA2034.json"), &json).expect("write b");
        let state = CompareState::new(root.join("cache"));
        let query: HashMap<String, String> = [
            ("measurement", "CEA2034"),
            ("a_speaker", "Spk A"),
            ("a_origin", "ASR"),
            ("a_version", "asr"),
            ("b_speaker", "Spk B"),
            ("b_origin", "ASR"),
            ("b_version", "asr"),
            ("w", "900"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let first = serve(&root.join("files"), &state, &query).expect("first serve");
        assert!(!first.cache_hit);
        let second = serve(&root.join("files"), &state, &query).expect("second serve");
        assert!(second.cache_hit);
        assert_eq!(first.body, second.body);

        let mut bad = query.clone();
        bad.insert("measurement".to_owned(), "SPL Horizontal Globe".to_owned());
        assert_eq!(
            serve(&root.join("files"), &state, &bad)
                .err()
                .map(|e| e.status),
            Some(400)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
