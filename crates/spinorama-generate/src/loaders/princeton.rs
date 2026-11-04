//! Princeton 3D3A MATLAB IR loader (`princeton`).
//!
//! Port of `loaders/princeton.py`: each version directory holds
//! `<name>_H_IR.mat` and `<name>_V_IR.mat` (MATLAB v4 files) with
//! `fs_H`/`fs_V` sample rates and `IR_H`/`IR_V` impulse-response
//! matrices (72 rows at 5° steps). The frequency response follows the
//! Python convention exactly: unnormalized forward DFT of the raw IR
//! (rectangular window, no 1/N or one-sided scaling), magnitude in
//! `105 + 20*log10(|Y|)` dB, first 3414 bins, every 10° kept, then the
//! `resample(..., 200)` row stride above 500 Hz.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::loaders::text::list_files;
use crate::measurements::{HvSpl, SplPlane};

/// Number of spectrum bins kept by the Python loader (up to ~20 kHz).
const SPECTRUM_BINS: usize = 3414;
/// Rows in the Princeton IR matrices (5° steps over 360°).
const IR_ROWS: usize = 72;
/// Reference level of the paper formula, in dB.
const DB_REFERENCE: f64 = 105.0;

/// One full numeric matrix from a MATLAB v4 file (column-major).
#[derive(Debug, Clone, PartialEq)]
struct MatMatrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl MatMatrix {
    fn get(&self, row: usize, col: usize) -> f64 {
        self.data[col * self.rows + row]
    }
}

/// Loads a `princeton` version directory.
///
/// # Errors
///
/// Returns an error when the H/V IR files or variables are missing.
pub fn load(version_dir: &Path) -> Result<HvSpl> {
    let (mut h_file, mut v_file): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    for path in list_files(version_dir)? {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.ends_with("_H_IR.mat") {
            h_file = Some(path);
        } else if name.ends_with("_V_IR.mat") {
            v_file = Some(path);
        }
    }
    let (Some(h_path), Some(v_path)) = (h_file, v_file) else {
        bail!(
            "Princeton H/V IR files not found in {}",
            version_dir.display()
        );
    };

    let (horizontal, full_on_axis) = load_plane(&h_path, "H", None)?;
    let (vertical, _) = load_plane(&v_path, "V", full_on_axis.as_deref())?;
    Ok(HvSpl {
        horizontal,
        vertical,
    })
}

