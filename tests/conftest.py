"""Temporary catalogues shared by metadata editor tests."""

import shutil

import pytest

from metaedit.storage import MetadataRepository, ROOT


@pytest.fixture
def repository(tmp_path):
    directory = tmp_path / "measurements"
    directory.mkdir()
    shutil.copyfile(ROOT / "datas/origins.toml", tmp_path / "origins.toml")
    repository = MetadataRepository(directory)
    if not repository.validator.is_file():
        pytest.skip("Build the native validator with make generate-build")
    return repository


@pytest.fixture
def metadata():
    return {
        "name": "Example Monitor",
        "brand": "Example",
        "model": "Monitor",
        "type": "active",
        "shape": "bookshelves",
        "price": "",
        "skip": True,
        "default_measurement": "vendor",
        "measurements": {
            "vendor": {
                "origin": "ASR",
                "format": "klippel",
                "quality": "high",
                "notes": "Keep this note\nAnd this line",
                "review_published": "2024",
                "parameters": {"mean_min": 300, "mean_max": 3000},
                "extras": {"is_equed": False},
                "data_acquisition": {"air_absorbtion": False},
                "specifications": {"SPL": {"peak": 110.0, "continuous": 98.0}},
            },
            "other": {"origin": "ASR", "format": "klippel"},
        },
    }
