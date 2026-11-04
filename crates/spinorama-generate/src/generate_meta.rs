//! Assembly helpers for the `generate-meta` binary.
//!
//! Ports of the pure pieces of Python `scripts/generate_meta.py`:
//! scaled ratings, quality derivation, REW EQ file assembly,
//! sensitivity distance correction, nearest-speaker distances, and
//! the `dist/json` metadata dump (full, head, and per-year chunks
//! with zip/bz2 companions).

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use autoeq_workflow::iir::{Biquad, BiquadFilterType};
use serde_json::{Map, Value};

use crate::filters::biquad::{Peq, peq_response_db};
use crate::meta::sanitize_filename;
use crate::metadata::{MeasurementQuality, SpeakerMeta};

/// Number of speakers in the `metadata-head.json` chunk.
pub const METADATA_HEAD_SIZE: usize = 20;

/// Maps a preference score to the 0–100 site bucket (Python
/// `compute_scaled_pref_score`).
#[must_use]
pub fn scaled_pref_score(pref_score: f64) -> f64 {
    for (threshold, scaled) in [
        (7.0, 100.0),
        (6.5, 90.0),
        (6.0, 80.0),
        (5.5, 70.0),
        (5.0, 60.0),
        (4.5, 50.0),
        (4.0, 40.0),
        (3.0, 30.0),
        (2.0, 20.0),
        (1.0, 10.0),
    ] {
        if pref_score > threshold {
            return scaled;
        }
    }
    0.0
}

/// Maps a reference band flatness to the 0–100 site bucket.
#[must_use]
pub fn scaled_flatness(flatness: f64) -> f64 {
    for (threshold, scaled) in [
        (2.0, 100.0),
        (2.25, 90.0),
        (2.5, 80.0),
        (3.0, 70.0),
        (4.0, 50.0),
        (5.0, 30.0),
        (6.0, 10.0),
    ] {
        if flatness < threshold {
            return scaled;
        }
    }
    0.0
}

/// Maps an LFX frequency to the 0–100 site bucket.
#[must_use]
pub fn scaled_lfx_hz(lfx_hz: f64) -> f64 {
    for (threshold, scaled) in [
        (25.0, 100.0),
        (30.0, 90.0),
        (35.0, 80.0),
        (40.0, 70.0),
        (50.0, 60.0),
        (60.0, 50.0),
        (70.0, 40.0),
        (80.0, 30.0),
        (100.0, 10.0),
    ] {
        if lfx_hz < threshold {
            return scaled;
        }
    }
    0.0
}

/// Maps an in-room smoothness to the 0–100 site bucket.
#[must_use]
pub fn scaled_sm_pir(sm_pir: f64) -> f64 {
    for (threshold, scaled) in [
        (0.95, 100.0),
        (0.9, 90.0),
        (0.85, 80.0),
        (0.8, 70.0),
        (0.7, 60.0),
        (0.6, 50.0),
        (0.5, 25.0),
    ] {
        if sm_pir > threshold {
            return scaled;
        }
    }
    0.0
}

/// Builds the `scaled_pref_rating` dictionary.
///
/// Note: Python passes `sm_pred_in_room` through the LFX bucketer
/// (`compute_scaled_lfx_hz`) for `scaled_sm_pred_in_room`; that
/// behaviour is preserved here.
#[must_use]
pub fn scaled_pref_rating(
    flatness: Option<f64>,
    pref_rating: &Map<String, Value>,
) -> Map<String, Value> {
    let get = |key: &str| pref_rating.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let mut out = Map::new();
    out.insert(
        "scaled_flatness".to_owned(),
        Value::from(flatness.map_or(0.0, scaled_flatness)),
    );
    out.insert(
        "scaled_pref_score".to_owned(),
        Value::from(scaled_pref_score(get("pref_score"))),
    );
    out.insert(
        "scaled_pref_wsub".to_owned(),
        Value::from(scaled_pref_score(get("pref_score_wsub"))),
    );
    out.insert(
        "scaled_lfx_hz".to_owned(),
        Value::from(scaled_lfx_hz(get("lfx_hz"))),
    );
    out.insert(
        "scaled_sm_pred_in_room".to_owned(),
        Value::from(scaled_lfx_hz(get("sm_pred_in_room"))),
    );
    out
}

