#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Pre-render comparison pictures for similar-speaker pairs, offline.

Reads each speaker's ``nearest`` neighbours from the generated metadata
and fills the same content-hash cache the compare render service serves,
so the weak web host only ever serves files for the predictable pairs.

Run from the repository root after ``generate_graphs.py`` and
``generate_meta.py``::

    PYTHONPATH=src:scripts:. .venv/bin/python scripts/generate_compare_cache.py
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "src"))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from generate_common import find_metadata_file
from spinorama.compare_render import COMPARE_CACHE_VERSION, render_compare
from spinorama.image_export import source_label
from spinorama.misc import sanitize_filename


def private_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_PRIVATE_SPEAKERS_ROOT", "dist-private/speakers"))


def cache_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("SPINORAMA_COMPARE_CACHE", "dist/compare-cache"))


def bundle_for(speaker_data: dict, measurement: str) -> tuple[pathlib.Path, str] | None:
    default = speaker_data.get("default_measurement")
    measurements = speaker_data.get("measurements", {})
    if not default or default not in measurements:
        return None
    origin = str(measurements[default].get("origin", "")).replace("Vendors-", "")
    speaker = sanitize_filename(f"{speaker_data.get('brand', '')} {speaker_data.get('model', '')}")
    path = private_root() / speaker / origin / str(default) / "traces" / f"{measurement}.json"
    return (path, origin) if path.is_file() else None


def cache_file_for(paths: list[pathlib.Path], measurement: str, width: int) -> pathlib.Path:
    digest = hashlib.sha256()
    digest.update(f"{COMPARE_CACHE_VERSION}|{measurement}|{width}|".encode())
    for path in paths:
        digest.update(path.read_bytes())
    return cache_root() / f"{digest.hexdigest()}.webp"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--measurement", default="CEA2034")
    parser.add_argument("--width", type=int, default=900, choices=(900, 1400))
    parser.add_argument("--limit", type=int, default=0, help="Max speakers to process (0 = all)")
    parser.add_argument("--metadata", default=None)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)

    metadata_path = args.metadata
    if metadata_path is None:
        metadata_path, _ = find_metadata_file()
    if metadata_path is None:
        print("Cannot find metadata file; run generate_meta.py first", file=sys.stderr)
        return 1
    raw = json.loads(pathlib.Path(metadata_path).read_text(encoding="utf-8"))
    records = list(raw.values()) if isinstance(raw, dict) else list(raw)
    by_name = {f"{r.get('brand', '')} {r.get('model', '')}": r for r in records if isinstance(r, dict)}

    rendered = skipped = cached = 0
    for index, (name, data) in enumerate(sorted(by_name.items())):
        if args.limit and index >= args.limit:
            break
        neighbours = data.get("nearest") or []
        if not neighbours:
            continue
        found_a = bundle_for(data, args.measurement)
        if found_a is None:
            skipped += 1
            continue
        path_a, origin_a = found_a
        for entry in neighbours:
            neighbour_name = entry[1] if isinstance(entry, (list, tuple)) and len(entry) > 1 else None
            if neighbour_name is None or neighbour_name not in by_name:
                continue
            found_b = bundle_for(by_name[neighbour_name], args.measurement)
            if found_b is None:
                skipped += 1
                continue
            path_b, origin_b = found_b
            target = cache_file_for([path_a, path_b], args.measurement, args.width)
            if target.is_file() and target.stat().st_size > 0 and not args.force:
                cached += 1
                continue
            body = render_compare(
                json.loads(path_a.read_text(encoding="utf-8")),
                json.loads(path_b.read_text(encoding="utf-8")),
                name_a=f"{sanitize_filename(name)} ({origin_a})",
                name_b=f"{sanitize_filename(neighbour_name)} ({origin_b})",
                graph_name=args.measurement,
                width=args.width,
                source_a=source_label(origin_a),
                source_b=source_label(origin_b),
            )
            target.parent.mkdir(parents=True, exist_ok=True)
            tmp = target.with_suffix(".webp.tmp")
            tmp.write_bytes(body)
            tmp.replace(target)
            rendered += 1
    print(f"compare cache: rendered={rendered} already-cached={cached} skipped={skipped}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
