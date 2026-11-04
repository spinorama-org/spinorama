//! Text-loader, symmetry, and graphic-EQ contract tests.

// Rust guideline compliant 2026-02-21

use spinorama_generate::loaders::text::{parse_text_curve, plane_from_curves};
use spinorama_generate::measurements::{HvSpl, SplPlane, symmetrise_hv};
use spinorama_generate::peq::grapheq::{graphic_eq, graphic_eq_names, optimize_grapheq};
use spinorama_generate::peq::optimize::{Fitness, PeqOptions};

#[test]
fn text_curve_sorts_dedupes_and_keeps_phase() {
    let curve = parse_text_curve(
        "Freq dB Phase\n200 1.0 10\n100 0.0 0\n100 9.9 99\n400 2.0 20\n",
        20.0,
        20_000.0,
        0,
    )
    .expect("curve parses");
    assert_eq!(curve.freq, vec![100.0, 200.0, 400.0]);
    assert_eq!(curve.spl, vec![0.0, 1.0, 2.0]);
    assert_eq!(curve.phase.expect("phase kept"), vec![0.0, 10.0, 20.0]);
}

#[test]
fn plane_from_curves_aligns_mismatched_grids() {
    let on_axis = parse_text_curve("100 0\n1000 0\n10000 0\n", 20.0, 20_000.0, 0).unwrap();
    let ten = parse_text_curve("100 0\n10000 -10\n", 20.0, 20_000.0, 0).unwrap();
    let plane = plane_from_curves(vec![
        ("On Axis".to_owned(), on_axis),
        ("10°".to_owned(), ten),
    ])
    .expect("plane builds");
    let values = plane.column("10°").expect("10 degree column");
    assert_eq!(values.len(), 3);
    assert!((values[1] - -5.0).abs() < 0.01, "log midpoint is -5 dB");
}

#[test]
fn vertical_symmetry_mirrors_negative_angles() {
    let plane = |columns: Vec<(&str, Vec<f64>)>| {
        SplPlane::new(
            vec![100.0, 1000.0],
            columns
                .into_iter()
                .map(|(name, values)| (name.to_owned(), values))
                .collect(),
        )
        .expect("plane")
    };
    let hv = HvSpl {
        horizontal: plane(vec![("On Axis", vec![0.0, 0.0])]),
        vertical: plane(vec![
            ("On Axis", vec![0.0, 0.0]),
            ("-20°", vec![-1.0, -2.0]),
        ]),
    };
    let mirrored = symmetrise_hv(&hv, Some("vertical")).expect("symmetrise");
    assert_eq!(
        mirrored.vertical.column("20°").expect("mirrored column"),
        &[-1.0, -2.0]
    );
    assert!(symmetrise_hv(&hv, Some("diagonal")).is_err());
}

#[test]
fn grapheq_cuts_a_synthetic_peak() {
    assert!(graphic_eq_names().contains(&"Denon AVR"));
    assert!(graphic_eq("No Such EQ").is_err());
    let device = graphic_eq("Denon AVR").expect("known device");
    assert_eq!(device.bands.len(), 9);

    let freq: Vec<f64> = (0..100)
        .map(|i| 20.0 * 1000f64.powf(f64::from(i) / 99.0))
        .collect();
    let error: Vec<f64> = freq
        .iter()
        .map(|f| 3.0 * (-(f / 1000.0).log2().powi(2)).exp())
        .collect();
    let measured = error.clone();
    let target = vec![0.0; freq.len()];
    let options = PeqOptions {
        fitness: Fitness::Flat,
        ..PeqOptions::default()
    };
    let peq = optimize_grapheq(&freq, &error, None, &measured, &target, &device, &options)
        .expect("grapheq optimizes");
    assert_eq!(peq.len(), device.bands.len());
    let at_1k = peq
        .iter()
        .find(|(_, filter)| (filter.freq - 1000.0).abs() < 1.0)
        .expect("1 kHz band");
    assert!(at_1k.1.db_gain < -1.0, "1 kHz gain {}", at_1k.1.db_gain);
}
