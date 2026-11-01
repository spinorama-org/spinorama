#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Identify which API key a leaked speaker-measurement JSON came from.

Usage (from the repository root, private data access required)::

    SPINORAMA_FINGERPRINT_SECRET=... PYTHONPATH=src:. \\
      .venv/bin/python scripts/fingerprint_identify.py \\
        --pristine /path/to/private/CEA2034.json \\
        --suspect /path/to/suspected.json \\
        --resource "Genelec 8341A/ASR/asr/CEA2034" \\
        --key-id KEY_ID_1 --key-id KEY_ID_2

The suspect may be a raw Plotly figure dict or the API wire shape
(a JSON array holding the figure text). Scores are residual/offset
correlations: the source key should lead by a clear margin. This tool
only reads files; it never modifies data.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from src.api.fingerprint import amplitude_from_env, rank_candidates, secret_from_env


def load_figure(path: str) -> dict:
    payload = json.loads(Path(path).read_text(encoding="utf8"))
    if isinstance(payload, list) and payload and isinstance(payload[0], str):
        payload = json.loads(payload[0])
    if not isinstance(payload, dict):
        message = f"{path}: not a figure dict"
        raise SystemExit(message)
    return payload


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pristine", required=True)
    parser.add_argument("--suspect", required=True)
    parser.add_argument("--resource", required=True, help="'Speaker/Origin/Version/Measurement'")
    parser.add_argument("--key-id", action="append", required=True, dest="key_ids")
    parser.add_argument("--amplitude-db", type=float, default=None)
    args = parser.parse_args()

    secret = secret_from_env()
    if not secret:
        message = "SPINORAMA_FINGERPRINT_SECRET is not set"
        raise SystemExit(message)
    amplitude = args.amplitude_db if args.amplitude_db is not None else amplitude_from_env()
    ranked = rank_candidates(
        load_figure(args.pristine),
        load_figure(args.suspect),
        args.key_ids,
        resource=args.resource,
        secret=secret,
        amplitude_db=amplitude,
    )
    for key_id, score in ranked:
        print(f"{score:12.4f}  {key_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
