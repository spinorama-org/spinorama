# -*- coding: utf-8 -*-
"""Speaker metadata, loaded from per-speaker TOML files.

The catalogue lives in `datas/measurements/<Speaker>/meta.toml` (one
file per speaker, migrated from the former `speaker_*.py` dict
literals by `scripts/convert_metadata_to_toml.py`) plus
`datas/origins.toml`. This module keeps the historical
`speakers_info` / `origins_info` dict interface for all consumers.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

_ROOT = Path(__file__).resolve().parent
_MEASUREMENTS = _ROOT / "measurements"


def _load_speakers() -> dict:
    speakers: dict = {}
    for meta_path in sorted(_MEASUREMENTS.glob("*/meta.toml")):
        with meta_path.open("rb") as handle:
            speaker = tomllib.load(handle)
        name = speaker.pop("name")
        if name in speakers:
            raise ValueError(f"duplicate speaker metadata: {name} ({meta_path})")
        speakers[name] = speaker
    return speakers


def _load_origins() -> dict:
    with (_ROOT / "origins.toml").open("rb") as handle:
        return tomllib.load(handle)["origins"]


speakers_info = _load_speakers()
origins_info = _load_origins()
