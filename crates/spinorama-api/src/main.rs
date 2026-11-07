//! `spinorama-api`: low-resource Rust API server for spinorama.org.
//!
//! Drop-in replacement for the FastAPI/Gunicorn service: same routes,
//! same Bearer-key SQLite database, same per-key fingerprinting, but a
//! single small process (no Python interpreter, no worker pool). It
//! serves a handful of requests per second; throughput is not a goal,
//! memory and CPU headroom on a 2 GB VPS are.
//!
//! Usage:
//!   spinorama-api serve            # HTTP server (default)
//!   spinorama-api keys init|issue|list|revoke ...
//!
//! Configuration is via the same environment variables as the Python
//! service (`SPINORAMA_API_FILES`, `SPINORAMA_SPINFILES`,
//! `SPINORAMA_API_KEY_DB`, `SPINORAMA_FINGERPRINT_SECRET`,
//! `SPINORAMA_FINGERPRINT_AMPLITUDE_DB`) plus `SPINORAMA_API_BIND`
//! (`unix:/path.sock` or `tcp:127.0.0.1:8000`, default the latter).

// Rust guideline compliant 2026-02-21

mod compare;
mod fingerprint;
mod keys;
mod validate;

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

const SOFTWARE_VERSION: &str = "v1.0";

const KNOWN_MEASUREMENTS: &[&str] = &[
    "CEA2034",
    "On Axis",
    "Estimated In-Room Response",
    "Early Reflections",
    "Horizontal Reflections",
    "Vertical Reflections",
    "SPL Horizontal",
    "SPL Horizontal Normalized",
    "SPL Vertical",
    "SPL Vertical Normalized",
    "SPL Horizontal Contour",
    "SPL Horizontal Contour Normalized",
    "SPL Vertical Contour",
    "SPL Vertical Contour Normalized",
    "SPL Horizontal Contour 3D",
    "SPL Horizontal Contour Normalized 3D",
    "SPL Vertical Contour 3D",
    "SPL Vertical Contour Normalized 3D",
    "SPL Horizontal Globe",
    "SPL Horizontal Globe Normalized",
    "SPL Vertical Globe",
    "SPL Vertical Globe Normalized",
    "SPL Horizontal Radar",
    "SPL Vertical Radar",
];

/// Filtered metadata store: speaker name -> serialized JSON record.
/// Records are kept as text so the resident set stays close to the
/// on-disk size instead of a fully materialized value tree.
struct Store {
    records: BTreeMap<String, String>,
    brands: Vec<String>,
    names: Vec<String>,
    origins: Vec<String>,
}

impl Store {
    fn empty() -> Self {
        Self {
            records: BTreeMap::new(),
            brands: Vec::new(),
            names: Vec::new(),
            origins: Vec::new(),
        }
    }

    fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let parsed: Value =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let filtered = filter_api_measurements(&parsed);
        let mut records = BTreeMap::new();
        let mut brand_set: Vec<String> = Vec::new();
        let mut origins: Vec<String> = Vec::new();
        if let Some(obj) = filtered.as_object() {
            for (name, record) in obj {
                if let Some(brand) = record.get("brand").and_then(Value::as_str)
                    && !brand_set.iter().any(|b| b == brand)
                {
                    brand_set.push(brand.to_owned());
                }
                if let Some(measurements) = record.get("measurements").and_then(Value::as_object) {
                    for measurement in measurements.values() {
                        if let Some(origin) = measurement.get("origin").and_then(Value::as_str)
                            && !origins.iter().any(|o| o == origin)
                        {
                            origins.push(origin.to_owned());
                        }
                    }
                }
                records.insert(
                    name.clone(),
                    serde_json::to_string(record).unwrap_or_default(),
                );
            }
        }
        brand_set.sort();
        let names: Vec<String> = records.keys().cloned().collect();
        Ok(Self {
            records,
            brands: brand_set,
            names,
            origins,
        })
    }

    fn parse_record(&self, name: &str) -> Option<Value> {
        self.records
            .get(name)
            .and_then(|text| serde_json::from_str(text).ok())
    }
}

