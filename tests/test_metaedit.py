"""GPUI editor behavior and persistent-session tests."""

from typing import List

import copy
import json
import os
from queue import Queue
import subprocess
import sys
from threading import Thread
import tomllib

import pytest
from gpui_toolkit import Event, SessionContext
from gpui_toolkit.effects import EffectResult, EffectStatus

from metaedit.app import MetadataApp
from metaedit.editor import EditorState


@pytest.fixture
def context(monkeypatch):
    messages = []
    context = SessionContext(capabilities=("events", "patches", "effects"))
    monkeypatch.setattr(context, "send", messages.append)
    monkeypatch.setattr(context, "_write_locked", messages.append)
    return context, messages


def send(app, context, action, node_id="button", value=None):
    event = Event(
        f"event-{node_id}",
        1,
        node_id,
        "change" if action == "edit" else "click",
        action,
        {"value": value},
    )
    app.on_action(event, context)


def find_node(spec, id):
    if isinstance(spec, dict):
        if spec.get("id") == id:
            return spec
        for value in spec.values():
            found = find_node(value, id)
            if found:
                return found
    elif isinstance(spec, list):
        for value in spec:
            found = find_node(value, id)
            if found:
                return found
    return None


@pytest.mark.parametrize("date", ["2024", "20240000", "20240131", "2024xxxx"])
def test_gpui_edit_review_and_save(repository, metadata, context, date):
    metadata["measurements"]["vendor"]["review_published"] = date
    path = repository.save(metadata)
    app = MetadataApp(repository, speaker=metadata["name"])
    spec = app.to_spec()
    assert spec["miniapp"]["with_theme"]
    prefix = app.editor.active_id
    assert (
        find_node(spec, f"{prefix}-notes")["value"] == metadata["measurements"]["vendor"]["notes"]
    )
    ctx, messages = context
    send(app, ctx, "next")
    assert app.stage == 2
    assert tomllib.loads(app.editor.preview()) == metadata
    send(app, ctx, "back")
    send(app, ctx, "edit", f"{prefix}-notes", "Edited in GPUI\nSecond line")
    send(app, ctx, "next")
    send(app, ctx, "preview-diff")
    assert "Edited in GPUI\\nSecond line" in app.editor.diff()
    send(app, ctx, "copy")
    effect = messages[-1]
    assert effect["effect"] == "clipboard_write"
    assert (
        tomllib.loads(effect["arguments"]["text"])["measurements"]["vendor"]["notes"]
        == "Edited in GPUI\nSecond line"
    )
    send(app, ctx, "save")
    expected = copy.deepcopy(metadata)
    expected["measurements"]["vendor"]["notes"] = "Edited in GPUI\nSecond line"
    assert tomllib.loads(path.read_text()) == expected
    assert app.message_variant == "success"


def test_measurement_rename_updates_default_and_other_versions(repository, metadata):
    repository.save(metadata)
    editor = EditorState(repository)
    editor.load(metadata["name"])
    editor.active.key = "renamed"
    exported = editor.export()
    assert exported["default_measurement"] == "renamed"
    assert exported["measurements"]["renamed"] == metadata["measurements"]["vendor"]
    assert exported["measurements"]["other"] == metadata["measurements"]["other"]
    editor.active.key = "other"
    with pytest.raises(ValueError, match="unique"):
        editor.export()


def test_review_keys_and_multiline_notes_survive_edits(repository, metadata):
    metadata["measurements"]["vendor"]["reviews"] = {"asr": "https://example.com/review"}
    repository.save(metadata)
    editor = EditorState(repository)
    editor.load(metadata["name"])
    review = editor.active.reviews[0]
    review.key = "updated"
    assert editor.export()["measurements"]["vendor"]["reviews"] == {"updated": review.url}
    editor.add_review(editor.active, "updated", "https://example.com/duplicate")
    with pytest.raises(ValueError, match="unique"):
        editor.export()


