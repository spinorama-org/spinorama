# -*- coding: utf-8 -*-
"""Per-API-key micro-variation of curve data, for leak identification.

Approved API clients receive curve data carrying a deterministic,
key-specific variation of at most ``MAX_AMPLITUDE_DB`` (default 0.03 dB):
a smooth, band-wise offset whose sign/magnitude pattern is derived from
``HMAC(secret, key_id | resource | trace | band)``. The variation is far
below measurement repeatability, but it is shared by thousands of points,
so a leaked copy can be matched to its key offline with
:func:`rank_candidates` even after rounding, resampling, or smoothing.

Pure standard library: the API environment has no numpy, and this runs on
the request path of a small VPS. Pristine files on disk are never modified;
the variation exists only in the response bytes.

Enable by setting ``SPINORAMA_FINGERPRINT_SECRET`` on the API host. The
amplitude can be tuned with ``SPINORAMA_FINGERPRINT_AMPLITUDE_DB`` and is
hard-capped so no configuration can silently corrupt scientific data.
"""

from __future__ import annotations

import base64
import bisect
import copy
import hashlib
import hmac
import math
import os
import struct

FINGERPRINT_VERSION = "v1"
DEFAULT_AMPLITUDE_DB = 0.03
MAX_AMPLITUDE_DB = 0.05
BANDS_PER_OCTAVE = 6
_REFERENCE_HZ = 20.0

_DTYPES: dict[str, str] = {
    "f8": "d",
    "f4": "f",
    "f2": "e",
    "i1": "b",
    "i2": "h",
    "i4": "i",
    "i8": "q",
    "u1": "B",
    "u2": "H",
    "u4": "I",
    "u8": "Q",
}


def secret_from_env() -> str:
    return os.environ.get("SPINORAMA_FINGERPRINT_SECRET", "")


def amplitude_from_env() -> float:
    raw = os.environ.get("SPINORAMA_FINGERPRINT_AMPLITUDE_DB", "")
    try:
        return _clamp_amplitude(float(raw)) if raw else DEFAULT_AMPLITUDE_DB
    except ValueError:
        return DEFAULT_AMPLITUDE_DB


def _clamp_amplitude(amplitude_db: float) -> float:
    return max(0.0, min(float(amplitude_db), MAX_AMPLITUDE_DB))


def _band_value(secret: str, key_id: str, resource: str, trace_id: str, band: int) -> float:
    """Deterministic value in [-1, 1] for one (key, resource, trace, band)."""
    msg = f"{FINGERPRINT_VERSION}|{key_id}|{resource}|{trace_id}|{band}".encode()
    digest = hmac.new(secret.encode(), msg, hashlib.sha256).digest()
    return struct.unpack_from("<I", digest)[0] / 2**31 - 1.0


def offset_at(
    freq_hz: float,
    *,
    key_id: str,
    resource: str,
    trace_id: str,
    secret: str,
    amplitude_db: float,
    cache: dict[int, float] | None = None,
) -> float:
    """Smooth fingerprint offset (dB) at one frequency.

    Linearly interpolated between adjacent 1/6-octave band centres, so the
    offset survives smoothing and resampling instead of averaging out.
    """
    if freq_hz <= 0:
        return 0.0
    t = math.log2(freq_hz / _REFERENCE_HZ) * BANDS_PER_OCTAVE
    band = math.floor(t)
    frac = t - band

    def value(b: int) -> float:
        if cache is not None:
            cached = cache.get(b)
            if cached is None:
                cached = _band_value(secret, key_id, resource, trace_id, b)
                cache[b] = cached
            return cached
        return _band_value(secret, key_id, resource, trace_id, b)

    return ((1.0 - frac) * value(band) + frac * value(band + 1)) * amplitude_db


def decode_array(value: object) -> list[float] | None:
    """Decode a Plotly numeric array (plain list or ``{dtype, bdata}``)."""
    if isinstance(value, list):
        try:
            return [float(v) for v in value]
        except (TypeError, ValueError):
            return None
    if isinstance(value, dict):
        dtype = value.get("dtype")
        bdata = value.get("bdata")
        shape = value.get("shape")
        if not isinstance(dtype, str) or not isinstance(bdata, str):
            return None
        if isinstance(shape, str) and "," in shape.strip().rstrip(","):
            return None  # multi-dimensional: not a curve trace
        fmt = _DTYPES.get(dtype)
        if fmt is None:
            return None
        raw = base64.b64decode(bdata)
        size = struct.calcsize("<" + fmt)
        if size == 0 or len(raw) % size:
            return None
        return [float(v) for v in struct.unpack(f"<{len(raw) // size}{fmt}", raw)]
    return None


def _encode_like(template: object, values: list[float]) -> object:
    if isinstance(template, dict) and isinstance(template.get("dtype"), str):
        fmt = _DTYPES[template["dtype"]]
        raw = struct.pack(f"<{len(values)}{fmt}", *values)
        encoded = dict(template)
        encoded["bdata"] = base64.b64encode(raw).decode()
        return encoded
    return values