/// Removes ErinsAudioCorner measurements (Python `filter_api_measurements`).
fn filter_api_measurements(metadata: &Value) -> Value {
    let mut out = Map::new();
    let Some(speakers) = metadata.as_object() else {
        return Value::Object(out);
    };
    for (name, record) in speakers {
        let Some(measurements) = record.get("measurements").and_then(Value::as_object) else {
            continue;
        };
        let kept: Map<String, Value> = measurements
            .iter()
            .filter(|(_, measurement)| {
                measurement
                    .get("origin")
                    .and_then(Value::as_str)
                    .is_none_or(|origin| origin.to_lowercase() != "erinsaudiocorner")
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if kept.is_empty() {
            continue;
        }
        let mut public = record.clone();
        if let Some(obj) = public.as_object_mut() {
            let default_missing = obj
                .get("default_measurement")
                .and_then(Value::as_str)
                .is_none_or(|default| !kept.contains_key(default));
            obj.insert("measurements".to_owned(), Value::Object(kept));
            if default_missing {
                if let Some(first) = obj
                    .get("measurements")
                    .and_then(Value::as_object)
                    .and_then(|m| m.keys().next().cloned())
                {
                    obj.insert("default_measurement".to_owned(), Value::from(first));
                }
                for key in ["default_eq", "eqs", "nearest"] {
                    obj.remove(key);
                }
            }
        }
        out.insert(name.clone(), public);
    }
    Value::Object(out)
}

struct State {
    speakers: Store,
    headphones: Store,
    spin_files: PathBuf,
    headphone_files: PathBuf,
    origins: Vec<String>,
    /// Public demo set: a fixed sample of ASR speakers (see
    /// [`select_demo_speakers`]), served without an API key under
    /// `/demo/v1/...`.
    demo_speakers: Vec<String>,
    /// On-the-fly comparison pictures (`/compare.webp`): disk cache,
    /// render lock, and per-minute render budget.
    compare: compare::CompareState,
}

/// How many speakers the public demo exposes.
const DEMO_COUNT: usize = 10;
/// Seed of the demo selection: the 2024 ASR catalogue. The selection
/// is a deterministic shuffle of the sorted candidate names, so every
/// server (and every restart) serves the same 10 speakers.
const DEMO_SEED: u64 = 2024;

/// SplitMix64: tiny deterministic PRNG for the demo shuffle.
struct DemoRng(u64);

impl DemoRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Picks the demo speakers out of `candidates` (speaker names whose
/// ASR measurement was published in 2024): a seeded Fisher–Yates
/// shuffle of the sorted names, first [`DEMO_COUNT`], sorted again
/// for stable listings. Always returns the same set for the same
/// catalogue.
fn select_demo_speakers(candidates: &[String]) -> Vec<String> {
    let mut shuffled: Vec<String> = candidates.to_vec();
    shuffled.sort();
    let mut rng = DemoRng(DEMO_SEED);
    for i in (1..shuffled.len()).rev() {
        let j = usize::try_from(rng.next() % (i as u64 + 1)).unwrap_or(0);
        shuffled.swap(i, j);
    }
    shuffled.truncate(DEMO_COUNT);
    shuffled.sort();
    shuffled
}

/// Names of speakers with an ASR measurement published in 2024.
fn demo_candidates(speakers: &Store) -> Vec<String> {
    let mut candidates = Vec::new();
    for name in &speakers.names {
        let Some(record) = speakers.parse_record(name) else {
            continue;
        };
        let is_demo = record
            .get("measurements")
            .and_then(Value::as_object)
            .is_some_and(|measurements| {
                measurements.values().any(|measurement| {
                    measurement
                        .get("origin")
                        .and_then(Value::as_str)
                        .is_some_and(|origin| vendor_stripped(origin) == "ASR")
                        && measurement
                            .get("review_published")
                            .and_then(Value::as_str)
                            .is_some_and(|date| date.starts_with("2024"))
                })
            });
        if is_demo {
            candidates.push(name.clone());
        }
    }
    candidates
}

/// The speaker record as the demo may see it: only ASR measurements,
/// and only for speakers in the demo set.
fn demo_record(state: &State, name: &str) -> Option<Value> {
    if !state.demo_speakers.iter().any(|demo| demo == name) {
        return None;
    }
    let mut record = state.speakers.parse_record(name)?;
    let measurements = record.get("measurements").and_then(Value::as_object)?;
    let kept: Map<String, Value> = measurements
        .iter()
        .filter(|(_, measurement)| {
            measurement
                .get("origin")
                .and_then(Value::as_str)
                .is_some_and(|origin| vendor_stripped(origin) == "ASR")
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.is_empty() {
        return None;
    }
    if let Some(obj) = record.as_object_mut() {
        let default_missing = obj
            .get("default_measurement")
            .and_then(Value::as_str)
            .is_none_or(|default| !kept.contains_key(default));
        obj.insert("measurements".to_owned(), Value::Object(kept));
        if default_missing
            && let Some(first) = obj
                .get("measurements")
                .and_then(Value::as_object)
                .and_then(|m| m.keys().next().cloned())
        {
            obj.insert("default_measurement".to_owned(), Value::from(first));
        }
    }
    Some(record)
}

fn env_path(name: &str, default: impl Fn() -> PathBuf) -> PathBuf {
    std::env::var_os(name).map_or_else(default, PathBuf::from)
}

fn load_state() -> Result<State> {
    let api_files = env_path("SPINORAMA_API_FILES", || {
        PathBuf::from("/var/www/html/spinorama-api")
    });
    let spin_files = env_path("SPINORAMA_SPINFILES", || api_files.join("speakers"));
    let metadata_path = api_files.join("assets/metadata.json");
    if !metadata_path.is_file() {
        bail!("Cannot find {}", metadata_path.display());
    }
    let speakers = Store::load(&metadata_path)?;
    let headphone_path = api_files.join("assets/headphone.json");
    let headphone_path = if headphone_path.is_file() {
        headphone_path
    } else {
        api_files.join("assets/headphone_metadata.json")
    };
    let headphones = if headphone_path.is_file() {
        Store::load(&headphone_path)?
    } else {
        eprintln!(
            "spinorama-api: cannot find {}, headphone endpoints disabled",
            headphone_path.display()
        );
        Store::empty()
    };
    let mut origins: Vec<String> = Vec::new();
    if let Ok(origins_path) = std::env::var("SPINORAMA_ORIGINS_TOML")
        && let Ok(text) = std::fs::read_to_string(&origins_path)
        && let Ok(parsed) = text.parse::<toml::Table>()
        && let Some(table) = parsed.get("origins").and_then(toml::Value::as_table)
    {
        origins = table.keys().cloned().collect();
    }
    if origins.is_empty() {
        // Fall back to the origins collected while loading metadata.
        for store in [&speakers, &headphones] {
            for origin in &store.origins {
                if !origins.contains(origin) {
                    origins.push(origin.clone());
                }
            }
        }
    }
    let demo_speakers = select_demo_speakers(&demo_candidates(&speakers));
    Ok(State {
        speakers,
        headphones,
        spin_files,
        headphone_files: api_files.join("assets/headphones"),
        origins,
        demo_speakers,
        compare: compare::CompareState::new(env_path("SPINORAMA_COMPARE_CACHE", || {
            PathBuf::from("dist/compare-cache")
        })),
    })
}

fn percent_decode(input: &str, plus_as_space: bool) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                };
                if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    out.push(hi * 16 + lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' if plus_as_space => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('\0')
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
}

fn safe_path(base: &Path, parts: &[&str]) -> Option<PathBuf> {
    for part in parts {
        if !safe_segment(part) {
            return None;
        }
    }
    let mut candidate = base.to_path_buf();
    for part in parts {
        candidate.push(part);
    }
    // Parts are single segments without separators or dotdots, so the
    // join cannot escape the base.
    Some(candidate)
}

fn vendor_stripped(origin: &str) -> &str {
    origin.strip_prefix("Vendors-").unwrap_or(origin)
}

struct Request {
    method: String,
    /// Percent-decoded path segments (empty segments dropped).
    segments: Vec<String>,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    extra_headers: Vec<(String, String)>,
}

impl Response {
    fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: "application/json".to_owned(),
            body: serde_json::to_vec(value).unwrap_or_default(),
            extra_headers: Vec::new(),
        }
    }

    fn json_text(status: u16, text: String) -> Self {
        Self {
            status,
            content_type: "application/json".to_owned(),
            body: text.into_bytes(),
            extra_headers: Vec::new(),
        }
    }

    fn error(message: &str) -> Self {
        Self::json(200, &serde_json::json!({ "error": message }))
    }

    fn unauthorized() -> Self {
        let mut response = Self::json(
            401,
            &serde_json::json!({
                "detail": "A valid API key is required. See /docs to request access."
            }),
        );
        response
            .extra_headers
            .push(("WWW-Authenticate".to_owned(), "Bearer".to_owned()));
        response
    }

    fn unavailable() -> Self {
        Self::json(
            503,
            &serde_json::json!({ "detail": "API key service unavailable" }),
        )
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

fn authenticate(request: &Request) -> std::result::Result<(String, String), Response> {
    let Some(header) = request.headers.get("authorization") else {
        return Err(Response::unauthorized());
    };
    let mut parts = header.splitn(2, ' ');
    let scheme = parts.next().unwrap_or("");
    let token = parts.next().unwrap_or("").trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || token.len() > 256 {
        return Err(Response::unauthorized());
    }
    match keys::lookup(token) {
        Ok(Some(account)) => Ok(account),
        Ok(None) => Err(Response::unauthorized()),
        Err(error) => {
            eprintln!("spinorama-api: key lookup failed: {error:#}");
            Err(Response::unavailable())
        }
    }
}

fn query_value<'a>(request: &'a Request, key: &str) -> Option<&'a str> {
    request.query.get(key).map(String::as_str)
}

fn speaker_measurements_list(state: &State, segments: &[String], demo: bool) -> Response {
    let (speaker_name, speaker_version) = (&segments[2], &segments[4]);
    if !safe_segment(speaker_name) || !safe_segment(speaker_version) {
        return Response::error(&format!(
            "Invalid speaker_name {speaker_name} or speaker_version {speaker_version}!"
        ));
    }
    if demo && !state.demo_speakers.iter().any(|name| name == speaker_name) {
        return Response::error(&format!(
            "Speaker {speaker_name} is not part of the demo set!"
        ));
    }
    let Some(record) = state.speakers.parse_record(speaker_name) else {
        return Response::error(&format!("Speaker {speaker_name} is not in our database!"));
    };
    let Some(measurements) = record.get("measurements").and_then(Value::as_object) else {
        return Response::error("No measurements");
    };
    let Some(measurement) = measurements.get(speaker_version) else {
        let valid = measurements.keys().cloned().collect::<Vec<_>>().join(", ");
        return Response::error(&format!(
            "Version {speaker_version} is not known for speaker {speaker_name}! Valid keys are ({valid})."
        ));
    };
    let origin = vendor_stripped(
        measurement
            .get("origin")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    if !safe_segment(origin) {
        return Response::error(&format!("Invalid origin {origin}!"));
    }
    if demo && origin != "ASR" {
        return Response::error("Demo access is limited to ASR measurements!");
    }
    let Some(upper_dir) = safe_path(&state.spin_files, &[speaker_name]) else {
        return Response::error(&format!("Invalid path for speaker {speaker_name}!"));
    };
    let Some(dir_data) = safe_path(&upper_dir, &[origin, speaker_version]) else {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements for origin {origin} and version {speaker_version}!"
        ));
    };
    if !upper_dir.exists() {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements!"
        ));
    }
    if !dir_data.exists() {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements for origin {origin} and version {speaker_version}!"
        ));
    }
    let mut stems: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir_data) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains('.') {
                let stem = name.split('.').next().unwrap_or("").to_owned();
                if !stems.contains(&stem) {
                    stems.push(stem);
                }
            }
        }
    }
    stems.sort();
    Response::json(
        200,
        &Value::Array(stems.into_iter().map(Value::from).collect()),
    )
}

