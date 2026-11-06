//! Spinorama graph and PEQ generation in Rust.
//!
//! This crate replaces the Python `scripts/generate_graphs.py` and
//! `scripts/generate_peqs.py` paths in slices: measurement loaders,
//! CEA2034 computation, figure payloads, and PEQ optimization on Rust
//! AutoEQ. The comparable output contract is the v2 trace bundle
//! (1024-point log grid, 0.01 dB quantization), not Plotly JSON byte
//! equality.

// Rust guideline compliant 2026-02-21

pub mod compute;
pub mod figures;
pub mod filters;
pub mod generate_meta;
pub mod headphone_meta;
pub mod loaders;
pub mod measurements;
pub mod meta;
pub mod metadata;
pub mod peq;
pub mod render;
