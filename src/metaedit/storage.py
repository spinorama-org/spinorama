"""Read and save per-speaker TOML metadata using Rust validation."""

from collections.abc import MutableMapping
import copy
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import tomllib

import tomlkit

ROOT = Path(__file__).resolve().parents[2]


def sanitize_filename(name: str) -> str:
    return "".join("_" if char in '|:<>"\\/?*' else char for char in name)


def update_table(table: MutableMapping, values: dict) -> None:
    """Keep unchanged TOML items and their comments."""
    for key in list(table):
        if key not in values:
            del table[key]
    for key, value in values.items():
        if isinstance(value, dict) and isinstance(table.get(key), MutableMapping):
            update_table(table[key], value)
        elif table.get(key) != value:
            table[key] = value


class MetadataRepository:
    def __init__(self, directory: Path | None = None, validator: Path | None = None):
        self.directory = (directory or ROOT / "datas/measurements").resolve()
        executable = "generate-graph.exe" if os.name == "nt" else "generate-graph"
        self.validator = (
            validator
            or Path(
                os.environ.get("SPINORAMA_GENERATE_GRAPH", ROOT / "target/release" / executable)
            )
        ).resolve()
        self.reload()

    def reload(self) -> None:
        self.entries: dict[str, dict] = {}
        self.paths: dict[str, Path] = {}
        self.sources: dict[str, str] = {}
        if not self.directory.is_dir():
            message = f"Metadata directory not found: {self.directory}"
            raise FileNotFoundError(message)
        for path in sorted(self.directory.glob("*/meta.toml")):
            text = path.read_text(encoding="utf-8")
            data = tomllib.loads(text)
            name = data["name"]
            if name in self.entries:
                message = f"Duplicate speaker {name!r}: {path} and {self.paths[name]}"
                raise ValueError(message)
            self.entries[name], self.paths[name], self.sources[name] = data, path, text

    def speakers(self) -> list[str]:
        return sorted(self.entries)

    def brands(self) -> list[str]:
        return sorted({entry["brand"] for entry in self.entries.values()})

    def load(self, name: str) -> dict | None:
        entry = self.entries.get(name)
        return copy.deepcopy(entry) if entry is not None else None

    def render(self, data: dict, original_name: str | None = None) -> str:
        document = tomlkit.parse(self.sources.get(original_name, ""))
        update_table(document, data)
        return tomlkit.dumps(document)

    def validate(self, data: dict, text: str) -> None:
        if not self.validator.is_file():
            message = (
                "Rust validator missing; run make generate-build or set SPINORAMA_GENERATE_GRAPH"
            )
            raise FileNotFoundError(message)
        with tempfile.TemporaryDirectory(prefix="metaedit-") as temporary:
            root = Path(temporary)
            metadata = root / "measurements"
            speaker = metadata / sanitize_filename(data["name"])
            speaker.mkdir(parents=True)
            (speaker / "meta.toml").write_text(text, encoding="utf-8")
            shutil.copyfile(self.directory.parent / "origins.toml", root / "origins.toml")
            source = self.directory / sanitize_filename(data["name"])
            for version in data.get("measurements", {}):
                if Path(version).name != version or version in (".", ".."):
                    message = f"Invalid measurement directory: {version!r}"
                    raise ValueError(message)
                if (source / version).is_dir():
                    # --check-meta checks directory presence; --check-files parses data separately.
                    (speaker / version).mkdir()
            result = subprocess.run(  # noqa: S603 - local validator, no shell
                [str(self.validator), "--metadata-dir", str(metadata), "--check-meta"],
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode:
                details = (
                    (result.stderr + result.stdout)
                    .strip()
                    .replace(str(metadata), str(self.directory))
                )
                message = f"Rust metadata validation failed:\n{details}"
                raise ValueError(message)

    def save(self, data: dict, *, original_name: str | None = None) -> Path:
        name = data["name"]
        if not name or sanitize_filename(name) in (".", ".."):
            message = "A speaker name is required"
            raise ValueError(message)
        if original_name is not None and name != original_name:
            message = "Rename the measurement directory separately before changing the speaker name"
            raise ValueError(message)
        target = (
            self.paths.get(original_name)
            if original_name
            else self.directory / sanitize_filename(name) / "meta.toml"
        )
        if target is None:
            message = f"Speaker no longer exists: {original_name}"
            raise ValueError(message)

        def check_target() -> None:
            if original_name and target.read_text(encoding="utf-8") != self.sources[original_name]:
                message = f"{target} changed on disk; reload it before saving"
                raise ValueError(message)
            if original_name is None and target.exists():
                message = f"Speaker metadata already exists: {target}"
                raise ValueError(message)

        check_target()
        text = self.render(data, original_name)
        self.validate(data, text)
        check_target()
        target.parent.mkdir(parents=True, exist_ok=True)
        fd, temporary = tempfile.mkstemp(prefix=".meta-", suffix=".toml", dir=target.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as stream:
                stream.write(text)
            os.chmod(temporary, stat.S_IMODE(target.stat().st_mode) if target.exists() else 0o644)
            os.replace(temporary, target)
        finally:
            Path(temporary).unlink(missing_ok=True)
        self.reload()
        return target