fn speaker_measurement_data(
    state: &State,
    segments: &[String],
    request: &Request,
    key_id: &str,
    demo: bool,
) -> Response {
    let (speaker_name, speaker_version) = (&segments[2], &segments[4]);
    let mut measurement_name = segments[6].clone();
    let measurement_format = query_value(request, "format")
        .or_else(|| query_value(request, "measurement_format"))
        .unwrap_or("json")
        .to_owned();
    if !safe_segment(speaker_name)
        || !safe_segment(speaker_version)
        || !safe_segment(&measurement_name)
    {
        return Response::error(&format!(
            "Invalid speaker_name {speaker_name}, speaker_version {speaker_version} or measurement_name {measurement_name}!"
        ));
    }
    if demo && !state.demo_speakers.iter().any(|name| name == speaker_name) {
        return Response::error(&format!(
            "Speaker {speaker_name} is not part of the demo set!"
        ));
    }
    let Some(record) = state.speakers.parse_record(speaker_name) else {
        return Response::error(&format!("Speaker {speaker_name} is not in our database!"));
    };
    let Some(measurements) = record.get("measurements").and_then(Value::as_object) else {
        return Response::error("No measurements");
    };
    let Some(measurement) = measurements.get(speaker_version) else {
        let valid = measurements.keys().cloned().collect::<Vec<_>>().join(", ");
        return Response::error(&format!(
            "Version {speaker_version} is not known for speaker {speaker_name}! Valid keys are ({valid})."
        ));
    };
    let origin = vendor_stripped(
        measurement
            .get("origin")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    if !safe_segment(origin) {
        return Response::error(&format!("Invalid origin {origin}!"));
    }
    if demo && origin != "ASR" {
        return Response::error("Demo access is limited to ASR measurements!");
    }
    let Some(upper_dir) = safe_path(&state.spin_files, &[speaker_name]) else {
        return Response::error(&format!("Invalid path for speaker {speaker_name}!"));
    };
    let Some(dir_data) = safe_path(&upper_dir, &[origin, speaker_version]) else {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements for origin {origin} and version {speaker_version}!"
        ));
    };
    if !upper_dir.exists() {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements!"
        ));
    }
    if !dir_data.exists() {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed measurements for origin {origin} and version {speaker_version}!"
        ));
    }
    if let Some(stripped) = measurement_name.strip_suffix("_unmelted") {
        measurement_name = stripped.to_owned();
    }
    if !KNOWN_MEASUREMENTS.contains(&measurement_name.as_str()) {
        return Response::error(&format!(
            "Version {measurement_name} is not known! Valid options are ({}).",
            KNOWN_MEASUREMENTS.join(", ")
        ));
    }
    if measurement_format != "json" {
        return Response::error(&format!(
            "Version {measurement_format} is not known! Only valid options is None or json."
        ));
    }
    let file_name = format!("{measurement_name}.{measurement_format}");
    let Some(measurement_file) = safe_path(&dir_data, &[&file_name]) else {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed {measurement_name} in format {measurement_format} for origin {origin} and version {speaker_version}!"
        ));
    };
    if !measurement_file.exists() {
        return Response::error(&format!(
            "Speaker {speaker_name} does not have precomputed {measurement_name} in format {measurement_format} for origin {origin} and version {speaker_version}!"
        ));
    }
    let resource = format!("{speaker_name}/{origin}/{speaker_version}/{measurement_name}");
    eprintln!("download key={key_id} resource={resource} format={measurement_format}");
    let Ok(text) = std::fs::read_to_string(&measurement_file) else {
        return Response::error("fetching measurements failed");
    };
    let secret = fingerprint::secret_from_env();
    if secret.is_empty() || demo {
        return Response::json(200, &Value::Array(vec![Value::from(text)]));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(payload) if payload.is_object() => {
            let fingerprinted = fingerprint::apply_to_figure(
                &payload,
                key_id,
                &resource,
                &secret,
                fingerprint::amplitude_from_env(),
            );
            let inner = serde_json::to_string(&fingerprinted).unwrap_or(text);
            Response::json(200, &Value::Array(vec![Value::from(inner)]))
        }
        _ => Response::json(200, &Value::Array(vec![Value::from(text)])),
    }
}

