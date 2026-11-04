//! AutoEQ `filters_*` figure family on a synthetic CEA2034.

// Rust guideline compliant 2026-02-21

use spinorama_generate::compute::cea2034::Cea2034;
use spinorama_generate::figures::{FigureKind, eq_filter_figures};
use spinorama_generate::filters::apo::parse_apo;
use spinorama_generate::render::plotly_json::to_json_string;

fn synthetic_cea() -> Cea2034 {
    let freq: Vec<f64> = (0..100)
        .map(|i| 20.0 * 1000f64.powf(f64::from(i) / 99.0))
        .collect();
    // Gentle tilt plus a 1 kHz bump, so the regression band is non-trivial.
    let curve: Vec<f64> = freq
        .iter()
        .map(|f| {
            let bump = 2.0 * (-0.5 * ((f.log2() - 1000f64.log2()) / 0.5).powi(2)).exp();
            -(f / 20.0).log2() / 10.0 + bump
        })
        .collect();
    Cea2034 {
        freq,
        on_axis: curve.clone(),
        listening_window: curve.clone(),
        early_reflections: curve.clone(),
        sound_power: curve.clone(),
        early_reflections_di: vec![0.0; 100],
        sound_power_di: vec![0.0; 100],
        estimated_inroom: curve,
        sensitivity_db: 85.0,
    }
}

#[test]
fn eq_filter_figures_cover_the_python_stems() {
    let cea = synthetic_cea();
    let cea_eq = synthetic_cea();
    let (_preamp, peq) = parse_apo(
        "Preamp: -2.0 dB\nFilter 1: ON PK Fc 1000 Hz Gain -2.0 dB Q 1.00\n",
        48_000.0,
    )
    .expect("APO parses");
    let figures = eq_filter_figures(&cea, &cea_eq, &peq, "Synthetic");
    let mut names: Vec<&str> = figures.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    let expected = [
        "filters_auto_eq",
        "filters_eq",
        "filters_eq_full",
        "filters_lw_auto",
        "filters_lw_hist_fullrange",
        "filters_lw_hist_midrange",
        "filters_lw_noeq",
        "filters_on_auto",
        "filters_on_hist_fullrange",
        "filters_on_hist_midrange",
        "filters_on_noeq",
        "filters_pir_auto",
        "filters_pir_hist_fullrange",
        "filters_pir_hist_midrange",
        "filters_pir_noeq",
        "filters_spin_auto",
        "filters_spin_noeq",
    ];
    assert_eq!(names, expected);

    // The combined sheet is a 16-cell subplot (4 + 3 curves x 4) and every
    // figure serializes to Plotly JSON.
    let sheet = figures
        .iter()
        .find(|f| f.name == "filters_auto_eq")
        .expect("sheet exists");
    let FigureKind::Subplot(cells) = &sheet.kind else {
        panic!("filters_auto_eq must be a subplot figure");
    };
    assert_eq!(cells.len(), 16);
    for figure in &figures {
        let json = to_json_string(figure).expect("figure serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(value.get("data").is_some(), "{} has data", figure.name);
        assert!(value.get("layout").is_some(), "{} has layout", figure.name);
    }
    // Subplot traces carry per-cell axis references.
    let sheet_json = to_json_string(sheet).expect("sheet serializes");
    assert!(
        sheet_json.contains("\"xaxis\":\"x16\""),
        "last cell axis ref"
    );
    assert!(sheet_json.contains("\"barmode\":\"group\""));

    // Histogram bars: np.histogram bins [0, .5, 1, 1.5, 2, 2.5, 3, 5].
    let hist = figures
        .iter()
        .find(|f| f.name == "filters_on_hist_fullrange")
        .expect("histogram exists");
    let FigureKind::Bar(bars) = &hist.kind else {
        panic!("histogram must be a bar figure");
    };
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].values.len(), 7);
    let total: f64 = bars[0].values.iter().sum();
    assert!(total > 0.0, "some samples fall in the bins");
}
