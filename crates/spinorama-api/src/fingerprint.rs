//! Per-API-key micro-variation of curve data, for leak identification.
//!
//! Port of Python `api/fingerprint.py` (response path only; the
//! offline `rank_candidates` identification stays in Python).
//! Deterministic HMAC-SHA256 band offsets of at most 0.05 dB.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;

use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Fingerprint scheme version, embedded in every HMAC message.
pub const VERSION: &str = "v1";
/// Default variation amplitude in dB.
pub const DEFAULT_AMPLITUDE_DB: f64 = 0.03;
/// Hard cap on the variation amplitude in dB.
pub const MAX_AMPLITUDE_DB: f64 = 0.05;
const BANDS_PER_OCTAVE: f64 = 6.0;
const REFERENCE_HZ: f64 = 20.0;

/// Returns the configured fingerprint secret (empty when disabled).
#[must_use]
pub fn secret_from_env() -> String {
    std::env::var("SPINORAMA_FINGERPRINT_SECRET").unwrap_or_default()
}

/// Returns the configured amplitude, clamped to the hard cap.
#[must_use]
pub fn amplitude_from_env() -> f64 {
    match std::env::var("SPINORAMA_FINGERPRINT_AMPLITUDE_DB") {
        Ok(raw) if !raw.is_empty() => raw
            .parse::<f64>()
            .map_or(DEFAULT_AMPLITUDE_DB, clamp_amplitude),
        _ => DEFAULT_AMPLITUDE_DB,
    }
}

fn clamp_amplitude(amplitude_db: f64) -> f64 {
    amplitude_db.clamp(0.0, MAX_AMPLITUDE_DB)
}

