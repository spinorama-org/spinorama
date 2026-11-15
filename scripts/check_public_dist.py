#!/usr/bin/env python3
"""Reject private measurement data in a public website build.

Nginx serves ``dist/`` as a static tree. Numeric speaker/headphone
data (graph JSON, trace bundles, frequency-response CSVs and their
compressed forms) belongs in ``dist-private/`` and, once deployed,
outside every nginx root. Catalogue/manifest JSON used by the
browser itself and owner-approved examples are explicitly allowed.
"""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys

# Only named catalogue formats may contain numeric JSON. Compressed variants
# are decoded and checked using the same schema, never trusted by directory.
import bz2
import gzip
import json
import re
import zipfile
import zlib

POLICY = json.loads(
    (pathlib.Path(__file__).resolve().parents[1] / "conf/public-data-policy.json").read_text()
)
APPROVED_EXAMPLES = {
    destination: POLICY["approved_source_blobs"][source]
    for destination, source in POLICY["approved_website_examples"].items()
}

MAX_JSON_BYTES = 32 * 1024 * 1024
CATALOGUE = re.compile(
    r"json/(?:metadata(?:-head|-20[0-9]{2})?|headphone_metadata|eqdata|headphone_eqdata)\.json$"
)
RECORD_FIELDS = set(
    [
        "amount",
        "brand",
        "default_eq",
        "default_measurement",
        "measurements",
        "model",
        "nearest",
        "price",
        "shape",
        "type",
        "kind",
        "score",
        "score_eq",
        "eqs",
    ]
)
MEASUREMENT_FIELDS = set(
    [
        "computed_sensitivity",
        "data_acquisition",
        "estimates",
        "estimates_eq",
        "extras",
        "format",
        "notes",
        "origin",
        "parameters",
        "pref_rating",
        "pref_rating_eq",
        "quality",
        "review",
        "review_published",
        "reviews",
        "scaled_pref_rating",
        "scaled_pref_rating_eq",
        "specifications",
        "symmetry",
        "recommendation",
        "sensitivity_mV_94dB",
    ]
)
PUBLIC_SUFFIXES = {
    ".html",
    ".css",
    ".js",
    ".mjs",
    ".webp",
    ".png",
    ".jpg",
    ".jpeg",
    ".svg",
    ".ico",
    ".woff",
    ".woff2",
    ".xml",
    ".version",
}


def scalar_tree(value):
    """Metadata contains scalar properties, never sampled numeric arrays."""
    if isinstance(value, dict):
        return all(
            k not in {"x", "y", "z", "bdata", "dtype", "traces", "data"} and scalar_tree(v)
            for k, v in value.items()
        )
    if isinstance(value, list):
        return all(isinstance(v, str) for v in value)
    return value is None or isinstance(value, (str, int, float, bool))


def valid_catalogue(value, eq):
    if not isinstance(value, dict):
        return False
    for record in value.values():
        if not isinstance(record, dict) or not set(record) <= RECORD_FIELDS:
            return False
        if not isinstance(record.get("brand"), str) or not isinstance(record.get("model"), str):
            return False
        for key, data in record.items():
            if key == "nearest":
                if not isinstance(data, list) or not all(
                    isinstance(pair, list)
                    and len(pair) == 2
                    and isinstance(pair[0], (int, float))
                    and isinstance(pair[1], str)
                    for pair in data
                ):
                    return False
            elif key == "measurements":
                if not isinstance(data, dict) or not all(
                    isinstance(m, dict) and set(m) <= MEASUREMENT_FIELDS and scalar_tree(m)
                    for m in data.values()
                ):
                    return False
            elif key == "eqs":
                if not eq or not isinstance(data, dict):
                    return False
                for settings in data.values():
                    if not isinstance(settings, dict) or not set(settings) <= {
                        "display_name",
                        "filename",
                        "peq",
                        "preamp_gain",
                        "type",
                    }:
                        return False
                    for field, item in settings.items():
                        if field == "peq":
                            if not isinstance(item, list) or not all(
                                isinstance(f, dict)
                                and set(f) <= {"Q", "dbGain", "freq", "srate", "type"}
                                and all(isinstance(v, (int, float)) for v in f.values())
                                for f in item
                            ):
                                return False
                        elif not scalar_tree(item):
                            return False
            elif not scalar_tree(data):
                return False
    return True


