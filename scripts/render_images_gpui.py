#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Render public graph pictures with the GPUI renderer (macOS, offline).

Fast alternative to the Kaleido/Chromium step in ``generate_graphs.py``
for the graph kinds the GPUI renderer covers (CEA2034 and SPL contours).
The Rust ``spinorama-render-images`` binary turns the private Plotly
JSON into chart-only PNGs; this driver then applies the exact same
downscale, watermark and lossy WebP hardening as the Kaleido path
(``spinorama.image_export.harden_png_to_webp``), writing to
``dist-gpui/speakers`` by default so the Kaleido output tree is never
touched. Graph kinds the GPUI renderer does not cover keep using
``generate_graphs.py``.

Run from the repository root, after ``generate_graphs.py`` produced the
private JSON::

    PYTHONPATH=src:scripts:. .venv/bin/python scripts/render_images_gpui.py \
        --binary /Volumes/home_tmp/src/gpui-toolkit/target/release/spinorama-render-images \
        [--filter 'KEF R3'] [--limit 100]
"""

from __future__ import annotations

import argparse
import io
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "src"))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from PIL import Image

from spinorama.image_export import harden_png_to_webp, source_label, watermark_text

TARGET_WIDTH = 1200


def private_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_PRIVATE_SPEAKERS_ROOT", "dist-private/speakers"))


def staging_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_GPUI_PNG_ROOT", "/Volumes/home_tmp/tmp/spinorama-gpui-png"))


def output_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_GPUI_OUT_ROOT", "dist-gpui/speakers"))


def atomic_write_bytes(path: pathlib.Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(data)
    tmp.replace(path)


def harden_png(png_path: pathlib.Path, webp_path: pathlib.Path, relative: pathlib.Path) -> None:
    """Downscale a 2x capture to site width, then apply the standard hardening."""
    parts = relative.parts
    origin = parts[1] if len(parts) > 1 else ""
    version = parts[2] if len(parts) > 2 else ""
    seed_text = relative.with_suffix("").as_posix()
    with Image.open(png_path) as img:
        img = img.convert("RGB")
        if img.width != TARGET_WIDTH:
            height = round(img.height * TARGET_WIDTH / img.width)
            img = img.resize((TARGET_WIDTH, height), Image.Resampling.LANCZOS)
        buffer = io.BytesIO()
        img.save(buffer, format="PNG")
    webp_bytes = harden_png_to_webp(
        buffer.getvalue(),
        seed_text,
        watermark=watermark_text(source_label(origin, version)),
    )
    atomic_write_bytes(webp_path, webp_bytes)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        default=os.environ.get(
            "SPINORAMA_RENDER_IMAGES",
            "/Volumes/home_tmp/src/gpui-toolkit/target/release/spinorama-render-images",
        ),
        help="path to the spinorama-render-images binary",
    )
    parser.add_argument("--private-root", type=pathlib.Path, default=private_root())
    parser.add_argument("--staging-root", type=pathlib.Path, default=staging_root(), help="intermediate PNG tree")
    parser.add_argument("--out-root", type=pathlib.Path, default=output_root(), help="hardened WebP output tree")
    parser.add_argument("--limit", type=int, default=None)
    parser.add_argument("--filter", default=None, help="substring filter on source paths")
    parser.add_argument("--skip-render", action="store_true", help="only harden PNGs already in the staging tree")
    args = parser.parse_args()

    if not args.skip_render:
        binary = pathlib.Path(args.binary)
        if not binary.is_file():
            print(f"renderer binary not found: {binary}", file=sys.stderr)
            return 2
        cmd = [str(binary), str(args.private_root), str(args.staging_root)]
        if args.limit is not None:
            cmd += ["--limit", str(args.limit)]
        if args.filter:
            cmd += ["--filter", args.filter]
        print("+", " ".join(cmd))
        result = subprocess.run(cmd, check=False)
        if result.returncode != 0:
            print(f"renderer exited with {result.returncode}; hardening what was produced", file=sys.stderr)

    pngs = sorted(args.staging_root.rglob("*.png"))
    if args.filter:
        pngs = [p for p in pngs if args.filter in str(p.relative_to(args.staging_root))]
    written = 0
    skipped = 0
    failed = 0
    for png_path in pngs:
        relative = png_path.relative_to(args.staging_root)
        webp_path = (args.out_root / relative).with_suffix(".webp")
        if webp_path.is_file() and webp_path.stat().st_mtime >= png_path.stat().st_mtime:
            skipped += 1
            continue
        try:
            harden_png(png_path, webp_path, relative)
            written += 1
        except Exception as exc:  # keep going; report at the end
            failed += 1
            print(f"FAILED {relative}: {exc}", file=sys.stderr)
    print(f"render_images_gpui: {written} written, {skipped} up-to-date, {failed} failed -> {args.out_root}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
