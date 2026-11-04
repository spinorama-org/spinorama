//! Per-angle SPL text loader (`spl_hv_txt`).

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::loaders::text::{TextCurve, list_files, plane_from_curves, read_text_curve};
use crate::measurements::{HvSpl, SplPlane};

/// Loads an `spl_hv_txt` version directory.
///
/// # Errors
///
/// Returns an error when either plane has no readable angle files.
pub fn load(version_dir: &Path) -> Result<HvSpl> {
    Ok(HvSpl {
        horizontal: load_plane(version_dir, "H", None)?,
        vertical: load_plane(version_dir, "V", None)?,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn angle_string(path: &Path) -> String {
    let basename = file_name(path);
    let parts: Vec<&str> = basename.split_whitespace().collect();
    if parts.len() > 2 {
        let last = parts[parts.len() - 1];
        return last.strip_suffix(".txt").unwrap_or(last).to_owned();
    }
    basename.split('_').next().unwrap_or_default().to_owned()
}

fn matches_orientation(path: &Path, orientation: &str) -> bool {
    let name = file_name(path);
    if !name.ends_with(".txt") {
        return false;
    }
    let marker = format!("_{orientation}");
    if let Some(pos) = name.find(&marker) {
        let after = &name[pos + marker.len()..name.len() - 4];
        if after.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    name.ends_with(&format!("_{orientation}.txt")) && name.chars().any(|c| c.is_ascii_digit())
}

fn find_files(dir: &Path, orientation: &str) -> Result<Vec<PathBuf>> {
    Ok(list_files(dir)?
        .into_iter()
        .filter(|path| matches_orientation(path, orientation))
        .collect())
}

fn shared_angle(angle: &str) -> &str {
    if angle == "-180" { "180" } else { angle }
}

fn load_plane(dir: &Path, orientation: &str, symmetry_mode: Option<&str>) -> Result<SplPlane> {
    let mut files = find_files(dir, orientation)?;
    if files.is_empty() {
        bail!("no {orientation} SPL text files in {}", dir.display());
    }

    let (do_mirror, do_shift) = match symmetry_mode {
        Some("mirror") => (true, false),
        Some("shift") => (false, true),
        Some("none") => (false, false),
        Some(other) => bail!("unknown spl_hv_txt symmetry mode: {other}"),
        None => {
            let has_negative = files.iter().any(|file| {
                angle_string(file)
                    .parse::<i32>()
                    .is_ok_and(|angle| angle < 0)
            });
            (!has_negative, false)
        }
    };

    let present: std::collections::BTreeSet<String> = files
        .iter()
        .map(|file| shared_angle(&angle_string(file)).to_owned())
        .collect();
    let mut missing_shared: Vec<&str> = ["0", "180"]
        .into_iter()
        .filter(|angle| !present.contains(*angle))
        .collect();
    if !missing_shared.is_empty() {
        let other = if orientation == "H" { "V" } else { "H" };
        let mut other_files = find_files(dir, other).unwrap_or_default();
        other_files.sort_by_key(|file| angle_string(file) == "-180");
        for file in other_files {
            let angle = shared_angle(&angle_string(&file)).to_owned();
            if let Some(pos) = missing_shared.iter().position(|missing| *missing == angle) {
                files.push(file);
                missing_shared.remove(pos);
            }
        }
    }

    files.sort_by_key(|file| angle_string(file) == "-180");
    let mut curves: BTreeMap<String, TextCurve> = BTreeMap::new();
    for file in files {
        let angle_text = angle_string(&file);
        let angle: i32 = angle_text
            .parse()
            .with_context(|| format!("{}: cannot parse angle from file name", file.display()))?;
        let curve = read_text_curve(&file, 20.0, 20_000.0, 0)?;
        if do_shift && angle > 180 {
            curves.entry(format!("{}°", angle - 360)).or_insert(curve);
            continue;
        }
        let name = match angle {
            0 => "On Axis".to_owned(),
            -180 => "180°".to_owned(),
            _ => format!("{angle}°"),
        };
        if curves.contains_key(&name) {
            continue;
        }
        curves.insert(name.clone(), curve.clone());
        if do_mirror && orientation == "H" && angle > 0 && angle != 180 {
            curves.entry(format!("-{angle}°")).or_insert(curve);
        }
    }
    plane_from_curves(curves.into_iter().collect())
}
