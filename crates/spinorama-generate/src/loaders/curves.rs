//! Curve-set loaders returning a partial CEA2034 measurement.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::compute::cea2034::{Cea2034, cea2034_from_curves};
use crate::loaders::text::read_text_curve;

/// Named raw curves: name -> (freq, spl), before CEA2034 assembly.
pub type NamedCurves = BTreeMap<String, (Vec<f64>, Vec<f64>)>;

/// Loads the raw `rew_text_dump` curves without CEA2034 assembly.
///
/// # Errors
///
/// Returns an error when a mandatory curve file is missing or unreadable.
pub fn load_rew_curves(version_dir: &Path) -> Result<NamedCurves> {
    let files = [
        ("On Axis.txt", "On Axis", true),
        ("ER.txt", "Early Reflections", true),
        ("LW.txt", "Listening Window", true),
        ("SP.txt", "Sound Power", true),
        ("PIR.txt", "Estimated In-Room Response", false),
    ];
    let mut curves = NamedCurves::new();
    for (file, name, mandatory) in files {
        let path = version_dir.join(file);
        if !path.is_file() {
            if mandatory {
                bail!("mandatory REW curve file missing: {}", path.display());
            }
            continue;
        }
        let curve = read_text_curve(&path, 10.0, 20_000.0, 0)?;
        curves.insert(name.to_owned(), (curve.freq, curve.spl));
    }
    Ok(curves)
}

/// Loads the raw `webplotdigitizer` curves without CEA2034 assembly.
///
/// # Errors
///
/// Returns an error when no WebPlotDigitizer JSON payload is found.
pub fn load_webplot_curves(version_dir: &Path, speaker: &str) -> Result<NamedCurves> {
    let json_text = find_webplot_json(version_dir, speaker)?;
    parse_webplot_json(&json_text)
}

/// Loads a `rew_text_dump` version directory.
///
/// # Errors
///
/// Returns an error when a mandatory curve file is missing or unreadable.
pub fn load_rew_text_dump(version_dir: &Path) -> Result<Cea2034> {
    cea2034_from_curves(&load_rew_curves(version_dir)?)
}

/// Loads a `webplotdigitizer` version directory.
///
/// # Errors
///
/// Returns an error when no WebPlotDigitizer JSON payload is found.
pub fn load_webplotdigitizer(version_dir: &Path, speaker: &str) -> Result<Cea2034> {
    cea2034_from_curves(&load_webplot_curves(version_dir, speaker)?)
}

fn find_webplot_json(version_dir: &Path, speaker: &str) -> Result<String> {
    let direct = version_dir.join(format!("{speaker}.json"));
    if direct.is_file() {
        return std::fs::read_to_string(&direct)
            .with_context(|| format!("reading {}", direct.display()));
    }
    let tar_path = version_dir.join(format!("{speaker}.tar"));
    if tar_path.is_file()
        && let Some(text) = read_webplot_tar(&tar_path)?
    {
        return Ok(text);
    }
    let mut candidates = Vec::new();
    collect_json_files(version_dir, 4, &mut candidates)?;
    for path in candidates {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if text.contains("datasetColl") {
            return Ok(text);
        }
    }
    bail!(
        "no WebPlotDigitizer JSON found in {}",
        version_dir.display()
    )
}

