#!/usr/bin/env python3
"""Reject private measurement data in a public website build.

Nginx serves ``dist/`` as a static tree. Numeric speaker/headphone
data (graph JSON, trace bundles, frequency-response CSVs and their
compressed forms) belongs in ``dist-private/`` and, once deployed,
outside every nginx root. Catalogue/manifest JSON used by the
browser itself is explicitly allowed.
"""

from __future__ import annotations

import argparse
import pathlib
import sys

PRIVATE_TREES = ("speakers", "headphones")
PRIVATE_SUFFIXES = (
    ".csv",
    ".json",
    ".json.bz2",
    ".json.gz",
    ".json.zip",
)
ALLOWED_JSON = {
    pathlib.PurePosixPath("manifest.json"),
    pathlib.PurePosixPath("docs/search.json"),
}
ALLOWED_JSON_PREFIX = pathlib.PurePosixPath("json")


def _is_private_data(relative: pathlib.PurePosixPath) -> bool:
    lower = pathlib.PurePosixPath(*[part.lower() for part in relative.parts])
    name = lower.name
    if lower in ALLOWED_JSON or (
        lower.parts and pathlib.PurePosixPath(lower.parts[0]) == ALLOWED_JSON_PREFIX
    ):
        return False
    if lower.parts and lower.parts[0] in PRIVATE_TREES:
        if "traces" in lower.parts:
            return True
        return name.endswith(PRIVATE_SUFFIXES)
    # No other machine-readable measurement payloads are expected in
    # the browser bundle; fail closed on JSON/CSV outside the allowlist.
    return name.endswith(PRIVATE_SUFFIXES)


def find_private_data(root: pathlib.Path) -> list[pathlib.Path]:
    """Return private measurement files found below a public root."""
    found: list[pathlib.Path] = []
    resolved_root = root.resolve()
    for path in root.rglob("*"):
        if path.is_symlink() and not path.resolve().is_relative_to(resolved_root):
            found.append(path)
            continue
        if path.is_dir() and not path.is_symlink():
            continue
        relative = pathlib.PurePosixPath(path.relative_to(root).as_posix())
        if _is_private_data(relative):
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
