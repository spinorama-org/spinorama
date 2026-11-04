//! APO parse/format contracts against the Python site file shape.

// Rust guideline compliant 2026-02-21

use spinorama_generate::filters::apo::{format_site_apo, parse_apo};
use spinorama_generate::filters::biquad::peq_response_db;

/// Excerpt of the Python-generated `datas/eq/KEF R3/iir-autoeq.txt`
/// (values only; see tests/fixtures/README.md for provenance).
const PYTHON_APO: &str = "EQ for KEF R3 computed from ASR data\n\
Preference Score 6.50 with EQ 7.09\n\
Preamp: -2.9 dB\n\
Filter  1: ON LS Fc    69 Hz Gain +2.87 dB Q 0.66\n\
Filter  2: ON PK Fc   245 Hz Gain -0.46 dB Q 2.42\n\
Filter  3: ON PK Fc 10361 Hz Gain -0.75 dB Q 1.42\n";

#[test]
fn parses_autoeq_settings_header() {
    // AutoEQ exports start with a "Filters: N" header line, which is
    // not a filter line and must be skipped, not a parse error.
    const AUTOEQ: &str = "AutoEQ Parametric Equalizer Settings\n\
Speaker: AsciLab C8C\n\
Loss Type: SpeakerScore\n\
Filters: 7\n\
Generated: 2026-03-09 18:48:24\n\
\n\
Preamp: -2.9 dB\n\
\n\
Filter  1: ON PK Fc    20 Hz Gain +1.28 dB Q 2.83\n\
Filter  2: ON PK Fc    29 Hz Gain +2.67 dB Q 1.16\n";
    let (preamp, peq) = parse_apo(AUTOEQ, 48_000.0).expect("AutoEQ settings parse");
    assert_eq!(preamp, -2.9);
    assert_eq!(peq.len(), 2);
}

#[test]
fn parses_python_site_apo() {
    let (preamp, peq) = parse_apo(PYTHON_APO, 48_000.0).expect("python APO parses");
    assert_eq!(preamp, -2.9);
    assert_eq!(peq.len(), 3);
    assert_eq!(peq[0].1.freq, 69.0);
    assert!((peq[1].1.db_gain - -0.46).abs() < 1e-12);
    assert!((peq[2].1.q - 1.42).abs() < 1e-12);
}

#[test]
fn format_then_parse_is_stable() {
    let (_, peq) = parse_apo(PYTHON_APO, 48_000.0).expect("python APO parses");
    let text = format_site_apo("KEF R3", "ASR", 6.50, 7.09, &peq);
    let (preamp, reparsed) = parse_apo(&text, 48_000.0).expect("site APO reparses");
    assert_eq!(reparsed.len(), peq.len());
    // Preamp is recomputed as -(max combined response) by
    // peq_format_apo and printed with 0.1 dB resolution; the Python
    // file used -2.9 dB, Rust computes -2.8 dB on its grid.
    assert!((preamp - -2.9).abs() <= 0.15, "preamp {preamp}");
    for ((_, a), (_, b)) in peq.iter().zip(&reparsed) {
        assert_eq!(a.filter_type, b.filter_type);
        assert!((a.freq - b.freq).abs() < 1e-9);
        assert!((a.db_gain - b.db_gain).abs() < 0.005);
        assert!((a.q - b.q).abs() < 0.005);
    }
    // Response equality is the numeric contract (design §8, 1e-6 bar
    // is for unrounded round-trips; APO text rounds to 0.01 dB/Hz).
    let freq: Vec<f64> = (0..64)
        .map(|i| 20.0 * (1000f64).powf(f64::from(i) / 63.0))
        .collect();
    let before = peq_response_db(&freq, &peq);
    let after = peq_response_db(&freq, &reparsed);
    let max_diff = before
        .iter()
        .zip(&after)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max);
    assert!(max_diff < 0.05, "max response diff {max_diff} dB");
}

#[test]
fn unknown_filter_type_is_a_hard_error() {
    let text = "Filter  1: ON ZZ Fc 100 Hz Gain +1.00 dB Q 1.00\n";
    let error = parse_apo(text, 48_000.0).expect_err("unknown type must fail");
    assert!(
        format!("{error:#}").contains("unknown APO filter type"),
        "{error:#}"
    );
}

#[test]
fn off_filters_are_skipped() {
    let text = "Filter  1: OFF PK Fc 100 Hz Gain +1.00 dB Q 1.00\n\
                Filter  2: ON PK Fc 200 Hz Gain -1.00 dB Q 1.00\n";
    let (_, peq) = parse_apo(text, 48_000.0).expect("parses");
    assert_eq!(peq.len(), 1);
    assert_eq!(peq[0].1.freq, 200.0);
}
