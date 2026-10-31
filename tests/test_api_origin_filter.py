"""API catalogs and direct downloads exclude ErinsAudioCorner measurements."""

from copy import deepcopy
import json

import pytest
from fastapi.testclient import TestClient

from src.api import state
from src.api.keys import initialize, issue
from src.api.main import app
from src.api.routers import headphone, speaker


CATALOG = {
    "Mixed": {
        "brand": "Mixed Brand",
        "default_measurement": "review-v2",
        "default_eq": "autoeq",
        "nearest": [[1.0, "Excluded"]],
        "measurements": {
            "review-v2": {"origin": "ErinsAudioCorner", "pref_rating": {"pref_score": 8}},
            "review-v2_eq": {"origin": "ErinsAudioCorner"},
            "asr": {"origin": "ASR"},
        },
    },
    "Excluded": {
        "brand": "Excluded Brand",
        "default_measurement": "review",
        "measurements": {"review": {"origin": "ErinsAudioCorner"}},
    },
    "Allowed": {
        "brand": "Allowed Brand",
        "default_measurement": "asr",
        "measurements": {"asr": {"origin": "ASR"}},
    },
}


def test_filter_preserves_input_and_selects_an_available_default():
    original = deepcopy(CATALOG)
    result = state.filter_api_measurements(CATALOG)
    assert CATALOG == original
    assert set(result) == {"Mixed", "Allowed"}
    assert result["Allowed"] == original["Allowed"]
    assert result["Mixed"]["measurements"] == {"asr": {"origin": "ASR"}}
    assert result["Mixed"]["default_measurement"] == "asr"
    assert "default_eq" not in result["Mixed"]
    assert "nearest" not in result["Mixed"]


@pytest.mark.parametrize("eager", [False, True], ids=["lazy-load", "startup-load"])
def test_filtered_catalog_blocks_listings_and_direct_downloads(tmp_path, monkeypatch, eager):
    metadata_file = tmp_path / "metadata.json"
    metadata_file.write_text(json.dumps(CATALOG))
    monkeypatch.setattr(state, "METADATA", str(metadata_file))
    monkeypatch.setattr(state, "HEADPHONE_METADATA", str(metadata_file))
    monkeypatch.setattr(state, "_metadata_cache", None)
    monkeypatch.setattr(state, "_headphone_metadata_cache", None)
    monkeypatch.setattr(speaker, "SPINFILES", str(tmp_path / "speakers"))
    monkeypatch.setattr(headphone, "HEADPHONE_FILES", str(tmp_path / "headphones"))

    # Even when excluded files exist, the API must not serve them.
    for name, record in CATALOG.items():
        for version, measurement in record["measurements"].items():
            folder = tmp_path / "speakers" / name / measurement["origin"] / version
            folder.mkdir(parents=True)
            (folder / "On Axis.json").write_text('{"data": []}')
            folder = tmp_path / "headphones" / name / version
            folder.mkdir(parents=True)
            (folder / "frequency_response.csv").write_text("frequency,raw\n100,0\n")

    monkeypatch.setenv("SPINORAMA_API_KEY_DB", str(tmp_path / "keys.sqlite3"))
    initialize()
    _, token = issue("test@example.org", "Test", "Tests")
    client = TestClient(app, headers={"Authorization": "Bearer " + token})
    if eager:
        client.__enter__()
    try:
        assert client.get("/v1/speakers").json() == ["Allowed", "Mixed"]
        assert client.get("/v1/brands").json() == ["Allowed Brand", "Mixed Brand"]
        assert client.get("/v1/headphones").json() == ["Allowed", "Mixed"]
        assert client.get("/v1/headphone/brands").json() == ["Allowed Brand", "Mixed Brand"]
        for kind in ("speaker", "headphone"):
            assert client.get(f"/v1/{kind}/Mixed/versions").json() == ["asr"]
            result = client.get(f"/v1/{kind}/Mixed/metadata").json()
            assert result["default_measurement"] == "asr"
            assert "ErinsAudioCorner" not in json.dumps(result)
            assert "error" in client.get(f"/v1/{kind}/Excluded/metadata").json()

        for name, version in [("Mixed", "review-v2"), ("Mixed", "review-v2_eq"), ("Excluded", "review")]:
            base = f"/v1/speaker/{name}/version/{version}/measurements"
            assert "error" in client.get(base).json()
            assert "error" in client.get(base + "/On Axis").json()
            assert "error" in client.get(f"/v1/headphone/{name}/frequency_response", params={"version": version}).json()

        base = "/v1/speaker/Mixed/version/asr/measurements"
        assert client.get(base).json() == ["On Axis"]
        assert client.get(base + "/On Axis").json() == ['{"data": []}']
        assert client.get("/v1/headphone/Mixed/frequency_response").text == "frequency,raw\n100,0\n"
    finally:
        if eager:
            client.__exit__(None, None, None)
        else:
            client.close()
