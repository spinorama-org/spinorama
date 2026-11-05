"""Headphone metadata dump: EQ loading, scores, JSON split."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))

import spinorama.constant_paths as cpaths
from generate_headphone_meta import (
    add_headphone_eq,
    add_headphone_scores,
    dump_headphone_metadata,
)


def _catalogue(tmp_path, monkeypatch):
    eq_dir = tmp_path / "headphone_eq" / "Test HP"
    eq_dir.mkdir(parents=True)
    (eq_dir / "iir-autoeq-score.txt").write_text(
        "Filter  1: ON PK Fc   100 Hz Gain -3.00 dB Q 1.00\n",
        encoding="utf-8",
    )
    (eq_dir / "autoeq_score.json").write_text(
        '{"pre_score": 50.0, "post_score": 70.0}', encoding="utf-8"
    )
    dist_json = tmp_path / "dist" / "json"
    monkeypatch.setattr(cpaths, "CPATH_DATAS_HEADPHONE_EQ", str(tmp_path / "headphone_eq"))
    monkeypatch.setattr(cpaths, "CPATH_DIST", str(tmp_path / "dist"))
    monkeypatch.setattr(cpaths, "CPATH_DIST_JSON", str(dist_json))
    monkeypatch.setattr(
        cpaths, "CPATH_DIST_HEADPHONE_METADATA_JSON", str(dist_json / "headphone_metadata.json")
    )
    monkeypatch.setattr(
        cpaths, "CPATH_DIST_HEADPHONE_EQDATA_JSON", str(dist_json / "headphone_eqdata.json")
    )
    return {"Test HP": {"brand": "Test", "model": "HP"}}


def test_headphone_meta_roundtrip(tmp_path, monkeypatch):
    catalogue = _catalogue(tmp_path, monkeypatch)
    add_headphone_eq(catalogue)
    add_headphone_scores(catalogue)
    dump_headphone_metadata(catalogue)

    info = catalogue["Test HP"]
    assert info["default_eq"] == "autoeq_score"
    eq = info["eqs"]["autoeq_score"]
    assert eq["display_name"] == "Harman Score EQ (IIR)"
    assert eq["type"] == "peq"
    assert len(eq["peq"]) == 1
    assert eq["peq"][0]["freq"] == 100.0
    assert info["score"] == 50.0
    assert info["score_eq"] == 70.0

    meta = json.loads((tmp_path / "dist/json/headphone_metadata.json").read_text())
    eqdata = json.loads((tmp_path / "dist/json/headphone_eqdata.json").read_text())
    assert meta["Test HP"]["kind"] == "headphone"
    assert "eqs" not in meta["Test HP"]
    assert eqdata["Test HP"]["default_eq"] == "autoeq_score"
    assert "autoeq_score" in eqdata["Test HP"]["eqs"]