/// Derives the quality grade when metadata does not set one.
///
/// Port of `add_quality` in Python `generate_meta.py`.
#[must_use]
pub fn derived_quality(speaker: &SpeakerMeta, version: &str) -> &'static str {
    let measurement = &speaker.measurements[version];
    if let Some(quality) = measurement.quality {
        return match quality {
            MeasurementQuality::High => "high",
            MeasurementQuality::Medium => "medium",
            MeasurementQuality::Low => "low",
            MeasurementQuality::Unknown => "unknown",
        };
    }
    let origin = measurement.origin.as_str();
    if measurement.format.as_str() == "klippel" {
        return "high";
    }
    if origin == "Princeton" {
        return "low";
    }
    if origin == "Misc" {
        if version.contains("napilopez") || version.contains("audioholics") {
            return "low";
        }
        return "unknown";
    }
    if origin.contains("Vendor") {
        return match speaker.brand.as_str() {
            "JBL" | "Revel" | "Infinity" | "Aalto Speakers" => "medium",
            "Ascend Acoustics" => "high",
            _ => "unknown",
        };
    }
    "unknown"
}

/// Corrects a sensitivity measured at `distance` metres to 1 m.
///
/// Port of `compute_sensitivity_distance`: monopole table, unknown
/// distances (other than 1 m) keep the measured value.
#[must_use]
pub fn sensitivity_at_1m(sensitivity: f64, distance: f64) -> f64 {
    let delta = match distance {
        2.0 => 6.0,
        2.5 => 8.0,
        3.0 => 9.5,
        4.0 => 12.0,
        5.0 => 14.0,
        10.0 => 20.0,
        _ => 0.0,
    };
    sensitivity + delta
}

/// Returns true for Princeton or `min_valid_freq > 20` measurements.
///
/// Port of `is_partial_measurement` (without the `_eq` suffix: callers
/// pass the base version).
#[must_use]
pub fn is_partial(speaker: &SpeakerMeta, version: &str) -> bool {
    let Some(measurement) = speaker.measurements.get(version) else {
        return false;
    };
    if measurement.origin == "Princeton" {
        return true;
    }
    measurement
        .data_acquisition
        .as_ref()
        .and_then(|da| da.min_valid_freq)
        .is_some_and(|min_valid| min_valid > 20.0)
}

/// One REW/APO filter with its Python biquad type code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RewFilter {
    /// Response weight (1.0 when ON, 0.0 when OFF).
    pub weight: f64,
    /// Python `Biquad` type code (LP 0, HP 1, BP 2, PK 3, NO 4, LS 5, HS 6).
    pub type_code: i64,
    /// Filter kind for the Rust biquad constructor.
    pub kind: BiquadFilterType,
    /// Centre/corner frequency in hertz (truncated to whole hertz,
    /// as in Python `parse_eq_line`).
    pub freq: f64,
    /// Sample rate stamped on the filter.
    pub srate: f64,
    /// Quality factor after Python defaults are applied.
    pub q: f64,
    /// Gain in dB.
    pub db_gain: f64,
}

fn bw2q(bw: f64) -> f64 {
    (2f64.powf(bw)).sqrt() / (2f64.powf(bw) - 1.0)
}

/// Parses a REW EQ file (`parse_eq_iir_rews`).
///
/// Missing files yield an empty vector. Lines that Python would skip
/// (comments, malformed, out-of-range values, unknown kinds) are
/// skipped here as well instead of failing the whole file.
#[must_use]
pub fn parse_rew_eq(path: &Path, srate: f64) -> Vec<RewFilter> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(filter) = parse_rew_line(line, srate) {
            out.push(filter);
        }
    }
    out
}

