//! Headphone metadata assembly for the `generate-headphone-meta` binary.
//!
//! Port of Python `scripts/generate_headphone_meta.py`: reads the
//! Python-literal catalogue in `datas/headphone_*.py`, attaches the
//! REW EQ files and AutoEQ scores from `datas/headphone_eq`, and
//! splits the result into `headphone_metadata.json` (without `eqs`)
//! and `headphone_eqdata.json` (only `eqs`/`brand`/`model`/`default_eq`).
//! Serialisation matches CPython `json.dumps` defaults (`, ` / `: `
//! separators, ASCII-escaped strings, insertion-ordered keys) so the
//! output is byte-for-byte identical to the Python script.

// Rust guideline compliant 2026-10-09

use std::path::Path;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;

use crate::generate_meta::{RewFilter, parse_rew_eq, preamp_gain, round1};

/// Sample rate stamped on REW filters, as in Python `parse_eq_iir_rews(..., 48000)`.
const EQ_SAMPLE_RATE: f64 = 48_000.0;

/// Display prefix used in EQ `filename` fields (Python `CPATH_DATAS_HEADPHONE_EQ`).
const EQ_DISPLAY_BASE: &str = "./datas/headphone_eq";

/// Ordered JSON value preserving Python `dict` insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer.
    Int(i64),
    /// JSON floating-point number.
    Float(f64),
    /// JSON string.
    Str(String),
    /// JSON array.
    Arr(Vec<Json>),
    /// JSON object as ordered key/value pairs.
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Returns the value for `key` in an object, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns true when `key` is present in an object.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// True for the Python truth value of `skip: True`.
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        matches!(self.get("skip"), Some(Json::Bool(true)))
    }

    fn obj_mut(&mut self) -> Option<&mut Vec<(String, Json)>> {
        match self {
            Json::Obj(pairs) => Some(pairs),
            _ => None,
        }
    }

    /// Sets `key`, keeping its position if present (Python `dict` assignment).
    pub fn set(&mut self, key: &str, value: Json) {
        if let Some(pairs) = self.obj_mut() {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                pairs.push((key.to_owned(), value));
            }
        }
    }

    /// Removes `key` from an object, returning it if present.
    pub fn remove(&mut self, key: &str) -> Option<Json> {
        let pairs = self.obj_mut()?;
        let pos = pairs.iter().position(|(k, _)| k == key)?;
        Some(pairs.remove(pos).1)
    }
}

/// Formats a float like CPython `repr(float)` for the magnitudes in this data.
///
/// Rust's shortest-round-trip formatting (via `ryu`, as in `serde_json`)
/// agrees with CPython for ordinary magnitudes; exponent notation is
/// normalised to CPython's signed two-digit exponent.
fn format_float(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }
    let raw = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
    if let Some((mantissa, exp)) = raw.split_once('e') {
        let exp: i32 = exp.parse().unwrap_or(0);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp.abs())
    } else if raw.contains('.') {
        raw
    } else {
        format!("{raw}.0")
    }
}

