//! Figure emission: Plotly JSON payloads, v2 trace bundles, PNG driving.

// Rust guideline compliant 2026-02-21

pub mod plotly_json;
pub mod trace_bundle;
pub mod webp;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::figures::{Figure, FigureKind};

/// Default location of the gpui-toolkit batch renderer binary.
///
/// `gpui-spinorama` exposes no library boundary (no `lib.rs`,
/// `publish = false`), so the existing `spinorama-render-images` binary
/// is the cleanest rendering boundary available; this constant matches
/// the path used by `scripts/render_images_gpui.py`. Override with
/// `--render-images-bin` or the `SPINORAMA_RENDER_IMAGES` env var.
pub const DEFAULT_RENDER_IMAGES_BIN: &str =
    "/Volumes/home_tmp/src/gpui-toolkit/target/release/spinorama-render-images";

/// Writes one figure as `<dir>/<name>.json` and, for scatter figures,
/// `<dir>/traces/<name>.json`.
///
/// Contour and radar figures have no v2 trace bundle, matching Python
/// `extract_trace_bundle`, which only encodes scatter traces.
///
/// # Errors
///
/// Returns an error when the directories cannot be created or a payload
/// cannot be encoded or written.
pub fn write_figure(dir: &Path, figure: &Figure) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let json_path = dir.join(format!("{}.json", figure.name));
    std::fs::write(&json_path, plotly_json::to_json_string(figure)?)
        .with_context(|| format!("writing {}", json_path.display()))?;
    if figure.kind == FigureKind::Scatter {
        std::fs::create_dir_all(dir.join("traces"))
            .with_context(|| format!("creating {}", dir.display()))?;
        let bundle_path = dir.join("traces").join(format!("{}.json", figure.name));
        std::fs::write(&bundle_path, trace_bundle::to_json_string(figure)?)
            .with_context(|| format!("writing {}", bundle_path.display()))?;
    }
    Ok(json_path)
}

/// Renders every Plotly JSON under `private_root` to PNGs under `out_root`.
///
/// Drives the existing Rust `spinorama-render-images` binary from
/// gpui-toolkit — the only subprocess this crate spawns, and never a
/// Python process.
///
/// # Errors
///
/// Returns an error when the binary is missing or exits non-zero.
pub fn render_pngs(bin: &Path, private_root: &Path, out_root: &Path) -> Result<()> {
    if !bin.is_file() {
        bail!("render-images binary not found: {}", bin.display());
    }
    let status = Command::new(bin)
        .arg(private_root)
        .arg(out_root)
        .status()
        .with_context(|| format!("spawning {}", bin.display()))?;
    if !status.success() {
        bail!("{} exited with {status}", bin.display());
    }
    Ok(())
}
