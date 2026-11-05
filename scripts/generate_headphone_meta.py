#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Headphone-only metadata dump (speaker metadata is Rust generate-meta).

Writes dist/json/headphone_metadata.json and headphone_eqdata.json
from datas/headphones.py plus the EQ/score files under
datas/headphone_eq. Kept in Python because the headphone catalogue
still lives in Python modules; it needs no measurement cache, so it
runs in well under a second.
"""

import json
import logging
import os
import sys
from pathlib import Path

import spinorama.constant_paths as cpaths
from datas import EQ, Peq
from generate_common import get_custom_logger
from spinorama.filters.peq import peq_preamp_gain as filter_peq_preamp_gain
from spinorama.loaders.rew_eq import parse_eq_iir_rews as load_parse_eq_iir_rews

logger = logging.getLogger("spinorama")


def _write_text_if_different(filename: str, content: str) -> bool:
    """Write a text artifact only when its content changed."""
    path = Path(filename)
    if path.is_file():
        try:
            if path.read_text(encoding="utf-8") == content:
                return False
        except OSError:
            pass
    path.write_text(content, encoding="utf-8")
    return True


def add_headphone_eq(headphone_meta):
    """Load EQ data for headphones from APO format files."""
    eq_base = cpaths.CPATH_DATAS_HEADPHONE_EQ
    for hp_name, hp_info in headphone_meta.items():
        if hp_info.get("skip", False):
            continue
        if "eqs" not in hp_info or not isinstance(hp_info.get("eqs"), dict):
            hp_info["eqs"] = {}

        for eq_key, filename, display in (
            ("autoeq_score", "iir-autoeq-score", "Harman Score EQ (IIR)"),
            ("autoeq_flat", "iir-autoeq-flat", "Flat Target EQ (IIR)"),
        ):
            eq_filename = "{}/{}/{}.txt".format(eq_base, hp_name, filename)
            iir = load_parse_eq_iir_rews(eq_filename, 48000)
            if iir is not None and len(iir) > 0:
                if "default_eq" not in hp_info:
                    hp_info["default_eq"] = eq_key
                peq_list: list[Peq] = []
                for iir_weight, iir_filter in iir:
                    if iir_weight != 0.0:
                        peq_list.append(
                            Peq(
                                type=iir_filter.biquad_type,
                                freq=iir_filter.freq,
                                srate=iir_filter.srate,
                                Q=iir_filter.q,
                                dbGain=iir_filter.db_gain,
                            )
                        )

                current_eq: EQ = EQ(
                    display_name=display,
                    filename=eq_filename,
                    preamp_gain=round(filter_peq_preamp_gain(iir), 1),
                    type="peq",
                    peq=peq_list,
                )
                hp_info["eqs"][eq_key] = current_eq
                logger.debug("adding headphone eq: %s for %s", eq_key, hp_name)


def add_headphone_scores(headphone_meta):
    """Load pre/post EQ scores from autoeq QA output."""
    eq_base = cpaths.CPATH_DATAS_HEADPHONE_EQ
    for hp_name, hp_info in headphone_meta.items():
        if hp_info.get("skip", False):
            continue
        score_file = "{}/{}/autoeq_score.json".format(eq_base, hp_name)
        if os.path.exists(score_file):
            try:
                with open(score_file, "r", encoding="utf-8") as f:
                    score_data = json.load(f)
                if "pre_score" in score_data:
                    hp_info["score"] = score_data["pre_score"]
                if "post_score" in score_data:
                    hp_info["score_eq"] = score_data["post_score"]
            except (json.JSONDecodeError, OSError):
                pass


def dump_headphone_metadata(headphone_meta):
    """Write headphone metadata to JSON files."""
    metadir = cpaths.CPATH_DIST
    os.makedirs(metadir, mode=0o755, exist_ok=True)
    os.makedirs(cpaths.CPATH_DIST_JSON, mode=0o755, exist_ok=True)

    hp_meta_full = {}
    hp_eq_full = {}
    for k, v in headphone_meta.items():
        if v.get("skip", False):
            continue
        hp_meta_full[k] = {k2: v2 for k2, v2 in v.items() if k2 != "eqs"}
        hp_meta_full[k]["kind"] = "headphone"
        hp_eq_full[k] = {
            k2: v2 for k2, v2 in v.items() if k2 in ("eqs", "brand", "model", "default_eq")
        }

    hp_metafile = cpaths.CPATH_DIST_HEADPHONE_METADATA_JSON
    hp_eqfile = cpaths.CPATH_DIST_HEADPHONE_EQDATA_JSON

    for filename, data in ((hp_metafile, hp_meta_full), (hp_eqfile, hp_eq_full)):
        js = json.dumps(data)
        if _write_text_if_different(filename, js):
            logger.info("Generated %s (%d entries)", filename, len(data))


def main() -> int:
    from datas.headphones import headphones_info

    add_headphone_eq(headphones_info)
    add_headphone_scores(headphones_info)
    dump_headphone_metadata(headphones_info)
    logger.info("Wrote headphone metadata for %d headphones", len(headphones_info))
    return 0


if __name__ == "__main__":
    logger = get_custom_logger(level=logging.INFO, duplicate=True)
    sys.exit(main())