fn parse_rew_line(line: &str, srate: f64) -> Option<RewFilter> {
    if line.starts_with('*') {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() <= 3 || words[0] != "Filter" {
        return None;
    }
    let weight = if words[2] == "ON" { 1.0 } else { 0.0 };
    let (kind, freq_s, gain_s, q_s, bw_s): (
        &str,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
    ) = match words.len() {
        13 => (
            words[3],
            Some(words[5]),
            Some(words[8]),
            None,
            Some(words[12]),
        ),
        12 => (
            words[3],
            Some(words[5]),
            Some(words[8]),
            Some(words[11]),
            None,
        ),
        10 => (words[3], Some(words[5]), Some(words[8]), None, None),
        9 => (
            words[3],
            Some(words[5].strip_suffix(',').unwrap_or(words[5])),
            None,
            Some(words[8]),
            None,
        ),
        8 => (
            words[3],
            Some(words[5].strip_suffix(',').unwrap_or(words[5])),
            Some(words[6].strip_suffix(',').unwrap_or(words[6])),
            Some(words[7]),
            None,
        ),
        7 => (words[3], Some(words[5]), None, None, None),
        _ => return None,
    };
    let mut freq = 0.0;
    if let Some(token) = freq_s {
        let value: f64 = token.parse().ok()?;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Python truncates via int(float)"
        )]
        let truncated = value.trunc() as i64;
        if truncated < 0 || value > srate / 2.0 {
            return None;
        }
        freq = value;
    }
    let mut gain: f64 = 0.0;
    if let Some(token) = gain_s {
        gain = token.parse().ok()?;
        if gain.abs() > 30.0 {
            return None;
        }
    }
    let mut q = 0.0;
    if let Some(token) = q_s {
        q = token.parse().ok()?;
        if !(0.0..=30.0).contains(&q) {
            return None;
        }
    }
    if let Some(token) = bw_s {
        if q_s.is_some() {
            return None;
        }
        q = bw2q(token.parse().ok()?);
    }
    // Python Biquad defaults, applied before coefficient computation.
    let (type_code, kind, q) = match kind {
        "PK" | "PEQ" | "Modal" => (3, BiquadFilterType::Peak, q),
        "NO" => (4, BiquadFilterType::Notch, 30.0),
        "BP" => (
            2,
            BiquadFilterType::Bandpass,
            if q == 0.0 {
                std::f64::consts::FRAC_1_SQRT_2
            } else {
                q
            },
        ),
        "HP" | "HPQ" => (
            1,
            BiquadFilterType::Highpass,
            if q == 0.0 {
                std::f64::consts::FRAC_1_SQRT_2
            } else {
                q
            },
        ),
        "LP" | "LPQ" => (
            0,
            BiquadFilterType::Lowpass,
            if q == 0.0 {
                std::f64::consts::FRAC_1_SQRT_2
            } else {
                q
            },
        ),
        "LS" | "LSQ" | "LSC" => (
            5,
            BiquadFilterType::Lowshelf,
            if q == 0.0 { bw2q(0.9) } else { q },
        ),
        "HS" | "HSQ" | "HSC" => (
            6,
            BiquadFilterType::Highshelf,
            if q == 0.0 { bw2q(0.9) } else { q },
        ),
        _ => return None,
    };
    Some(RewFilter {
        weight,
        type_code,
        kind,
        freq,
        srate,
        q,
        db_gain: gain,
    })
}

impl RewFilter {
    /// Converts to a Rust AutoEQ biquad (keeping the weight).
    #[must_use]
    pub fn to_biquad(&self) -> (f64, Biquad) {
        (
            self.weight,
            Biquad::new(self.kind, self.freq, self.srate, self.q, self.db_gain),
        )
    }

    /// Serializes as one Python `Peq` JSON object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("type".to_owned(), Value::from(self.type_code));
        obj.insert("freq".to_owned(), Value::from(self.freq));
        obj.insert("srate".to_owned(), Value::from(self.srate));
        obj.insert("Q".to_owned(), Value::from(self.q));
        obj.insert("dbGain".to_owned(), Value::from(self.db_gain));
        Value::Object(obj)
    }
}

/// Computes the APO preamp gain of a filter set (Python
/// `peq_preamp_gain`, rounded by the caller).
#[must_use]
pub fn preamp_gain(filters: &[RewFilter]) -> f64 {
    if filters.is_empty() {
        return 0.0;
    }
    let peq: Peq = filters.iter().map(RewFilter::to_biquad).collect();
    // np.logspace(1+log10(2), 4+log10(2), 1000) == 20 Hz..20 kHz.
    let freq: Vec<f64> = (0..1000)
        .map(|i| 20.0 * 10f64.powf(3.0 * f64::from(i) / 999.0))
        .collect();
    let response = peq_response_db(&freq, &peq);
    let overall = response
        .into_iter()
        .fold(0.0_f64, |acc, v| acc.max(v.max(0.0)));
    -overall
}

/// Rounds to one decimal, Python `round(x, 1)` (half-to-even).
#[must_use]
pub fn round1(x: f64) -> f64 {
    // Python round uses banker's rounding on the decimal repr; the
    // values here are binary floats, so match via format-free scaling.
    let scaled = x * 10.0;
    let rounded = if (scaled.fract().abs() - 0.5).abs() < 1e-12 {
        let floor = scaled.floor();
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        scaled.round()
    };
    rounded / 10.0
}

const EQ_SUFFIXES: &[(&str, &str)] = &[
    ("autoeq", "AutomaticEQ (IIR)"),
    ("autoeq-lw", "AutomaticEQ LW (IIR)"),
    ("autoeq-score", "AutomaticEQ Score (IIR)"),
    ("amirm", "amirm@ASR (IIR)"),
    ("maiky76", "maiky76@ASR (IIR)"),
    ("maiky76-lw", "maiky76@ASR LW (IIR)"),
    ("maiky76-score", "maiky76@ASR (IIR)"),
    ("flipflop", "flipflop@ASR (IIR)"),
    ("autoeq-dbx-1215", "Graphic EQ 15 bands"),
    ("autoeq-dbx-1231", "Graphic EQ 31 bands"),
];

