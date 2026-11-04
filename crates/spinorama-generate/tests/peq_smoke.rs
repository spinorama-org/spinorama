//! End-to-end PEQ optimization smoke tests on synthetic curves.

// Rust guideline compliant 2026-02-21

use spinorama_generate::filters::apo::{format_site_apo, parse_apo};
use spinorama_generate::filters::biquad::apply_peq;
use spinorama_generate::peq::optimize::{
    PeqOptions, loss_from_fitness, optimize_peq, rms_deviation,
};
use spinorama_generate::peq::target::{default_slope, minus_3db_freq, slope_target};

fn log_grid() -> Vec<f64> {
    (0..96)
        .map(|i| 20.0 * (1000f64).powf(f64::from(i) / 95.0))
        .collect()
}

/// A measured curve with a broad +4 dB hump around 500 Hz.
fn humped(freq: &[f64]) -> Vec<f64> {
    freq.iter()
        .map(|f| 80.0 + 4.0 * (-((f.log2() - 500f64.log2()).powi(2)) / 0.5).exp())
        .collect()
}

#[test]
fn slope_target_is_flat_for_on_axis() {
    let freq = log_grid();
    let spl = humped(&freq);
    let target = slope_target(&freq, &spl, 0.0, 60.0, 16_000.0, 100.0, 300.0).expect("target");
    let spread = target.iter().fold(f64::MIN, |a, v| a.max(*v))
        - target.iter().fold(f64::MAX, |a, v| a.min(*v));
    assert!(spread < 1e-9, "zero slope must be flat, spread {spread}");
    assert_eq!(default_slope("On Axis").expect("known curve"), 0.0);
    assert!(default_slope("Nope").is_err());
}

#[test]
fn minus_3db_finds_bass_rolloff() {
    let freq = log_grid();
    // Flat 80 dB above 100 Hz, -12 dB/octave below.
    let spl: Vec<f64> = freq
        .iter()
        .map(|f| {
            if *f >= 100.0 {
                80.0
            } else {
                80.0 - 12.0 * (100.0 / f).log2()
            }
        })
        .collect();
    let f3 = minus_3db_freq(&freq, &spl, 100.0, 300.0).expect("-3dB point");
    assert!((70.0..=95.0).contains(&f3), "f3 {f3}");
}

#[test]
fn optimization_reduces_deviation_and_roundtrips() {
    let freq = log_grid();
    let measured = humped(&freq);
    let target = slope_target(&freq, &measured, 0.0, 60.0, 16_000.0, 100.0, 300.0).expect("target");
    let options = PeqOptions {
        num_filters: 3,
        maxeval: 1_500,
        population: 40,
        ..PeqOptions::default()
    };
    let peq = optimize_peq(&freq, &measured, &target, None, &options).expect("optimization runs");
    assert_eq!(peq.len(), 3);
    let corrected = apply_peq(&freq, &measured, &peq);
    let before = rms_deviation(&freq, &measured, &target, 60.0, 16_000.0).expect("rms");
    let after = rms_deviation(&freq, &corrected, &target, 60.0, 16_000.0).expect("rms");
    assert!(after < before * 0.6, "RMS {before:.3} -> {after:.3} dB");

    let text = format_site_apo("Synthetic", "test", 0.0, 0.0, &peq);
    let (_, reparsed) = parse_apo(&text, options.sample_rate).expect("APO reparses");
    assert_eq!(reparsed.len(), peq.len());
}

#[test]
fn fitness_mapping_is_explicit() {
    assert!(loss_from_fitness("flat").is_ok());
    assert!(loss_from_fitness("leastsquare").is_ok());
    assert!(loss_from_fitness("score").is_ok());
    assert!(loss_from_fitness("combine").is_ok());
    assert!(loss_from_fitness("flatpir").is_ok());
    assert!(loss_from_fitness("bogus").is_err());
}
