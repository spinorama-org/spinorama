//! `GET /compare.webp`: two-speaker overlay pictures, rendered on the fly.
//!
//! Rust replacement for the Python `scripts/compare_render_server.py` +
//! `spinorama/compare_render.py` pair. The overlay uses one colour per
//! speaker (A solid, B dashed) and one marker shape per measurement —
//! the same shape for both speakers — so curves cannot be separated
//! by colour for digitising; the single-speaker variant draws every
//! curve in black with the same markers. Pictures are drawn from the
//! pristine trace bundles under
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
pub(crate) const COMPARE_CACHE_VERSION: &str = "compare-v11-connected-phase";

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

/// The two comparison colours: one per speaker (A, then B), taken
/// from the site palette so overlays stay in the house colours.
const SPEAKER_COLORS: [Rgb<u8>; 2] = [Rgb([0x5c, 0x77, 0xa5]), Rgb([0xdc, 0x84, 0x2a])];

/// Curve colour of the single-speaker graph: black for every curve.
const SINGLE_COLOR: Rgb<u8> = Rgb([0, 0, 0]);

/// Colour of `N/A` valid-range zones (grey, like the static graphs).
const ZONE_COLOR: Rgb<u8> = Rgb([204, 204, 204]);

const GRID: Rgb<u8> = Rgb([205, 205, 205]);
const TEXT: Rgb<u8> = Rgb([35, 35, 35]);
const AXIS: Rgb<u8> = Rgb([120, 120, 120]);

/// Display options for a rendered graph picture, supplied as query
/// parameters (`trend`, `zones`, `grid`, `legend`, `smooth`,
/// `splrange`). Mirrors the old client-side plot configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RenderOptions {
    pub trend: bool,
    pub zones: bool,
    pub grid: bool,
    pub legend: bool,
    /// Octave smoothing width (0 = off), applied in log-frequency.
    pub smooth_oct: f64,
    /// Force the primary Y axis to span this many dB.
    pub spl_range_db: Option<f64>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            trend: true,
            zones: false,
            grid: true,
            legend: true,
            smooth_oct: 0.0,
            spl_range_db: None,
        }
    }
}

impl RenderOptions {
    fn parse(query: &HashMap<String, String>, zones_default: bool) -> Self {
        let flag = |key: &str, default: bool| {
            query
                .get(key)
                .map_or(default, |v| !matches!(v.as_str(), "0" | "false" | "off"))
        };
        let smooth_oct = query
            .get("smooth")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
            .unwrap_or(0.0);
        let spl_range_db = query
            .get("splrange")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && (10.0..=120.0).contains(v));
        Self {
            trend: flag("trend", true),
            zones: flag("zones", zones_default),
            grid: flag("grid", true),
            legend: flag("legend", true),
            smooth_oct,
            spl_range_db,
        }
    }

    /// Stable cache-key fragment; empty for the default options so
    /// existing compare cache entries stay valid.
    fn cache_key(&self) -> String {
        if *self == Self::default() {
            return String::new();
        }
        format!(
            "t{}z{}g{}l{}s{:.4}r{}",
            u8::from(self.trend),
            u8::from(self.zones),
            u8::from(self.grid),
            u8::from(self.legend),
            self.smooth_oct,
            self.spl_range_db.unwrap_or(0.0),
        )
    }
}

/// Moving-average smoothing over a log-spaced frequency axis with a
/// window `octaves` wide. NaN samples are skipped; NaN stays NaN.
fn smooth_octaves(
    values: &[f64],
    points: usize,
    freq_min: f64,
    freq_max: f64,
    octaves: f64,
) -> Vec<f64> {
    if octaves <= 0.0 || points < 3 || freq_min <= 0.0 || freq_max <= freq_min {
        return values.to_vec();
    }
    #[expect(clippy::cast_precision_loss, reason = "point counts fit in f64")]
    let points_per_octave = points as f64 / (freq_max / freq_min).log2();
    #[expect(clippy::cast_possible_truncation, reason = "window size is small")]
    #[expect(clippy::cast_sign_loss, reason = "window size is positive")]
    let half = ((octaves * points_per_octave) / 2.0).round() as usize;
    if half == 0 {
        return values.to_vec();
    }
    let mut out = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        if !values[i].is_finite() {
            out.push(f64::NAN);
            continue;
        }
        let lo = i.saturating_sub(half);
        let hi = (i + half).min(values.len() - 1);
        let (mut sum, mut count) = (0.0, 0_usize);
        for v in &values[lo..=hi] {
            if v.is_finite() {
                sum += v;
                count += 1;
            }
        }
        #[expect(clippy::cast_precision_loss, reason = "window size fits in f64")]
        out.push(if count > 0 {
            sum / count as f64
        } else {
            f64::NAN
        });
    }
    out
}

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

/// Trend/helper traces toggled by the `trend` option (old plot-config
/// `trendNames`, matched loosely on the lower-cased name).
fn is_trend_trace(lower_name: &str) -> bool {
    lower_name.contains("band")
        || lower_name.contains("midrange")
        || lower_name.contains("linear interpolation")
        || lower_name.contains("linear regression")
        || lower_name.contains("regression")
        || lower_name.contains("trend")
}

