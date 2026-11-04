//! Klippel NFS text export loader (`SPL Horizontal.txt` / `SPL Vertical.txt`).

// Rust guideline compliant 2026-02-21

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::measurements::{HvSpl, SplPlane};

/// Loads a Klippel version directory.
///
/// The version directory must contain `SPL Horizontal.txt` and
/// `SPL Vertical.txt`, the mandatory pair of the Python
/// `loaders/klippel.py` format.
///
/// # Errors
///
/// Returns an error when either file is missing or malformed.
pub fn load(version_dir: &Path) -> Result<HvSpl> {
    let horizontal = load_plane(&version_dir.join("SPL Horizontal.txt"))?;
    let vertical = load_plane(&version_dir.join("SPL Vertical.txt"))?;
    Ok(HvSpl {
        horizontal,
        vertical,
    })
}

fn load_plane(path: &Path) -> Result<SplPlane> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading Klippel file {}", path.display()))?;
    let mut lines = text.lines();
    let _title = lines.next();
    let header = lines
        .next()
        .with_context(|| format!("{}: missing angle header line", path.display()))?;
    let _units = lines.next();

    // Header fields alternate angle name / empty: each angle owns a
    // (frequency, SPL) column pair in the data rows below.
    let names: Vec<String> = header
        .split('\t')
        .map(|f| f.trim().trim_matches('"').to_owned())
        .filter(|f| !f.is_empty())
        .map(|f| {
            if f == "On-Axis" {
                "On Axis".to_owned()
            } else {
                f
            }
        })
        .collect();
    if names.is_empty() {
        bail!("{}: no angle columns in header", path.display());
    }

    let mut freq: Vec<f64> = Vec::new();
    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
    for (row, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < names.len() * 2 {
            bail!(
                "{}: data row {} has {} fields, expected {}",
                path.display(),
                row,
                fields.len(),
                names.len() * 2
            );
        }
        freq.push(parse_number(fields[0], path)?);
        for (i, column) in columns.iter_mut().enumerate() {
            column.push(parse_number(fields[i * 2 + 1], path)?);
        }
    }
    SplPlane::new(freq, names.into_iter().zip(columns).collect())
}

fn parse_number(field: &str, path: &Path) -> Result<f64> {
    // Klippel exports use `,` as thousands separator (Python loader passes
    // thousands=","); strip it before parsing.
    field
        .trim()
        .replace(',', "")
        .parse::<f64>()
        .with_context(|| format!("{}: unparsable number {field:?}", path.display()))
}