fn band_value(secret: &str, key_id: &str, resource: &str, trace_id: &str, band: i64) -> f64 {
    let msg = format!("{VERSION}|{key_id}|{resource}|{trace_id}|{band}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .unwrap_or_else(|_| HmacSha256::new_from_slice(b"").expect("empty HMAC key"));
    mac.update(msg.as_bytes());
    let digest = mac.finalize().into_bytes();
    let raw = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    f64::from(raw) / 2_147_483_648.0 - 1.0
}

/// Smooth fingerprint offset (dB) at one frequency.
fn offset_at(
    freq_hz: f64,
    key_id: &str,
    resource: &str,
    trace_id: &str,
    secret: &str,
    amplitude_db: f64,
    cache: &mut HashMap<i64, f64>,
) -> f64 {
    if freq_hz <= 0.0 {
        return 0.0;
    }
    let t = (freq_hz / REFERENCE_HZ).log2() * BANDS_PER_OCTAVE;
    let band = t.floor();
    let frac = t - band;
    #[expect(clippy::cast_possible_truncation, reason = "band index is small")]
    let band_i = band as i64;
    let value = |b: i64, cache: &mut HashMap<i64, f64>| {
        *cache
            .entry(b)
            .or_insert_with(|| band_value(secret, key_id, resource, trace_id, b))
    };
    let lo = value(band_i, cache);
    let hi = value(band_i + 1, cache);
    ((1.0 - frac) * lo + frac * hi) * amplitude_db
}

fn dtype_format(dtype: &str) -> Option<(usize, char)> {
    // (byte width, kind) for the Plotly bdata dtypes Python supports.
    Some(match dtype {
        "f8" => (8, 'd'),
        "f4" => (4, 'f'),
        "f2" => (2, 'e'),
        "i1" => (1, 'b'),
        "i2" => (2, 'h'),
        "i4" => (4, 'i'),
        "i8" => (8, 'q'),
        "u1" => (1, 'B'),
        "u2" => (2, 'H'),
        "u4" => (4, 'I'),
        "u8" => (8, 'Q'),
        _ => return None,
    })
}

fn read_f16(bytes: &[u8]) -> f64 {
    // Minimal IEEE-754 half-precision decode.
    let bits = u16::from_le_bytes([bytes[0], bytes[1]]);
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (bits >> 10) & 0x1f;
    let frac = f64::from(bits & 0x03ff) / 1024.0;
    sign * match exp {
        0 => frac * 2f64.powi(-14),
        31 => {
            if frac == 0.0 {
                f64::INFINITY
            } else {
                f64::NAN
            }
        }
        e => (1.0 + frac) * 2f64.powi(i32::from(e) - 15),
    }
}

fn write_f16(value: f64) -> [u8; 2] {
    // Round-trip of values produced by read_f16 plus a small offset;
    // a plain conversion via f32 is adequate for fingerprint deltas.
    let bits = f32::to_bits(value as f32);
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp32 = ((bits >> 23) & 0xff) as i32;
    let frac32 = bits & 0x7f_ffff;
    let out: u16 = if exp32 == 255 {
        sign | 0x7c00 | if frac32 != 0 { 0x0200 } else { 0 }
    } else if exp32 - 112 >= 31 {
        sign | 0x7c00
    } else if exp32 - 112 <= 0 {
        if exp32 - 101 < 0 {
            sign
        } else {
            sign | ((frac32 | 0x80_0000) >> (101 - exp32 + 13)) as u16
        }
    } else {
        sign | (((exp32 - 112) as u16) << 10) | (frac32 >> 13) as u16
    };
    out.to_le_bytes()
}

/// Decodes a Plotly numeric array (plain list or `{dtype, bdata}`).
#[must_use]
pub fn decode_array(value: &Value) -> Option<Vec<f64>> {
    if let Some(list) = value.as_array() {
        let mut out = Vec::with_capacity(list.len());
        for item in list {
            out.push(item.as_f64()?);
        }
        return Some(out);
    }
    let obj = value.as_object()?;
    let dtype = obj.get("dtype")?.as_str()?;
    let bdata = obj.get("bdata")?.as_str()?;
    if let Some(shape) = obj.get("shape").and_then(Value::as_str)
        && shape.trim().trim_end_matches(',').contains(',')
    {
        return None; // multi-dimensional: not a curve trace
    }
    let (width, kind) = dtype_format(dtype)?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(bdata)
        .ok()?;
    if raw.is_empty() || raw.len() % width != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / width);
    for chunk in raw.chunks_exact(width) {
        out.push(match kind {
            'd' => f64::from_le_bytes(chunk.try_into().ok()?),
            'f' => f64::from(f32::from_le_bytes(chunk.try_into().ok()?)),
            'e' => read_f16(chunk),
            'b' => f64::from(chunk[0] as i8),
            'h' => f64::from(i16::from_le_bytes(chunk.try_into().ok()?)),
            'i' => f64::from(i32::from_le_bytes(chunk.try_into().ok()?)),
            'q' => i64::from_le_bytes(chunk.try_into().ok()?) as f64,
            'B' => f64::from(chunk[0]),
            'H' => f64::from(u16::from_le_bytes(chunk.try_into().ok()?)),
            'I' => f64::from(u32::from_le_bytes(chunk.try_into().ok()?)),
            'Q' => u64::from_le_bytes(chunk.try_into().ok()?) as f64,
            _ => return None,
        });
    }
    Some(out)
}

fn encode_like(template: &Value, values: &[f64]) -> Value {
    let Some(obj) = template.as_object() else {
        return Value::from(values.to_vec());
    };
    let Some(dtype) = obj.get("dtype").and_then(Value::as_str) else {
        return Value::from(values.to_vec());
    };
    let Some((_, kind)) = dtype_format(dtype) else {
        return Value::from(values.to_vec());
    };
    let mut raw = Vec::new();
    for value in values {
        match kind {
            'd' => raw.extend_from_slice(&value.to_le_bytes()),
            'f' => raw.extend_from_slice(&(*value as f32).to_le_bytes()),
            'e' => raw.extend_from_slice(&write_f16(*value)),
            'b' => raw.push((*value as i8) as u8),
            'h' => raw.extend_from_slice(&(*value as i16).to_le_bytes()),
            'i' => raw.extend_from_slice(&(*value as i32).to_le_bytes()),
            'q' => raw.extend_from_slice(&(*value as i64).to_le_bytes()),
            'B' => raw.push(*value as u8),
            'H' => raw.extend_from_slice(&(*value as u16).to_le_bytes()),
            'I' => raw.extend_from_slice(&(*value as u32).to_le_bytes()),
            'Q' => raw.extend_from_slice(&(*value as u64).to_le_bytes()),
            _ => {}
        }
    }
    let mut encoded = obj.clone();
    encoded.insert(
        "bdata".to_owned(),
        Value::from(base64::engine::general_purpose::STANDARD.encode(raw)),
    );
    Value::Object(encoded)
}