fn load_plane(
    path: &Path,
    suffix: &str,
    on_axis_fallback: Option<&[f64]>,
) -> Result<(SplPlane, Option<Vec<f64>>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let variables = parse_mat_v4(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let fs = variables
        .get(&format!("fs_{suffix}"))
        .and_then(|mat| mat.data.first().copied())
        .ok_or_else(|| anyhow::anyhow!("{}: missing fs_{suffix}", path.display()))?;
    let ir = variables
        .get(&format!("IR_{suffix}"))
        .ok_or_else(|| anyhow::anyhow!("{}: missing IR_{suffix}", path.display()))?;
    if ir.rows != IR_ROWS || ir.cols == 0 {
        bail!(
            "{}: IR_{suffix} has shape {}x{}, expected {IR_ROWS}xN",
            path.display(),
            ir.rows,
            ir.cols
        );
    }

    let n = ir.cols;
    let mut planner = FftPlanner::<f64>::new();
    let fft: Arc<dyn Fft<f64>> = planner.plan_fft_forward(n);
    #[expect(clippy::cast_precision_loss, reason = "FFT length fits exactly in f64")]
    let freq: Vec<f64> = (0..SPECTRUM_BINS.min(n))
        .map(|k| k as f64 * fs / n as f64)
        .collect();

    let mut columns: Vec<(String, Vec<f64>)> = Vec::new();
    for row in 0..IR_ROWS {
        #[expect(clippy::cast_possible_wrap, reason = "row index is below 72")]
        let mut angle = (row as i32) * 5;
        if angle > 180 {
            angle -= 360;
        }
        if angle % 10 != 0 {
            continue;
        }
        let mut buffer: Vec<Complex<f64>> = (0..n)
            .map(|col| Complex::new(ir.get(row, col), 0.0))
            .collect();
        fft.process(&mut buffer);
        let magnitudes: Vec<f64> = buffer[..freq.len()]
            .iter()
            .map(|value| value.norm())
            .collect();
        if magnitudes.iter().copied().fold(0.0_f64, f64::max) == 0.0 {
            continue; // 0 means not measured
        }
        let spl: Vec<f64> = magnitudes
            .iter()
            .map(|mag| DB_REFERENCE + 20.0 * mag.log10())
            .collect();
        let label = if angle == 0 {
            "On Axis".to_owned()
        } else {
            format!("{angle}°")
        };
        columns.push((label, spl));
    }

    let full_on_axis = columns
        .iter()
        .find(|(name, _)| name == "On Axis")
        .map(|(_, values)| values.clone());
    if full_on_axis.is_none() {
        match (suffix, on_axis_fallback) {
            ("V", Some(on_axis)) if on_axis.len() == freq.len() => {
                columns.push(("On Axis".to_owned(), on_axis.to_vec()));
            }
            _ => bail!("{}: On Axis not measured", path.display()),
        }
    }

    // Python: keep Freq >= 500, then resample(df, 200) row stride.
    let keep: Vec<usize> = freq
        .iter()
        .enumerate()
        .filter(|(_, f)| **f >= 500.0)
        .map(|(index, _)| index)
        .collect();
    let stride = if keep.len() > 400 {
        keep.len() / 200
    } else {
        1
    };
    let sampled: Vec<usize> = keep
        .iter()
        .copied()
        .skip(1)
        .step_by(stride.max(1))
        .collect();
    let sampled = if stride <= 1 { keep } else { sampled };
    let out_freq: Vec<f64> = sampled.iter().map(|index| freq[*index]).collect();
    let out_columns: Vec<(String, Vec<f64>)> = columns
        .into_iter()
        .map(|(name, values)| (name, sampled.iter().map(|index| values[*index]).collect()))
        .collect();
    let mut plane = SplPlane::new(out_freq, out_columns)?;
    plane.sort_by_angle();
    Ok((plane, full_on_axis))
}

/// Parses MATLAB v4 full numeric matrices into f64 column-major data.
fn parse_mat_v4(bytes: &[u8]) -> Result<BTreeMap<String, MatMatrix>> {
    let mut out = BTreeMap::new();
    let mut offset = 0usize;
    while offset + 20 <= bytes.len() {
        // Header: type, mrows, ncols, imagf, namelen (all i32).
        // The type's thousands digit is the endianness (0=LE, 1=BE);
        // peek big-endian first, as shipped Princeton files use it.
        let type_be = i32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]));
        let big_endian = (0..=1999).contains(&type_be) && type_be >= 1000;
        let read_i32 = |at: usize| -> Result<i32> {
            let raw: [u8; 4] = bytes
                .get(at..at + 4)
                .ok_or_else(|| anyhow::anyhow!("truncated MAT header"))?
                .try_into()
                .unwrap_or([0; 4]);
            Ok(if big_endian {
                i32::from_be_bytes(raw)
            } else {
                i32::from_le_bytes(raw)
            })
        };
        let mat_type = read_i32(offset)?;
        let rows = read_i32(offset + 4)?;
        let cols = read_i32(offset + 8)?;
        let imagf = read_i32(offset + 12)?;
        let name_len = read_i32(offset + 16)?;
        if rows < 0 || cols < 0 || name_len <= 0 {
            break;
        }
        let precision = (mat_type / 10) % 10;
        let elem_size: usize = match precision {
            0 => 8, // f64
            1 => 4, // f32
            2 => 4, // i32
            3 => 2, // i16
            4 => 2, // u16
            5 => 1, // u8
            other => bail!("unsupported MAT v4 precision code {other}"),
        };
        let name_start = offset + 20;
        #[expect(clippy::cast_sign_loss, reason = "name_len checked positive above")]
        let name_end = name_start + name_len as usize;
        let name_bytes = bytes
            .get(name_start..name_end)
            .ok_or_else(|| anyhow::anyhow!("truncated MAT variable name"))?;
        let name = String::from_utf8_lossy(name_bytes)
            .trim_end_matches('\0')
            .to_owned();
        #[expect(
            clippy::cast_sign_loss,
            reason = "rows/cols checked non-negative above"
        )]
        let count = (rows as usize) * (cols as usize);
        let data_start = name_end;
        let data_end = data_start + count * elem_size * if imagf != 0 { 2 } else { 1 };
        let raw = bytes
            .get(data_start..data_start + count * elem_size)
            .ok_or_else(|| anyhow::anyhow!("truncated MAT data for {name}"))?;
        let mut data = Vec::with_capacity(count);
        for chunk in raw.chunks_exact(elem_size) {
            let value = match precision {
                0 => {
                    let arr: [u8; 8] = chunk.try_into().unwrap_or([0; 8]);
                    if big_endian {
                        f64::from_be_bytes(arr)
                    } else {
                        f64::from_le_bytes(arr)
                    }
                }
                1 => {
                    let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
                    f64::from(if big_endian {
                        f32::from_be_bytes(arr)
                    } else {
                        f32::from_le_bytes(arr)
                    })
                }
                2 => {
                    let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
                    f64::from(if big_endian {
                        i32::from_be_bytes(arr)
                    } else {
                        i32::from_le_bytes(arr)
                    })
                }
                3 => {
                    let arr: [u8; 2] = chunk.try_into().unwrap_or([0; 2]);
                    f64::from(if big_endian {
                        i16::from_be_bytes(arr)
                    } else {
                        i16::from_le_bytes(arr)
                    })
                }
                4 => {
                    let arr: [u8; 2] = chunk.try_into().unwrap_or([0; 2]);
                    f64::from(if big_endian {
                        u16::from_be_bytes(arr)
                    } else {
                        u16::from_le_bytes(arr)
                    })
                }
                _ => f64::from(chunk[0]),
            };
            data.push(value);
        }
        #[expect(
            clippy::cast_sign_loss,
            reason = "rows/cols checked non-negative above"
        )]
        out.insert(
            name,
            MatMatrix {
                rows: rows as usize,
                cols: cols as usize,
                data,
            },
        );
        offset = data_end;
    }
    if out.is_empty() {
        bail!("no MATLAB v4 variables found");
    }
    Ok(out)
}