/// Whether a trace is drawn without markers: trend/band helpers,
/// confidence zones, slope helpers, and `N/A` valid-range zones.
fn is_helper_trace(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "n/a"
        || lower == "zero"
        || lower.starts_with("phase")
        || lower.starts_with("recommended")
        || lower.ends_with(" zone")
        || lower.ends_with(" slope")
        || is_trend_trace(&lower)
}

/// Marker shape distinguishing measurement curves that share one
/// colour. The order of [`MARKERS`] mirrors `SCAN_SYMBOLS` in
/// `spinorama-generate` (the static speaker-graph style), including
/// the hash fallback for unknown names; keep the two in sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Circle,
    Square,
    Diamond,
    Cross,
    X,
    TriangleUp,
    TriangleDown,
    TriangleLeft,
    TriangleRight,
    Star,
}

const MARKERS: [Marker; 10] = [
    Marker::Circle,
    Marker::Square,
    Marker::Diamond,
    Marker::Cross,
    Marker::X,
    Marker::TriangleUp,
    Marker::TriangleDown,
    Marker::TriangleLeft,
    Marker::TriangleRight,
    Marker::Star,
];

/// Canonical marker per measurement name (mirror of `MARKER_BY_NAME`
/// in `spinorama-generate`; keep in sync). Keyed by name, not trace
/// position, so one measurement carries the same shape for both
/// speakers. Mirror angles share a shape.
const MARKER_BY_NAME: &[(&str, Marker)] = &[
    ("On Axis", Marker::Circle),
    ("ON", Marker::Circle),
    ("Listening Window", Marker::Square),
    ("Early Reflections", Marker::Diamond),
    ("Sound Power", Marker::TriangleUp),
    ("Early Reflections DI", Marker::Cross),
    ("Sound Power DI", Marker::X),
    ("Estimated In-Room Response", Marker::Star),
    ("Group Delay (ms)", Marker::Circle),
    ("10°", Marker::Square),
    ("-10°", Marker::Square),
    ("20°", Marker::Diamond),
    ("-20°", Marker::Diamond),
    ("30°", Marker::TriangleUp),
    ("-30°", Marker::TriangleUp),
    ("40°", Marker::TriangleDown),
    ("-40°", Marker::TriangleDown),
    ("50°", Marker::Cross),
    ("-50°", Marker::Cross),
    ("60°", Marker::X),
    ("-60°", Marker::X),
    ("70°", Marker::Star),
    ("-70°", Marker::Star),
    ("80°", Marker::TriangleLeft),
    ("-80°", Marker::TriangleLeft),
    ("90°", Marker::TriangleRight),
    ("-90°", Marker::TriangleRight),
    ("Ceiling Bounce", Marker::Square),
    ("Floor Bounce", Marker::Diamond),
    ("Front Wall Bounce", Marker::TriangleUp),
    ("Rear Wall Bounce", Marker::TriangleDown),
    ("Side Wall Bounce", Marker::Cross),
    ("Total Early Reflection", Marker::Star),
    ("Ceiling Reflection", Marker::Square),
    ("Floor Reflection", Marker::Diamond),
    ("Front", Marker::Square),
    ("Rear", Marker::Diamond),
    ("Side", Marker::TriangleUp),
    ("Total Horizontal Reflection", Marker::Star),
    ("Total Vertical Reflection", Marker::Star),
    ("+6dB", Marker::Circle),
    ("-6dB", Marker::Square),
];

/// Marker for a measurement name, `None` for helper traces. Unknown
/// names fall back to a stable hash (djb2, as in `spinorama-generate`).
fn marker_for(name: &str) -> Option<Marker> {
    if is_helper_trace(name) {
        return None;
    }
    if let Some((_, marker)) = MARKER_BY_NAME.iter().find(|(key, _)| *key == name) {
        return Some(*marker);
    }
    let mut hash: u32 = 5381;
    for byte in name.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    Some(MARKERS[(hash as usize) % MARKERS.len()])
}

/// Colour of one trace: black for a single speaker, one colour per
/// speaker in a comparison, grey for `N/A` zones. The bundle's own
/// per-measurement colours are deliberately ignored.
fn trace_color(trace_name: &str, speaker_idx: usize, speaker_count: usize) -> Rgb<u8> {
    if trace_name.eq_ignore_ascii_case("N/A") {
        return ZONE_COLOR;
    }
    if speaker_count >= 2 {
        SPEAKER_COLORS[speaker_idx % SPEAKER_COLORS.len()]
    } else {
        SINGLE_COLOR
    }
}

/// One decoded bundle trace. The bundle's per-trace `color` is
/// ignored on purpose: rendering colours by speaker, not measurement.
struct Trace {
    name: String,
    axis: String,
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

