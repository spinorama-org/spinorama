//! GLL meridian/parallel text loader (`gll_hv_txt`).

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::loaders::text::{TextCurve, list_files, parse_text_curve, plane_from_curves};
use crate::measurements::HvSpl;

/// Loads a `gll_hv_txt` version directory for `speaker`.
///
/// Data may come from a zip archive in the directory or from an
/// already extracted `tmp/` tree. Like Python, the archive named
/// `<speaker>.zip` wins when several are present. Unlike the Python
/// loader, this loader reads zip entries in memory and does not
/// write extracted files into the source measurement tree.
///
/// # Errors
///
/// Returns an error when no usable GLL text files are found.
pub fn load(version_dir: &Path, speaker: &str) -> Result<HvSpl> {
    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    for (name, text) in read_zip_entries(version_dir, speaker)? {
        entries.entry(name).or_insert(text);
    }
    for (name, text) in read_extracted_entries(version_dir)? {
        entries.entry(name).or_insert(text);
    }
    if entries.is_empty() {
        bail!("no GLL text files in {}", version_dir.display());
    }

    let mut horizontal: BTreeMap<String, TextCurve> = BTreeMap::new();
    let mut vertical: BTreeMap<String, TextCurve> = BTreeMap::new();
    for (name, text) in entries {
        let Some((orientation, angle)) = parse_gll_name(&name) else {
            continue;
        };
        if angle == -180 {
            continue;
        }
        let mut curve = parse_text_curve(&text, 20.0, 20_000.0, 6)
            .with_context(|| format!("parsing GLL entry {name}"))?;
        filter_strict_band(&mut curve);
        if curve.freq.len() < 2 {
            continue;
        }
        let column = if angle == 0 {
            "On Axis".to_owned()
        } else {
            format!("{angle}°")
        };
        let target = if orientation == "H" {
            &mut horizontal
        } else {
            &mut vertical
        };
        target.entry(column).or_insert(curve);
    }
    if horizontal.is_empty() || vertical.is_empty() {
        bail!(
            "GLL data in {} produced {} horizontal and {} vertical curves",
            version_dir.display(),
            horizontal.len(),
            vertical.len()
        );
    }
    Ok(HvSpl {
        horizontal: plane_from_curves(horizontal.into_iter().collect())?,
        vertical: plane_from_curves(vertical.into_iter().collect())?,
    })
}

fn filter_strict_band(curve: &mut TextCurve) {
    let mut freq = Vec::with_capacity(curve.freq.len());
    let mut spl = Vec::with_capacity(curve.spl.len());
    let mut phase = curve
        .phase
        .as_ref()
        .map(|values| Vec::with_capacity(values.len()));
    for (index, value) in curve.freq.iter().enumerate() {
        if *value > 20.0 && *value < 20_000.0 {
            freq.push(*value);
            spl.push(curve.spl[index]);
            if let (Some(out), Some(values)) = (&mut phase, &curve.phase) {
                out.push(values[index]);
            }
        }
    }
    curve.freq = freq;
    curve.spl = spl;
    curve.phase = phase;
}

fn parse_gll_name(name: &str) -> Option<(&'static str, i32)> {
    let base = name.rsplit(['/', '\\']).next()?.strip_suffix(".txt")?;
    let parts: Vec<&str> = base.split('-').collect();
    if parts.len() < 2 {
        return None;
    }
    let meridian: i32 = parts[parts.len() - 2].get(1..)?.parse().ok()?;
    let parallel: i32 = parts[parts.len() - 1].get(1..)?.parse().ok()?;
    match meridian {
        0 => Some(("H", parallel)),
        90 => Some(("V", parallel)),
        180 => Some(("H", -parallel)),
        270 => Some(("V", -parallel)),
        _ => None,
    }
}

fn read_zip_entries(dir: &Path, speaker: &str) -> Result<Vec<(String, String)>> {
    let zips: Vec<PathBuf> = list_files(dir)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "zip"))
        .collect();
    // Python prefers `<speaker>.zip`; otherwise a single zip is used
    // and several are an error.
    let preferred = dir.join(format!("{}.zip", crate::meta::sanitize_filename(speaker)));
    let zip_path = if preferred.is_file() {
        Some(preferred)
    } else if zips.len() == 1 {
        zips.into_iter().next()
    } else if zips.len() > 1 {
        bail!("multiple zip files in {}", dir.display());
    } else {
        None
    };
    let Some(zip_path) = zip_path else {
        return Ok(Vec::new());
    };
    let file = std::fs::File::open(&zip_path)
        .with_context(|| format!("reading {}", zip_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if !name.ends_with(".txt") {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        out.push((name, text));
    }
    Ok(out)
}

fn read_extracted_entries(dir: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    collect_text_files(&dir.join("tmp"), 3, &mut out)?;
    Ok(out)
}

fn collect_text_files(dir: &Path, depth: usize, out: &mut Vec<(String, String)>) -> Result<()> {
    if depth == 0 || !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_text_files(&path, depth - 1, out)?;
        } else if path.extension().is_some_and(|ext| ext == "txt") {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            out.push((name, text));
        }
    }
    Ok(())
}