fn escape_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x7f => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// Serialises `value` exactly like CPython `json.dumps(value)` (defaults).
#[must_use]
pub fn dumps_python(value: &Json) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(v) => out.push_str(&v.to_string()),
        Json::Float(v) => out.push_str(&format_float(*v)),
        Json::Str(s) => escape_string(out, s),
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Json::Obj(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                escape_string(out, key);
                out.push_str(": ");
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

// ---------------------------------------------------------------------------
// Python-literal catalogue parser (datas/headphone_*.py)
// ---------------------------------------------------------------------------

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'#' {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<Json> {
        match self.peek().context("unexpected end of Python literal")? {
            b'{' => self.parse_dict(),
            b'[' | b'(' => self.parse_list(),
            b'\'' | b'"' => Ok(Json::Str(self.parse_string()?)),
            b'T' | b'F' => {
                if self.bytes[self.pos..].starts_with(b"True") {
                    self.pos += 4;
                    Ok(Json::Bool(true))
                } else if self.bytes[self.pos..].starts_with(b"False") {
                    self.pos += 5;
                    Ok(Json::Bool(false))
                } else {
                    bail!("bad literal at byte {}", self.pos)
                }
            }
            b'N' if self.bytes[self.pos..].starts_with(b"None") => {
                self.pos += 4;
                Ok(Json::Null)
            }
            _ => self.parse_number(),
        }
    }

    fn parse_dict(&mut self) -> Result<Json> {
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            let key = match self.parse_value()? {
                Json::Str(s) => s,
                _ => bail!("dict key is not a string at byte {}", self.pos),
            };
            if self.peek() != Some(b':') {
                bail!("expected ':' at byte {}", self.pos);
            }
            self.pos += 1;
            let value = self.parse_value()?;
            pairs.push((key, value));
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                other => bail!("expected ',' or '}}' at byte {}, got {other:?}", self.pos),
            }
        }
        Ok(Json::Obj(pairs))
    }

    fn parse_list(&mut self) -> Result<Json> {
        let close = if self.bytes[self.pos] == b'[' {
            b']'
        } else {
            b')'
        };
        self.pos += 1;
        let mut items = Vec::new();
        if self.peek() == Some(close) {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    if self.peek() == Some(close) {
                        self.pos += 1;
                        break;
                    }
                }
                Some(c) if c == close => {
                    self.pos += 1;
                    break;
                }
                other => bail!(
                    "expected ',' or closing bracket at byte {}, got {other:?}",
                    self.pos
                ),
            }
        }
        Ok(Json::Arr(items))
    }

    fn parse_string(&mut self) -> Result<String> {
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            self.pos += 1;
            match byte {
                b if b == quote => return Ok(out),
                b'\\' => {
                    let esc = *self.bytes.get(self.pos).context("truncated escape")?;
                    self.pos += 1;
                    match esc {
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'\\' => out.push('\\'),
                        b'\'' => out.push('\''),
                        b'"' => out.push('"'),
                        b'u' | b'x' => {
                            let digits = if esc == b'u' { 4 } else { 2 };
                            let hex = std::str::from_utf8(
                                self.bytes
                                    .get(self.pos..self.pos + digits)
                                    .context("bad hex escape")?,
                            )?;
                            let code = u32::from_str_radix(hex, 16).context("bad hex escape")?;
                            self.pos += digits;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        other => {
                            out.push('\\');
                            out.push(other as char);
                        }
                    }
                }
                _ => {
                    // Re-decode one UTF-8 character starting at pos-1.
                    let start = self.pos - 1;
                    let len = utf8_len(byte);
                    let text = std::str::from_utf8(&self.bytes[start..start + len])?;
                    out.push_str(text);
                    self.pos = start + len;
                }
            }
        }
        bail!("unterminated string")
    }

    fn parse_number(&mut self) -> Result<Json> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.bytes.len()
            && matches!(
                self.bytes[self.pos],
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' | b'_'
            )
        {
            self.pos += 1;
        }
        let token: String = std::str::from_utf8(&self.bytes[start..self.pos])?.replace('_', "");
        if token.contains(['.', 'e', 'E']) {
            Ok(Json::Float(token.parse().context("bad float")?))
        } else {
            Ok(Json::Int(token.parse().context("bad int")?))
        }
    }
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Parses the dict assigned in one `datas/headphone_*.py` file.
///
/// # Errors
///
/// Returns an error when the file contains no `= { ... }` dict or the
/// literal uses syntax outside strings/numbers/bools/None/dicts/lists.
pub fn parse_python_catalogue(text: &str) -> Result<Vec<(String, Json)>> {
    let eq = text.find('=').context("no assignment in catalogue file")?;
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: eq + 1,
    };
    match parser.parse_value()? {
        Json::Obj(pairs) => Ok(pairs),
        _ => bail!("catalogue assignment is not a dict"),
    }
}

/// Loads every `datas/headphone_*.py` shard in filename order (digits
/// before letters, matching the merge order in `datas/headphones.py`).
///
/// # Errors
///
/// Returns an error when the directory cannot be read or a shard fails to parse.
pub fn load_catalogue(datas_dir: &Path) -> Result<Vec<(String, Json)>> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for entry in
        std::fs::read_dir(datas_dir).with_context(|| format!("reading {}", datas_dir.display()))?
    {
        let path = entry?.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if name.starts_with("headphone_") && name.ends_with(".py") {
            files.push(path);
        }
    }
    files.sort();
    let mut catalogue: Vec<(String, Json)> = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let entries =
            parse_python_catalogue(&text).with_context(|| format!("parsing {}", path.display()))?;
        for (name, entry) in entries {
            // Python `dict | dict` keeps the first position on duplicates.
            if let Some(slot) = catalogue.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = entry;
            } else {
                catalogue.push((name, entry));
            }
        }
    }
    Ok(catalogue)
}

// ---------------------------------------------------------------------------
// EQ / score attachment and output split (generate_headphone_meta.py)
// ---------------------------------------------------------------------------