/// Returns a fingerprinted copy of a Plotly figure payload.
///
/// Only 1-D scatter traces are varied, as in Python `apply_to_figure`.
#[must_use]
pub fn apply_to_figure(
    payload: &Value,
    key_id: &str,
    resource: &str,
    secret: &str,
    amplitude_db: f64,
) -> Value {
    let amplitude = clamp_amplitude(amplitude_db);
    let mut result = payload.clone();
    if secret.is_empty() || key_id.is_empty() || amplitude == 0.0 {
        return result;
    }
    let Some(data) = result.get_mut("data").and_then(Value::as_array_mut) else {
        return result;
    };
    let mut pending: Vec<(usize, Value)> = Vec::new();
    for (index, trace) in data.iter().enumerate() {
        let trace_type = trace.get("type");
        if !matches!(trace_type, None | Some(Value::Null))
            && trace_type.and_then(Value::as_str) != Some("scatter")
            && trace_type.and_then(Value::as_str) != Some("scattergl")
        {
            continue;
        }
        let Some(xs) = trace.get("x").and_then(decode_array) else {
            continue;
        };
        let Some(ys) = trace.get("y").and_then(decode_array) else {
            continue;
        };
        if xs.len() != ys.len() || xs.is_empty() {
            continue;
        }
        let trace_id = trace
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .or_else(|| {
                trace
                    .get("legendgroup")
                    .filter(|v| !v.is_null())
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
            })
            .unwrap_or_else(|| index.to_string());
        let full_trace_id = format!("{index}:{trace_id}");
        let mut cache = HashMap::new();
        let new_y: Vec<f64> = xs
            .iter()
            .zip(&ys)
            .map(|(x, y)| {
                y + offset_at(
                    *x,
                    key_id,
                    resource,
                    &full_trace_id,
                    secret,
                    amplitude,
                    &mut cache,
                )
            })
            .collect();
        let template = trace.get("y").cloned().unwrap_or(Value::Null);
        pending.push((index, encode_like(&template, &new_y)));
    }
    for (index, new_y) in pending {
        if let Some(trace_mut) = data[index].as_object_mut() {
            trace_mut.insert("y".to_owned(), new_y);
        }
    }
    result
}

/// Fingerprints a headphone frequency-response CSV (Left/Right pairs).
#[must_use]
pub fn apply_to_csv_text(
    text: &str,
    key_id: &str,
    resource: &str,
    secret: &str,
    amplitude_db: f64,
) -> String {
    let amplitude = clamp_amplitude(amplitude_db);
    if secret.is_empty() || key_id.is_empty() || amplitude == 0.0 {
        return text.to_owned();
    }
    let mut caches: HashMap<&str, HashMap<i64, f64>> = HashMap::new();
    let mut out = String::with_capacity(text.len());
    for (lineno, line) in text.split_inclusive('\n').enumerate() {
        if lineno < 4 {
            out.push_str(line);
            continue;
        }
        let (body, ending) = if let Some(stripped) = line.strip_suffix("\r\n") {
            (stripped, "\r\n")
        } else if let Some(stripped) = line.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (line, "")
        };
        let fields: Vec<&str> = body.split(',').collect();
        if fields.len() != 4 {
            out.push_str(line);
            continue;
        }
        let parsed: Result<Vec<f64>, _> = fields.iter().map(|f| f.parse::<f64>()).collect();
        let Ok(mut values) = parsed else {
            out.push_str(line);
            continue;
        };
        for (freq_idx, spl_idx, trace_id) in [(0usize, 1usize, "Left"), (2, 3, "Right")] {
            let cache = caches.entry(trace_id).or_default();
            values[spl_idx] += offset_at(
                values[freq_idx],
                key_id,
                resource,
                trace_id,
                secret,
                amplitude,
                cache,
            );
        }
        // Python formats with repr(float): shortest round-trip decimal.
        let rendered: Vec<String> = values
            .iter()
            .map(|v| {
                if v.fract() == 0.0 && v.abs() < 1e16 {
                    format!("{v:.1}")
                } else {
                    format!("{v}")
                }
            })
            .collect();
        out.push_str(&rendered.join(","));
        out.push_str(ending);
    }
    out
}
