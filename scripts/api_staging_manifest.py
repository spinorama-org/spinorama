#!/usr/bin/env python3
"""Hash every required API staging input before any transfer starts."""

import argparse
import hashlib
from pathlib import Path


def staging_files(data_dir: str, run_dir: str):
    files = {}

    def add(source: Path, destination: Path):
        if source.is_symlink() or not source.is_file():
            message = f"Required regular staging input missing: {source}"
            raise ValueError(message)
        if any(char in str(destination) for char in "\\\n\r"):
            message = "Unsupported manifest filename"
            raise ValueError(message)
        files[destination] = source

    data, run = Path(data_dir), Path(run_dir)
    add(Path("scripts/spinorama-api-start.sh"), run / "spinorama-api-start.sh")
    add(Path("datas/origins.toml"), run / "datas/origins.toml")
    for name in (
        "metadata.json",
        "headphone_metadata.json",
        "eqdata.json",
        "headphone_eqdata.json",
    ):
        add(Path("dist/json") / name, data / "assets" / name)
    for pattern in ("metadata*.json*", "headphone*.json*", "eqdata.json*"):
        for source in Path("dist/json").glob(pattern):
            add(source, data / "assets" / source.name)
    for source_dir, dest_dir, optional in (
        (Path("datas/headphones"), data / "assets/headphones", False),
        (Path("dist-private/speakers"), data / "speakers", False),
        (Path("dist-private/headphones"), data / "headphones-graphs", True),
    ):
        if optional and not source_dir.exists():
            continue
        if not source_dir.is_dir():
            message = f"Required staging tree missing: {source_dir}"
            raise ValueError(message)
        for source in source_dir.rglob("*"):
            if source.is_file() or source.is_symlink():
                add(source, dest_dir / source.relative_to(source_dir))
    for source, name in (
        ("supervisor/conf.d/spinorama-app.conf", "spinorama-app.conf"),
        ("nginx/sites-available/spinorama-api", "spinorama-api.nginx"),
        ("nginx/sites-available/spinorama-prod", "spinorama-prod.nginx"),
        ("nginx/conf.d/bot-protection.conf", "bot-protection.conf"),
        ("nginx/snippets/bot-protection-server.conf", "bot-protection-server.conf"),
        ("nginx/snippets/spinorama-banned.conf", "spinorama-banned.conf"),
        ("nginx/snippets/deny-repository-metadata.conf", "deny-repository-metadata.conf"),
    ):
        add(Path("conf/etc") / source, run / "etc" / name)
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("data_dir")
    parser.add_argument("run_dir")
    args = parser.parse_args()
    for destination, source in sorted(staging_files(args.data_dir, args.run_dir).items()):
        with source.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        print(f"{digest}  {destination}")


if __name__ == "__main__":
    main()
