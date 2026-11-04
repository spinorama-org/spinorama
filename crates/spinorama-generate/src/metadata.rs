//! Per-speaker TOML metadata (`datas/measurements/<Speaker>/meta.toml`).
//!
//! The catalogue migrated from Python dict literals to one TOML file
//! per speaker. These structs are the strict schema: unknown keys,
//! missing required fields, and unknown enum values are hard errors
//! (no default fallback), replacing the runtime checks that used to
//! live in Python `datas/checks.py`.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::meta::sanitize_filename;

/// Speaker kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerType {
    /// Passive speaker (external amplifier).
    Passive,
    /// Active speaker (built-in amplifier).
    Active,
}

/// Speaker enclosure shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerShape {
    /// Bookshelf / standmount.
    Bookshelves,
    /// CBT array.
    Cbt,
    /// Center channel.
    Center,
    /// Cinema speaker.
    Cinema,
    /// Column array.
    Columns,
    /// Floorstander.
    Floorstanders,
    /// In-wall.
    Inwall,
    /// Portable live speaker.
    Liveportable,
    /// Omnidirectional.
    Omnidirectional,
    /// Outdoor.
    Outdoor,
    /// Panel.
    Panel,
    /// Soundbar.
    Soundbar,
    /// Surround.
    Surround,
    /// Touring sound.
    Toursound,
}

impl SpeakerShape {
    /// Returns the lowercase shape name used in metadata.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bookshelves => "bookshelves",
            Self::Cbt => "cbt",
            Self::Center => "center",
            Self::Cinema => "cinema",
            Self::Columns => "columns",
            Self::Floorstanders => "floorstanders",
            Self::Inwall => "inwall",
            Self::Liveportable => "liveportable",
            Self::Omnidirectional => "omnidirectional",
            Self::Outdoor => "outdoor",
            Self::Panel => "panel",
            Self::Soundbar => "soundbar",
            Self::Surround => "surround",
            Self::Toursound => "toursound",
        }
    }
}

/// Sales unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Amount {
    /// Sold each.
    Each,
    /// Sold as a pair.
    Pair,
}

/// Measurement quality grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasurementQuality {
    /// Low quality.
    Low,
    /// Medium quality.
    Medium,
    /// High quality.
    High,
    /// Unknown quality.
    Unknown,
}

/// Plane symmetry of an HV measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Symmetry {
    /// No symmetry.
    None,
    /// Coaxial driver symmetry.
    Coaxial,
    /// Vertical symmetry.
    Vertical,
    /// Horizontal symmetry.
    Horizontal,
}

impl Symmetry {
    /// Returns the lowercase name used by the loaders.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Coaxial => "coaxial",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }
}

/// Measurement file format, mirroring the loader dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum MeasurementFormat {
    /// Klippel NFS export.
    #[serde(rename = "klippel")]
    Klippel,
    /// Whitespace SPL horizontal/vertical text.
    #[serde(rename = "spl_hv_txt")]
    SplHvTxt,
    /// GLL zip with HV text.
    #[serde(rename = "gll_hv_txt")]
    GllHvTxt,
    /// Princeton 3D3A MATLAB IRs.
    #[serde(rename = "princeton")]
    Princeton,
    /// REW text dump.
    #[serde(rename = "rew_text_dump")]
    RewTextDump,
    /// WebPlotDigitizer CSV curves.
    #[serde(rename = "webplotdigitizer")]
    Webplotdigitizer,
    /// Headphone frequency/SPL CSV.
    #[serde(rename = "csv_freq_spl")]
    CsvFreqSpl,
}

impl MeasurementFormat {
    /// Returns the format name used by the loaders.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Klippel => "klippel",
            Self::SplHvTxt => "spl_hv_txt",
            Self::GllHvTxt => "gll_hv_txt",
            Self::Princeton => "princeton",
            Self::RewTextDump => "rew_text_dump",
            Self::Webplotdigitizer => "webplotdigitizer",
            Self::CsvFreqSpl => "csv_freq_spl",
        }
    }
}

/// Enclosure dimensions in millimetres.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    /// Height in mm.
    pub height: f64,
    /// Width in mm.
    pub width: f64,
    /// Depth in mm.
    pub depth: f64,
}

/// Sound pressure level capabilities in dB.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SplCapabilities {
    /// Peak SPL.
    pub peak: Option<f64>,
    /// Maximum SPL.
    pub max: Option<f64>,
    /// Continuous SPL.
    pub continuous: Option<f64>,
    /// M-noise SPL.
    pub m_noise: Option<f64>,
    /// B-noise SPL.
    pub b_noise: Option<f64>,
    /// Pink-noise SPL.
    pub pink_noise: Option<f64>,
}

/// Dispersion angles in degrees.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Dispersion {
    /// Horizontal angle.
    pub horizontal: Option<f64>,
    /// Vertical angle.
    pub vertical: Option<f64>,
}

