#!/usr/bin/env python3
"""Compute isolated, resumable EQ candidates using the Rust generate-peq binary."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
import hashlib
import json
import os
import shlex
from pathlib import Path
import subprocess
import sys

from eqs_common import (
    ROOT,
    Measurement,
    catalogue,
    eq_digest,
    read_eq,
    sanitize_filename,
    write_manifest,
)


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        message = "must be positive"
        raise argparse.ArgumentTypeError(message)
    return number


def peq_counts(value: str) -> list[int]:
    try:
        return list(dict.fromkeys(positive(part) for part in value.split(",")))
    except ValueError as error:
        message = "use comma-separated positive filter counts"
        raise argparse.ArgumentTypeError(message) from error


def parser() -> argparse.ArgumentParser:
    cli = argparse.ArgumentParser(
        description=__doc__,
        epilog='Example: eqs_compute.py "Topping MA4" --fitness Score --peq-counts 7 --max-iter 200000 --starts 3. '
        "The default sweep searches Flat and Score with 3-7 filters and three seeds (30 candidates per speaker).",
    )
    cli.add_argument(
        "speakers", nargs="*", help="exact speaker names; default: all eligible TOML entries"
    )
    cli.add_argument("--metadata-dir", type=Path, default=ROOT / "datas/measurements")
    cli.add_argument("--output-dir", type=Path, default=ROOT / "build/eqs")
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    cli.add_argument(
        "--generate-peq",
        type=Path,
        default=Path(os.environ.get("SPINORAMA_GENERATE_PEQ", target / "release/generate-peq")),
    )
    cli.add_argument(
        "--algo", default="autoeq:cmaes", help="Rust optimizer (default: autoeq:cmaes)"
    )
    cli.add_argument(
        "--max-iter",
        "--max-evals",
        dest="max_iter",
        type=positive,
        default=100_000,
        help="evaluation budget per run (default: 100000)",
    )
    cli.add_argument(
        "--starts",
        type=positive,
        default=3,
        help="independent seeded runs per configuration (default: 3)",
    )
    cli.add_argument(
        "--seed", type=int, default=42, help="first seed; subsequent starts use consecutive seeds"
    )
    cli.add_argument(
        "--peq-counts",
        type=peq_counts,
        default=[3, 4, 5, 6, 7],
        help="filter counts (default: 3,4,5,6,7)",
    )
    cli.add_argument(
        "--fitness",
        action="append",
        choices=["Flat", "Score", "FlatPir", "Combine", "LeastSquare"],
        help="repeat to search multiple losses (default: Flat and Score)",
    )
    cli.add_argument("--peq-model", default="pk")
    cli.add_argument(
        "--jobs", "--processes", dest="jobs", type=positive, default=min(os.cpu_count() or 1, 4)
    )
    cli.add_argument(
        "--threads-per-job",
        type=positive,
        default=1,
        help="Rust Rayon threads per optimizer process",
    )
    cli.add_argument("--force", action="store_true", help="recompute completed candidates")
    cli.add_argument(
        "--dry-run", action="store_true", help="print workloads without invoking the optimizer"
    )
    return cli


@dataclass(frozen=True)
class Workload:
    measurement: Measurement
    directory: Path
    command: tuple[str, ...]
    signature: str


def generate_workloads(measurements: list[Measurement], args: argparse.Namespace) -> list[Workload]:
    workloads = []
    binary = args.generate_peq.resolve()
    binary_stamp = (binary.stat().st_size, binary.stat().st_mtime_ns) if binary.is_file() else None
    for measurement in measurements:
        sources = [
            measurement.metadata,
            *sorted(p for p in measurement.directory.rglob("*") if p.is_file()),
        ]
        stamps = [(str(path), path.stat().st_size, path.stat().st_mtime_ns) for path in sources]
        for fitness in dict.fromkeys(args.fitness or ["Flat", "Score"]):
            for peqs in args.peq_counts:
                for seed in range(args.seed, args.seed + args.starts):
                    label = (
                        f"{fitness}-{peqs}-{args.peq_model}-{args.algo}-n{args.max_iter}-seed{seed}"
                    )
                    directory = (
                        args.output_dir.resolve()
                        / sanitize_filename(measurement.name)
                        / sanitize_filename(label)
                    )
                    command = [
                        str(binary),
                        "--speaker-dir",
                        str(measurement.directory.resolve()),
                        "--speaker",
                        measurement.name,
                        "--origin",
                        measurement.origin,
                        "--mversion",
                        measurement.version,
                        "--format",
                        measurement.format,
                        "--output-dir",
                        str(directory),
                        "--fitness",
                        fitness,
                        "--optimisation",
                        "global",
                        "--algo",
                        args.algo,
                        "--max-iter",
                        str(args.max_iter),
                        "--max-peq",
                        str(peqs),
                        "--peq-model",
                        args.peq_model,
                        "--seed",
                        str(seed),
                        "--force",
                    ]
                    if measurement.symmetry:
                        command.extend(["--symmetry", measurement.symmetry])
                    signature = hashlib.sha256(
                        json.dumps([command, stamps, binary_stamp]).encode()
                    ).hexdigest()
                    workloads.append(Workload(measurement, directory, tuple(command), signature))
    return workloads


def compute_eq(work: Workload, *, force: bool, threads: int) -> tuple[str, str]:
    manifest_path = work.directory / "run.json"
    output = work.directory / "iir-autoeq.txt"
    if not force and manifest_path.is_file() and output.is_file():
        try:
            previous = json.loads(manifest_path.read_text())
            read_eq(output)
            if (
                previous.get("status") == "complete"
                and previous.get("signature") == work.signature
                and previous.get("eq_sha256") == eq_digest(output)
            ):
                return "skipped", str(work.directory)
        except (OSError, ValueError):
            pass
    work.directory.mkdir(parents=True, exist_ok=True)
    manifest = {
        "speaker": work.measurement.name,
        "origin": work.measurement.origin,
        "version": work.measurement.version,
        "command": list(work.command),
        "signature": work.signature,
        "status": "running",
    }
    write_manifest(manifest_path, manifest)
    log = work.directory / "optimizer.log"
    try:
        with log.open("w", encoding="utf-8") as stream:
            subprocess.run(  # noqa: S603 - arguments use a list and shell execution is disabled.
                work.command,
                cwd=ROOT,
                stdout=stream,
                stderr=subprocess.STDOUT,
                env={**os.environ, "RAYON_NUM_THREADS": str(threads)},
                check=True,
            )
        eq = read_eq(output)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        manifest.update(status="failed", error=str(error))
        write_manifest(manifest_path, manifest)
        return "failed", f"{work.directory}: {error}; see {log}"
    else:
        manifest.update(
            status="complete", score=eq.score, baseline=eq.baseline, eq_sha256=eq_digest(output)
        )
        write_manifest(manifest_path, manifest)
        return "complete", f"{work.directory}: {eq.baseline:.2f} -> {eq.score:.2f}"


def main(argv: list[str] | None = None) -> int:
    cli = parser()
    args = cli.parse_args(argv)
    if args.seed < 0 or args.seed + args.starts > 2**64:
        cli.error("seeds must fit an unsigned 64-bit integer")
    try:
        entries = catalogue(args.metadata_dir.resolve())
        names = list(dict.fromkeys(args.speakers)) if args.speakers else sorted(entries)
        unknown = [name for name in names if name not in entries]
        if unknown:
            cli.error(f"unknown or skipped speakers: {', '.join(unknown)}")
        selected = [entries[name] for name in names]
        missing = [str(item.directory) for item in selected if not item.directory.is_dir()]
        if missing:
            cli.error(f"missing measurement directories: {', '.join(missing)}")
        if not args.dry_run and not args.generate_peq.is_file():
            cli.error(
                "Rust binary missing; run cargo build --release -p spinorama-generate --bin generate-peq, or use --generate-peq"
            )
        workloads = generate_workloads(selected, args)
    except (OSError, ValueError, KeyError) as error:
        cli.error(str(error))
    print(
        f"{len(workloads)} candidates; {args.algo}; {args.max_iter} evaluations; {args.starts} starts; {args.jobs} jobs"
    )
    if args.dry_run:
        for work in workloads:
            print(shlex.join(work.command))
        return 0
    counts = {"complete": 0, "skipped": 0, "failed": 0}
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [
            pool.submit(compute_eq, work, force=args.force, threads=args.threads_per_job)
            for work in workloads
        ]
        for index, future in enumerate(as_completed(futures), 1):
            status, message = future.result()
            counts[status] += 1
            print(f"[{index}/{len(workloads)}] {status}: {message}", flush=True)
    print("; ".join(f"{count} {status}" for status, count in counts.items()))
    return int(counts["failed"] > 0)


if __name__ == "__main__":
    sys.exit(main())