def test_add_remove_and_validation_keep_draft(repository, metadata, context):
    path = repository.save(metadata)
    before = path.read_text()
    app = MetadataApp(repository, speaker=metadata["name"])
    app.to_spec()
    ctx, _ = context
    send(app, ctx, "add-measurement")
    added = app.editor.active_id
    send(app, ctx, "edit", f"{added}-key", "new")
    send(app, ctx, "edit", f"{added}-origin", "ASR")
    send(app, ctx, "edit", f"{added}-format", "klippel")
    assert app.editor.active.data["quality"] == "high"
    send(app, ctx, "remove-measurement")
    assert len(app.editor.measurements) == 2
    prefix = app.editor.active_id
    send(app, ctx, "edit", f"{prefix}-parameters-mean_min", "bad")
    send(app, ctx, "next")
    assert app.stage == 1
    assert app.message_variant == "error"
    assert path.read_text() == before


def test_gll_defaults_and_active_type_keep_values(repository, metadata, context):
    repository.save(metadata)
    app = MetadataApp(repository, speaker=metadata["name"])
    app.to_spec()
    ctx, _ = context
    prefix = app.editor.active_id
    send(app, ctx, "edit", f"{prefix}-data_acquisition-via", "GLL")
    assert app.editor.active.data["data_acquisition"]["distance"] == 10
    assert app.editor.active.data["data_acquisition"]["max_valid_freq"] == 20000
    send(app, ctx, "edit", "speaker-type", "passive")
    assert not find_node(app.to_spec(), f"{prefix}-specifications-sensitivity")["disabled"]
    send(app, ctx, "edit", f"{prefix}-specifications-sensitivity", 85.5)
    send(app, ctx, "edit", "speaker-type", "active")
    assert find_node(app.to_spec(), f"{prefix}-specifications-sensitivity")["disabled"]
    assert app.editor.export()["measurements"]["vendor"]["specifications"]["sensitivity"] == 85.5


def test_search_and_new_speaker_flow(repository, metadata, context):
    repository.save(metadata)
    app = MetadataApp(repository)
    app.to_spec()
    ctx, messages = context
    send(app, ctx, "edit", "speaker-search", "monitor")
    assert messages[-1]["ops"][0]["value"][-1]["value"] == metadata["name"]
    send(app, ctx, "select-speaker", "speaker-selection", metadata["name"])
    send(app, ctx, "next")
    assert app.editor.name == metadata["name"]
    send(app, ctx, "start-over")
    send(app, ctx, "mode", value=1)
    send(app, ctx, "edit", "new-brand", "New")
    send(app, ctx, "edit", "new-model", "Monitor")
    send(app, ctx, "next")
    assert app.editor.name == "New Monitor"
    prefix = app.editor.active_id
    for id, value in [("key", "vendor"), ("origin", "ASR"), ("format", "klippel")]:
        send(app, ctx, "edit", f"{prefix}-{id}", value)
    send(app, ctx, "edit", "speaker-skip", True)
    send(app, ctx, "next")
    send(app, ctx, "save")
    assert repository.load("New Monitor")["skip"] is True


def test_picture_effect_and_clipboard_failure(repository, metadata, context, tmp_path):
    repository.save(metadata)
    app = MetadataApp(repository, speaker=metadata["name"], pictures_dir=tmp_path / "pictures")
    app.to_spec()
    ctx, messages = context
    send(app, ctx, "choose-picture")
    assert messages[-1]["effect"] == "open_file"
    request = messages[-1]["request_id"]
    source = tmp_path / "test.png"
    source.write_bytes(b"test picture")
    app.on_effect_result(
        request, EffectResult(request, EffectStatus.SUCCEEDED, {"paths": [str(source)]}), ctx
    )
    assert app.editor.picture.name == "Example Monitor.png"
    assert app.editor.picture.read_bytes() == source.read_bytes()
    assert "picture" not in app.editor.export()
    app.on_effect_result(
        "copy:failure",
        EffectResult("copy:failure", EffectStatus.DENIED, error="Clipboard denied"),
        ctx,
    )
    assert app.message_variant == "error"
    assert app.message == "Clipboard denied"


