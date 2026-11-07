//! Figure emission: Plotly JSON payloads, v2 trace bundles, PNG driving.

// Rust guideline compliant 2026-02-21

pub mod plotly_json;
pub mod trace_bundle;
pub mod webp;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::Digest as _;

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

/// Writes one file when its content differs from what is on disk.
///
/// Returns whether the file was (re)written. Unchanged files keep
/// their mtime, so downstream freshness checks stay cheap.
fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == content
    {
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// SHA-256 hex digest of a file, when it can be read.
fn file_digest(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// Returns whether `spec` matches `path`'s renderer-version file
/// (`.renderer.version` next to the payloads).
#[must_use]
pub fn renderer_current(root: &Path, spec: &str) -> bool {
    std::fs::read_to_string(root.join(".renderer.version")).is_ok_and(|text| text.trim() == spec)
}

/// Records the renderer-version spec that produced `root`'s images.
///
/// # Errors
///
/// Returns an error when the stamp file cannot be written.
pub fn write_renderer_stamp(root: &Path, spec: &str) -> Result<()> {
    std::fs::write(root.join(".renderer.version"), format!("{spec}\n"))
        .with_context(|| format!("writing renderer stamp in {}", root.display()))
}

/// Computes the renderer-version spec for image freshness checks.
///
/// The default renderer binary is identified by its content digest,
/// so rebuilding `spinorama-render-images` (colormaps, layout,
/// legends, new chart kinds) invalidates previously rendered images
/// even when every payload is unchanged. Explicitly chosen binaries
/// use their path: tests and benches share one checkout.
///
/// # Errors
///
/// Returns an error when the default renderer binary cannot be read
/// to compute its digest.
pub fn renderer_spec(bin: &Path) -> Result<String> {
    if bin == Path::new(DEFAULT_RENDER_IMAGES_BIN) {
        let digest = file_digest(bin)
            .with_context(|| format!("hashing renderer binary {}", bin.display()))?;
        Ok(format!("sha256:{digest}"))
    } else {
        Ok(format!("bin:{}", bin.display()))
    }
}

/// Writes one figure as `<dir>/<name>.json` and, for scatter figures,
/// `<dir>/traces/<name>.json`.
///
/// Contour and radar figures have no v2 trace bundle, matching Python
/// `extract_trace_bundle`, which only encodes scatter traces.
///
/// Files whose serialized content is unchanged are left untouched;
/// the returned flag reports whether anything was written, so callers
/// can tell a code change (new payload bytes) from a true re-run and
/// only then pay for re-rendering.
///
/// # Errors
///
/// Returns an error when the directories cannot be created or a payload
/// cannot be encoded or written.
pub fn write_figure(dir: &Path, figure: &Figure) -> Result<(PathBuf, bool)> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let json_path = dir.join(format!("{}.json", figure.name));
    let mut changed = write_if_changed(&json_path, &plotly_json::to_json_string(figure)?)?;
    if figure.kind == FigureKind::Scatter {
        std::fs::create_dir_all(dir.join("traces"))
            .with_context(|| format!("creating {}", dir.display()))?;
        let bundle_path = dir.join("traces").join(format!("{}.json", figure.name));
        changed |= write_if_changed(&bundle_path, &trace_bundle::to_json_string(figure)?)?;
    }
    Ok((json_path, changed))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figures::{Figure, Trace};

    fn figure(trace_name: &str) -> Figure {
        Figure::scatter(
            "On Axis",
            "title".to_owned(),
            vec![100.0, 1_000.0],
            vec![Trace::new(
                trace_name,
                vec![1.0, 2.0],
                crate::figures::Axis::Primary,
            )],
        )
    }

    #[test]
    fn write_figure_reports_changes_by_content() {
        let dir = std::env::temp_dir().join(format!("write-figure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (_, first) = write_figure(&dir, &figure("On Axis")).expect("first write");
        assert!(first, "missing file must count as changed");
        let (_, second) = write_figure(&dir, &figure("On Axis")).expect("identical rewrite");
        assert!(!second, "identical content must not count as changed");
        let (_, third) = write_figure(&dir, &figure("Listening Window")).expect("changed write");
        assert!(third, "different content must count as changed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
