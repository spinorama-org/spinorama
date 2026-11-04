//! Speaker metadata validation endpoint.
//!
//! Port of Python `datas/checks.py::validate_speaker_data` (the part
//! the API exposes). Origin names come from `origins.toml` when
//! available, else from the origins seen in the loaded metadata.

// Rust guideline compliant 2026-02-21

use serde_json::{Map, Value};

const VALID_AMOUNTS: &[&str] = &["each", "pair"];
const VALID_TYPES: &[&str] = &["active", "passive"];
const VALID_SHAPES: &[&str] = &[
    "floorstanders",
    "bookshelves",
    "center",
    "surround",
    "omnidirectional",
    "columns",
    "cbt",
    "outdoor",
    "panel",
    "inwall",
    "soundbar",
    "liveportable",
    "toursound",
    "cinema",
];
// Matches the Rust metadata enum (Python checks.py predates csv_freq_spl).
const VALID_FORMATS: &[&str] = &[
    "klippel",
    "webplotdigitizer",
    "spl_hv_txt",
    "gll_hv_txt",
    "princeton",
    "rew_text_dump",
    "csv_freq_spl",
];
const VALID_QUALITIES: &[&str] = &["low", "medium", "high", "unknown"];
const VENDORS_PREFIX: &str = "Vendors-";

struct Result_ {
    valid: bool,
    messages: Vec<String>,
}

impl Result_ {
    fn error(&mut self, message: String) {
        self.valid = false;
        self.messages.push(format!("ERROR: {message}"));
    }
    fn warning(&mut self, message: String) {
        self.messages.push(format!("WARNING: {message}"));
    }
}

fn get_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn is_valid_origin(origin: &str, origins: &[String]) -> bool {
    if origin.is_empty() {
        return false;
    }
    if origins.iter().any(|known| known == origin) {
        return true;
    }
    origin.starts_with(VENDORS_PREFIX) && origin.len() > VENDORS_PREFIX.len()
}

/// Label, unit, and plausible range for a numeric specification.
struct SpecRange {
    label: &'static str,
    unit: &'static str,
    min: f64,
    max: f64,
}

fn validate_specs_number(
    result: &mut Result_,
    name: &str,
    key: &str,
    spec: &SpecRange,
    value: &Value,
) {
    let (label, unit, min, max) = (spec.label, spec.unit, spec.min, spec.max);
    let Some(number) = value.as_f64() else {
        result.error(format!(
            "{label} must be a number for measurement '{key}' in {name}"
        ));
        return;
    };
    if number < min || number > max {
        result.warning(format!(
            "Unlikely {label_lower} value {number}{unit} for measurement '{key}' in {name}. \
             Expected range: {min}-{max}{unit}",
            label_lower = label.to_lowercase()
        ));
    }
}

fn validate_measurement(
    result: &mut Result_,
    name: &str,
    key: &str,
    measurement: &Map<String, Value>,
    origins: &[String],
) {
    match get_str(measurement, "origin") {
        None => result.error(format!(
            "Origin is missing for measurement '{key}' in {name}"
        )),
        Some("") => result.error(format!("Origin is empty for measurement '{key}' in {name}")),
        Some(origin) if !is_valid_origin(origin, origins) => {
            let valid: Vec<&str> = origins
                .iter()
                .map(String::as_str)
                .filter(|o| !o.starts_with(VENDORS_PREFIX))
                .collect();
            result.error(format!(
                "Invalid origin '{origin}' for measurement '{key}' in {name}. \
                 Valid origins: {} or '{VENDORS_PREFIX}<name>'",
                valid.join(", ")
            ));
        }
        _ => {}
    }
    match get_str(measurement, "format") {
        None => result.error(format!(
            "Format is missing for measurement '{key}' in {name}"
        )),
        Some(format) if !VALID_FORMATS.contains(&format) => {
            result.error(format!(
                "Invalid format '{format}' for measurement '{key}' in {name}. \
                 Valid formats: {}",
                VALID_FORMATS.join(", ")
            ));
        }
        _ => {}
    }
    if let Some(quality) = measurement.get("quality")
        && let Some(q) = quality.as_str()
        && !VALID_QUALITIES.contains(&q)
    {
        result.error(format!(
            "Invalid quality '{q}' for measurement '{key}' in {name}. \
             Valid qualities: {}",
            VALID_QUALITIES.join(", ")
        ));
    }
    const IMPEDANCE: SpecRange = SpecRange {
        label: "Impedance",
        unit: "Ω",
        min: 2.0,
        max: 16.0,
    };
    const SENSITIVITY: SpecRange = SpecRange {
        label: "Sensitivity",
        unit: "dB",
        min: 70.0,
        max: 120.0,
    };
    if let Some(specs) = measurement.get("specifications").and_then(Value::as_object) {
        if let Some(impedance) = specs.get("impedance") {
            validate_specs_number(result, name, key, &IMPEDANCE, impedance);
        }
        if let Some(sensitivity) = specs.get("sensitivity") {
            validate_specs_number(result, name, key, &SENSITIVITY, sensitivity);
        }
    }
    if let Some(sensitivity) = measurement.get("sensitivity") {
        validate_specs_number(result, name, key, &SENSITIVITY, sensitivity);
    }
}