def test_stale_measurement_input_cannot_change_another_speaker(repository, metadata, context):
    repository.save(metadata)
    app = MetadataApp(repository, speaker=metadata["name"])
    app.to_spec()
    old = app.editor.active_id
    app.editor.create("Different", "Speaker")
    app.to_spec()
    send(app, context[0], "edit", f"{old}-notes", "Stale edit")
    assert "notes" not in app.editor.active.data


def test_gpui_session_roundtrip(repository, metadata):
    path = repository.save(metadata)
    env = os.environ.copy()
    env["GPUI_TOOLKIT_SESSION"] = "1"
    process = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "metaedit.app",
            "--metadata-dir",
            str(repository.directory),
            "--speaker",
            metadata["name"],
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    messages = Queue()

    def read_output():
        for line in process.stdout:
            messages.put(json.loads(line))

    reader = Thread(target=read_output, daemon=True)
    reader.start()

    def write(message):
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()

    def receive(kind):
        while True:
            message = messages.get(timeout=15)
            assert message["type"] != "error", message
            if message["type"] == kind:
                return message

    try:
        write(
            {
                "type": "initialize",
                "session_version": 1,
                "capabilities": ["events", "patches", "effects"],
            }
        )
        receive("ready")
        snapshot = receive("snapshot")["app_ir"]
        assert snapshot["miniapp"]["title"] == "Spinorama Metadata"
        prefix = find_node(snapshot, "active-measurement")["value"]
        write(
            {
                "type": "event",
                "id": "edit",
                "sequence": 1,
                "node_id": f"{prefix}-notes",
                "event": "change",
                "action": "edit",
                "payload": {"value": "Session edit"},
            }
        )
        write(
            {
                "type": "event",
                "id": "review",
                "sequence": 2,
                "node_id": "button",
                "event": "click",
                "action": "next",
                "payload": {},
            }
        )
        receive("patch")
        write(
            {
                "type": "event",
                "id": "save",
                "sequence": 3,
                "node_id": "button",
                "event": "click",
                "action": "save",
                "payload": {},
            }
        )
        receive("patch")
        assert tomllib.loads(path.read_text())["measurements"]["vendor"]["notes"] == "Session edit"
        write({"type": "shutdown"})
        process.wait(timeout=10)
        assert process.returncode == 0, process.stderr.read()
    finally:
        if process.poll() is None:
            process.terminate()
        process.communicate(timeout=10)


