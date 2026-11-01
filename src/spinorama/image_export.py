# -*- coding: utf-8 -*-
"""Public graph images: hardened WebP export and private trace bundles.

The public website serves raster images only. This module converts a
Plotly figure to a watermarked, textured WebP (see
:func:`harden_png_to_webp`) and extracts a compact, quantised trace
bundle used later by the cheap comparison renderer. Bundles and the
pristine Plotly JSON live in the private tree, never on the public site.
"""

from __future__ import annotations

import base64
import bisect
import hashlib
import io
import json
import math
import os
import pathlib
import random
import struct
from typing import Any

from PIL import Image, ImageDraw, ImageFont

from spinorama import logger

WATERMARK_TEXT = "spinorama.org  CC BY-NC-SA 4.0"

# Human-readable measurement sources burned into public pictures, keyed by
# the alphanumeric-normalised origin or version code used in the metadata.
_SOURCE_LABELS = {
    "asr": "Audio Science Review",
    "audiosciencereview": "Audio Science Review",
    "eac": "Erin's Audio Corner",
    "erinsaudiocorner": "Erin's Audio Corner",
    "erin": "Erin's Audio Corner",
    "princeton": "Princeton 3D3A",
    "3d3a": "Princeton 3D3A",
    "sr": "Sound & Recording",
    "pp": "Production Partners",
    "archimago": "Archimago",
    "audioxpress": "AudioXPress",
    "audioholics": "Audioholics",
    "soundstageultra": "Sound Stage Ultra",
    "ssn": "SoundStage Network",
    "soundstagenetwork": "SoundStage Network",
    "nuyes": "Nuyes",
    "stereophile": "Stereophile",
    "soundstagesolo": "SoundStage Solo",
}


def _normalised_key(value: str) -> str:
    return "".join(c.lower() for c in value if c.isalnum())


def source_label(origin: str, version: str = "") -> str:
    """Return the display name of the measurement source for a watermark.

    Accepts either the full metadata origin (``ASR``, ``ErinsAudioCorner``,
    ``Vendors-Genelec``…) or, via ``version``, the short measurement code
    (``asr``, ``eac``…), so every writer can supply whatever it already has.
    Unknown vendor origins fall back to the vendor name itself; genuinely
    unknown sources return an empty string.
    """
    raw_origin = (origin or "").strip()
    raw_version = (version or "").strip()
    version_code = raw_version.removesuffix("_eq").removesuffix("-eq")

    for candidate in (raw_origin, version_code):
        label = _SOURCE_LABELS.get(_normalised_key(candidate))
        if label:
            return label

    if raw_origin.startswith("Vendors-"):
        return raw_origin.removeprefix("Vendors-")

    if _normalised_key(raw_origin) == "misc":
        suffix = version_code.rsplit("-", 1)[-1] if "-" in version_code else version_code
        label = _SOURCE_LABELS.get(_normalised_key(suffix))
        if label:
            return label
        if version_code.startswith("misc-") and len(version_code) > 5:
            return version_code[5:].replace("-", " ").title()
        return ""

    # Misc versions encode the measurer after a dash, e.g. "misc-archimago".
    for part in reversed(version_code.replace("_", "-").split("-")):
        label = _SOURCE_LABELS.get(_normalised_key(part))
        if label:
            return label

    return raw_origin if _normalised_key(raw_origin) not in ("", "misc") else ""


def watermark_text(source: str = "") -> str:
    """Full watermark line, including the measurement source when known."""
    source = " ".join(str(source).split())
    return f"{WATERMARK_TEXT}  |  {source}" if source else WATERMARK_TEXT
_BUNDLE_VERSION = 2
_SENTINEL = -32768

_FONT_CANDIDATES = (
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/opt/homebrew/share/fonts/DejaVuSans.ttf",
)


def _seed_bytes(seed_text: str) -> bytes:
    salt = os.environ.get("SPINORAMA_IMAGE_SALT", "")
    return hashlib.sha256(f"{salt}|{seed_text}".encode()).digest()