/// Manufacturer specifications.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Specifications {
    /// Enclosure size.
    pub size: Option<Size>,
    /// Sensitivity in dB.
    pub sensitivity: Option<f64>,
    /// Impedance in ohms.
    pub impedance: Option<f64>,
    /// Weight in kg.
    pub weight: Option<f64>,
    /// Dispersion angles.
    pub dispersion: Option<Dispersion>,
    /// SPL capabilities.
    #[serde(rename = "SPL")]
    pub spl: Option<SplCapabilities>,
}

/// How the measurement data was acquired.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DataAcquisition {
    /// Acquisition path (e.g. `gll`).
    pub via: Option<String>,
    /// Measurement distance.
    pub distance: Option<f64>,
    /// Test signal description.
    pub signal: Option<String>,
    /// Whether air absorption was applied.
    pub air_absorbtion: Option<bool>,
    /// Angular resolution in degrees.
    pub resolution: Option<f64>,
    /// Free-form notes.
    pub notes: Option<String>,
    /// Lowest valid frequency in Hz.
    pub min_valid_freq: Option<f64>,
    /// Highest valid frequency in Hz.
    pub max_valid_freq: Option<f64>,
}

/// Mean-band parameters.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameters {
    /// Mean band start.
    pub mean_min: i64,
    /// Mean band end.
    pub mean_max: i64,
}

/// Extra flags.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Extras {
    /// Whether the speaker ships with EQ.
    pub is_equed: Option<bool>,
    /// Score penalty applied by the site.
    pub score_penalty: Option<f64>,
}

/// One measurement version of a speaker.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    /// Origin key (see `origins.toml`).
    pub origin: String,
    /// File format.
    pub format: MeasurementFormat,
    /// Single review URL (legacy field).
    pub review: Option<String>,
    /// Publication date `YYYYMMDD`.
    pub review_published: Option<String>,
    /// Quality grade.
    pub quality: Option<MeasurementQuality>,
    /// Manufacturer specifications.
    pub specifications: Option<Specifications>,
    /// Review URLs per site key.
    pub reviews: Option<BTreeMap<String, String>>,
    /// Free-form notes.
    pub notes: Option<String>,
    /// Acquisition details.
    pub data_acquisition: Option<DataAcquisition>,
    /// Plane symmetry.
    pub symmetry: Option<Symmetry>,
    /// Mean-band parameters.
    pub parameters: Option<Parameters>,
    /// Extra flags.
    pub extras: Option<Extras>,
}

/// One speaker's metadata (`meta.toml`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerMeta {
    /// Display name (also the catalogue key).
    pub name: String,
    /// Brand name.
    pub brand: String,
    /// Model name.
    pub model: String,
    /// Speaker kind.
    #[serde(rename = "type")]
    pub kind: SpeakerType,
    /// Price as free text (may be empty).
    pub price: String,
    /// Enclosure shape.
    pub shape: SpeakerShape,
    /// Sales unit.
    pub amount: Option<Amount>,
    /// Default measurement version key.
    pub default_measurement: String,
    /// Skip generation (metadata-only entry).
    #[serde(default)]
    pub skip: bool,
    /// Measurement versions by key.
    pub measurements: BTreeMap<String, Measurement>,
}

/// Origin information (`datas/origins.toml`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginInfo {
    /// Lowest valid frequency (`min hz`).
    #[serde(rename = "min hz")]
    pub min_hz: f64,
    /// Highest valid frequency (`max hz`).
    #[serde(rename = "max hz")]
    pub max_hz: f64,
    /// Lowest SPL (`min dB`).
    #[serde(rename = "min dB")]
    pub min_db: f64,
    /// Highest SPL (`max dB`).
    #[serde(rename = "max dB")]
    pub max_db: f64,
    /// Logo path.
    pub logo: Option<String>,
    /// Small logo path.
    #[serde(rename = "logo-small")]
    pub logo_small: Option<String>,
    /// Origin website.
    pub url: Option<String>,
}

#[derive(Deserialize)]
struct OriginsFile {
    origins: BTreeMap<String, OriginInfo>,
}

/// One catalogue row for batch generation.
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    /// Speaker display name.
    pub speaker: String,
    /// Measurement version key.
    pub version: String,
    /// Origin key.
    pub origin: String,
    /// Format name (loader spelling).
    pub format: String,
    /// Symmetry name, when set.
    pub symmetry: Option<String>,
    /// Directory holding the measurement data.
    pub speaker_dir: PathBuf,
    /// True for the speaker's default measurement.
    pub is_default: bool,
    /// Speaker shape, when known.
    pub shape: Option<String>,
    /// Lowest valid frequency from acquisition metadata.
    pub min_valid_freq: Option<f64>,
    /// Highest valid frequency from acquisition metadata.
    pub max_valid_freq: Option<f64>,
}

