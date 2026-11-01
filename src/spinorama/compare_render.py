# -*- coding: utf-8 -*-
"""Cheap comparison-picture renderer over private trace bundles.

Renders the overlay of two speakers' bundles (see
:mod:`spinorama.image_export`) with Pillow only — no Plotly, no numpy,
no browser — so it can run in a single small worker on a weak VPS.
Speaker A is drawn solid, speaker B dashed, mirroring the historical
Plotly compare view. The output passes through the same watermark /
texture hardening as single-speaker pictures.
"""

from __future__ import annotations

import io
import math
from typing import Any

from PIL import Image, ImageDraw

from spinorama.image_export import _load_font, bundle_y_values, harden_png_to_webp, watermark_text

COMPARE_CACHE_VERSION = "compare-v2-source-watermark"

_FALLBACK_COLORS = (
    "#5c77a5", "#dc842a", "#c85857", "#89b5b1", "#71a152",
    "#bab0ac", "#e15759", "#b07aa1", "#76b7b2", "#ff9da7",
)
_GRID = (205, 205, 205)
_TEXT = (35, 35, 35)
_AXIS = (120, 120, 120)
_SS = 2  # supersampling factor for anti-aliasing


def _parse_color(value: str, fallback_idx: int) -> tuple[int, int, int]:
    text = (value or "").strip()
    if text.startswith("#") and len(text) == 7:
        try:
            return int(text[1:3], 16), int(text[3:5], 16), int(text[5:7], 16)
        except ValueError:
            pass
    if text.startswith("rgb(") and text.endswith(")"):
        try:
            parts = [int(float(p.strip())) for p in text[4:-1].split(",")[:3]]
            if len(parts) == 3:
                return parts[0], parts[1], parts[2]
        except ValueError:
            pass
    return _parse_color(_FALLBACK_COLORS[fallback_idx % len(_FALLBACK_COLORS)], 0)


def _bundle_freqs(bundle: dict[str, Any]) -> list[float]:
    points = int(bundle["points"])
    fmin = float(bundle["freq_min"])
    fmax = float(bundle["freq_max"])
    if points < 2 or fmin <= 0 or fmax <= fmin:
        return []
    return [fmin * (fmax / fmin) ** (i / (points - 1)) for i in range(points)]


def _axis_range(
    bundles: list[dict[str, Any]], axis: str
) -> tuple[float, float] | None:
    """Union of layout ranges for one axis, or data range as fallback."""
    lo: float | None = None
    hi: float | None = None
    key = "y_range" if axis == "y" else "y2_range"
    for bundle in bundles:
        rng = bundle.get(key)
        if isinstance(rng, list) and len(rng) == 2:
            lo = float(rng[0]) if lo is None else min(lo, float(rng[0]))
            hi = float(rng[1]) if hi is None else max(hi, float(rng[1]))
    if lo is not None and hi is not None and hi > lo:
        return lo, hi
    values: list[float] = []
    for bundle in bundles:
        for trace in bundle.get("traces", []):
            if trace.get("axis", "y") != axis:
                continue
            values.extend(v for v in bundle_y_values(trace) if math.isfinite(v))
    if not values:
        return None
    lo, hi = min(values), max(values)
    pad = max(1.0, (hi - lo) * 0.08)
    return lo - pad, hi + pad


def _has_axis(bundle: dict[str, Any], axis: str) -> bool:
    return any(t.get("axis", "y") == axis for t in bundle.get("traces", []))


def _visible_traces(bundle: dict[str, Any]) -> list[dict[str, Any]]:
    """Measurement traces only: drop recommended zones and slope helpers."""
    kept: list[dict[str, Any]] = []
    for trace in bundle.get("traces", []):
        name = str(trace.get("name", "")).lower()
        if name.startswith("recommended") or name.endswith(" slope") or name.endswith(" zone"):
            continue
        kept.append(trace)
    return kept


