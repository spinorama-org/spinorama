//! Parity with the original Python CEA2034 overlays on synthetic curves.

use serde_json::Value;
use spinorama_generate::{
    compute::cea2034::Cea2034,
    figures::common_figures,
    render::{plotly_json, trace_bundle},
};

#[test]
fn cea_overlays_match_original_python_algorithms() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/cea_overlays_python.json")).unwrap();
    let curve = |name: &str| serde_json::from_value(reference["curves"][name].clone()).unwrap();
    let cea = Cea2034 {
        freq: serde_json::from_value(reference["freq"].clone()).unwrap(),
        on_axis: curve("On Axis"),
        listening_window: curve("Listening Window"),
        early_reflections: curve("Early Reflections"),
        sound_power: curve("Sound Power"),
        early_reflections_di: curve("Early Reflections DI"),
        sound_power_di: curve("Sound Power DI"),
        estimated_inroom: curve("On Axis"),
        sensitivity_db: 90.0,
    };
    let figures = common_figures(&cea, None, "Synthetic", "Lab", (20.0, 20000.0));
    let figure = &figures[0];
    for (name, expected) in reference["overlays"].as_object().unwrap() {
        let actual = figure
            .traces
            .iter()
            .find(|t| &t.name == name)
            .expect("original overlay exists");
        for (key, values) in [("x", actual.x.as_ref().unwrap()), ("y", &actual.values)] {
            let expected: Vec<f64> = serde_json::from_value(expected[key].clone()).unwrap();
            assert_eq!(values.len(), expected.len());
            for (a, b) in values.iter().zip(expected) {
                assert!((a - b).abs() < 1e-9, "{name}: {key} {a} != {b}");
            }
        }
        assert!(
            !actual.show_legend,
            "helper overlays do not clutter the legend"
        );
    }
    let bundle: Value =
        serde_json::from_str(&trace_bundle::to_json_string(figure).unwrap()).unwrap();
    assert_eq!(bundle["annotation_requests"].as_array().unwrap().len(), 6);
    for request in bundle["annotation_requests"].as_array().unwrap() {
        let metrics = &reference["metrics"][request[0].as_str().unwrap()];
        assert_eq!(
            request[4],
            format!(
                "{:.2} db/oct sm {:.2}",
                metrics[0].as_f64().unwrap(),
                metrics[1].as_f64().unwrap()
            )
        );
    }
    let plot: Value = serde_json::from_str(&plotly_json::to_json_string(figure).unwrap()).unwrap();
    let annotations = plot["layout"]["annotations"].as_array().unwrap();
    assert_eq!(annotations.len(), 6);
    assert_eq!(annotations.iter().filter(|a| a["yref"] == "y2").count(), 2);
    assert!(
        annotations
            .iter()
            .all(|a| a["axref"] == "x" && a["showarrow"] == true)
    );
    let zone = bundle["traces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "recommended LW zone")
        .unwrap();
    assert_eq!(
        zone["polygon"].as_array().unwrap().len(),
        5,
        "polygon geometry survives bundle encoding"
    );
}