/// Loads every `meta.toml` under `root` (one per speaker directory).
///
/// # Errors
///
/// Returns an error when a file cannot be read, parsed, or fails the
/// strict schema, or when two files declare the same speaker name.
pub fn load_metadata_dir(root: &Path) -> Result<BTreeMap<String, SpeakerMeta>> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let path = entry?.path().join("meta.toml");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    let mut speakers = BTreeMap::new();
    for path in paths {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let speaker: SpeakerMeta =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let name = speaker.name.clone();
        if speakers.insert(name.clone(), speaker).is_some() {
            anyhow::bail!("duplicate speaker metadata: {name} ({})", path.display());
        }
    }
    Ok(speakers)
}

/// Loads `origins.toml` (measurement origins).
///
/// # Errors
///
/// Returns an error when the file cannot be read or parsed.
pub fn load_origins(path: &Path) -> Result<BTreeMap<String, OriginInfo>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: OriginsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(file.origins)
}

/// Builds the flat batch catalogue from a metadata directory.
///
/// Entries are sorted by speaker, then version. Speakers flagged
/// `skip` produce no entries.
///
/// # Errors
///
/// Returns an error when the metadata fails to load.
pub fn catalogue_from_dir(root: &Path) -> Result<Vec<CatalogueEntry>> {
    let speakers = load_metadata_dir(root)?;
    let mut entries = Vec::new();
    for (name, speaker) in &speakers {
        if speaker.skip {
            continue;
        }
        let speaker_root = root.join(sanitize_filename(name));
        for (version, measurement) in &speaker.measurements {
            entries.push(CatalogueEntry {
                speaker: name.clone(),
                version: version.clone(),
                origin: measurement.origin.clone(),
                format: measurement.format.as_str().to_owned(),
                symmetry: measurement.symmetry.map(|s| s.as_str().to_owned()),
                speaker_dir: speaker_root.join(version),
                is_default: *version == speaker.default_measurement,
                shape: Some(speaker.shape.as_str().to_owned()),
                min_valid_freq: measurement
                    .data_acquisition
                    .as_ref()
                    .and_then(|d| d.min_valid_freq),
                max_valid_freq: measurement
                    .data_acquisition
                    .as_ref()
                    .and_then(|d| d.max_valid_freq),
            });
        }
    }
    Ok(entries)
}

/// Result of a metadata validation run.
#[derive(Debug, Default)]
pub struct CheckReport {
    /// Hard failures (structural or semantic).
    pub errors: Vec<String>,
    /// Non-fatal range warnings.
    pub warnings: Vec<String>,
    /// Number of speakers checked.
    pub speakers: usize,
}

/// Validates a metadata directory: strict schema (via loading),
/// cross-field rules, known origins, and measurement directories.
///
/// # Errors
///
/// Returns an error only when the origins file cannot be loaded;
/// per-speaker problems are collected in the report.
pub fn check_metadata(root: &Path) -> Result<CheckReport> {
    let mut report = CheckReport::default();
    let origins = load_origins(&root.join("../origins.toml"))?;
    let speakers = match load_metadata_dir(root) {
        Ok(speakers) => speakers,
        Err(error) => {
            report.errors.push(format!("{error:#}"));
            return Ok(report);
        }
    };
    report.speakers = speakers.len();
    for (name, speaker) in &speakers {
        if !speaker
            .measurements
            .contains_key(&speaker.default_measurement)
        {
            report.errors.push(format!(
                "{name}: default_measurement '{}' not in measurements",
                speaker.default_measurement
            ));
        }
        let speaker_root = root.join(sanitize_filename(name));
        for (version, measurement) in &speaker.measurements {
            if !origins.contains_key(&measurement.origin) {
                report.errors.push(format!(
                    "{name}/{version}: unknown origin '{}'",
                    measurement.origin
                ));
            }
            // Partial dates like `2024xxxx` are used for in-progress
            // reviews, so this is a warning, not an error.
            if let Some(date) = &measurement.review_published
                && !(date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
            {
                report.warnings.push(format!(
                    "{name}/{version}: review_published '{date}' is not YYYYMMDD"
                ));
            }
            if let Some(specs) = &measurement.specifications {
                if let Some(sensitivity) = specs.sensitivity
                    && !(70.0..=120.0).contains(&sensitivity)
                {
                    report.warnings.push(format!(
                        "{name}/{version}: sensitivity {sensitivity} dB outside 70..=120"
                    ));
                }
                if let Some(impedance) = specs.impedance
                    && !(2.0..=16.0).contains(&impedance)
                {
                    report.warnings.push(format!(
                        "{name}/{version}: impedance {impedance} ohm outside 2..=16"
                    ));
                }
            }
            if !speaker.skip {
                let dir = speaker_root.join(version);
                if !dir.is_dir() {
                    report.errors.push(format!(
                        "{name}/{version}: missing directory {}",
                        dir.display()
                    ));
                }
            }
        }
    }
    Ok(report)
}