def valid_json(path, relative):
    name = relative.as_posix()
    base = re.sub(r"\.(bz2|gz|zip)$", "", name)
    if not (CATALOGUE.fullmatch(base) or base in {"manifest.json", "docs/search.json"}):
        return False
    try:
        if name.endswith(".zip") or (name.endswith(".bz2") and zipfile.is_zipfile(path)):
            with zipfile.ZipFile(path) as archive:
                entries = archive.infolist()
                if len(entries) != 1 or entries[0].filename not in {
                    pathlib.PurePosixPath(base).name,
                    base,
                    "dist/" + base,
                }:
                    return False
                with archive.open(entries[0]) as stream:
                    raw = stream.read(MAX_JSON_BYTES + 1)
        else:
            opener = (
                bz2.open if name.endswith(".bz2") else gzip.open if name.endswith(".gz") else open
            )
            with opener(path, "rb") as stream:
                raw = stream.read(MAX_JSON_BYTES + 1)
        if len(raw) > MAX_JSON_BYTES:
            return False
        value = json.loads(raw)
        if CATALOGUE.fullmatch(base):
            return valid_catalogue(value, "eqdata" in base)
        if base == "docs/search.json":
            return isinstance(value, list) and all(
                isinstance(v, dict)
                and set(v) <= {"objectID", "href", "title", "section", "text", "crumbs"}
                and all(
                    isinstance(t, str)
                    if k != "crumbs"
                    else isinstance(t, list) and all(isinstance(c, str) for c in t)
                    for k, t in v.items()
                )
                for v in value
            )
        return (
            isinstance(value, dict)
            and set(value)
            <= {
                "manifest_version",
                "name",
                "short_name",
                "description",
                "version",
                "start_url",
                "display",
                "theme_color",
                "background_color",
                "icons",
                "scope",
                "orientation",
            }
            and all(scalar_tree(v) for k, v in value.items() if k != "icons")
            and all(
                isinstance(icon, dict)
                and set(icon) <= {"src", "sizes", "type", "purpose"}
                and all(isinstance(v, str) for v in icon.values())
                for icon in value.get("icons", [])
            )
        )
    except (
        OSError,
        ValueError,
        TypeError,
        RuntimeError,
        NotImplementedError,
        EOFError,
        zipfile.BadZipFile,
        zlib.error,
    ):
        return False


def matches_approved_blob(path, expected):
    # Git object IDs identify the exact public examples approved by the owner.
    digest = hashlib.sha1(usedforsecurity=False)
    digest.update(f"blob {path.stat().st_size}\0".encode())
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest() == expected


def _is_private_data(relative, path):
    if expected := APPROVED_EXAMPLES.get(relative.as_posix()):
        return not matches_approved_blob(path, expected)
    lower = pathlib.PurePosixPath(str(relative).lower())
    if "traces" in lower.parts:
        return True
    if ".json" in lower.name:
        return not valid_json(path, relative)
    if lower.name == "robots.txt" and str(lower) in {"robots.txt", "docs/robots.txt"}:
        return False
    if lower.name == "measurement-license.txt":
        # License text may be public; numeric response rows may not.
        return bool(
            re.search(r"(?m)^\s*[+-]?[0-9.]+[\s,;]+[+-]?[0-9.]+", path.read_text(errors="replace"))
        )
    return lower.suffix not in PUBLIC_SUFFIXES


def find_private_data(root: pathlib.Path) -> list[pathlib.Path]:
    """Return private measurement files found below a public root."""
    found: list[pathlib.Path] = []
    for path in root.rglob("*"):
        if path.is_symlink():
            found.append(path)
            continue
        if path.is_dir() and not path.is_symlink():
            continue
        relative = pathlib.PurePosixPath(path.relative_to(root).as_posix())
        if _is_private_data(relative, path):
            found.append(path)
    return sorted(found)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        type=pathlib.Path,
        default=pathlib.Path("dist"),
        help="public website tree to inspect (default: dist)",
    )
    args = parser.parse_args(argv)
    root = args.root
    if not root.is_dir():
        print(f"KO public dist check: {root} is not a directory", file=sys.stderr)
        return 2
    found = find_private_data(root)
    if not found:
        print(f"OK public dist check: no private measurement data under {root}")
        return 0
    print(
        f"KO public dist check: {len(found)} private measurement file(s) under {root}:",
        file=sys.stderr,
    )
    for path in found[:50]:
        print(f"  {path.relative_to(root)}", file=sys.stderr)
    if len(found) > 50:
        print(f"  ... and {len(found) - 50} more", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
