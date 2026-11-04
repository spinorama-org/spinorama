//! Speaker metadata helpers shared by the generator binaries.

// Rust guideline compliant 2026-02-21

/// Characters replaced by Python `spinorama.misc.sanitize_filename`.
const UNSAFE_CHARS: &[char] = &['|', ':', '<', '>', '"', '/', '\\', '?', '*'];

/// Returns `name` with unsafe filename characters replaced by `_`.
///
/// This matches `spinorama/src/spinorama/misc.py::sanitize_filename`, which
/// defines the on-disk speaker directory names under `dist/` and
/// `dist-private/`.
///
/// # Examples
///
/// ```
/// use spinorama_generate::meta::sanitize_filename;
/// assert_eq!(sanitize_filename("KEF R3"), "KEF R3");
/// assert_eq!(sanitize_filename("A/B:C"), "A_B_C");
/// ```
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if UNSAFE_CHARS.contains(&c) { '_' } else { c })
        .collect()
}

/// Returns the output origin directory name for `origin`.
///
/// Python strips the `Vendors-` prefix when building public and private
/// output trees (`speaker.py::build_filename`).
///
/// # Examples
///
/// ```
/// use spinorama_generate::meta::origin_dir_name;
/// assert_eq!(origin_dir_name("Vendors-KEF"), "KEF");
/// assert_eq!(origin_dir_name("ASR"), "ASR");
/// ```
#[must_use]
pub fn origin_dir_name(origin: &str) -> &str {
    origin.strip_prefix("Vendors-").unwrap_or(origin)
}

fn normalised_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Returns the watermark source label for an origin/version pair.
///
/// Mirrors Python `image_export.source_label`: known measurers map to
/// display names, `Vendors-` is stripped, `Misc` versions encode the
/// measurer after a dash, and genuinely unknown sources return empty.
#[must_use]
pub fn source_label(origin: &str, version: &str) -> String {
    let labels: &[(&str, &str)] = &[
        ("asr", "Audio Science Review"),
        ("audiosciencereview", "Audio Science Review"),
        ("eac", "Erin's Audio Corner"),
        ("erinsaudiocorner", "Erin's Audio Corner"),
        ("erin", "Erin's Audio Corner"),
        ("princeton", "Princeton 3D3A"),
        ("3d3a", "Princeton 3D3A"),
        ("sr", "Sound & Recording"),
        ("pp", "Production Partners"),
        ("archimago", "Archimago"),
        ("audioxpress", "AudioXPress"),
        ("audioholics", "Audioholics"),
        ("soundstageultra", "Sound Stage Ultra"),
        ("ssn", "SoundStage Network"),
        ("soundstagenetwork", "SoundStage Network"),
        ("nuyes", "Nuyes"),
        ("stereophile", "Stereophile"),
        ("soundstagesolo", "SoundStage Solo"),
    ];
    let lookup = |key: &str| {
        labels
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, label)| (*label).to_owned())
    };
    let version_code = version
        .strip_suffix("_eq")
        .or_else(|| version.strip_suffix("-eq"))
        .unwrap_or(version);
    for candidate in [origin, version_code] {
        if let Some(label) = lookup(&normalised_key(candidate)) {
            return label;
        }
    }
    if let Some(vendor) = origin.strip_prefix("Vendors-") {
        return vendor.to_owned();
    }
    if normalised_key(origin) == "misc" {
        let suffix = version_code.rsplit('-').next().unwrap_or(version_code);
        if let Some(label) = lookup(&normalised_key(suffix)) {
            return label;
        }
        if let Some(rest) = version_code.strip_prefix("misc-")
            && !rest.is_empty()
        {
            return rest
                .split('-')
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        return String::new();
    }
    for part in version_code.replace('_', "-").split('-').rev() {
        if let Some(label) = lookup(&normalised_key(part)) {
            return label;
        }
    }
    let key = normalised_key(origin);
    if key.is_empty() || key == "misc" {
        String::new()
    } else {
        origin.to_owned()
    }
}

/// Returns the `measured by` label used in figure titles.
///
/// Mirrors Python `speaker.py::build_title` for the ASR and Misc special
/// cases; other origins are used as stored in speaker metadata.
#[must_use]
pub fn measured_by_label(origin: &str, version: &str) -> String {
    if let Some(vendor) = origin.strip_prefix("Vendors-") {
        return vendor.to_owned();
    }
    if origin == "ASR" {
        return "Audio Science Review".to_owned();
    }
    if origin == "Misc" {
        if version.ends_with("-sr") {
            return "Sound & Recording (data scanned)".to_owned();
        }
        if version.ends_with("-pp") {
            return "Production Partners (data scanned)".to_owned();
        }
        if let Some((_, rest)) = version.split_once('-')
            && !rest.is_empty()
        {
            let mut chars = rest.chars();
            if let Some(first) = chars.next() {
                return first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase();
            }
        }
    }
    origin.to_owned()
}
