#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Lazy comparison-picture service with a hard render budget.

Serves ``GET /compare.webp`` overlays rendered from private trace
bundles, cached on disk by content hash. Designed for a weak VPS:

- stdlib HTTP server + Pillow renderer only (no Plotly / browser);
- at most one render at a time, plus a per-minute render budget;
- cache hits are plain file reads; budget exhaustion returns 503 with
  ``Retry-After`` so the frontend can show side-by-side pictures;
- only whitelisted line-graph measurements and width buckets.

Environment:
  SPINORAMA_PRIVATE_SPEAKERS_ROOT  private speakers tree (default
    ``dist-private/speakers`` relative to the current directory)
  SPINORAMA_COMPARE_CACHE          cache directory
    (default ``dist/compare-cache``)
  SPINORAMA_COMPARE_MAX_RENDERS_PER_MIN  default 20
  SPINORAMA_COMPARE_HOST / _PORT   bind address (default 127.0.0.1:8765)
"""

from __future__ import annotations

import collections
import contextlib
import hashlib
import json
import os
import pathlib
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "src"))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from spinorama.compare_render import COMPARE_CACHE_VERSION, render_compare
from spinorama.image_export import source_label

ALLOWED_MEASUREMENTS = {
    "CEA2034",
    "CEA2034 Normalized",
    "On Axis",
    "Estimated In-Room Response",
    "Estimated In-Room Response Normalized",
    "Early Reflections",
    "Horizontal Reflections",
    "Vertical Reflections",
    "SPL Horizontal",
    "SPL Horizontal Normalized",
    "SPL Vertical",
    "SPL Vertical Normalized",
    "Horizontal Directivity",
    "Vertical Directivity",
}
ALLOWED_WIDTHS = {900, 1400}

_render_lock = threading.Lock()
_render_times: collections.deque[float] = collections.deque()


def private_root() -> pathlib.Path:
    return pathlib.Path(
        os.environ.get("SPINORAMA_PRIVATE_SPEAKERS_ROOT", "dist-private/speakers")
    )


def cache_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_COMPARE_CACHE", "dist/compare-cache"))


def max_renders_per_min() -> int:
    try:
        return max(1, int(os.environ.get("SPINORAMA_COMPARE_MAX_RENDERS_PER_MIN", "20")))
    except ValueError:
        return 20


def _safe(value: str) -> bool:
    return bool(value) and "/" not in value and "\\" not in value and ".." not in value


def bundle_path(speaker: str, origin: str, version: str, measurement: str) -> pathlib.Path:
    return private_root() / speaker / origin / version / "traces" / f"{measurement}.json"


def _budget_ok(now: float) -> bool:
    while _render_times and now - _render_times[0] > 60.0:
        _render_times.popleft()
    return len(_render_times) < max_renders_per_min()


class Handler(BaseHTTPRequestHandler):
    server_version = "spinorama-compare/1"

    def log_message(self, fmt: str, *args: object) -> None:
        sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))

    def _send(self, code: int, body: bytes, content_type: str, extra: dict[str, str] | None = None) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for key, value in (extra or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def _error(self, code: int, message: str, extra: dict[str, str] | None = None) -> None:
        self._send(code, (message + "\n").encode(), "text/plain; charset=utf-8", extra)

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path == "/healthz":
            self._send(200, b"ok\n", "text/plain")
            return
        if parsed.path != "/compare.webp":
            self._error(404, "not found")
            return
        params = {k: v[0] for k, v in parse_qs(parsed.query).items() if v}
        measurement = params.get("measurement", "")
        if measurement not in ALLOWED_MEASUREMENTS:
            self._error(400, "measurement not allowed for comparison")
            return
        try:
            width = int(params.get("w", "1400"))
        except ValueError:
            width = 0
        if width not in ALLOWED_WIDTHS:
            self._error(400, "width must be one of 900, 1400")
            return
        sides: list[tuple[str, str, str]] = []
        for prefix in ("a", "b"):
            speaker = params.get(f"{prefix}_speaker", "")
            origin = params.get(f"{prefix}_origin", "")
            version = params.get(f"{prefix}_version", "")
            if not (_safe(speaker) and _safe(origin) and _safe(version)):
                self._error(400, "invalid speaker, origin, or version")
                return
            sides.append((speaker, origin.replace("Vendors-", ""), version))

        paths = [bundle_path(s, o, v, measurement) for s, o, v in sides]
        if not all(p.is_file() for p in paths):
            self._error(404, "comparison data not available")
            return

        digest = hashlib.sha256()
        digest.update(f"{COMPARE_CACHE_VERSION}|{measurement}|{width}|".encode())
        for path in paths:
            digest.update(path.read_bytes())
        cache_file = cache_root() / f"{digest.hexdigest()}.webp"
        if cache_file.is_file() and cache_file.stat().st_size > 0:
            self._send(
                200,
                cache_file.read_bytes(),
                "image/webp",
                {"Cache-Control": "public, max-age=31536000, immutable", "X-Compare-Cache": "hit"},
            )
            return

        if not _render_lock.acquire(blocking=False):
            self._error(503, "renderer busy", {"Retry-After": "5"})
            return
        try:
            now = time.time()
            if not _budget_ok(now):
                self._error(503, "render budget exhausted", {"Retry-After": "30"})
                return
            if cache_file.is_file() and cache_file.stat().st_size > 0:
                body = cache_file.read_bytes()
            else:
                _render_times.append(now)
                bundle_a = json.loads(paths[0].read_text(encoding="utf-8"))
                bundle_b = json.loads(paths[1].read_text(encoding="utf-8"))
                body = render_compare(
                    bundle_a,
                    bundle_b,
                    name_a=f"{sides[0][0]} ({sides[0][1]})",
                    name_b=f"{sides[1][0]} ({sides[1][1]})",
                    graph_name=measurement,
                    width=width,
                    source_a=source_label(sides[0][1], sides[0][2]),
                    source_b=source_label(sides[1][1], sides[1][2]),
                )
                cache_root().mkdir(parents=True, exist_ok=True)
                tmp = cache_file.with_suffix(".webp.tmp")
                tmp.write_bytes(body)
                tmp.replace(cache_file)
        except Exception as error:  # keep the service alive on bad bundles
            self._error(500, f"render failed: {error}")
            return
        finally:
            _render_lock.release()
        self._send(
            200,
            body,
            "image/webp",
            {"Cache-Control": "public, max-age=31536000, immutable", "X-Compare-Cache": "miss"},
        )


def main() -> int:
    host = os.environ.get("SPINORAMA_COMPARE_HOST", "127.0.0.1")
    port = int(os.environ.get("SPINORAMA_COMPARE_PORT", "8765"))
    cache_root().mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer((host, port), Handler)
    print(f"compare render service on {host}:{port}", flush=True)
    with contextlib.suppress(KeyboardInterrupt):
        server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