fn filter_to_json(filter: &RewFilter) -> Json {
    Json::Obj(vec![
        ("type".to_owned(), Json::Int(filter.type_code)),
        ("freq".to_owned(), Json::Float(filter.freq)),
        ("srate".to_owned(), Json::Float(filter.srate)),
        ("Q".to_owned(), Json::Float(filter.q)),
        ("dbGain".to_owned(), Json::Float(filter.db_gain)),
    ])
}

/// Builds one Python `EQ` object from parsed REW filters.
#[must_use]
pub fn build_eq(filters: &[RewFilter], filename: &str, display: &str) -> Json {
    let peq: Vec<Json> = filters
        .iter()
        .filter(|f| f.weight != 0.0)
        .map(filter_to_json)
        .collect();
    Json::Obj(vec![
        ("display_name".to_owned(), Json::Str(display.to_owned())),
        ("filename".to_owned(), Json::Str(filename.to_owned())),
        (
            "preamp_gain".to_owned(),
            Json::Float(round1(preamp_gain(filters))),
        ),
        ("type".to_owned(), Json::Str("peq".to_owned())),
        ("peq".to_owned(), Json::Arr(peq)),
    ])
}

/// Attaches `eqs`/`default_eq` from `datas/headphone_eq/<name>/iir-*.txt`.
pub fn add_headphone_eq(catalogue: &mut [(String, Json)], eq_dir: &Path) {
    // Entries are independent; each reads only its own EQ directory.
    catalogue.par_iter_mut().for_each(|(name, info)| {
        if info.is_skipped() {
            return;
        }
        if !matches!(info.get("eqs"), Some(Json::Obj(_))) {
            info.set("eqs", Json::Obj(Vec::new()));
        }
        for (key, file, display) in [
            ("autoeq_score", "iir-autoeq-score", "Harman Score EQ (IIR)"),
            ("autoeq_flat", "iir-autoeq-flat", "Flat Target EQ (IIR)"),
        ] {
            let path = eq_dir.join(name.as_str()).join(format!("{file}.txt"));
            let filters = parse_rew_eq(&path, EQ_SAMPLE_RATE);
            if filters.is_empty() {
                continue;
            }
            if !info.contains("default_eq") {
                info.set("default_eq", Json::Str(key.to_owned()));
            }
            let filename = format!("{EQ_DISPLAY_BASE}/{name}/{file}.txt");
            let eq = build_eq(&filters, &filename, display);
            if let Some(Json::Obj(eqs)) = info
                .obj_mut()
                .and_then(|pairs| pairs.iter_mut().find(|(k, _)| k == "eqs").map(|(_, v)| v))
            {
                eqs.push((key.to_owned(), eq));
            }
        }
    });
}

fn score_value(value: &serde_json::Value) -> Option<Json> {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Json::Int(i))
            } else {
                n.as_f64().map(Json::Float)
            }
        }
        _ => None,
    }
}

/// Attaches `score`/`score_eq` from `datas/headphone_eq/<name>/autoeq_score.json`.
pub fn add_headphone_scores(catalogue: &mut [(String, Json)], eq_dir: &Path) {
    catalogue.par_iter_mut().for_each(|(name, info)| {
        if info.is_skipped() {
            return;
        }
        let path = eq_dir.join(name).join("autoeq_score.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        if let Some(v) = data.get("pre_score").and_then(score_value) {
            info.set("score", v);
        }
        if let Some(v) = data.get("post_score").and_then(score_value) {
            info.set("score_eq", v);
        }
    });
}

