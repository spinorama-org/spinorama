"""Per-API-key data fingerprinting: imperceptible, deterministic, identifiable."""

import base64
import copy
import json
import math
import struct

from fastapi.testclient import TestClient

import src.api.routers.speaker as speaker_router
from src.api import keys
from src.api.main import app, load_metadata

from src.api.fingerprint import (
    apply_to_csv_text,
    apply_to_figure,
    rank_candidates,
)

SECRET = "test-secret-do-not-ship"  # noqa: S105 -- test fixture value
RESOURCE = "Example Speaker/asr/CEA2034"
AMP = 0.03


def _bdata(values):
    return {"dtype": "f8", "bdata": base64.b64encode(struct.pack(f"<{len(values)}d", *values)).decode()}


def _decode(arr):
    raw = base64.b64decode(arr["bdata"])
    return list(struct.unpack(f"<{len(raw) // 8}d", raw))


def make_figure():
    freqs = [20.0 * (2.0 ** (i / 24.0)) for i in range(241)]  # 20 Hz..~20 kHz, 1/24 oct
    on = [85.0 + 2.0 * math.sin(math.log2(f / 100.0)) for f in freqs]
    lw = [84.0 + 1.5 * math.sin(math.log2(f / 100.0) + 0.4) for f in freqs]
    return {
        "data": [
            {"type": "scatter", "name": "On Axis", "x": _bdata(freqs), "y": _bdata(on)},
            {"type": "scatter", "name": "Listening Window", "x": _bdata(freqs), "y": _bdata(lw)},
            # A non-scatter trace must be left untouched.
            {"type": "heatmap", "name": "contour", "z": [[1.0, 2.0], [3.0, 4.0]]},
        ],
        "layout": {"title": {"text": "Example"}},
    }


def test_deterministic_and_bounded():
    fig = make_figure()
    out1 = apply_to_figure(copy.deepcopy(fig), key_id="key-A", resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    out2 = apply_to_figure(copy.deepcopy(fig), key_id="key-A", resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    assert out1 == out2
    original_y = _decode(fig["data"][0]["y"])
    new_y = _decode(out1["data"][0]["y"])
    deltas = [abs(n - o) for n, o in zip(new_y, original_y, strict=True)]
    assert max(deltas) <= AMP + 1e-12
    assert max(deltas) > 0.005  # a fingerprint was actually applied
    assert out1["data"][2] == fig["data"][2]  # heatmap untouched


def test_different_keys_differ():
    fig = make_figure()
    a = apply_to_figure(copy.deepcopy(fig), key_id="key-A", resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    b = apply_to_figure(copy.deepcopy(fig), key_id="key-B", resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    assert _decode(a["data"][0]["y"]) != _decode(b["data"][0]["y"])


def test_identifies_leaker_after_rounding():
    fig = make_figure()
    leaked = apply_to_figure(copy.deepcopy(fig), key_id="key-B", resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    # Simulate republication damage: round every y to 0.1 dB, plain lists.
    for trace in leaked["data"][:2]:
        trace["y"] = [round(v, 1) for v in _decode(trace["y"])]
        trace["x"] = _decode(trace["x"])
    ranked = rank_candidates(fig, leaked, ["key-A", "key-B", "key-C"], resource=RESOURCE, secret=SECRET, amplitude_db=AMP)
    assert ranked[0][0] == "key-B"
    assert ranked[0][1] > ranked[1][1]


def test_speaker_route_serves_fingerprinted_json(monkeypatch, tmp_path):
    fig = make_figure()
    version_dir = tmp_path / "Example Speaker" / "TestOrigin" / "v1"
    version_dir.mkdir(parents=True)
    (version_dir / "CEA2034.json").write_text(json.dumps(fig), encoding="utf8")
    monkeypatch.setattr(speaker_router, "SPINFILES", str(tmp_path))
    monkeypatch.setenv("SPINORAMA_API_KEY_DB", str(tmp_path / "keys.sqlite3"))
    monkeypatch.setenv("SPINORAMA_FINGERPRINT_SECRET", SECRET)
    keys.initialize()
    key_id, token = keys.issue("fp@example.org", "FP", "Tests")
    metadata = {
        "Example Speaker": {
            "brand": "Example",
            "measurements": {"v1": {"origin": "TestOrigin"}},
        }
    }
    app.dependency_overrides[load_metadata] = lambda: metadata
    try:
        client = TestClient(app, headers={"Authorization": "Bearer " + token})
        response = client.get("/v1/speaker/Example Speaker/version/v1/measurements/CEA2034")
        assert response.status_code == 200
        body = response.json()
        served = json.loads(body[0]) if isinstance(body, list) else body
        # Served data differs from pristine, within the amplitude cap, and
        # identification against the pristine file points at this key.
        pristine_y = _decode(fig["data"][0]["y"])
        served_y = _decode(served["data"][0]["y"])
        assert max(abs(s - p) for s, p in zip(served_y, pristine_y, strict=True)) <= AMP
        ranked = rank_candidates(
            fig,
            served,
            [key_id, "some-other-key"],
            resource="Example Speaker/TestOrigin/v1/CEA2034",
            secret=SECRET,
            amplitude_db=AMP,
        )
        assert ranked[0][0] == key_id
    finally:
        app.dependency_overrides.clear()


def test_csv_fingerprint_roundtrip_shape():
    csv_text = (
        '"Frequency Response Reference 425 Hz @ 94 dBSPL",,,\r\n'
        '"Left (94 dB)",,Right,\r\n'
        "X,Y,X,Y\r\n"
        "Hz,dBSPL,Hz,dBSPL\r\n"
        + "".join(f"{f},{90.0 + i * 0.01},{f},{91.0 + i * 0.01}\r\n" for i, f in enumerate([20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0]))
    )
    out = apply_to_csv_text(csv_text, key_id="key-A", resource="HP/asr", secret=SECRET, amplitude_db=AMP)
    lines = out.strip().splitlines()
    assert lines[:4] == csv_text.strip().splitlines()[:4]
    assert len(lines) == 12
    first_data = lines[4].split(",")
    assert abs(float(first_data[1]) - 90.0) <= AMP
    # untouched when no secret
    assert apply_to_csv_text(csv_text, key_id="key-A", resource="HP/asr", secret="", amplitude_db=AMP) == csv_text
