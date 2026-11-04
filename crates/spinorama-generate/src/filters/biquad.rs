//! PEQ magnitude response from Rust AutoEQ biquads.

// Rust guideline compliant 2026-02-21

use autoeq_workflow::iir;
use ndarray::Array1;

pub use autoeq_workflow::iir::Peq;

/// Returns the combined PEQ magnitude response in dB at `freq`.
///
/// Thin adapter over `math-iir-fir` (`autoeq_workflow::iir::peq_spl`),
/// the same RBJ biquad math the optimizer uses internally.
///
/// # Examples
///
/// ```
/// use spinorama_generate::filters::biquad::{Peq, peq_response_db};
///
/// let empty: Peq = Vec::new();
/// let response = peq_response_db(&[20.0, 1_000.0, 20_000.0], &empty);
/// assert_eq!(response, vec![0.0, 0.0, 0.0]);
/// ```
#[must_use]
pub fn peq_response_db(freq: &[f64], peq: &Peq) -> Vec<f64> {
    let freq = Array1::from_vec(freq.to_vec());
    iir::peq_spl(&freq, peq).to_vec()
}

/// Returns `spl` shifted by the PEQ response (EQ applied, in dB).
///
/// # Examples
///
/// ```
/// use spinorama_generate::filters::biquad::{Peq, apply_peq};
///
/// let empty: Peq = Vec::new();
/// let corrected = apply_peq(&[1_000.0], &[80.0], &empty);
/// assert_eq!(corrected, vec![80.0]);
/// ```
#[must_use]
pub fn apply_peq(freq: &[f64], spl: &[f64], peq: &Peq) -> Vec<f64> {
    let response = peq_response_db(freq, peq);
    spl.iter().zip(response).map(|(s, r)| s + r).collect()
}