/// Validates one speaker metadata object (Python `validate_speaker_data`).
#[must_use]
pub fn validate_speaker_data(
    name: &str,
    speaker: &Value,
    origins: &[String],
) -> (bool, Vec<String>) {
    let mut result = Result_ {
        valid: true,
        messages: Vec::new(),
    };
    let Some(obj) = speaker.as_object() else {
        result.error("Speaker data must be a dictionary".to_owned());
        return (result.valid, result.messages);
    };

    // Brand and model.
    match get_str(obj, "brand") {
        None => result.error(format!("Brand is missing for {name}")),
        Some("") => result.error(format!("Brand is empty for {name}")),
        Some(brand) => {
            if !name.starts_with(brand) {
                result.error(format!("{name} doesn't start with brand {brand}"));
            }
            if brand.ends_with(' ') {
                result.warning(format!(
                    "Suspicious space at the end of brand '{brand}' for {name}"
                ));
            }
        }
    }
    let brand = get_str(obj, "brand").unwrap_or("");
    match get_str(obj, "model") {
        None => result.error(format!("Model is missing for {name}")),
        Some("") => result.error(format!("Model is empty for {name}")),
        Some(model) => {
            if model.split(' ').next() == Some(brand) && !brand.is_empty() {
                result.warning(format!(
                    "Model '{model}' starts with brand '{brand}' for {name}"
                ));
            }
            if !name.ends_with(model) {
                result.error(format!("{name} doesn't end with model {model}"));
            }
            if model.starts_with(' ') {
                result.warning(format!(
                    "Suspicious space at the beginning of model '{model}' for {name}"
                ));
            }
        }
    }

    // Type, shape, amount.
    match get_str(obj, "type") {
        None => result.error(format!("Type is missing for {name}")),
        Some(kind) if !VALID_TYPES.contains(&kind) => {
            result.error(format!(
                "Invalid type '{kind}' for {name}. Valid types: {}",
                VALID_TYPES.join(", ")
            ));
        }
        _ => {}
    }
    match get_str(obj, "shape") {
        None => result.error(format!("Shape is missing for {name}")),
        Some(shape) if !VALID_SHAPES.contains(&shape) => {
            result.error(format!(
                "Invalid shape '{shape}' for {name}. Valid shapes: {}",
                VALID_SHAPES.join(", ")
            ));
        }
        _ => {}
    }
    if let Some(amount) = get_str(obj, "amount")
        && !VALID_AMOUNTS.contains(&amount)
    {
        result.error(format!(
            "Invalid amount '{amount}' for {name}. Valid amounts: {}",
            VALID_AMOUNTS.join(", ")
        ));
    }

    // Measurements.
    let Some(measurements_value) = obj.get("measurements") else {
        result.error(format!("Measurements are missing for {name}"));
        return (result.valid, result.messages);
    };
    let Some(measurements) = measurements_value.as_object() else {
        result.error(format!("Measurements must be a dictionary for {name}"));
        return (result.valid, result.messages);
    };
    if measurements.is_empty() {
        result.error(format!("At least one measurement is required for {name}"));
        return (result.valid, result.messages);
    }
    match get_str(obj, "default_measurement") {
        None | Some("") => result.error(format!("Default measurement is missing for {name}")),
        Some(default) if !measurements.contains_key(default) => {
            result.error(format!(
                "Default measurement '{default}' not found in measurements for {name}"
            ));
        }
        _ => {}
    }
    for (key, measurement) in measurements {
        if let Some(measurement) = measurement.as_object() {
            validate_measurement(&mut result, name, key, measurement, origins);
        }
    }
    (result.valid, result.messages)
}
