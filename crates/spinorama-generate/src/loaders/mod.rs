//! Measurement format loaders.

// Rust guideline compliant 2026-02-21

pub mod curves;
pub mod gll_hv_txt;
pub mod klippel;
pub mod princeton;
pub mod spl_hv_txt;
pub mod text;

use std::path::Path;

use anyhow::{Result, bail};

use crate::compute::cea2034::Cea2034;
use crate::measurements::HvSpl;

/// One loaded measurement version, before figure computation.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedMeasurement {
    /// Horizontal/vertical SPL planes from an HV loader.
    Hv(HvSpl),
    /// Already unified CEA2034 curves from a curve loader.
    Cea(Cea2034),
    /// Single frequency-response curve (headphone `csv_freq_spl`).
    Curve {
        /// Frequency axis in hertz.
        freq: Vec<f64>,
        /// SPL values in dB aligned with `freq`.
        spl: Vec<f64>,
    },
}

/// Loads one HV measurement version directory in the named format.
///
/// # Errors
///
/// Returns an error for unknown formats, curve-only formats, and known
/// formats that are not ported yet. There is no fallback loader.
pub fn load_hv(format: &str, version_dir: &Path, speaker: &str) -> Result<HvSpl> {
    match format {
        "klippel" => klippel::load(version_dir),
        "spl_hv_txt" => spl_hv_txt::load(version_dir),
        "gll_hv_txt" => gll_hv_txt::load(version_dir, speaker),
        "princeton" => princeton::load(version_dir),
        "rew_text_dump" | "webplotdigitizer" | "csv_freq_spl" => {
            bail!("measurement format {format} is a curve loader, not an HV loader")
        }
        other => bail!("unknown measurement format: {other}"),
    }
}

/// Loads one measurement version in the named format.
///
/// # Errors
///
/// Returns an error for unknown or not-yet-ported formats, with no fallback.
pub fn load_measurement(
    format: &str,
    version_dir: &Path,
    speaker: &str,
) -> Result<LoadedMeasurement> {
    match format {
        "klippel" | "spl_hv_txt" | "gll_hv_txt" | "princeton" => Ok(LoadedMeasurement::Hv(
            load_hv(format, version_dir, speaker)?,
        )),
        "rew_text_dump" => Ok(LoadedMeasurement::Cea(curves::load_rew_text_dump(
            version_dir,
        )?)),
        "webplotdigitizer" => Ok(LoadedMeasurement::Cea(curves::load_webplotdigitizer(
            version_dir,
            speaker,
        )?)),
        "csv_freq_spl" => {
            let (freq, spl) = curves::load_csv_freq_spl(version_dir)?;
            Ok(LoadedMeasurement::Curve { freq, spl })
        }
        other => bail!("unknown measurement format: {other}"),
    }
}
