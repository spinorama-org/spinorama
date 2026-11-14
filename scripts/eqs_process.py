#!/usr/bin/env python3
"""Promote the best completed EQ candidates when their preference score improves."""

import argparse
import json
import math
from pathlib import Path
import sys

from eqs_common import ROOT, Equalization, atomic_write, eq_digest, read_eq, sanitize_filename


def best_candidate(directory: Path) -> Equalization | None:
    candidates = []
    for path in sorted(directory.glob("*/iir-autoeq.txt")):
        try:
            manifest_path = path.parent / "run.json"
            if manifest_path.is_file():
                manifest = json.loads(manifest_path.read_text())
                if manifest.get("status") != "complete" or manifest.get("eq_sha256") != eq_digest(
                    path
                ):
                    continue
            eq = read_eq(path)
            if eq.score > eq.baseline:
                candidates.append(eq)
        except (OSError, ValueError) as error:
            print(f"Skipping {path}: {error}", file=sys.stderr)
    # Stable ties prefer fewer filters, then the lexically first configuration.
    return (
        min(candidates, key=lambda eq: (-eq.score, eq.filters, str(eq.path)))
        if candidates
        else None
    )


def promote(candidate: Equalization, destination: Path, *, activate: bool = False) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    output = destination / "iir-autoeq.txt"
    active = destination / "iir.txt"
    update_active_copy = (
        active.is_file()
        and not active.is_symlink()
        and output.is_file()
        and active.read_bytes() == output.read_bytes()
    )
    config = candidate.path.parent / "conf-autoeq.json"
    if config.is_file():
        text = config.read_text()
        json.loads(text)
        atomic_write(destination / config.name, text)
    atomic_write(output, candidate.text)
    keep_auto_alias = active.is_symlink() and active.readlink() == Path("iir-autoeq.txt")
    missing_active = not active.exists() and not active.is_symlink()
    if (update_active_copy or activate or missing_active) and not keep_auto_alias:
        atomic_write(active, candidate.text)


def process(
    directory: Path,
    output_root: Path,
    *,
    dry_run: bool = False,
    activate: bool = False,
    min_improvement: float = 0.0,
) -> str:
    candidate = best_candidate(directory)
    if candidate is None:
        print(f"{directory.name}: no completed candidate improves the uncorrected score")
        return "skipped"
    destination = output_root / directory.name
    previous_path = destination / "iir-autoeq.txt"
    previous = read_eq(previous_path).score if previous_path.is_file() else candidate.baseline
    if candidate.score <= previous + min_improvement:
        print(f"{directory.name}: keep {previous:.2f}; best candidate {candidate.score:.2f}")
        return "skipped"
    print(
        f"{directory.name}: {candidate.score:.2f} (previous {previous:.2f}, uncorrected {candidate.baseline:.2f}) from {candidate.path.parent.name}"
    )
    if not dry_run:
        promote(candidate, destination, activate=activate)
    return "selected" if dry_run else "updated"


def main(argv: list[str] | None = None) -> int:
    cli = argparse.ArgumentParser(
        description=__doc__,
        epilog='Example: eqs_process.py "Topping MA4" --dry-run; remove --dry-run to install the winner. '
        "Existing autoeq aliases follow the new result; manual iir.txt selections are preserved unless --activate is supplied.",
    )
    cli.add_argument("speakers", nargs="*", help="candidate speaker directory names; default: all")
    cli.add_argument("--input-dir", type=Path, default=ROOT / "build/eqs")
    cli.add_argument("--output-dir", type=Path, default=ROOT / "datas/eq")
    cli.add_argument("--dry-run", action="store_true")
    cli.add_argument(
        "--activate", action="store_true", help="also replace a manually selected iir.txt"
    )
    cli.add_argument("--min-improvement", type=float, default=0.0)
    args = cli.parse_args(argv)
    if not math.isfinite(args.min_improvement) or args.min_improvement < 0:
        cli.error("--min-improvement must be finite and nonnegative")
    if not args.input_dir.is_dir():
        cli.error(f"candidate directory does not exist: {args.input_dir}")
    if args.speakers:
        directories = [
            args.input_dir / sanitize_filename(name) for name in dict.fromkeys(args.speakers)
        ]
        missing = [str(path) for path in directories if not path.is_dir()]
        if missing:
            cli.error(f"candidate speaker directories not found: {', '.join(missing)}")
    else:
        directories = sorted(path for path in args.input_dir.iterdir() if path.is_dir())
    failed = 0
    for directory in directories:
        try:
            process(
                directory,
                args.output_dir,
                dry_run=args.dry_run,
                activate=args.activate,
                min_improvement=args.min_improvement,
            )
        except (OSError, ValueError) as error:
            failed += 1
            print(f"{directory.name}: {error}", file=sys.stderr)
    return int(failed > 0)


if __name__ == "__main__":
    sys.exit(main())