/// Loads every EQ file of one speaker (Python `_eq_worker`).
///
/// `datas_dir` is the `datas/` root; returned filenames keep the
/// Python `./datas/eq/<speaker>/iir-<suffix>.txt` spelling when
/// `datas_dir` is `./datas` — the caller passes the display prefix.
#[must_use]
pub fn speaker_eqs(datas_dir: &Path, speaker_name: &str) -> (Option<String>, Map<String, Value>) {
    let mut default_eq = None;
    let mut eqs = Map::new();
    for (suffix, display) in EQ_SUFFIXES {
        let path = datas_dir
            .join("eq")
            .join(sanitize_filename(speaker_name))
            .join(format!("iir-{suffix}.txt"));
        let filters = parse_rew_eq(&path, 48_000.0);
        let active: Vec<&RewFilter> = filters.iter().filter(|f| f.weight != 0.0).collect();
        if active.is_empty() {
            continue;
        }
        if *suffix == "autoeq" {
            default_eq = Some("autoeq".to_owned());
        }
        let mut eq = Map::new();
        eq.insert("display_name".to_owned(), Value::from(*display));
        eq.insert(
            "filename".to_owned(),
            Value::from(path.to_string_lossy().into_owned()),
        );
        eq.insert(
            "preamp_gain".to_owned(),
            Value::from(round1(preamp_gain(&filters))),
        );
        eq.insert("type".to_owned(), Value::from("peq"));
        eq.insert(
            "peq".to_owned(),
            Value::Array(active.iter().map(|f| f.to_json()).collect()),
        );
        eqs.insert(suffix.replace('-', "_"), Value::Object(eq));
    }
    (default_eq, eqs)
}

/// Interpolates `data1` onto `freq` with Python `interpolate`
/// semantics (linear, 0 outside on the left edge cases it produces).
#[must_use]
pub fn interpolate(freq: &[f64], freq1: &[f64], data1: &[f64]) -> Vec<f64> {
    let len1 = freq1.len();
    let mut out = Vec::with_capacity(freq.len());
    let mut i: isize = 0;
    for &f in freq {
        let step = |i: &mut isize| -> Option<f64> {
            while {
                let idx = usize::try_from(*i).ok().filter(|u| *u < len1)?;
                freq1[idx] < f && (*i as usize) < len1
            } {
                *i += 1;
            }
            if (*i as usize) >= len1 {
                return Some(0.0);
            }
            let idx = *i as usize;
            if freq1[idx] >= f {
                if *i == 0 {
                    return Some(0.0);
                }
                *i -= 1;
            }
            let start = *i as usize;
            let mut j = start;
            while j < len1 && freq1[j] < f {
                j += 1;
            }
            if j >= len1 {
                return Some(data1[start]);
            }
            Some(
                data1[start]
                    + (data1[j] - data1[start]) * (f - freq1[start]) / (freq1[j] - freq1[start]),
            )
        };
        out.push(step(&mut i).unwrap_or(0.0));
    }
    out
}

