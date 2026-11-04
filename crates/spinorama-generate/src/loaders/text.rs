//! Shared text measurement parsing helpers.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::measurements::{SplPlane, resample_log_freq};

/// One frequency/SPL curve parsed from a text file.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCurve {
    /// Frequency axis in hertz.
    pub freq: Vec<f64>,
    /// SPL values in dB aligned with `freq`.
    pub spl: Vec<f64>,
    /// Optional phase values in degrees aligned with `freq`.
    pub phase: Option<Vec<f64>>,
}

/// Lists regular files directly inside `dir`, sorted by file name.
///
/// # Errors
///
/// Returns an error when the directory cannot be read.
pub fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Reads and parses one frequency/SPL text file.
///
/// # Errors
///
/// Returns an error when the file cannot be read or contains no usable rows.
pub fn read_text_curve(
    path: &Path,
    min_freq: f64,
    max_freq: f64,
    skip_lines: usize,
) -> Result<TextCurve> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_text_curve(&text, min_freq, max_freq, skip_lines)
        .with_context(|| format!("parsing {}", path.display()))
}

/// Parses frequency/SPL rows from text.
///
/// Rows may be comma-, semicolon-, tab-, or whitespace-separated. A third
/// numeric column is preserved as phase degrees. Header rows are skipped only
/// before the first numeric data row.
///
/// # Errors
///
/// Returns an error when fewer than two usable points remain in range.
pub fn parse_text_curve(
    text: &str,
    min_freq: f64,
    max_freq: f64,
    skip_lines: usize,
) -> Result<TextCurve> {
    let mut rows: Vec<(f64, f64, Option<f64>)> = Vec::new();
    for raw in text.lines().skip(skip_lines) {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with('*')
            || line.starts_with('#')
            || line.starts_with(';')
        {
            continue;
        }
        let fields: Vec<&str> = line
            .split([',', ';', '\t', ' '])
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() < 2 {
            continue;
        }
        let parsed = (fields[0].parse::<f64>(), fields[1].parse::<f64>());
        let (Ok(freq), Ok(spl)) = parsed else {
            if rows.is_empty() {
                continue;
            }
            bail!("non-numeric frequency/SPL row: {line:?}");
        };
        if !freq.is_finite() || !spl.is_finite() || freq < min_freq || freq > max_freq {
            continue;
        }
        let phase = match fields.get(2) {
            Some(value) => Some(
                value
                    .parse::<f64>()
                    .with_context(|| format!("non-numeric phase value {value:?}"))?,
            ),
            None => None,
        };
        rows.push((freq, spl, phase));
    }
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    rows.dedup_by(|next, keep| next.0 == keep.0);
    if rows.len() < 2 {
        bail!("only {} usable frequency/SPL points", rows.len());
    }
    let has_phase = rows.iter().all(|row| row.2.is_some());
    Ok(TextCurve {
        freq: rows.iter().map(|row| row.0).collect(),
        spl: rows.iter().map(|row| row.1).collect(),
        phase: has_phase.then(|| rows.iter().map(|row| row.2.unwrap_or(f64::NAN)).collect()),
    })
}

/// Builds an SPL plane by aligning named text curves on one frequency axis.
///
/// The reference axis is `On Axis` when present, otherwise the first curve.
/// Other curves are explicitly resampled in log-frequency; loaders never zip
/// mismatched grids by row index.
///
/// # Errors
///
/// Returns an error when no curves are supplied or the plane is invalid.
pub fn plane_from_curves(curves: Vec<(String, TextCurve)>) -> Result<SplPlane> {
    let reference = curves
        .iter()
        .find(|(name, _)| name == "On Axis")
        .or_else(|| curves.first())
        .ok_or_else(|| anyhow::anyhow!("no text curves to build an SPL plane"))?;
    let freq = reference.1.freq.clone();
    let mut phase_on_axis = None;
    let mut columns = Vec::with_capacity(curves.len());
    for (name, curve) in curves {
        let values = if curve.freq == freq {
            curve.spl.clone()
        } else {
            resample_log_freq(&freq, &curve.freq, &curve.spl)
        };
        if name == "On Axis"
            && let Some(phase) = &curve.phase
        {
            phase_on_axis = Some(if curve.freq == freq {
                phase.clone()
            } else {
                resample_log_freq(&freq, &curve.freq, phase)
            });
        }
        columns.push((name, values));
    }
    let mut plane = SplPlane::new(freq, columns)?;
    if let Some(phase) = phase_on_axis {
        plane = plane.with_phase(phase)?;
    }
    plane.sort_by_angle();
    Ok(plane)
}
