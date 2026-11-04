//! Equalizer APO file parsing and spinorama site formatting.
//!
//! The comparable output contract is the APO text written by Python
//! `scripts/generate_peqs.py` to `datas/eq/<speaker>/iir-autoeq.txt`:
//! a four-line header, a `Preamp:` line, then one `Filter N:` line per
//! biquad. Filter lines and preamp come from Rust AutoEQ
//! (`iir::peq_format_apo`); the header reproduces the Python wording.

// Rust guideline compliant 2026-02-21

use anyhow::{Context, Result, bail};
use autoeq_workflow::iir::{self, Biquad, BiquadFilterType, Peq};

/// Maps an APO filter-type token to a biquad type.
///
/// Unknown tokens are a hard error: silently defaulting would change
/// the applied EQ curve.
fn filter_type(token: &str) -> Result<BiquadFilterType> {
    Ok(match token {
        "PK" | "MODAL" => BiquadFilterType::Peak,
        "LS" | "LSC" => BiquadFilterType::Lowshelf,
        "HS" | "HSC" => BiquadFilterType::Highshelf,
        "LP" | "LPQ" => BiquadFilterType::Lowpass,
        "HP" | "HPQ" => BiquadFilterType::Highpass,
        "NO" => BiquadFilterType::Notch,
        "BP" => BiquadFilterType::Bandpass,
        other => bail!("unknown APO filter type: {other}"),
    })
}

/// Parses an Equalizer APO file into `(preamp_db, peq)`.
///
/// Accepts both the Python spinorama files (free-form header lines, no
/// leading `#`) and Rust AutoEQ files (`#` comment header). Disabled
/// (`OFF`) filters are skipped. `sample_rate` stamps the created
/// biquads because APO files do not carry one.
///
/// # Examples
///
/// ```
/// use spinorama_generate::filters::apo::parse_apo;
///
/// let text = "Preamp: -2.9 dB\nFilter  1: ON PK Fc   245 Hz Gain -0.46 dB Q 2.42\n";
/// let (preamp, peq) = parse_apo(text, 48_000.0).expect("valid APO");
/// assert_eq!(preamp, -2.9);
/// assert_eq!(peq.len(), 1);
/// assert_eq!(peq[0].1.freq, 245.0);
/// ```
///
/// # Errors
///
/// Returns an error on malformed `Preamp:`/`Filter` lines, unknown
/// filter types, or non-finite numeric fields.
pub fn parse_apo(text: &str, sample_rate: f64) -> Result<(f64, Peq)> {
    let mut preamp_db = 0.0;
    let mut peq: Peq = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let line_no = index + 1;
        if let Some(rest) = line.strip_prefix("Preamp:") {
            let token = rest
                .split_whitespace()
                .next()
                .with_context(|| format!("line {line_no}: Preamp without value"))?;
            preamp_db = token
                .parse()
                .with_context(|| format!("line {line_no}: bad preamp value {token}"))?;
        } else if line.split_whitespace().next() == Some("Filter") {
            // Filter  1: ON PK Fc   245 Hz Gain -0.46 dB Q 2.42
            // (AutoEQ headers like "Filters: 7" are not filter lines.)
            let mut tokens = line.split_whitespace();
            let _filter = tokens.next();
            let _number = tokens.next();
            let state = tokens
                .next()
                .with_context(|| format!("line {line_no}: truncated filter line"))?;
            if state == "OFF" {
                continue;
            }
            if state != "ON" {
                bail!("line {line_no}: expected ON/OFF, got {state}");
            }
            let kind = tokens
                .next()
                .with_context(|| format!("line {line_no}: missing filter type"))?;
            let filter_type = filter_type(kind).with_context(|| format!("line {line_no}"))?;
            let mut freq: Option<f64> = None;
            let mut gain_db = 0.0;
            let mut q = 0.0;
            let rest: Vec<&str> = tokens.collect();
            let mut i = 0;
            while i < rest.len() {
                let keyword = rest[i];
                let Some(value) = rest.get(i + 1) else { break };
                match keyword {
                    "Fc" => {
                        freq =
                            Some(value.parse().with_context(|| {
                                format!("line {line_no}: bad Fc value {value}")
                            })?);
                        i += 2;
                    }
                    "Gain" => {
                        gain_db = value
                            .parse()
                            .with_context(|| format!("line {line_no}: bad Gain value {value}"))?;
                        i += 2;
                    }
                    "Q" => {
                        q = value
                            .parse()
                            .with_context(|| format!("line {line_no}: bad Q value {value}"))?;
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let freq = freq.with_context(|| format!("line {line_no}: filter without Fc"))?;
            for (name, value) in [("Fc", freq), ("Gain", gain_db), ("Q", q)] {
                if !value.is_finite() {
                    bail!("line {line_no}: non-finite {name} value {value}");
                }
            }
            // Peq tuple head is a response weight; x2peq uses 1.0.
            peq.push((1.0, Biquad::new(filter_type, freq, sample_rate, q, gain_db)));
        }
    }
    Ok((preamp_db, peq))
}

/// Formats a PEQ as a spinorama site APO file (`iir-autoeq.txt`).
///
/// The header mirrors Python `auto_save.py` wording; `Preamp:` and
/// `Filter` lines come from `iir::peq_format_apo`. Frequencies are
/// rounded to whole hertz first, matching the Rust AutoEQ CLI export
/// (the formatter truncates to `i32`).
#[must_use]
pub fn format_site_apo(
    speaker: &str,
    origin: &str,
    score_without_eq: f64,
    score_with_eq: f64,
    peq: &Peq,
) -> String {
    let header = format!(
        "EQ for {speaker} computed from {origin} data\n\
         Preference Score {score_without_eq:.2} with EQ {score_with_eq:.2}\n\
         Generated by spinorama-generate generate-peq (Rust AutoEQ)"
    );
    let mut rounded = peq.clone();
    for (_, filter) in &mut rounded {
        filter.freq = filter.freq.round();
    }
    iir::peq_format_apo(&header, &rounded)
}