    fn visible_traces(&self, opts: &RenderOptions, comparison: bool) -> Vec<&Trace> {
        self.traces
            .iter()
            .filter(|trace| {
                let name = trace.name.to_lowercase();
                // Filled bands lose their polygon geometry in trace bundles.
                // Keep short reference lines such as Midrange Band +3dB.
                if comparison && name.starts_with("band ") {
                    return false;
                }
                // Slope helpers are never drawn (Python rule).
                if name.ends_with(" slope") {
                    return false;
                }
                let is_zone = name.starts_with("recommended") || name.ends_with(" zone");
                if is_zone && !opts.zones {
                    return false;
                }
                if is_trend_trace(&name) && !opts.trend {
                    return false;
                }
                true
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

/// Draws text at horizontal origin `x` and baseline `y`, alpha-blended.
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

/// Baseline that centers the actual glyph ink around `center`.
fn centered_baseline(font: &Font, text: &str, px: f32, center: f64) -> f64 {
    let top = text
        .chars()
        .map(|c| {
            let m = font.metrics(c, px);
            m.ymin + m.height as i32
        })
        .max()
        .unwrap_or(0);
    let bottom = text
        .chars()
        .map(|c| font.metrics(c, px).ymin)
        .min()
        .unwrap_or(0);
    center + f64::from(top + bottom) / 2.0
}

fn fit_text(font: &Font, text: &str, px: f32, width: f64) -> f32 {
    px * (width / text_width(font, text, px).max(1.0)).min(1.0) as f32
}

fn draw_axis_title(
    img: &mut RgbImage,
    font: &Font,
    text: &str,
    px: f32,
    x: f64,
    center_y: f64,
    right: bool,
) {
    let width = text_width(font, text, px).ceil() as u32 + 4;
    let height = (px * 1.5).ceil() as u32;
    let mut label = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    let baseline = centered_baseline(font, text, px, f64::from(height) / 2.0);
    draw_text(&mut label, font, text, px, 2.0, baseline, TEXT);
    let rotated = if right {
        image::imageops::rotate90(&label)
    } else {
        image::imageops::rotate270(&label)
    };
    image::imageops::overlay(
        img,
        &rotated,
        (x - f64::from(rotated.width()) / 2.0).round() as i64,
        (center_y - f64::from(rotated.height()) / 2.0).round() as i64,
    );
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

    let opts = RenderOptions::parse(query, false);
    let mut digest = Sha256::new();
    digest.update(
        format!(
            "{COMPARE_CACHE_VERSION}|{measurement}|{width}|{}|",
            opts.cache_key()
        )
        .as_bytes(),
    );
    for bytes in &bundle_bytes {
        digest.update(bytes);
    }
    let cache_file = state
        .cache_dir
        .join(format!("{:x}.webp", digest.finalize()));
    cached_render(state, &cache_file, || {
        let bundle_a = parse_bundle(std::str::from_utf8(&bundle_bytes[0])?)?;
        let bundle_b = parse_bundle(std::str::from_utf8(&bundle_bytes[1])?)?;
        let name_a = format!("{} ({})", sides[0].0, sides[0].1);
        let name_b = format!("{} ({})", sides[1].0, sides[1].1);
        let source_a = source_label(&sides[0].1, &sides[0].2);
        let source_b = source_label(&sides[1].1, &sides[1].2);
        render_bundles(
            &[&bundle_a, &bundle_b],
            &[name_a.as_str(), name_b.as_str()],
            measurement,
            width,
            &[source_a.as_str(), source_b.as_str()],
            &opts,
        )
    })
}

/// Cache lookup + one-render lock + budget + atomic cache write,
/// shared by the compare and single-graph endpoints.
fn cached_render(
    state: &CompareState,
    cache_file: &Path,
    render: impl FnOnce() -> Result<Vec<u8>>,
) -> CompareResult<ComparePicture> {
    if let Ok(body) = std::fs::read(cache_file)
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
    if let Ok(body) = std::fs::read(cache_file)
        && !body.is_empty()
    {
        return Ok(ComparePicture {
            body,
            cache_hit: true,
        });
    }

    let body = match render() {
        Ok(body) => body,
        Err(error) => return fail(500, format!("render failed: {error:#}")),
    };
    if let Some(parent) = cache_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = cache_file.with_extension("webp.tmp");
    if std::fs::write(&tmp, &body).is_ok() {
        let _ = std::fs::rename(&tmp, cache_file);
    }
    Ok(ComparePicture {
        body,
        cache_hit: false,
    })
}

/// Single-speaker dynamic graph (`/graph-render/graph.webp`): renders
/// one measurement bundle with display options, cached like compare.
pub(crate) fn serve_graph(
    spin_files: &Path,
    state: &CompareState,
    query: &HashMap<String, String>,
) -> CompareResult<ComparePicture> {
    let measurement = query.get("measurement").map_or("", String::as_str);
    if !ALLOWED_MEASUREMENTS.contains(&measurement) {
        return fail(400, "measurement not allowed for graph rendering");
    }
    let width: u32 = query.get("w").map_or(1400, |w| w.parse().unwrap_or(0));
    if !ALLOWED_WIDTHS.contains(&width) {
        return fail(400, "width must be one of 900, 1400");
    }
    let param = |key: &str| query.get(key).map_or("", String::as_str);
    let safe = |value: &str| {
        !value.is_empty() && !value.contains('/') && !value.contains('\\') && !value.contains("..")
    };
    let (speaker, origin, version) = (param("speaker"), param("origin"), param("version"));
    if !safe(speaker) || !safe(origin) || !safe(version) {
        return fail(400, "invalid speaker, origin, or version");
    }
    let origin = origin.strip_prefix("Vendors-").unwrap_or(origin);
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
    if !path.is_file() {
        return fail(404, "graph data not available");
    }
    let bundle_bytes = std::fs::read(&path).unwrap_or_default();
    let opts = RenderOptions::parse(query, true);
    let mut digest = Sha256::new();
    digest.update(
        format!(
            "{COMPARE_CACHE_VERSION}|graph|{measurement}|{width}|{}|",
            opts.cache_key()
        )
        .as_bytes(),
    );
    digest.update(&bundle_bytes);
    let cache_file = state
        .cache_dir
        .join("graph")
        .join(format!("{:x}.webp", digest.finalize()));
    let speaker = speaker.to_owned();
    let origin = origin.to_owned();
    let version = version.to_owned();
    cached_render(state, &cache_file, || {
        let bundle = parse_bundle(std::str::from_utf8(&bundle_bytes)?)?;
        let name = format!("{speaker} ({origin})");
        let source = source_label(&origin, &version);
        render_bundles(
            &[&bundle],
            &[name.as_str()],
            measurement,
            width,
            &[source.as_str()],
            &opts,
        )
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

/// Fills a convex-or-star polygon scanline by scanline.
fn fill_polygon(img: &mut RgbImage, verts: &[(f64, f64)], color: Rgb<u8>) {
    if verts.len() < 3 {
        return;
    }
    let y_lo = verts
        .iter()
        .map(|v| v.1)
        .fold(f64::INFINITY, f64::min)
        .ceil();
    let y_hi = verts
        .iter()
        .map(|v| v.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .floor();
    let mut y = y_lo;
    while y <= y_hi {
        let mut xs: Vec<f64> = Vec::new();
        for i in 0..verts.len() {
            let (x0, y0) = verts[i];
            let (x1, y1) = verts[(i + 1) % verts.len()];
            if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                xs.push(x0 + (y - y0) / (y1 - y0) * (x1 - x0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if pair.len() == 2 {
                draw_segment(img, (pair[0], y), (pair[1], y), color, 1.6);
            }
        }
        y += 1.0;
    }
}

/// Convex-hull area at unit radius for each marker's geometry.
fn marker_hull_area(marker: Marker) -> f64 {
    use std::f64::consts::PI;
    let d = (2.2 - 0.45) / 2.0;
    let cap = 0.45 / 2.0;
    match marker {
        Marker::Circle => PI,
        Marker::Square => 4.0,
        Marker::Diamond => 2.5,
        Marker::TriangleUp
        | Marker::TriangleDown
        | Marker::TriangleLeft
        | Marker::TriangleRight => 2.2 * 2.05 / 2.0,
        Marker::Star => 2.5 * 1.3_f64.powi(2) * (2.0 * PI / 5.0).sin(),
        // The stroked cross/X hulls are a diamond/square expanded by the
        // round line caps: A + perimeter * cap radius + pi * radius².
        Marker::Cross => 2.0 * d * d + 4.0 * d * 2.0_f64.sqrt() * cap + PI * cap * cap,
        Marker::X => 4.0 * d * d + 8.0 * d * cap + PI * cap * cap,
    }
}

/// Draws markers with the small triangle's convex-hull area.
fn draw_marker(img: &mut RgbImage, x: f64, y: f64, r: f64, marker: Marker, color: Rgb<u8>) {
    let r = r * (marker_hull_area(Marker::TriangleUp) / marker_hull_area(marker)).sqrt();
    match marker {
        Marker::Circle => stamp(img, x, y, r, color),
        Marker::Square => fill_polygon(
            img,
            &[
                (x - r, y - r),
                (x + r, y - r),
                (x + r, y + r),
                (x - r, y + r),
            ],
            color,
        ),
        Marker::Diamond => fill_polygon(
            img,
            &[(x, y - r * 1.25), (x + r, y), (x, y + r * 1.25), (x - r, y)],
            color,
        ),
        Marker::TriangleUp => fill_polygon(
            img,
            &[
                (x, y - r * 1.15),
                (x + r * 1.1, y + r * 0.9),
                (x - r * 1.1, y + r * 0.9),
            ],
            color,
        ),
        Marker::TriangleDown => fill_polygon(
            img,
            &[
                (x, y + r * 1.15),
                (x - r * 1.1, y - r * 0.9),
                (x + r * 1.1, y - r * 0.9),
            ],
            color,
        ),
        Marker::TriangleLeft => fill_polygon(
            img,
            &[
                (x - r * 1.15, y),
                (x + r * 0.9, y - r * 1.1),
                (x + r * 0.9, y + r * 1.1),
            ],
            color,
        ),
        Marker::TriangleRight => fill_polygon(
            img,
            &[
                (x + r * 1.15, y),
                (x - r * 0.9, y + r * 1.1),
                (x - r * 0.9, y - r * 1.1),
            ],
            color,
        ),
        Marker::Star => {
            let mut verts: Vec<(f64, f64)> = Vec::with_capacity(10);
            for i in 0..10 {
                let angle =
                    -std::f64::consts::FRAC_PI_2 + f64::from(i) * std::f64::consts::PI / 5.0;
                let radius = if i % 2 == 0 { r * 1.3 } else { r * 0.55 };
                verts.push((x + angle.cos() * radius, y + angle.sin() * radius));
            }
            fill_polygon(img, &verts, color);
        }
        Marker::Cross => {
            let w = r * 0.45;
            let d = (2.2 * r - w) / 2.0;
            draw_segment(img, (x - d, y), (x + d, y), color, w);
            draw_segment(img, (x, y - d), (x, y + d), color, w);
        }
        Marker::X => {
            let w = r * 0.45;
            let d = (2.2 * r - w) / 2.0;
            draw_segment(img, (x - d, y - d), (x + d, y + d), color, w);
            draw_segment(img, (x - d, y + d), (x + d, y - d), color, w);
        }
    }
}

/// Stamps markers along one curve, roughly evenly spaced in screen
/// space and clipped to the plot rectangle. The dense bundle grid
/// (1024 points) would merge into a solid band if every point got a
/// marker, so placement is by travelled distance.
fn draw_markers(
    img: &mut RgbImage,
    points: &[(f64, f64)],
    marker: Marker,
    color: Rgb<u8>,
    plot: (f64, f64, f64, f64),
) {
    let spacing = 30.0 * f64::from(SS);
    let radius = 4.5 * f64::from(SS);
    let (x0, y0, x1, y1) = plot;
    let mut last: Option<(f64, f64)> = None;
    for &point in points {
        if !(point.0.is_finite() && point.1.is_finite()) {
            continue;
        }
        if point.0 < x0 || point.0 > x1 || point.1 < y0 || point.1 > y1 {
            continue;
        }
        let due = last.is_none_or(|prev| {
            ((point.0 - prev.0).powi(2) + (point.1 - prev.1).powi(2)).sqrt() >= spacing
        });
        if due {
            draw_marker(img, point.0, point.1, radius, marker, color);
            last = Some(point);
        }
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

/// Renders one or two bundles (single graph, or A solid / B dashed
/// overlay) at `width`, then hardens the result to WebP bytes.
fn render_bundles(
    bundles: &[&Bundle],
    names: &[&str],
    graph_name: &str,
    width: u32,
    sources: &[&str],
    opts: &RenderOptions,
) -> Result<Vec<u8>> {
    let has_y2 = bundles
        .iter()
        .any(|b| b.traces.iter().any(|t| t.axis == "y2"));
    let mut y_range = axis_range(bundles, "y").unwrap_or((-40.0, 10.0));
    if let Some(span) = opts.spl_range_db {
        y_range = (y_range.1 - span, y_range.1);
    }
    let y_range = y_range;
    let phase_axis = bundles.iter().any(|bundle| {
        bundle
            .traces
            .iter()
            .any(|trace| trace.axis == "y2" && trace.name == "Phase (deg)")
    });
    let y2_range = if phase_axis {
        axis_range(bundles, "y2").or(Some((-180.0, 360.0)))
    } else if has_y2 {
        Some((-5.0, 45.0))
    } else {
        None
    };
    let n_traces = bundles
        .iter()
        .map(|b| b.visible_traces(opts, bundles.len() > 1).len())
        .max()
        .unwrap_or(0);
    #[expect(clippy::cast_precision_loss, reason = "trace counts fit in f64")]
    #[expect(clippy::cast_possible_truncation, reason = "bundle count fits in u32")]
    let legend_rows = if opts.legend {
        ((n_traces as f64 / 3.0).ceil() as u32).max(1) * (bundles.len() as u32) + 5
    } else {
        1
    };
    #[expect(clippy::cast_possible_truncation, reason = "height fits in u32")]
    #[expect(clippy::cast_sign_loss, reason = "height is positive")]
    let height = (f64::from(width) * 2.0 / 3.0) as u32 + legend_rows * 22;

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

    let margin_l = 76.0;
    let margin_r = if has_y2 { 76.0 } else { 18.0 };
    let margin_t = if names.len() > 1 { 94.0 } else { 64.0 };
    let margin_b = 72.0 + f64::from(legend_rows) * 22.0;
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

    let title_lines = if names.len() > 1 {
        vec![
            format!("{graph_name}: {}", names[0]),
            format!("vs {}", names[1]),
        ]
    } else {
        vec![format!(
            "{graph_name} for {}",
            names.first().copied().unwrap_or_default()
        )]
    };
    if let Some(font) = &font {
        let title_size = title_lines
            .iter()
            .map(|line| fit_text(font, line, title_px, plot_w))
            .fold(title_px, f32::min);
        for (row, line) in title_lines.iter().enumerate() {
            draw_text(
                &mut img,
                font,
                line,
                title_size,
                plot_x0,
                s(14.0) + f64::from(title_size) + row as f64 * s(30.0),
                TEXT,
            );
        }
    }
    for freq in [
        20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
    ] {
        if freq_lo <= freq && freq <= freq_hi {
            let x = x_of(freq);
            if opts.grid {
                draw_segment(&mut img, (x, plot_y0), (x, plot_y1), GRID, f64::from(SS));
            }
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
        if opts.grid {
            draw_segment(&mut img, (plot_x0, y), (plot_x1, y), GRID, f64::from(SS));
        }
        if let Some(font) = &font {
            draw_text(
                &mut img,
                font,
                &fmt_tick(tick),
                small_px,
                plot_x0 - s(6.0) - text_width(font, &fmt_tick(tick), small_px),
                centered_baseline(font, &fmt_tick(tick), small_px, y),
                TEXT,
            );
        }
    }
    if let (Some(range), Some(font)) = (y2_range, &font) {
        let ticks = if phase_axis {
            vec![-180.0, -90.0, 0.0, 90.0, 180.0, 270.0, 360.0]
        } else {
            nice_ticks(range.0, range.1)
        };
        for tick in ticks {
            let y = y_of(tick, range);
            draw_text(
                &mut img,
                font,
                &fmt_tick(tick),
                small_px,
                plot_x1 + s(6.0),
                centered_baseline(font, &fmt_tick(tick), small_px, y),
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
        draw_axis_title(
            &mut img,
            font,
            "SPL (dB)",
            small_px,
            s(15.0),
            (plot_y0 + plot_y1) / 2.0,
            false,
        );
        if has_y2 {
            draw_axis_title(
                &mut img,
                font,
                if phase_axis { "Phase (deg)" } else { "DI (dB)" },
                small_px,
                s(f64::from(width) - 15.0),
                (plot_y0 + plot_y1) / 2.0,
                true,
            );
        }
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
        for trace in bundle.visible_traces(opts, bundles.len() > 1) {
            let range = if trace.axis == "y2" {
                y2_range.unwrap_or(y_range)
            } else {
                y_range
            };
            // One colour per speaker (black when alone); the curve
            // identity comes from its marker shape, shared across
            // speakers for the same measurement.
            let color = trace_color(&trace.name, speaker_idx, bundles.len());
            let is_phase = phase_axis && trace.axis == "y2";
            let values = if is_phase {
                trace.values.clone()
            } else {
                smooth_octaves(
                    &trace.values,
                    bundle.points,
                    bundle.freq_min,
                    bundle.freq_max,
                    opts.smooth_oct,
                )
            };
            let points: Vec<(f64, f64)> = freqs
                .iter()
                .zip(&values)
                .map(|(f, v)| {
                    if v.is_finite() {
                        (x_of(*f), y_of(*v, range))
                    } else {
                        (f64::NAN, f64::NAN)
                    }
                })
                .collect();
            draw_polyline(
                &mut img,
                &points,
                color,
                dashed || is_phase,
                2.0 * f64::from(SS),
            );
            if let Some(marker) = marker_for(&trace.name) {
                draw_markers(
                    &mut img,
                    &points,
                    marker,
                    color,
                    (plot_x0, plot_y0, plot_x1, plot_y1),
                );
            }
        }
    }

    if opts.legend
        && let Some(font) = &font
    {
        let mut legend_y = plot_y1 + s(50.0);
        for (speaker_idx, bundle) in bundles.iter().enumerate() {
            let name = names[speaker_idx];
            let label = if speaker_idx == 1 {
                format!("{name} (dashed)")
            } else {
                name.to_owned()
            };
            draw_text(
                &mut img,
                font,
                &label,
                fit_text(font, &label, font_px, plot_w),
                plot_x0,
                legend_y + f64::from(font_px),
                TEXT,
            );
            legend_y += s(20.0);
            for (trace_idx, trace) in bundle
                .visible_traces(opts, bundles.len() > 1)
                .iter()
                .enumerate()
            {
                let x = plot_x0 + (trace_idx % 3) as f64 * plot_w / 3.0;
                let label_px = fit_text(font, &trace.name, small_px, plot_w / 3.0 - s(40.0));
                let baseline = centered_baseline(font, &trace.name, label_px, legend_y + s(9.0));
                let color = trace_color(&trace.name, speaker_idx, bundles.len());
                draw_polyline(
                    &mut img,
                    &[(x, legend_y + s(9.0)), (x + s(26.0), legend_y + s(9.0))],
                    color,
                    speaker_idx == 1,
                    2.0 * f64::from(SS),
                );
                if let Some(marker) = marker_for(&trace.name) {
                    draw_marker(
                        &mut img,
                        x + s(13.0),
                        legend_y + s(9.0),
                        4.5 * f64::from(SS),
                        marker,
                        color,
                    );
                }
                draw_text(
                    &mut img,
                    font,
                    &trace.name,
                    label_px,
                    x + s(32.0),
                    baseline,
                    TEXT,
                );
                if (trace_idx + 1) % 3 == 0 {
                    legend_y += s(22.0);
                }
            }
            legend_y += s(22.0);
        }
    }

    let img = image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3);
    let sources = sources
        .iter()
        .copied()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" + ");
    let seed = format!("graph/{graph_name}/{}", names.join("/"));
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
        let opts = RenderOptions::default();
        let values = &bundle.traces[0].values;
        assert_eq!(values[1], 1.0);
        assert!(values[2].is_nan());
        // Recommended zones are dropped from the overlay.
        assert_eq!(bundle.visible_traces(&opts, true).len(), 1);
        assert_eq!(bundle.freqs().len(), 3);
    }

    #[test]
    fn render_produces_webp_bytes() {
        let bundle_a =
            parse_bundle(&bundle_json(&[0, -100, -200, -300], &[0, 0, 0, 0])).expect("bundle a");
        let bundle_b =
            parse_bundle(&bundle_json(&[-50, -150, -250, -350], &[0, 0, 0, 0])).expect("bundle b");
        let body = render_bundles(
            &[&bundle_a, &bundle_b],
            &["A (ASR)", "B (ASR)"],
            "CEA2034",
            900,
            &["Audio Science Review", "Audio Science Review"],
            &RenderOptions::default(),
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

    #[test]
    fn comparison_omits_bands_but_keeps_trends_and_short_reference_lines() {
        let mut bundle = parse_bundle(&bundle_json(&[0, 0], &[0, 0])).unwrap();
        bundle.traces = [
            "Band ±3dB",
            "Band ±1.5dB",
            "Midrange ±3dB",
            "Midrange Band +3dB",
            "Midrange Band -3dB",
            "Linear Regression",
            "On Axis",
        ]
        .into_iter()
        .map(|name| Trace {
            name: name.to_owned(),
            axis: "y".to_owned(),
            values: vec![0.0; 2],
        })
        .collect();
        let opts = RenderOptions::default();
        let names: Vec<_> = bundle
            .visible_traces(&opts, true)
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Midrange ±3dB",
                "Midrange Band +3dB",
                "Midrange Band -3dB",
                "Linear Regression",
                "On Axis"
            ]
        );
        assert_eq!(bundle.visible_traces(&opts, false).len(), 7);
    }

    #[test]
    fn text_ink_is_centered_on_ticks_and_legend_samples() {
        let font = load_font().expect("font for renderer tests");
        for text in ["-45", "0", "20", "On Axis", "Sound Power DI"] {
            let mut img = RgbImage::from_pixel(400, 80, Rgb([255, 255, 255]));
            let baseline = centered_baseline(&font, text, 24.0, 40.0);
            draw_text(&mut img, &font, text, 24.0, 10.0, baseline, TEXT);
            let rows: Vec<_> = (0..80)
                .filter(|&y| (0..400).any(|x| img.get_pixel(x, y).0[0] < 128))
                .collect();
            let center = f64::from(rows[0] + rows[rows.len() - 1]) / 2.0;
            assert!(
                (center - 40.0).abs() <= 1.0,
                "{text}: ink centered at {center}"
            );
        }
    }

    #[test]
    fn markers_are_shared_per_measurement_and_helpers_have_none() {
        // Same measurement, same shape for both speakers, keyed by name.
        assert_eq!(marker_for("On Axis"), Some(Marker::Circle));
        assert_eq!(marker_for("ON"), marker_for("On Axis"));
        assert_eq!(marker_for("Listening Window"), Some(Marker::Square));
        assert_ne!(marker_for("On Axis"), marker_for("Listening Window"));
        assert_eq!(marker_for("30°"), marker_for("-30°"));
        assert_eq!(marker_for("Band ±3dB"), None);
        assert_eq!(marker_for("recommended LW zone"), None);
        assert_eq!(marker_for("On Axis slope"), None);
        assert_eq!(marker_for("N/A"), None);
        // Unknown measurements are stable across calls.
        assert_eq!(marker_for("Custom Curve"), marker_for("Custom Curve"));
        assert!(marker_for("Custom Curve").is_some());
    }

    #[test]
    fn trace_colors_are_black_single_and_one_per_speaker_compared() {
        assert_eq!(trace_color("On Axis", 0, 1), Rgb([0, 0, 0]));
        assert_eq!(trace_color("Listening Window", 0, 1), Rgb([0, 0, 0]));
        assert_eq!(trace_color("On Axis", 0, 2), SPEAKER_COLORS[0]);
        assert_eq!(trace_color("On Axis", 1, 2), SPEAKER_COLORS[1]);
        // Every measurement of one speaker shares that speaker's colour.
        assert_eq!(trace_color("Sound Power", 1, 2), SPEAKER_COLORS[1]);
        assert_eq!(trace_color("N/A", 0, 2), ZONE_COLOR);
    }

    #[test]
    fn rendered_pictures_use_speaker_colors_not_bundle_colors() {
        // Chromatic predicates, not raw distance: grey axes/text sit
        // within a loose distance of the blue, but have no chroma.
        // Hardening (texture + lossy WebP) only shifts channels a bit.
        fn count_where(body: &[u8], pred: impl Fn(u8, u8, u8) -> bool) -> usize {
            let img = image::load_from_memory(body)
                .expect("decode webp")
                .to_rgb8();
            img.pixels().filter(|p| pred(p[0], p[1], p[2])).count()
        }
        let is_a_blue = |r: u8, g: u8, b: u8| b > r.saturating_add(35) && b > g.saturating_add(12);
        let is_b_orange = |r: u8, g: u8, b: u8| {
            r > b.saturating_add(80) && r > g.saturating_add(40) && g > b.saturating_add(25)
        };
        let is_pure_red =
            |r: u8, g: u8, b: u8| r > g.saturating_add(120) && r > b.saturating_add(120) && g < 70;
        let is_black = |r: u8, g: u8, b: u8| r < 45 && g < 45 && b < 45;
        // Bundle traces claim pure red; rendering must ignore that.
        let bundle_a =
            parse_bundle(&bundle_json(&[0, -100, -200, -300], &[0, 0, 0, 0])).expect("bundle a");
        let bundle_b =
            parse_bundle(&bundle_json(&[-50, -150, -250, -350], &[0, 0, 0, 0])).expect("bundle b");
        let compared = render_bundles(
            &[&bundle_a, &bundle_b],
            &["A (ASR)", "B (ASR)"],
            "CEA2034",
            900,
            &["", ""],
            &RenderOptions::default(),
        )
        .expect("compare render");
        assert!(count_where(&compared, is_a_blue) > 40, "speaker A colour");
        assert!(count_where(&compared, is_b_orange) > 40, "speaker B colour");
        assert_eq!(
            count_where(&compared, is_pure_red),
            0,
            "bundle colour leaked"
        );

        let single = render_bundles(
            &[&bundle_a],
            &["A (ASR)"],
            "CEA2034",
            900,
            &[""],
            &RenderOptions::default(),
        )
        .expect("single render");
        assert!(count_where(&single, is_black) > 40, "black curves");
        // The single view uses no speaker colours at all.
        assert_eq!(count_where(&single, is_a_blue), 0, "speaker colour leaked");
        assert_eq!(
            count_where(&single, is_b_orange),
            0,
            "speaker colour leaked"
        );
        assert_eq!(count_where(&single, is_pure_red), 0, "bundle colour leaked");
    }

    #[test]
    fn rendered_marker_hulls_match_the_small_triangles() {
        use d3rs::polygon::{polygon_area, polygon_hull};
        let mut areas = Vec::new();
        for marker in [
            Marker::Circle,
            Marker::Square,
            Marker::Diamond,
            Marker::TriangleUp,
            Marker::TriangleDown,
            Marker::TriangleLeft,
            Marker::TriangleRight,
            Marker::Star,
            Marker::Cross,
            Marker::X,
        ] {
            let mut img = RgbImage::from_pixel(201, 201, Rgb([255, 255, 255]));
            draw_marker(&mut img, 100.0, 100.0, 30.0, marker, Rgb([0, 0, 0]));
            let points: Vec<_> = img
                .enumerate_pixels()
                .filter(|(_, _, p)| p[0] < 128)
                .map(|(x, y, _)| (f64::from(x), f64::from(y)))
                .collect();
            areas.push(polygon_area(&polygon_hull(&points)).abs());
        }
        let triangle = areas[3];
        assert!(
            areas
                .iter()
                .all(|area| (area / triangle - 1.0).abs() < 0.12),
            "rasterized hull areas: {areas:?}"
        );
    }

    #[test]
    fn marker_shapes_draw_distinct_glyphs() {
        // Square fills its corner; diamond and triangle-up do not.
        let probe = |marker: Marker| {
            let mut img = RgbImage::from_pixel(41, 41, Rgb([255, 255, 255]));
            draw_marker(&mut img, 20.0, 20.0, 10.0, marker, Rgb([0, 0, 0]));
            (
                *img.get_pixel(13, 13),
                *img.get_pixel(20, 20),
                *img.get_pixel(20, 9),
            )
        };
        let (square_corner, square_centre, _) = probe(Marker::Square);
        let (diamond_corner, _, diamond_top) = probe(Marker::Diamond);
        assert_eq!(square_corner, Rgb([0, 0, 0]));
        assert_eq!(square_centre, Rgb([0, 0, 0]));
        assert_eq!(diamond_corner, Rgb([255, 255, 255]));
        assert_eq!(diamond_top, Rgb([0, 0, 0]));
    }

    #[test]
    fn smooth_octaves_averages_and_keeps_nan() {
        let values = [0.0, 10.0, 0.0, f64::NAN, 0.0, 10.0, 0.0];
        let out = smooth_octaves(&values, 701, 20.0, 20_000.0, 1.0 / 48.0);
        assert!(out[3].is_nan());
        // The spikes are pulled toward their neighbours.
        assert!(out[1] < 10.0 && out[1] > 0.0);
        // Zero smoothing is the identity.
        assert_eq!(smooth_octaves(&values, 701, 20.0, 20_000.0, 0.0)[1], 10.0);
    }

    #[test]
    fn serve_graph_renders_and_caches_with_options() {
        let root = std::env::temp_dir().join(format!("graph-test-{}", std::process::id()));
        let traces = root.join("files/Spk A/ASR/asr/traces");
        std::fs::create_dir_all(&traces).expect("mkdir");
        let json = bundle_json(&[0, -100, -200, -300], &[0, 0, 0, 0]);
        std::fs::write(traces.join("CEA2034.json"), &json).expect("write");
        let state = CompareState::new(root.join("cache"));
        let base: HashMap<String, String> = [
            ("measurement", "CEA2034"),
            ("speaker", "Spk A"),
            ("origin", "ASR"),
            ("version", "asr"),
            ("w", "900"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let first = serve_graph(&root.join("files"), &state, &base).expect("render");
        assert!(!first.cache_hit);
        assert_eq!(&first.body[..4], b"RIFF");
        let second = serve_graph(&root.join("files"), &state, &base).expect("cached");
        assert!(second.cache_hit);
        // Different options render a different picture.
        let mut no_grid = base.clone();
        no_grid.insert("grid".to_owned(), "0".to_owned());
        let third = serve_graph(&root.join("files"), &state, &no_grid).expect("render no grid");
        assert!(!third.cache_hit);
        assert_ne!(third.body, first.body);
        let _ = std::fs::remove_dir_all(&root);
    }
}