fn headphone_frequency_response(
    state: &State,
    segments: &[String],
    request: &Request,
    key_id: &str,
) -> Response {
    let headphone_name = &segments[2];
    let Some(record) = state.headphones.parse_record(headphone_name) else {
        return Response::error(&format!(
            "Headphone {headphone_name} is not in our database!"
        ));
    };
    if !safe_segment(headphone_name) {
        return Response::error(&format!("Invalid headphone_name {headphone_name}!"));
    }
    let measurements = record
        .get("measurements")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let meas_key = query_value(request, "version")
        .map(ToOwned::to_owned)
        .or_else(|| {
            record
                .get("default_measurement")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default();
    if meas_key.is_empty() || !measurements.contains_key(&meas_key) {
        let valid: Vec<&String> = measurements.keys().collect();
        return Response::error(&format!(
            "Unknown version '{meas_key}' for {headphone_name}. Valid: {valid:?}"
        ));
    }
    if !safe_segment(&meas_key) {
        return Response::error(&format!("Invalid version {meas_key}!"));
    }
    let Some(csv_path) = safe_path(
        &state.headphone_files,
        &[headphone_name, &meas_key, "frequency_response.csv"],
    ) else {
        return Response::error(&format!("Invalid path for {headphone_name} ({meas_key})"));
    };
    if !csv_path.exists() {
        return Response::error(&format!(
            "No frequency response data for {headphone_name} ({meas_key})"
        ));
    }
    let resource = format!("{headphone_name}/{meas_key}/frequency_response");
    eprintln!("download key={key_id} resource={resource} format=csv");
    let secret = fingerprint::secret_from_env();
    let Ok(text) = std::fs::read_to_string(&csv_path) else {
        return Response::error("fetching frequency response failed");
    };
    let body = if secret.is_empty() {
        text
    } else {
        fingerprint::apply_to_csv_text(
            &text,
            key_id,
            &resource,
            &secret,
            fingerprint::amplitude_from_env(),
        )
    };
    Response {
        status: 200,
        content_type: "text/csv; charset=utf-8".to_owned(),
        body: body.into_bytes(),
        extra_headers: Vec::new(),
    }
}

fn route(state: &State, request: &Request) -> Response {
    let segments: Vec<&str> = request.segments.iter().map(String::as_str).collect();
    // Public documentation, as with FastAPI.
    match segments.as_slice() {
        ["docs"] => return docs_response("swagger"),
        ["redoc"] => return docs_response("redoc"),
        ["openapi.json"] => {
            return Response::json_text(200, openapi_spec());
        }
        _ => {}
    }
    // Public comparison picture, rendered from the pristine bundles
    // and hardened so the underlying data is never exposed. Query:
    // `measurement`, `a_speaker/a_origin/a_version`,
    // `b_speaker/b_origin/b_version`, `w` (900 or 1400). The website
    // requests `/compare-render/compare.webp` (nginx snippet location),
    // the API's public route is `/compare.webp`; accept both so the
    // www host can proxy the original URI straight to this socket,
    // replacing the Python compare service on port 8765.
    if segments.as_slice() == ["compare.webp"]
        || segments.as_slice() == ["compare-render", "compare.webp"]
    {
        return match compare::serve(&state.spin_files, &state.compare, &request.query) {
            Ok(picture) => Response {
                status: 200,
                content_type: "image/webp".to_owned(),
                body: picture.body,
                extra_headers: vec![
                    (
                        "Cache-Control".to_owned(),
                        "public, max-age=31536000, immutable".to_owned(),
                    ),
                    (
                        "X-Compare-Cache".to_owned(),
                        if picture.cache_hit { "hit" } else { "miss" }.to_owned(),
                    ),
                ],
            },
            Err(failure) => {
                let mut response = Response {
                    status: failure.status,
                    content_type: "text/plain; charset=utf-8".to_owned(),
                    body: format!("{}\n", failure.message).into_bytes(),
                    extra_headers: Vec::new(),
                };
                if let Some(retry) = failure.retry_after {
                    response
                        .extra_headers
                        .push(("Retry-After".to_owned(), retry.to_string()));
                }
                response
            }
        };
    }
    // `/demo/v1/...` mirrors the speaker routes without an API key,
    // restricted to the demo set and its ASR measurements.
    let demo = segments.first() == Some(&"demo");
    let effective: Vec<String> = if demo {
        request.segments.iter().skip(1).cloned().collect()
    } else {
        request.segments.clone()
    };
    let segments: Vec<&str> = effective.iter().map(String::as_str).collect();
    let is_speaker_route = matches!(
        segments.as_slice(),
        ["v1", "brands"]
            | ["v1", "speakers"]
            | ["v1", "speaker", _, "metadata"]
            | ["v1", "speaker", _, "versions"]
            | ["v1", "speaker", _, "version", _, "measurements"]
            | ["v1", "speaker", _, "version", _, "measurements", _]
    );
    let is_api_route = is_speaker_route
        || matches!(
            segments.as_slice(),
            ["v1", "headphones"]
                | ["v1", "headphone", "brands"]
                | ["v1", "headphone", "shapes"]
                | ["v1", "headphone", _, "metadata"]
                | ["v1", "headphone", _, "versions"]
                | ["v1", "headphone", _, "frequency_response"]
                | ["v1", "validate"]
        );
    if !is_api_route || (demo && !is_speaker_route) {
        return Response::json(404, &serde_json::json!({ "detail": "Not Found" }));
    }
    let (key_id, _email) = if demo {
        ("demo".to_owned(), String::new())
    } else {
        match authenticate(request) {
            Ok(account) => account,
            Err(response) => return response,
        }
    };

    match segments.as_slice() {
        ["v1", "brands"] => {
            if demo {
                let mut brands: Vec<String> = Vec::new();
                for name in &state.demo_speakers {
                    if let Some(brand) = state.speakers.parse_record(name).and_then(|record| {
                        record
                            .get("brand")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    }) && !brands.contains(&brand)
                    {
                        brands.push(brand);
                    }
                }
                brands.sort();
                Response::json(
                    200,
                    &Value::Array(brands.into_iter().map(Value::from).collect()),
                )
            } else {
                Response::json(
                    200,
                    &Value::Array(
                        state
                            .speakers
                            .brands
                            .iter()
                            .cloned()
                            .map(Value::from)
                            .collect(),
                    ),
                )
            }
        }
        ["v1", "speakers"] => Response::json(
            200,
            &Value::Array(
                if demo {
                    &state.demo_speakers
                } else {
                    &state.speakers.names
                }
                .iter()
                .cloned()
                .map(Value::from)
                .collect(),
            ),
        ),
        ["v1", "speaker", name, "metadata"] => {
            if demo {
                match demo_record(state, name) {
                    Some(record) => Response::json(200, &record),
                    None => Response::error("Speaker not found"),
                }
            } else {
                match state.speakers.records.get(*name) {
                    Some(raw) => Response::json_text(200, raw.clone()),
                    None => Response::error("Speaker not found"),
                }
            }
        }
        ["v1", "speaker", name, "versions"] => {
            let record = if demo {
                demo_record(state, name)
            } else {
                state.speakers.parse_record(name)
            };
            let Some(record) = record else {
                return Response::error(&format!("Speaker {name} is not in our database!"));
            };
            let versions: Vec<Value> = record
                .get("measurements")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().map(Value::from).collect())
                .unwrap_or_default();
            Response::json(200, &Value::Array(versions))
        }
        ["v1", "speaker", _, "version", _, "measurements"] => {
            speaker_measurements_list(state, &effective, demo)
        }
        ["v1", "speaker", _, "version", _, "measurements", _] => {
            speaker_measurement_data(state, &effective, request, &key_id, demo)
        }
        ["v1", "headphones"] => {
            let brand = query_value(request, "brand").map(str::to_lowercase);
            let shape = query_value(request, "shape").map(str::to_lowercase);
            let recommendation = query_value(request, "recommendation").map(str::to_lowercase);
            let mut names = Vec::new();
            for name in &state.headphones.names {
                let Some(record) = state.headphones.parse_record(name) else {
                    continue;
                };
                if let Some(want) = &brand
                    && record
                        .get("brand")
                        .and_then(Value::as_str)
                        .is_none_or(|b| b.to_lowercase() != *want)
                {
                    continue;
                }
                if let Some(want) = &shape
                    && record
                        .get("shape")
                        .and_then(Value::as_str)
                        .is_none_or(|s| s.to_lowercase() != *want)
                {
                    continue;
                }
                if let Some(want) = &recommendation {
                    let matches = record
                        .get("measurements")
                        .and_then(Value::as_object)
                        .is_some_and(|measurements| {
                            measurements.values().any(|m| {
                                m.get("recommendation")
                                    .and_then(Value::as_str)
                                    .is_some_and(|r| r.to_lowercase() == *want)
                            })
                        });
                    if !matches {
                        continue;
                    }
                }
                names.push(Value::from(name.clone()));
            }
            Response::json(200, &Value::Array(names))
        }
        ["v1", "headphone", "brands"] => Response::json(
            200,
            &Value::Array(
                state
                    .headphones
                    .brands
                    .iter()
                    .cloned()
                    .map(Value::from)
                    .collect(),
            ),
        ),
        ["v1", "headphone", "shapes"] => {
            let mut shapes: Vec<String> = Vec::new();
            for name in &state.headphones.names {
                if let Some(shape) = state.headphones.parse_record(name).and_then(|r| {
                    r.get("shape")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }) && !shapes.contains(&shape)
                {
                    shapes.push(shape);
                }
            }
            shapes.sort();
            Response::json(
                200,
                &Value::Array(shapes.into_iter().map(Value::from).collect()),
            )
        }
        ["v1", "headphone", name, "metadata"] => match state.headphones.records.get(*name) {
            Some(raw) => Response::json_text(200, raw.clone()),
            None => Response::error("Headphone not found"),
        },
        ["v1", "headphone", name, "versions"] => {
            let Some(record) = state.headphones.parse_record(name) else {
                return Response::error(&format!("Headphone {name} is not in our database!"));
            };
            let versions: Vec<Value> = record
                .get("measurements")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().map(Value::from).collect())
                .unwrap_or_default();
            Response::json(200, &Value::Array(versions))
        }
        ["v1", "headphone", _, "frequency_response"] => {
            headphone_frequency_response(state, &request.segments, request, &key_id)
        }
        ["v1", "validate"] => {
            if request.method != "POST" {
                return Response::json(405, &serde_json::json!({ "detail": "Method Not Allowed" }));
            }
            let parsed: std::result::Result<Value, _> = serde_json::from_slice(&request.body);
            match parsed {
                Ok(data) => {
                    let brand = data
                        .get("brand")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown");
                    let model = data
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown");
                    let speaker_name = format!("{brand} {model}");
                    let (valid, messages) =
                        validate::validate_speaker_data(&speaker_name, &data, &state.origins);
                    Response::json(
                        200,
                        &serde_json::json!({
                            "valid": valid,
                            "messages": messages,
                            "speaker_name": speaker_name,
                        }),
                    )
                }
                Err(_) => Response::json(
                    200,
                    &serde_json::json!({
                        "valid": false,
                        "messages": ["ERROR: Invalid JSON format"],
                        "speaker_name": "Unknown",
                    }),
                ),
            }
        }
        _ => Response::json(404, &serde_json::json!({ "detail": "Not Found" })),
    }
}