@pytest.mark.parametrize("stage", [0, 1, 2, "error"])
def test_pypi_native_host_validates_each_screen(repository, metadata, tmp_path, stage):
    repository.save(metadata)
    script = tmp_path / "native_metadata_screen.py"
    script.write_text(
        "from pathlib import Path\n"
        "from metaedit.app import MetadataApp\n"
        "from metaedit.storage import MetadataRepository\n"
        f"app = MetadataApp(MetadataRepository(Path({str(repository.directory)!r})), speaker={metadata['name']!r})\n"
        f"app.stage = {1 if stage == 'error' else stage}\n"
        + (
            "app.message = 'Metadata validation failed'\napp.message_variant = 'error'\n"
            if stage == "error"
            else ""
        )
        + "app.run()\n"
    )
    env = os.environ.copy()
    env["GPUI_TOOLKIT_VALIDATE_ONLY"] = "1"
    env["GPUI_PYTHON"] = sys.executable
    env.pop("GPUI_TOOLKIT_SESSION", None)
    env.pop("GPUI_TOOLKIT_DUMP_IR", None)
    result = subprocess.run(
        [sys.executable, str(script)],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "validated Python GPUI app" in result.stdout


def test_cli_preserves_catalogue_when_native_host_restarts_python(repository, metadata):
    repository.save(metadata)
    env = os.environ.copy()
    env["GPUI_TOOLKIT_VALIDATE_ONLY"] = "1"
    env.pop("GPUI_TOOLKIT_SESSION", None)
    env.pop("GPUI_TOOLKIT_DUMP_IR", None)
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "metaedit.app",
            "--metadata-dir",
            str(repository.directory),
            "--speaker",
            metadata["name"],
            "--generate-graph",
            str(repository.validator),
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "validated Python GPUI app" in result.stdout


def _keys_in_order(keys: List[str], ordered_subset: List[str]) -> bool:
    """Return True if keys from ordered_subset appear in keys in the same relative order.

    Missing keys are ignored; only the relative order of present keys is checked.
    """
    positions = {k: i for i, k in enumerate(keys)}
    present = [k for k in ordered_subset if k in positions]
    return all(positions[present[i]] < positions[present[i + 1]] for i in range(len(present) - 1))


def test_top_level_order_min_diff() -> None:
    from metaedit.models import (
        Measurement,
        SpeakerMetadata,
        export_speaker_metadata,
    )

    sm = SpeakerMetadata(
        brand="BrandX",
        model="ModelY",
        shape="bookshelves",
        type="passive",
        price="$999",
        amount="pair",
        default_measurement="m1",
        measurements={
            "m1": Measurement(origin="TestOrigin", format="klippel"),
        },
    )

    out = export_speaker_metadata(sm)
    keys = list(out.keys())

    expected_order = [
        "brand",
        "model",
        "type",
        "price",
        "amount",
        "shape",
        "default_measurement",
        "measurements",
    ]

    assert keys[: len(expected_order)] == expected_order


def test_measurement_order_and_prune() -> None:
    from metaedit.models import (
        DataAcquisition,
        Measurement,
        Size,
        SpeakerMetadata,
        Specifications,
        export_speaker_metadata,
    )

    meas = Measurement(
        origin="Lab",
        format="klippel",
        quality="A",
        reviews={"default": "Great"},
        review_published="20220101",
        extras=None,  # should be omitted
        data_acquisition=DataAcquisition(via="Klippel"),
        specifications=Specifications(
            sensitivity=85.0,
            impedance=None,
            size=Size(height=10.0),
            weight=10.0,
        ),
    )

    sm = SpeakerMetadata(
        brand="B",
        model="M",
        default_measurement="m1",
        measurements={"m1": meas},
    )

    out = export_speaker_metadata(sm)

    m1 = out["measurements"]["m1"]
    m1_keys = list(m1.keys())

    # Ensure extras and notes are pruned if empty
    assert "extras" not in m1_keys
    assert "notes" not in m1_keys

    expected_meas_order = [
        "origin",
        "format",
        "quality",
        "reviews",
        "review_published",
        "extras",
        "notes",
        "data_acquisition",
        "specifications",
    ]
    assert _keys_in_order(m1_keys, expected_meas_order)

    # Check specs order
    specs = m1["specifications"]
    spec_keys = list(specs.keys())
    expected_spec_order = [
        "sensitivity",
        "impedance",
        "dispersion",
        "SPL",
        "size",
        "weight",
    ]
    assert _keys_in_order(spec_keys, expected_spec_order)


def test_convert_legacy_reviews_single_field():
    from metaedit.models import SpeakerMetadata

    data = {
        "brand": "X",
        "model": "Y",
        "measurements": {"asr": {"origin": "ASR", "review": "https://example.com/r"}},
    }
    converted = SpeakerMetadata.convert_legacy_reviews(data)
    assert "review" not in converted["measurements"]["asr"]
    assert converted["measurements"]["asr"]["reviews"] == {"default": "https://example.com/r"}


def test_date_formatting_roundtrip():
    from metaedit.models import SpeakerMetadata

    src = "20240131"
    as_input = SpeakerMetadata.format_date_for_input(src)
    assert as_input == "2024-01-31"
    back = SpeakerMetadata.format_date_for_metadata(as_input)
    assert back == src