/// Mean of the three curve norms (Python `compute_near`).
#[must_use]
pub fn compute_near(a: &(Vec<f64>, Vec<f64>, Vec<f64>), b: &(Vec<f64>, Vec<f64>, Vec<f64>)) -> f64 {
    let norm = |x: &[f64], y: &[f64]| {
        x.iter()
            .zip(y)
            .map(|(u, v)| (u - v).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let near = (norm(&a.0, &b.0) + norm(&a.1, &b.1) + norm(&a.2, &b.2)) / 3.0;
    if near.is_nan() { 1_000_000.0 } else { near }
}

/// The 100-point log grid used for nearest-speaker distances.
#[must_use]
pub fn near_freq_grid() -> Vec<f64> {
    (0..100)
        .map(|i| 25.0 * (16000.0 / 25.0f64).powf(f64::from(i) / 99.0))
        .collect()
}

fn write_if_different(path: &Path, content: &str) -> Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == content
    {
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn write_compressed(path: &Path, entry_name: &str, content: &str, bzip2: bool) -> Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = if bzip2 {
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Bzip2)
    } else {
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
    };
    writer
        .start_file(entry_name, options)
        .context("starting zip entry")?;
    writer
        .write_all(content.as_bytes())
        .context("writing zip entry")?;
    writer.finish().context("finishing zip archive")?;
    Ok(())
}

fn dict_to_json(
    json_dir: &Path,
    stem: &str,
    data: &Map<String, Value>,
    expected: &mut Vec<PathBuf>,
) -> Result<()> {
    let js = serde_json::to_string(&Value::Object(data.clone())).context("serializing metadata")?;
    let filename = json_dir.join(format!("{stem}.json"));
    let changed = write_if_different(&filename, &js)?;
    expected.push(filename.clone());
    for (ext, bzip2) in [("zip", false), ("bz2", true)] {
        let compressed = json_dir.join(format!("{stem}.json.{ext}"));
        if changed || !compressed.is_file() {
            write_compressed(&compressed, &filename.to_string_lossy(), &js, bzip2)?;
        }
        expected.push(compressed);
    }
    Ok(())
}

fn cleanup_outputs(json_dir: &Path, expected: &[PathBuf]) {
    let expected_set: std::collections::HashSet<PathBuf> = expected.iter().cloned().collect();
    let Ok(entries) = std::fs::read_dir(json_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_metadata_slice = (name.starts_with("metadata") || name.starts_with("eqdata"))
            && (name.ends_with(".json")
                || name.ends_with(".json.zip")
                || name.ends_with(".json.bz2"));
        if is_metadata_slice && !expected_set.contains(&path) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Writes the speaker metadata JSON files (Python `dump_metadata`).
///
/// `meta` maps speaker names to their full JSON objects (including
/// `eqs`); `skip` speakers must already be removed by the caller.
/// Output order inside objects follows insertion order of the maps.
///
/// # Errors
///
/// Returns an error when a file cannot be written or serialized.
pub fn dump_metadata(json_dir: &Path, meta: &BTreeMap<String, Value>) -> Result<()> {
    std::fs::create_dir_all(json_dir)
        .with_context(|| format!("creating {}", json_dir.display()))?;
    let mut meta_full: Map<String, Value> = Map::new();
    let mut eq_full: Map<String, Value> = Map::new();
    for (name, speaker) in meta {
        let Some(obj) = speaker.as_object() else {
            continue;
        };
        let mut without_eqs = obj.clone();
        without_eqs.remove("eqs");
        meta_full.insert(name.clone(), Value::Object(without_eqs));
        let mut eq_only = Map::new();
        for key in ["eqs", "brand", "model"] {
            if let Some(value) = obj.get(key) {
                eq_only.insert(key.to_owned(), value.clone());
            }
        }
        eq_full.insert(name.clone(), Value::Object(eq_only));
    }

    let mut expected: Vec<PathBuf> = Vec::new();
    dict_to_json(json_dir, "metadata", &meta_full, &mut expected)?;
    dict_to_json(json_dir, "eqdata", &eq_full, &mut expected)?;

    // Sort by default measurement publication date, newest first.
    let date_of = |speaker: &Value| -> String {
        let default = speaker
            .get("default_measurement")
            .and_then(Value::as_str)
            .unwrap_or("");
        speaker
            .get("measurements")
            .and_then(|m| m.get(default))
            .and_then(|m| m.get("review_published"))
            .and_then(Value::as_str)
            .unwrap_or("20170101")
            .to_owned()
    };
    let mut sorted: Vec<(&String, &Value)> = meta_full.iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(date_of(a.1)));

    let head: Map<String, Value> = sorted
        .iter()
        .take(METADATA_HEAD_SIZE)
        .map(|(k, v)| ((*k).clone(), (*v).clone()))
        .collect();
    dict_to_json(json_dir, "metadata-head", &head, &mut expected)?;

    // Group the tail by year (years before 2020 collapse into 2019).
    let tail = &sorted[METADATA_HEAD_SIZE.min(sorted.len())..];
    let mut groups: Vec<(i64, Map<String, Value>)> = Vec::new();
    for (name, speaker) in tail {
        let date = date_of(speaker);
        let year_raw: i64 = date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(1970);
        let year = if (1970..2020).contains(&year_raw) && year_raw > 1970 {
            2019
        } else {
            year_raw
        };
        match groups.last_mut() {
            Some((last_year, map)) if *last_year == year => {
                map.insert((*name).clone(), (*speaker).clone());
            }
            _ => {
                let mut map = Map::new();
                map.insert((*name).clone(), (*speaker).clone());
                groups.push((year, map));
            }
        }
    }
    for (year, group) in &groups {
        dict_to_json(
            json_dir,
            &format!("metadata-{year:04}"),
            group,
            &mut expected,
        )?;
    }

    cleanup_outputs(json_dir, &expected);
    Ok(())
}