fn docs_response(kind: &str) -> Response {
    let html = if kind == "redoc" {
        "<!DOCTYPE html><html><head><title>Spinorama API</title></head><body>\
         <redoc spec-url=\"/openapi.json\"></redoc>\
         <script src=\"https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js\"></script>\
         </body></html>"
    } else {
        "<!DOCTYPE html><html><head><title>Spinorama API</title>\
         <link rel=\"stylesheet\" href=\"https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css\">\
         </head><body><div id=\"ui\"></div>\
         <script src=\"https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js\"></script>\
         <script>window.onload=()=>{SwaggerUIBundle({url:'/openapi.json',dom_id:'#ui',persistAuthorization:false})};</script>\
         </body></html>"
    };
    Response {
        status: 200,
        content_type: "text/html; charset=utf-8".to_owned(),
        body: html.as_bytes().to_vec(),
        extra_headers: Vec::new(),
    }
}

fn openapi_spec() -> String {
    let description = "## Request an API key\n\nAPI access requires **manual approval** \
        and a personal API key. There is no public registration or automatic approval.\n\n\
        Email pierre@spinorama.org with the subject **[AUTH]: API access request — your name or project**.\n\n\
        In your email, explain:\n\n- **Who you are:** your name and, if relevant, organization or project.\n\
        - **Why you want access:** what you plan to build or research and how you will use the data.\n\
        - Whether you intend to publish or redistribute data, and whether your use is commercial.\n\n\
        Missing, invalid, expired or revoked keys return **401 Unauthorized**.\n\n\
        ## Demo\n\nThe `/demo/v1/...` prefix mirrors the speaker routes without a key, \
        for a fixed demo set of 10 speakers measured by ASR (Audio Science Review) \
        and published in 2024. Only those speakers and their ASR measurements are \
        available there, with pristine (unfingerprinted) data.\n\n\
        Curve responses carry an imperceptible per-key variation (at most 0.05 dB) \
        used only to identify the source of leaked data. ErinsAudioCorner measurements \
        are not available through this API.\n\n## Measurement data license\n\n\
        Unless otherwise stated, measurement data is licensed under CC BY-NC-SA 4.0. \
        Commercial use is not permitted under this license.";
    let operation = |summary: &str| {
        serde_json::json!({
            "summary": summary,
            "security": [{ "APIKey": [] }],
            "responses": { "200": { "description": "OK" } }
        })
    };
    let demo_operation = |summary: &str| {
        serde_json::json!({
            "summary": summary,
            "responses": { "200": { "description": "OK" } }
        })
    };
    let spec = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Spinorama API",
            "description": description,
            "version": SOFTWARE_VERSION,
            "license": {
                "name": "Measurement data: CC BY-NC-SA 4.0 (unless otherwise stated)",
                "url": "https://creativecommons.org/licenses/by-nc-sa/4.0/"
            }
        },
        "tags": [
            { "name": "speaker", "description": "Speaker measurements and metadata" },
            { "name": "headphone", "description": "Headphone measurements and metadata" },
            { "name": "validation", "description": "Data validation endpoints" }
        ],
        "components": {
            "securitySchemes": {
                "APIKey": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Request a key from pierre@spinorama.org (subject: [AUTH]: API access request). Paste only your key here, without the Bearer prefix."
                }
            }
        },
        "paths": {
            "/v1/brands": { "get": operation("List brands") },
            "/v1/speakers": { "get": operation("List speakers") },
            "/v1/speaker/{speaker_name}/metadata": { "get": operation("Speaker metadata") },
            "/v1/speaker/{speaker_name}/versions": { "get": operation("Speaker versions") },
            "/v1/speaker/{speaker_name}/version/{speaker_version}/measurements": { "get": operation("Measurement names") },
            "/v1/speaker/{speaker_name}/version/{speaker_version}/measurements/{measurement_name}": { "get": operation("Measurement data") },
            "/v1/headphones": { "get": operation("List headphones") },
            "/v1/headphone/brands": { "get": operation("Headphone brands") },
            "/v1/headphone/shapes": { "get": operation("Headphone shapes") },
            "/v1/headphone/{headphone_name}/metadata": { "get": operation("Headphone metadata") },
            "/v1/headphone/{headphone_name}/versions": { "get": operation("Headphone versions") },
            "/v1/headphone/{headphone_name}/frequency_response": { "get": operation("Headphone frequency response (CSV)") },
            "/demo/v1/brands": { "get": demo_operation("List brands (demo set, no key required)") },
            "/demo/v1/speakers": { "get": demo_operation("List the 10 demo speakers (no key required)") },
            "/demo/v1/speaker/{speaker_name}/metadata": { "get": demo_operation("Demo speaker metadata (ASR measurements only)") },
            "/demo/v1/speaker/{speaker_name}/versions": { "get": demo_operation("Demo speaker versions (ASR only)") },
            "/demo/v1/speaker/{speaker_name}/version/{speaker_version}/measurements": { "get": demo_operation("Demo measurement names") },
            "/demo/v1/speaker/{speaker_name}/version/{speaker_version}/measurements/{measurement_name}": { "get": demo_operation("Demo measurement data (pristine)") },
            "/v1/validate": { "post": operation("Validate speaker metadata") },
            "/compare.webp": { "get": { "summary": "Two-speaker overlay picture (A solid, B dashed), rendered from private bundles and cached by content hash; query: measurement, a_speaker/a_origin/a_version, b_speaker/b_origin/b_version, w (900|1400). No key required; the hardened WebP never exposes the underlying data.", "responses": { "200": { "description": "image/webp" }, "400": { "description": "Invalid parameters" }, "404": { "description": "Comparison data not available" }, "503": { "description": "Render budget exhausted (Retry-After)" } } } }
        }
    });
    serde_json::to_string(&spec).unwrap_or_default()
}

