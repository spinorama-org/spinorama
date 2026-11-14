"""Shared catalogue and Equalizer APO handling for the Rust EQ runners."""

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
NUMBER = r"[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?"
SCORE = re.compile(rf"^Preference Score\s+({NUMBER})\s+with EQ\s+({NUMBER})", re.MULTILINE)
PREAMP = re.compile(rf"^Preamp:\s*({NUMBER})\s+dB\s*$", re.MULTILINE)
FILTER = re.compile(
    rf"^Filter\s+\d+:\s+ON\s+(?:PK|LS|HS)\s+Fc\s+({NUMBER})\s+Hz\s+Gain\s+({NUMBER})\s+dB\s+Q\s+({NUMBER})\s*$",
    re.MULTILINE,
)


def sanitize_filename(name: str) -> str:
    """Match the Rust generator's speaker directory names."""
    return re.sub(r'[|:<>"\\/?*]', "_", name)


@dataclass(frozen=True)
class Measurement:
    name: str
    directory: Path
    metadata: Path
    origin: str
    version: str
    format: str
    symmetry: str | None


def catalogue(metadata_dir: Path) -> dict[str, Measurement]:
    """Read eligible default measurements from the current TOML catalogue."""
    result = {}
    for path in sorted(metadata_dir.glob("*/meta.toml")):
        with path.open("rb") as stream:
            speaker = tomllib.load(stream)
        version = speaker.get("default_measurement")
        measurement = speaker.get("measurements", {}).get(version, {})
        if not version or speaker.get("skip") or measurement.get("skip"):
            continue
        if Path(version).name != version or version in (".", ".."):
            message = f"{path}: invalid measurement version {version!r}"
            raise ValueError(message)
        name = speaker.get("name", path.parent.name)
        result[name] = Measurement(
            name,
            path.parent / version,
            path,
            measurement["origin"],
            version,
            measurement["format"],
            measurement.get("symmetry"),
        )
    if not result:
        message = f"No eligible speaker metadata in {metadata_dir}"
        raise ValueError(message)
    return result


@dataclass(frozen=True)
class Equalization:
    path: Path
    text: str
    baseline: float
    score: float
    filters: int


def read_eq(path: Path) -> Equalization:
    """Read the Rust/legacy score header and validate the APO filter payload."""
    text = path.read_text(encoding="utf-8")
    score = SCORE.search(text)
    preamp = PREAMP.search(text)
    filters = list(FILTER.finditer(text))
    active_lines = [line for line in text.splitlines() if re.match(r"^Filter\s+\d+:\s+ON\b", line)]
    if not score or not preamp or not filters or len(filters) != len(active_lines):
        message = f"{path}: incomplete score header, preamp, or filter payload"
        raise ValueError(message)
    baseline, corrected = map(float, score.groups())
    numbers = [baseline, corrected, float(preamp[1])]
    for filt in filters:
        freq, gain, quality = map(float, filt.groups())
        if freq <= 0 or quality <= 0:
            message = f"{path}: filter frequency and Q must be positive"
            raise ValueError(message)
        numbers.extend((freq, gain, quality))
    if not all(math.isfinite(value) for value in numbers):
        message = f"{path}: nonfinite score or filter parameter"
        raise ValueError(message)
    return Equalization(path, text, baseline, corrected, len(filters))


def atomic_write(path: Path, text: str) -> None:
    """Replace one output only after its complete contents are written."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(fd, stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(text)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def write_manifest(path: Path, value: dict) -> None:
    atomic_write(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


def eq_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()