fn collect_json_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth == 0 || !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_json_files(&path, depth - 1, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

fn read_webplot_tar(path: &Path) -> Result<Option<String>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
        read_tar_entries(decoder, &mut entries)?;
    } else {
        read_tar_entries(Cursor::new(bytes), &mut entries)?;
    }

    let mut target: Option<String> = None;
    for (name, data) in &entries {
        if name.ends_with("info.json") {
            let info: Value = serde_json::from_slice(data)
                .with_context(|| format!("parsing tar entry {name}"))?;
            if let Some(json_name) = info["json"].as_str() {
                let prefix = name.strip_suffix("info.json").unwrap_or_default();
                target = Some(format!("{prefix}{json_name}"));
            }
        }
    }
    if let Some(target) = target {
        if let Some(data) = entries.get(&target) {
            return Ok(Some(String::from_utf8_lossy(data).into_owned()));
        }
        let suffix = target.rsplit('/').next().unwrap_or(&target).to_owned();
        for (name, data) in &entries {
            if name.ends_with(&suffix) {
                return Ok(Some(String::from_utf8_lossy(data).into_owned()));
            }
        }
    }
    for (name, data) in &entries {
        if name.ends_with(".json") {
            let text = String::from_utf8_lossy(data).into_owned();
            if text.contains("datasetColl") {
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}

fn read_tar_entries<R: Read>(reader: R, entries: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.insert(name, data);
    }
    Ok(())
}

fn pretty_curve_name(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.as_str() {
        "on axis" | "on-axis" | "oa" | "onaxis" | "on" => "On Axis".to_owned(),
        "listening window" | "lw" => "Listening Window".to_owned(),
        "early reflections" | "early reflection" | "early reflexion" | "first reflections"
        | "first reflection" | "first reflexion" | "er" => "Early Reflections".to_owned(),
        "sound power" | "sp" => "Sound Power".to_owned(),
        _ => name.to_owned(),
    }
}

fn parse_webplot_json(text: &str) -> Result<NamedCurves> {
    let root: Value = serde_json::from_str(text).context("parsing WebPlotDigitizer JSON")?;
    let datasets = root["datasetColl"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("WebPlotDigitizer JSON has no datasetColl array"))?;

    // Python reference grid: logspace(1+log10(2), 4+log10(2), 1000).
    let ref_freq: Vec<f64> = (0..1000)
        .map(|i| {
            let exponent = 1.0 + 2f64.log10() + (3.0 * i as f64) / 999.0;
            10f64.powf(exponent)
        })
        .collect();

    let mut grouped: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    for dataset in datasets {
        let Some(raw_name) = dataset["name"].as_str() else {
            continue;
        };
        let name = pretty_curve_name(raw_name);
        let Some(data) = dataset["data"].as_array() else {
            continue;
        };
        let mut points: Vec<(f64, f64)> = data
            .iter()
            .filter_map(|point| {
                let values = point["value"].as_array()?;
                Some((values.first()?.as_f64()?, values.get(1)?.as_f64()?))
            })
            .collect();
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if points.len() < 2 {
            continue;
        }
        let mut ref_p = 0usize;
        let out = grouped.entry(name).or_default();
        for segment in points.windows(2) {
            let (fr, db) = segment[0];
            let (frn, dbn) = segment[1];
            if fr == frn {
                continue;
            }
            while ref_freq[ref_p] <= fr {
                if ref_p >= ref_freq.len() - 1 {
                    break;
                }
                ref_p += 1;
            }
            let ref_f = ref_freq[ref_p];
            if ref_f > frn {
                continue;
            }
            let ref_db = db + (dbn - db) * (ref_f - fr) / (frn - fr);
            if ref_f > 0.0 && ref_f <= 20_000.0 && ref_db > -50.0 && ref_db < 200.0 {
                out.push((ref_f, ref_db));
            } else {
                break;
            }
        }
    }

    expend_low_frequencies(&mut grouped);
    let mut curves = NamedCurves::new();
    for (name, mut points) in grouped {
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        points.dedup_by(|next, keep| next.0 == keep.0);
        if points.len() >= 2 {
            curves.insert(
                name,
                (
                    points.iter().map(|point| point.0).collect(),
                    points.iter().map(|point| point.1).collect(),
                ),
            );
        }
    }
    Ok(curves)
}

fn expend_low_frequencies(grouped: &mut BTreeMap<String, Vec<(f64, f64)>>) {
    let mins: BTreeMap<String, f64> = grouped
        .iter()
        .filter_map(|(name, points)| {
            points
                .iter()
                .map(|point| point.0)
                .fold(None::<f64>, |acc, value| {
                    Some(acc.map_or(value, |current| current.min(value)))
                })
                .map(|min| (name.clone(), min))
        })
        .collect();
    let Some((min_measurement, _)) = mins
        .iter()
        .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
    else {
        return;
    };
    let min_measurement = min_measurement.clone();
    let source = grouped.get(&min_measurement).cloned().unwrap_or_default();
    for (name, min_freq) in mins {
        if name == min_measurement {
            continue;
        }
        if let Some(points) = grouped.get_mut(&name) {
            points.extend(source.iter().filter(|point| point.0 < min_freq).copied());
        }
    }
}

/// Loads a headphone `csv_freq_spl` version directory.
///
/// Finds `frequency_response.csv` (or `freq_response.csv` / `fr.csv`)
/// and parses it with the Python `load_headphone_csv` rules, including
/// L+R pressure averaging for 4-column ASR exports.
///
/// # Errors
///
/// Returns an error when no CSV file or no usable rows are found.
pub fn load_csv_freq_spl(version_dir: &Path) -> Result<(Vec<f64>, Vec<f64>)> {
    for candidate in ["frequency_response.csv", "freq_response.csv", "fr.csv"] {
        let path = version_dir.join(candidate);
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            return parse_headphone_csv(&text)
                .with_context(|| format!("parsing {}", path.display()));
        }
    }
    bail!("no frequency_response.csv in {}", version_dir.display())
}