fn read_request(reader: &mut BufReader<impl Read>) -> Result<Option<Request>> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("/").to_owned();
    if method.is_empty() {
        return Ok(None);
    }
    let mut headers = HashMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length.min(16 * 1024 * 1024)];
    if !body.is_empty() {
        reader.read_exact(&mut body)?;
    }
    let (path, query_string) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target, String::new()),
    };
    let segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| percent_decode(segment, false))
        .collect();
    let mut query = HashMap::new();
    for pair in query_string.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode(key, true), percent_decode(value, true));
    }
    Ok(Some(Request {
        method,
        segments,
        query,
        headers,
        body,
    }))
}

fn write_response(writer: &mut impl Write, response: &Response) -> Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: private, no-store\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    for (name, value) in &response.extra_headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes())?;
    writer.write_all(&response.body)?;
    writer.flush()?;
    Ok(())
}

fn handle_stream<S: Read + Write + TryCloneStream>(state: &State, stream: S) {
    let mut writer = match stream.try_clone_stream() {
        Ok(clone) => std::io::BufWriter::new(clone),
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    match read_request(&mut reader) {
        Ok(Some(request)) => {
            let response = route(state, &request);
            let _ = write_response(&mut writer, &response);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("spinorama-api: request error: {error:#}");
            let _ = write_response(
                &mut writer,
                &Response::json(400, &serde_json::json!({ "detail": "Bad Request" })),
            );
        }
    }
}

/// Helper so `handle_stream` can split a stream into reader/writer.
trait TryCloneStream: Sized {
    fn try_clone_stream(&self) -> std::io::Result<Self>;
}

impl TryCloneStream for std::net::TcpStream {
    fn try_clone_stream(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
}

#[cfg(unix)]
impl TryCloneStream for std::os::unix::net::UnixStream {
    fn try_clone_stream(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
}

fn serve(state: Arc<State>) -> Result<()> {
    let bind =
        std::env::var("SPINORAMA_API_BIND").unwrap_or_else(|_| "tcp:127.0.0.1:8000".to_owned());
    if let Some(socket_path) = bind.strip_prefix("unix:") {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = Path::new(socket_path);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
            let listener = std::os::unix::net::UnixListener::bind(path)
                .with_context(|| format!("binding unix:{}", path.display()))?;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
            eprintln!("spinorama-api: listening on unix:{}", path.display());
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let state = Arc::clone(&state);
                        std::thread::spawn(move || handle_stream(&state, stream));
                    }
                    Err(error) => eprintln!("spinorama-api: accept failed: {error}"),
                }
            }
            return Ok(());
        }
        #[cfg(not(unix))]
        bail!("unix sockets are not supported on this platform");
    }
    let address = bind.strip_prefix("tcp:").unwrap_or(&bind);
    let listener = TcpListener::bind(address).with_context(|| format!("binding tcp:{address}"))?;
    eprintln!("spinorama-api: listening on tcp:{address}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle_stream(&state, stream));
            }
            Err(error) => eprintln!("spinorama-api: accept failed: {error}"),
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("keys") {
        return match keys::cli(&args[1..]) {
            Ok(output) => {
                println!("{output}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let run = || -> Result<()> {
        let state = Arc::new(load_state()?);
        eprintln!(
            "spinorama-api: {} speakers, {} headphones, {} demo speakers",
            state.speakers.names.len(),
            state.headphones.names.len(),
            state.demo_speakers.len()
        );
        serve(state)
    };
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spinorama-api: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_selection_is_deterministic_and_fixed_size() {
        let candidates: Vec<String> = (0..25).map(|i| format!("Speaker {i:02}")).collect();
        let first = select_demo_speakers(&candidates);
        let second = select_demo_speakers(&candidates);
        assert_eq!(first, second, "same catalogue must give the same demo set");
        assert_eq!(first.len(), DEMO_COUNT);
        assert!(
            first.iter().all(|name| candidates.contains(name)),
            "demo set must be a subset of the candidates"
        );
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(first, sorted, "demo listing is sorted");
    }

    #[test]
    fn demo_selection_takes_all_when_fewer_candidates() {
        let candidates = vec!["A Speaker".to_owned(), "B Speaker".to_owned()];
        assert_eq!(select_demo_speakers(&candidates), candidates);
        assert!(select_demo_speakers(&[]).is_empty());
    }
}