class _Canvas:
    def __init__(self, width: int, height: int) -> None:
        self.w = width * _SS
        self.h = height * _SS
        self.img = Image.new("RGB", (self.w, self.h), "white")
        self.draw = ImageDraw.Draw(self.img)
        self.font = _load_font(max(18, width // 64) * _SS)
        self.font_small = _load_font(max(15, width // 80) * _SS)
        self.font_title = _load_font(max(22, width // 48) * _SS)

    def s(self, value: float) -> float:
        return value * _SS


def _draw_polyline(
    draw: ImageDraw.ImageDraw,
    points: list[tuple[float, float]],
    color: tuple[int, int, int],
    *,
    dashed: bool,
    width: int,
) -> None:
    """Draw a polyline, optionally dashed, skipping gaps (NaN breaks)."""
    dash = 14 * _SS
    gap = 9 * _SS
    remaining = dash
    drawing = True
    prev: tuple[float, float] | None = None
    for point in points:
        if not (math.isfinite(point[0]) and math.isfinite(point[1])):
            prev = None
            continue
        if prev is not None:
            dist = math.hypot(point[0] - prev[0], point[1] - prev[1])
            if not dashed:
                draw.line([prev, point], fill=color, width=width)
            else:
                # Walk the segment toggling dash/gap every dash/gap pixels.
                travelled = 0.0
                while travelled < dist:
                    step = min(remaining, dist - travelled)
                    if dist > 0 and drawing and step > 0:
                        a = (
                            prev[0] + (point[0] - prev[0]) * (travelled / dist),
                            prev[1] + (point[1] - prev[1]) * (travelled / dist),
                        )
                        b = (
                            prev[0] + (point[0] - prev[0]) * ((travelled + step) / dist),
                            prev[1] + (point[1] - prev[1]) * ((travelled + step) / dist),
                        )
                        draw.line([a, b], fill=color, width=width)
                    travelled += step
                    remaining -= step
                    if remaining <= 0:
                        drawing = not drawing
                        remaining = dash if drawing else gap
        prev = point


def _nice_ticks(lo: float, hi: float) -> list[float]:
    span = hi - lo
    for step in (1, 2, 5, 10, 20, 25, 50):
        if span / step <= 10:
            break
    start = math.ceil(lo / step) * step
    ticks: list[float] = []
    value = float(start)
    while value <= hi + 1e-9:
        ticks.append(value)
        value += step
    return ticks


def render_compare(
    bundle_a: dict[str, Any],
    bundle_b: dict[str, Any],
    *,
    name_a: str,
    name_b: str,
    graph_name: str,
    width: int = 1400,
    source_a: str = "",
    source_b: str = "",
) -> bytes:
    """Render an A-vs-B overlay comparison as hardened WebP bytes."""
    bundles = [bundle_a, bundle_b]
    names = [name_a, name_b]
    has_y2 = any(_has_axis(b, "y2") for b in bundles)
    y_range = _axis_range(bundles, "y") or (-40.0, 10.0)
    y2_range = _axis_range(bundles, "y2") if has_y2 else None

    n_traces = max(len(_visible_traces(b)) for b in bundles)
    legend_rows = max(1, math.ceil(n_traces / 3)) * 2 + 5
    height = int(width * 2 / 3) + legend_rows * 18
    canvas = _Canvas(width, height)
    d = canvas.draw
    s = canvas.s

    margin_l, margin_r = 64, (64 if has_y2 else 18)
    margin_t, margin_b = 64, 52 + legend_rows * 18
    plot_x0, plot_y0 = s(margin_l), s(margin_t)
    plot_x1, plot_y1 = s(width - margin_r), s(height - margin_b)
    plot_w, plot_h = plot_x1 - plot_x0, plot_y1 - plot_y0

    freq_lo = min(float(b["freq_min"]) for b in bundles)
    freq_hi = max(float(b["freq_max"]) for b in bundles)
    log_lo, log_hi = math.log10(max(freq_lo, 1.0)), math.log10(freq_hi)

    def x_of(freq: float) -> float:
        return plot_x0 + (math.log10(freq) - log_lo) / (log_hi - log_lo) * plot_w

    def y_of(value: float, rng: tuple[float, float]) -> float:
        return plot_y1 - (value - rng[0]) / (rng[1] - rng[0]) * plot_h

    title = f"{graph_name}: {name_a} vs {name_b}"
    d.text((plot_x0, s(14)), title, font=canvas.font_title, fill=_TEXT)

    # Grid and axes.
    for freq in (20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000):
        if freq_lo <= freq <= freq_hi:
            x = x_of(float(freq))
            d.line([(x, plot_y0), (x, plot_y1)], fill=_GRID, width=_SS)
            label = f"{freq // 1000}k" if freq >= 1000 else str(freq)
            d.text((x - s(10), plot_y1 + s(6)), label, font=canvas.font_small, fill=_TEXT)
    for tick in _nice_ticks(*y_range):
        y = y_of(tick, y_range)
        d.line([(plot_x0, y), (plot_x1, y)], fill=_GRID, width=_SS)
        d.text((s(8), y - s(8)), f"{tick:g}", font=canvas.font_small, fill=_TEXT)
    if y2_range is not None:
        for tick in _nice_ticks(*y2_range):
            y = y_of(tick, y2_range)
            d.text((plot_x1 + s(8), y - s(8)), f"{tick:g}", font=canvas.font_small, fill=_TEXT)
    d.rectangle([plot_x0, plot_y0, plot_x1, plot_y1], outline=_AXIS, width=_SS)
    d.text((plot_x0 + plot_w / 2 - s(40), s(height - margin_b + 28)), "Frequency (Hz)",
           font=canvas.font_small, fill=_TEXT)

    # Traces.
    for speaker_idx, bundle in enumerate(bundles):
        freqs = _bundle_freqs(bundle)
        if not freqs:
            continue
        dashed = speaker_idx % 2 == 1
        for trace_idx, trace in enumerate(_visible_traces(bundle)):
            ys = bundle_y_values(trace)
            rng = y2_range if trace.get("axis", "y") == "y2" and y2_range else y_range
            color = _parse_color(str(trace.get("color", "")), trace_idx)
            points = [
                (x_of(f), y_of(v, rng)) if math.isfinite(v) else (math.nan, math.nan)
                for f, v in zip(freqs, ys, strict=False)
            ]
            _draw_polyline(d, points, color, dashed=dashed, width=2 * _SS)

    # Legend: one block per speaker, trace names from speaker A's bundle
    # (falling back to B), swatches solid for A and dashed for B.
    legend_y = plot_y1 + s(34)
    for speaker_idx, bundle in enumerate(bundles):
        d.text((plot_x0, legend_y), names[speaker_idx] + (" (dashed)" if speaker_idx else ""),
               font=canvas.font, fill=_TEXT)
        legend_y += s(20)
        x = plot_x0
        for trace_idx, trace in enumerate(_visible_traces(bundle)):
            col = trace_idx
            color = _parse_color(str(trace.get("color", "")), trace_idx)
            _draw_polyline(
                d, [(x, legend_y + s(7)), (x + s(26), legend_y + s(7))],
                color, dashed=bool(speaker_idx), width=2 * _SS,
            )
            label = str(trace.get("name", ""))
            d.text((x + s(32), legend_y), label, font=canvas.font_small, fill=_TEXT)
            try:
                text_w = d.textlength(label, font=canvas.font_small)
            except Exception:
                text_w = s(80)
            x += s(38) + text_w + s(18)
            if (col + 1) % 3 == 0:
                x = plot_x0
                legend_y += s(18)
        legend_y += s(22)

    final = canvas.img.resize((width, height), Image.Resampling.LANCZOS)
    buf = io.BytesIO()
    final.save(buf, format="PNG")
    seed = f"compare/{graph_name}/{name_a}/{name_b}"
    sources = " + ".join(dict.fromkeys(s for s in (source_a, source_b) if s))
    return harden_png_to_webp(buf.getvalue(), seed, watermark=watermark_text(sources))