def apply_to_figure(
    payload: dict,
    *,
    key_id: str,
    resource: str,
    secret: str,
    amplitude_db: float = DEFAULT_AMPLITUDE_DB,
) -> dict:
    """Return a fingerprinted copy of a Plotly figure dict.

    Only 1-D scatter traces with a frequency-like x axis are varied.
    Heatmaps, surfaces, and other trace types pass through untouched.
    """
    amplitude = _clamp_amplitude(amplitude_db)
    result = copy.deepcopy(payload)
    if not secret or not key_id or amplitude == 0.0:
        return result
    data = result.get("data")
    if not isinstance(data, list):
        return result
    for index, trace in enumerate(data):
        if not isinstance(trace, dict):
            continue
        if trace.get("type") not in (None, "scatter", "scattergl"):
            continue
        xs = decode_array(trace.get("x"))
        ys = decode_array(trace.get("y"))
        if xs is None or ys is None or len(xs) != len(ys) or not xs:
            continue
        trace_id = str(trace.get("name") or trace.get("legendgroup") or index)
        cache: dict[int, float] = {}
        new_y = [
            y
            + offset_at(
                x,
                key_id=key_id,
                resource=resource,
                trace_id=f"{index}:{trace_id}",
                secret=secret,
                amplitude_db=amplitude,
                cache=cache,
            )
            for x, y in zip(xs, ys, strict=True)
        ]
        trace["y"] = _encode_like(trace.get("y"), new_y)
    return result


def _interp(xs: list[float], ys: list[float], x: float) -> float:
    """Linear interpolation in log-x, clamped at the ends."""
    if x <= xs[0]:
        return ys[0]
    if x >= xs[-1]:
        return ys[-1]
    pos = bisect.bisect_left(xs, x)
    x0, x1 = xs[pos - 1], xs[pos]
    y0, y1 = ys[pos - 1], ys[pos]
    if x0 <= 0 or x1 <= 0 or x1 == x0:
        return y0
    frac = (math.log(x) - math.log(x0)) / (math.log(x1) - math.log(x0))
    return y0 + frac * (y1 - y0)


def _scatter_traces(payload: dict) -> list[tuple[int, str, list[float], list[float]]]:
    traces: list[tuple[int, str, list[float], list[float]]] = []
    data = payload.get("data")
    if not isinstance(data, list):
        return traces
    for index, trace in enumerate(data):
        if not isinstance(trace, dict) or trace.get("type") not in (None, "scatter", "scattergl"):
            continue
        xs = decode_array(trace.get("x"))
        ys = decode_array(trace.get("y"))
        if xs is None or ys is None or len(xs) != len(ys) or not xs:
            continue
        name = str(trace.get("name") or trace.get("legendgroup") or index)
        traces.append((index, name, xs, ys))
    return traces


def rank_candidates(
    pristine: dict,
    suspect: dict,
    key_ids: list[str],
    *,
    resource: str,
    secret: str,
    amplitude_db: float = DEFAULT_AMPLITUDE_DB,
) -> list[tuple[str, float]]:
    """Rank keys by how well their fingerprint explains ``suspect - pristine``.

    Score is the residual/offset correlation summed over matched traces;
    the source key should lead by a clear margin. The suspect may be
    resampled or rounded: pristine values are interpolated onto its grid.
    """
    amplitude = _clamp_amplitude(amplitude_db)
    pristine_by_name: dict[str, tuple[int, list[float], list[float]]] = {}
    for index, name, xs, ys in _scatter_traces(pristine):
        pristine_by_name.setdefault(name, (index, xs, ys))

    pairs: list[tuple[str, list[float], list[float]]] = []  # (trace_id, x, residual)
    for _index, name, xs, ys in _scatter_traces(suspect):
        match = pristine_by_name.get(name)
        if match is None:
            continue
        pristine_index, pristine_xs, pristine_ys = match
        residuals = [y - _interp(pristine_xs, pristine_ys, x) for x, y in zip(xs, ys, strict=True) if x > 0]
        valid_xs = [x for x in xs if x > 0]
        if residuals:
            pairs.append((f"{pristine_index}:{name}", valid_xs, residuals))

    ranked: list[tuple[str, float]] = []
    for key_id in key_ids:
        dot = 0.0
        norm = 0.0
        for trace_id, xs, residuals in pairs:
            cache: dict[int, float] = {}
            for x, residual in zip(xs, residuals, strict=True):
                expected = offset_at(
                    x,
                    key_id=key_id,
                    resource=resource,
                    trace_id=trace_id,
                    secret=secret,
                    amplitude_db=amplitude,
                    cache=cache,
                )
                dot += residual * expected
                norm += expected * expected
        ranked.append((key_id, dot / math.sqrt(norm) if norm > 0 else 0.0))
    ranked.sort(key=lambda item: item[1], reverse=True)
    return ranked


def apply_to_csv_text(
    text: str,
    *,
    key_id: str,
    resource: str,
    secret: str,
    amplitude_db: float = DEFAULT_AMPLITUDE_DB,
) -> str:
    """Fingerprint a headphone frequency-response CSV (Left/Right X,Y pairs).

    The four header lines are preserved verbatim. Data rows that do not
    parse as four numbers pass through unchanged.
    """
    amplitude = _clamp_amplitude(amplitude_db)
    if not secret or not key_id or amplitude == 0.0:
        return text
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    caches: dict[str, dict[int, float]] = {"Left": {}, "Right": {}}
    for lineno, line in enumerate(lines):
        if lineno < 4:
            out.append(line)
            continue
        ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
        fields = line.rstrip("\r\n").split(",")
        if len(fields) != 4:
            out.append(line)
            continue
        try:
            values = [float(f) for f in fields]
        except ValueError:
            out.append(line)
            continue
        values[1] += offset_at(
            values[0], key_id=key_id, resource=resource, trace_id="Left",
            secret=secret, amplitude_db=amplitude, cache=caches["Left"],
        )
        values[3] += offset_at(
            values[2], key_id=key_id, resource=resource, trace_id="Right",
            secret=secret, amplitude_db=amplitude, cache=caches["Right"],
        )
        out.append(",".join(repr(v) for v in values) + ending)
    return "".join(out)
