"""The editor must preserve fields from the per-speaker TOML schema."""

import copy
import tomllib

import pytest

from metaedit.models import SpeakerMetadata, export_speaker_metadata, prune_empty
from metaedit.storage import MetadataRepository


def test_export_preserves_current_toml_fields():
    data = {
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
                "notes": "Keep this note",
                "parameters": {"mean_min": 300, "mean_max": 3000},
            }
        },
    }
    exported = export_speaker_metadata(SpeakerMetadata(**data))
    assert exported["name"] == data["name"]
    assert exported["price"] == ""
    assert exported["skip"] is True
    assert exported["measurements"]["vendor"]["parameters"] == {"mean_min": 300, "mean_max": 3000}


def test_save_and_reload_with_native_validation(repository, metadata):
    path = repository.save(metadata)
    assert tomllib.loads(path.read_text()) == metadata
    assert repository.speakers() == [metadata["name"]]
    assert repository.brands() == [metadata["brand"]]
    loaded = repository.load(metadata["name"])
    loaded["brand"] = "Different"
    assert repository.load(metadata["name"])["brand"] == "Example"


def test_save_checks_existing_measurement_directories(repository, metadata):
    for version in metadata["measurements"]:
        (repository.directory / metadata["name"] / version).mkdir(parents=True)
    metadata["skip"] = False
    path = repository.save(metadata)
    assert tomllib.loads(path.read_text()) == metadata


def test_edit_during_validation_is_not_overwritten(repository, metadata, monkeypatch):
    path = repository.save(metadata)
    external = path.read_text() + "\n# Concurrent change\n"
    validate = repository.validate

    def validate_then_edit(data, text):
        validate(data, text)
        path.write_text(external)

    monkeypatch.setattr(repository, "validate", validate_then_edit)
    with pytest.raises(ValueError, match="changed on disk"):
        repository.save(metadata, original_name=metadata["name"])
    assert path.read_text() == external


def test_edit_preserves_comments_and_other_measurements(repository, metadata):
    path = repository.save(metadata)
    path.write_text("# Preserve this comment\n" + path.read_text())
    repository.reload()
    metadata["measurements"]["vendor"]["notes"] = "Updated note"
    repository.save(metadata, original_name=metadata["name"])
    assert path.read_text().startswith("# Preserve this comment\n")
    assert repository.load(metadata["name"]) == metadata


@pytest.mark.parametrize("change", ["format", "default", "missing_files", "rename"])
def test_invalid_save_keeps_original_file(repository, metadata, change):
    path = repository.save(metadata)
    original = path.read_text()
    edited = copy.deepcopy(metadata)
    if change == "format":
        edited["measurements"]["vendor"]["format"] = "invalid"
    elif change == "default":
        edited["default_measurement"] = "missing"
    elif change == "missing_files":
        edited["skip"] = False
    else:
        edited["name"] = "Example Renamed"
    with pytest.raises(ValueError):
        repository.save(edited, original_name=metadata["name"])
    assert path.read_text() == original


def test_external_edits_are_not_overwritten(repository, metadata):
    path = repository.save(metadata)
    external = path.read_text() + "\n# Changed by someone else\n"
    path.write_text(external)
    with pytest.raises(ValueError, match="changed on disk"):
        repository.save(metadata, original_name=metadata["name"])
    assert path.read_text() == external


def test_new_save_does_not_overwrite_existing_metadata(repository, metadata):
    repository.save(metadata)
    with pytest.raises(ValueError, match="already exists"):
        repository.save(metadata)


def test_all_catalogue_models_preserve_metadata():
    repository = MetadataRepository()
    for name in repository.speakers():
        raw = repository.load(name)
        converted = SpeakerMetadata.convert_legacy_reviews(raw)
        exported = export_speaker_metadata(SpeakerMetadata(**converted))
        expected = prune_empty(converted)
        expected["price"] = converted.get("price", "")
        assert exported == expected, name