def _load_font(size: int) -> Any:
    env_font = os.environ.get("SPINORAMA_FONT", "")
    for candidate in (env_font, *_FONT_CANDIDATES):
        if candidate and os.path.isfile(candidate):
            try:
                return ImageFont.truetype(candidate, size)
            except OSError:
                continue
    try:
        return ImageFont.load_default(size=size)
    except TypeError:
        return ImageFont.load_default()


def harden_png_to_webp(
    png_bytes: bytes,
    seed_text: str,
    *,
    watermark: str = WATERMARK_TEXT,
    quality: int = 82,
) -> bytes:
    """Apply anti-scraping hardening to a rendered graph and encode as WebP.

    Layers, all deterministic for a given ``seed_text`` (usually
    ``Speaker/Origin/Version/Graph``):

    - a fine, seeded background texture plus a smooth luminance drift, so
      there is no single background colour/template to threshold or
      subtract across images;
    - a repeated diagonal attribution watermark crossing the plot area,
      which breaks trace continuity for automatic line followers;
    - lossy WebP encoding, whose chroma bleed further blurs trace edges.

    The data geometry is never distorted.
    """
    img = Image.open(io.BytesIO(png_bytes)).convert("RGB")
    width, height = img.size
    rng = random.Random(_seed_bytes(seed_text))  # noqa: S311 -- visual texture only

    fine = Image.frombytes("L", (width, height), rng.randbytes(width * height)).convert("RGB")
    img = Image.blend(img, fine, 0.035)
    small_w, small_h = max(8, width // 16), max(8, height // 16)
    blotch = (
        Image.frombytes("L", (small_w, small_h), rng.randbytes(small_w * small_h))
        .resize((width, height), Image.Resampling.BICUBIC)
        .convert("RGB")
    )
    img = Image.blend(img, blotch, 0.03)

    overlay = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay)
    font_size = max(20, width // 44)
    font = _load_font(font_size)
    step_y = max(120, height // 5)
    text_width = int(draw.textlength(watermark, font=font)) if watermark else 0
    step_x = max(360, width // 2, text_width + 80)
    for row, y in enumerate(range(-step_y, height + step_y, step_y)):
        x_offset = (row % 2) * (step_x // 2) - step_x // 3
        for x in range(x_offset, width + step_x, step_x):
            draw.text((x, y), watermark, font=font, fill=(65, 65, 65, 30))
    overlay = overlay.rotate(24, resample=Image.Resampling.BICUBIC, center=(width // 2, height // 2))
    img = Image.alpha_composite(img.convert("RGBA"), overlay).convert("RGB")

    buf = io.BytesIO()
    img.save(buf, format="WEBP", quality=quality, method=2)
    return buf.getvalue()


def _atomic_write_bytes(filename: str, content: bytes) -> None:
    target = pathlib.Path(filename)
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_suffix(target.suffix + ".tmp")
    tmp.write_bytes(content)
    tmp.replace(target)


def write_public_webp(fig: Any, filename: str, seed_text: str, source: str = "") -> bool:
    """Render ``fig`` to a hardened WebP at ``filename`` (atomic write)."""
    import plotly.io as pio  # noqa: PLC0415 -- heavy; keeps compare imports light

    try:
        width = int(getattr(fig.layout, "width", None) or 1200)
        height = int(getattr(fig.layout, "height", None) or 800)
        png_bytes = pio.to_image(fig, format="png", width=width, height=height)
        webp_bytes = harden_png_to_webp(png_bytes, seed_text, watermark=watermark_text(source))
    except Exception:
        logger.exception("public image export failed for %s", filename)
        return False
    _atomic_write_bytes(filename, webp_bytes)
    return True


def write_public_webp_batch(items: list[tuple[Any, str, str, str]]) -> int:
    """Render many figures to hardened WebP in one Kaleido session.

    ``items`` holds ``(figure, filename, seed_text, source)`` tuples.
    Batching reuses one browser instead of restarting it per picture
    (measured ~0.4 s/image instead of ~1.4 s/image). Falls back to
    per-figure rendering if the batch call fails. Returns the number
    of pictures written.
    """
    if not items:
        return 0
    import plotly.io as pio  # noqa: PLC0415 -- heavy; keeps compare imports light

    written = 0
    first_parent = pathlib.Path(items[0][1]).parent
    first_parent.mkdir(parents=True, exist_ok=True)
    tmp_root = first_parent / f".kaleido-tmp-{os.getpid()}"

    def figure_size(fig: Any) -> tuple[int, int]:
        return (
            int(getattr(fig.layout, "width", None) or 1200),
            int(getattr(fig.layout, "height", None) or 800),
        )

    groups: dict[tuple[int, int], list[tuple[Any, str, str, str]]] = {}
    for item in items:
        groups.setdefault(figure_size(item[0]), []).append(item)

    try:
        chunk_size = 32
        for (fig_width, fig_height), group in groups.items():
            for start in range(0, len(group), chunk_size):
                chunk = group[start:start + chunk_size]
                tmp_root.mkdir(parents=True, exist_ok=True)
                png_paths = [str(tmp_root / f"{i}.png") for i in range(len(chunk))]
                figs = [item[0] for item in chunk]
                try:
                    pio.write_images(figs, png_paths, format="png", width=fig_width, height=fig_height)  # pyright: ignore[reportArgumentType]
                except Exception:
                    logger.exception("batch image export failed; falling back per figure")
                    for fig, filename, seed_text, source in chunk:
                        written += bool(write_public_webp(fig, filename, seed_text, source=source))
                    continue
                for (_fig, filename, seed_text, source), png_path in zip(chunk, png_paths, strict=True):
                    try:
                        png_bytes = pathlib.Path(png_path).read_bytes()
                        webp_bytes = harden_png_to_webp(
                            png_bytes, seed_text, watermark=watermark_text(source)
                        )
                        _atomic_write_bytes(filename, webp_bytes)
                        written += 1
                    except Exception:
                        logger.exception("public image hardening failed for %s", filename)
                for png_path in png_paths:
                    pathlib.Path(png_path).unlink(missing_ok=True)
    finally:
        try:
            tmp_root.rmdir()
        except OSError:
            pass
    return written


_BDATA_DTYPES = {"f8": "d", "f4": "f", "i1": "b", "i2": "h", "i4": "i", "i8": "q", "u1": "B", "u2": "H", "u4": "I", "u8": "Q"}


def _numeric_list(value: Any) -> list[float] | None:
    if value is None:
        return None
    tolist = getattr(value, "tolist", None)
    if callable(tolist):
        value = tolist()
    if isinstance(value, dict):
        dtype = value.get("dtype")
        bdata = value.get("bdata")
        fmt = _BDATA_DTYPES.get(dtype) if isinstance(dtype, str) else None
        if fmt is None or not isinstance(bdata, str):
            return None
        raw = base64.b64decode(bdata)
        size = struct.calcsize("<" + fmt)
        if size == 0 or len(raw) % size:
            return None
        return [float(v) for v in struct.unpack(f"<{len(raw) // size}{fmt}", raw)]
    try:
        return [float(v) for v in value]
    except (TypeError, ValueError):
        return None


def _interp(xs: list[float], ys: list[float], x: float) -> float:
    if x < xs[0] or x > xs[-1]:
        return math.nan
    pos = bisect.bisect_left(xs, x)
    if pos == 0:
        return ys[0]
    if pos >= len(xs):
        return ys[-1]
    x0, x1 = xs[pos - 1], xs[pos]
    y0, y1 = ys[pos - 1], ys[pos]
    if x0 <= 0 or x1 <= x0:
        return y0
    frac = (math.log(x) - math.log(x0)) / (math.log(x1) - math.log(x0))
    return y0 + frac * (y1 - y0)


def extract_trace_bundle(fig: Any, graph_name: str, points: int = 1024) -> dict[str, Any] | None:
    """Resample scatter traces onto one log-frequency grid, int16-quantised.

    Returns ``None`` when the figure has no frequency scatter traces
    (contours, 3D surfaces, radar are served as side-by-side images
    instead of comparison overlays).
    """
    traces_in: list[tuple[str, str, str, str, list[float], list[float]]] = []
    for trace in getattr(fig, "data", []) or []:
        if getattr(trace, "type", None) not in (None, "scatter", "scattergl"):
            continue
        if getattr(trace, "visible", True) in (False, "legendonly"):
            continue
        xs = _numeric_list(getattr(trace, "x", None))
        ys = _numeric_list(getattr(trace, "y", None))
        if xs is None or ys is None or len(xs) != len(ys) or len(xs) < 2:
            continue
        pairs = sorted((x, y) for x, y in zip(xs, ys, strict=True) if x > 0 and math.isfinite(y))
        if len(pairs) < 2:
            continue
        sx = [p[0] for p in pairs]
        sy = [p[1] for p in pairs]
        line = getattr(trace, "line", None)
        color = str(getattr(line, "color", "") or "")
        dash = str(getattr(line, "dash", "") or "solid")
        name = str(getattr(trace, "name", "") or f"trace{len(traces_in)}")
        axis = "y2" if str(getattr(trace, "yaxis", "y") or "y") == "y2" else "y"
        traces_in.append((name, color, dash, axis, sx, sy))
    if not traces_in:
        return None

    xmin = min(t[4][0] for t in traces_in)
    xmax = max(t[4][-1] for t in traces_in)
    if xmin <= 0 or xmax <= xmin:
        return None
    grid = [xmin * (xmax / xmin) ** (i / (points - 1)) for i in range(points)]

    title = ""
    layout_title = getattr(getattr(fig, "layout", None), "title", None)
    if layout_title is not None:
        title = str(getattr(layout_title, "text", "") or "")

    traces_out: list[dict[str, Any]] = []
    ymin: float | None = None
    ymax: float | None = None
    for name, color, dash, axis, sx, sy in traces_in:
        quantised: list[int] = []
        for x in grid:
            y = _interp(sx, sy, x)
            if math.isfinite(y):
                if axis == "y":
                    ymin = y if ymin is None else min(ymin, y)
                    ymax = y if ymax is None else max(ymax, y)
                quantised.append(max(-32767, min(32767, round(y * 100))))
            else:
                quantised.append(_SENTINEL)
        traces_out.append(
            {
                "name": name,
                "color": color,
                "dash": dash,
                "axis": axis,
                "y_int16_b64": base64.b64encode(
                    struct.pack(f"<{len(quantised)}h", *quantised)
                ).decode(),
            }
        )
    def axis_range(attr: str) -> list[float] | None:
        axis_obj = getattr(getattr(fig, "layout", None), attr, None)
        rng = getattr(axis_obj, "range", None)
        if rng is None:
            return None
        try:
            values = [float(v) for v in rng]
        except (TypeError, ValueError):
            return None
        return values if len(values) == 2 else None

    return {
        "v": _BUNDLE_VERSION,
        "graph": graph_name,
        "title": title,
        "points": points,
        "freq_min": xmin,
        "freq_max": xmax,
        "y_min": ymin,
        "y_max": ymax,
        "y_range": axis_range("yaxis"),
        "y2_range": axis_range("yaxis2"),
        "traces": traces_out,
    }


def bundle_y_values(trace: dict[str, Any]) -> list[float]:
    """Decode one bundle trace back to dB values (NaN where out of range)."""
    raw = base64.b64decode(str(trace["y_int16_b64"]))
    ints = struct.unpack(f"<{len(raw) // 2}h", raw)
    return [math.nan if v == _SENTINEL else v / 100.0 for v in ints]


def write_trace_bundle(fig: Any, graph_name: str, filename: str) -> bool:
    bundle = extract_trace_bundle(fig, graph_name)
    if bundle is None:
        return False
    target = pathlib.Path(filename)
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_suffix(target.suffix + ".tmp")
    tmp.write_text(json.dumps(bundle), encoding="utf-8")
    tmp.replace(target)
    return True