/// Splits the catalogue into `(headphone_metadata, headphone_eqdata)`.
///
/// Metadata drops `eqs` and gains `kind: "headphone"`; eqdata keeps
/// only `eqs`, `brand`, `model`, and `default_eq`, in entry order.
/// Skipped entries are omitted from both.
#[must_use]
pub fn split_outputs(catalogue: &[(String, Json)]) -> (Json, Json) {
    let mut meta = Vec::new();
    let mut eqdata = Vec::new();
    for (name, info) in catalogue {
        if info.is_skipped() {
            continue;
        }
        let Json::Obj(pairs) = info else { continue };
        let meta_entry: Vec<(String, Json)> = pairs
            .iter()
            .filter(|(k, _)| k != "eqs")
            .cloned()
            .chain([("kind".to_owned(), Json::Str("headphone".to_owned()))])
            .collect();
        meta.push((name.clone(), Json::Obj(meta_entry)));
        let eq_entry: Vec<(String, Json)> = pairs
            .iter()
            .filter(|(k, _)| matches!(k.as_str(), "eqs" | "brand" | "model" | "default_eq"))
            .cloned()
            .collect();
        eqdata.push((name.clone(), Json::Obj(eq_entry)));
    }
    (Json::Obj(meta), Json::Obj(eqdata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_catalogue_shard() {
        let text = "# generated\nfrom . import HeadphoneDatabase\n\n\
            headphones_info_a: HeadphoneDatabase = {\n\
            'AKG K240': {\"brand\": 'AKG', \"price\": '70.00', \
            \"measurements\": {'asr': {\"sensitivity_mV_94dB\": 230.0, 'n': 3}}},\n\
            'Trió X': {\"brand\": 'T', \"skip\": False},\n}\n";
        let entries = parse_python_catalogue(text).expect("parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "AKG K240");
        assert_eq!(
            entries[0]
                .1
                .get("measurements")
                .and_then(|m| m.get("asr"))
                .and_then(|m| m.get("n")),
            Some(&Json::Int(3))
        );
        assert_eq!(entries[1].1.get("skip"), Some(&Json::Bool(false)));
    }

    #[test]
    fn dumps_matches_cpython_defaults() {
        let value = Json::Obj(vec![
            ("a".to_owned(), Json::Float(48000.0)),
            ("b".to_owned(), Json::Str("Trió \"x\"".to_owned())),
            (
                "c".to_owned(),
                Json::Arr(vec![Json::Int(3), Json::Float(-6.0)]),
            ),
        ]);
        assert_eq!(
            dumps_python(&value),
            "{\"a\": 48000.0, \"b\": \"Tri\\u00f3 \\\"x\\\"\", \"c\": [3, -6.0]}"
        );
    }

    #[test]
    fn split_outputs_drops_eqs_and_keeps_eq_keys_in_order() {
        let catalogue = vec![
            (
                "HP One".to_owned(),
                Json::Obj(vec![
                    ("brand".to_owned(), Json::Str("B".to_owned())),
                    ("model".to_owned(), Json::Str("M".to_owned())),
                    ("eqs".to_owned(), Json::Obj(Vec::new())),
                    (
                        "default_eq".to_owned(),
                        Json::Str("autoeq_score".to_owned()),
                    ),
                    ("score".to_owned(), Json::Float(1.5)),
                ]),
            ),
            (
                "Skipped".to_owned(),
                Json::Obj(vec![("skip".to_owned(), Json::Bool(true))]),
            ),
        ];
        let (meta, eqdata) = split_outputs(&catalogue);
        let Json::Obj(meta_pairs) = &meta else {
            panic!("meta obj")
        };
        assert_eq!(meta_pairs.len(), 1);
        let entry = &meta_pairs[0].1;
        assert!(!entry.contains("eqs"));
        assert_eq!(entry.get("kind"), Some(&Json::Str("headphone".to_owned())));
        let Json::Obj(eq_pairs) = &eqdata else {
            panic!("eq obj")
        };
        let keys: Vec<&str> = match &eq_pairs[0].1 {
            Json::Obj(p) => p.iter().map(|(k, _)| k.as_str()).collect(),
            _ => Vec::new(),
        };
        assert_eq!(keys, ["brand", "model", "eqs", "default_eq"]);
    }

    #[test]
    fn build_eq_skips_off_filters_and_rounds_preamp() {
        // One ON peak with a negative gain needs no positive headroom.
        let filters = vec![
            RewFilter {
                weight: 1.0,
                type_code: 3,
                kind: autoeq_workflow::iir::BiquadFilterType::Peak,
                freq: 100.0,
                srate: 48000.0,
                q: 1.0,
                db_gain: -3.0,
            },
            RewFilter {
                weight: 0.0,
                type_code: 3,
                kind: autoeq_workflow::iir::BiquadFilterType::Peak,
                freq: 200.0,
                srate: 48000.0,
                q: 1.0,
                db_gain: 9.0,
            },
        ];
        let eq = build_eq(&filters, "./datas/headphone_eq/X/iir-autoeq-score.txt", "D");
        let Some(Json::Arr(peq)) = eq.get("peq") else {
            panic!("peq")
        };
        assert_eq!(peq.len(), 1);
        assert_eq!(eq.get("preamp_gain"), Some(&Json::Float(-0.0)));
        assert_eq!(
            dumps_python(&peq[0]),
            "{\"type\": 3, \"freq\": 100.0, \"srate\": 48000.0, \"Q\": 1.0, \"dbGain\": -3.0}"
        );
    }
}