/// Parses one headphone target/measurement CSV text into freq/SPL.
///
/// # Errors
///
/// Returns an error when no header or numeric data is recognisable.
pub fn parse_headphone_csv(text: &str) -> Result<(Vec<f64>, Vec<f64>)> {
    let lines: Vec<&str> = text.lines().collect();
    let fields = |line: &str| -> Vec<String> {
        line.split(',')
            .map(|part| part.trim().trim_matches('"').to_lowercase())
            .collect()
    };
    let strict_freq = ["frequency", "freq", "hz"];
    let strict_db = ["raw", "spl", "db", "dbspl", "level"];
    let mut candidates: Vec<(usize, Vec<String>)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let cols = fields(line);
        if cols.iter().any(|c| strict_freq.contains(&c.as_str()))
            && cols.iter().any(|c| strict_db.contains(&c.as_str()))
        {
            candidates.push((index, cols));
        }
    }
    let is_freq_col = |col: &str| col.starts_with("hz") || col.starts_with("freq");
    let is_db_col =
        |col: &str| col.starts_with("dbspl") || col.starts_with("spl") || col.starts_with("db");
    let header = candidates
        .iter()
        .find(|(_, cols)| {
            cols.iter().filter(|c| is_freq_col(c)).count() >= 2
                && cols.iter().filter(|c| is_db_col(c)).count() >= 2
        })
        .or_else(|| candidates.first());

    let numeric_row = |line: &str| -> Vec<f64> {
        line.split(',')
            .map(|part| {
                part.trim()
                    .trim_matches('"')
                    .parse::<f64>()
                    .unwrap_or(f64::NAN)
            })
            .collect()
    };

    if let Some((header_idx, cols)) = header {
        let freq_cols: Vec<usize> = cols
            .iter()
            .enumerate()
            .filter(|(_, c)| is_freq_col(c))
            .map(|(i, _)| i)
            .collect();
        let db_cols: Vec<usize> = cols
            .iter()
            .enumerate()
            .filter(|(_, c)| is_db_col(c))
            .map(|(i, _)| i)
            .collect();
        let rows: Vec<Vec<f64>> = lines[header_idx + 1..]
            .iter()
            .map(|line| numeric_row(line))
            .filter(|row| row.iter().any(|v| v.is_finite()))
            .collect();
        if cols.len() >= 4 && freq_cols.len() >= 2 && db_cols.len() >= 2 {
            let mut freq = Vec::new();
            let mut spl = Vec::new();
            for row in &rows {
                let (Some(&fl), Some(&dl), Some(&fr), Some(&dr)) = (
                    row.get(freq_cols[0]),
                    row.get(db_cols[0]),
                    row.get(freq_cols[1]),
                    row.get(db_cols[1]),
                ) else {
                    continue;
                };
                if !(fl.is_finite() && dl.is_finite() && fr.is_finite() && dr.is_finite()) {
                    continue;
                }
                if let Some(&last) = freq.last()
                    && fl < last
                {
                    break; // repeated REW section: keep first contiguous sweep
                }
                let p_avg =
                    (10f64.powf((dl - 105.0) / 20.0) + 10f64.powf((dr - 105.0) / 20.0)) / 2.0;
                freq.push(fl);
                spl.push(20.0 * p_avg.log10() + 105.0);
            }
            if freq.len() >= 2 {
                return Ok((freq, spl));
            }
        }
        let freq_col = ["frequency", "freq", "hz", "x"]
            .iter()
            .find_map(|kw| cols.iter().position(|c| c == kw));
        let db_col = ["raw", "spl", "db", "dbspl", "level", "y"]
            .iter()
            .find_map(|kw| cols.iter().position(|c| c == kw));
        if let (Some(fc), Some(dc)) = (freq_col, db_col) {
            let mut freq = Vec::new();
            let mut spl = Vec::new();
            for row in &rows {
                if let (Some(&f), Some(&db)) = (row.get(fc), row.get(dc))
                    && f.is_finite()
                    && db.is_finite()
                {
                    freq.push(f);
                    spl.push(db);
                }
            }
            if freq.len() >= 2 {
                return Ok((freq, spl));
            }
        }
        bail!("could not identify freq/dB columns");
    }

    // Headerless: first row whose first two fields are numeric.
    let start = lines
        .iter()
        .position(|line| {
            let row = numeric_row(line);
            row.len() >= 2 && row[0].is_finite() && row[1].is_finite()
        })
        .ok_or_else(|| anyhow::anyhow!("no recognisable header or numeric data"))?;
    let mut freq = Vec::new();
    let mut spl = Vec::new();
    for line in &lines[start..] {
        let row = numeric_row(line);
        if row.len() >= 2 && row[0].is_finite() && row[1].is_finite() {
            freq.push(row[0]);
            spl.push(row[1]);
        }
    }
    if freq.len() < 2 {
        bail!("only {} usable headphone CSV points", freq.len());
    }
    Ok((freq, spl))
}
