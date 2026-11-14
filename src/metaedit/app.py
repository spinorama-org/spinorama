"""Python GPUI desktop editor for the per-speaker TOML catalogue."""

from __future__ import annotations

import argparse
from collections.abc import Callable
import os
from pathlib import Path
import sys

from gpui_toolkit import App, Event, SessionContext, effects, section, ui
from gpui_toolkit.effects import EffectResult, EffectStatus
from gpui_toolkit.miniapp import MiniAppConfig

from metaedit.editor import EditorState, nested_value, set_nested
from metaedit.storage import MetadataRepository

SHAPES = (
    "bookshelves",
    "cbt",
    "center",
    "cinema",
    "columns",
    "floorstanders",
    "inwall",
    "liveportable",
    "omnidirectional",
    "outdoor",
    "panel",
    "soundbar",
    "surround",
    "toursound",
)
FORMATS = (
    "klippel",
    "spl_hv_txt",
    "gll_hv_txt",
    "princeton",
    "rew_text_dump",
    "webplotdigitizer",
    "csv_freq_spl",
)
DA_FIELDS = (
    ("distance", "Distance (m)"),
    ("resolution", "Resolution (degrees)"),
    ("min_valid_freq", "Min valid frequency (Hz)"),
    ("max_valid_freq", "Max valid frequency (Hz)"),
)
SPEC_FIELDS = (
    (("sensitivity",), "Sensitivity (dB)"),
    (("impedance",), "Impedance (ohms)"),
    (("weight",), "Weight (kg)"),
    (("size", "height"), "Height (mm)"),
    (("size", "width"), "Width (mm)"),
    (("size", "depth"), "Depth (mm)"),
    (("dispersion", "horizontal"), "Horizontal dispersion (degrees)"),
    (("dispersion", "vertical"), "Vertical dispersion (degrees)"),
    (("SPL", "peak"), "Peak SPL (dB)"),
    (("SPL", "continuous"), "Continuous SPL (dB)"),
    (("SPL", "max"), "Max SPL (dB)"),
    (("SPL", "m_noise"), "M-noise SPL (dB)"),
    (("SPL", "b_noise"), "B-noise SPL (dB)"),
    (("SPL", "pink_noise"), "Pink-noise SPL (dB)"),
)


def _button(label: str, *, action: str, id: str | None = None, **props):
    return ui.button(label, id=id or f"button-{action}", action=action, **props)


class MetadataApp(App):
    def __init__(
        self,
        repository: MetadataRepository | None = None,
        *,
        speaker: str | None = None,
        pictures_dir: Path | None = None,
    ):
        self.editor = EditorState(repository or MetadataRepository(), pictures_dir)
        self.stage = 0
        self.mode = 0
        self.query = ""
        self.selected_speaker = ""
        self.new_brand = ""
        self.new_model = ""
        self.message = ""
        self.message_variant = "info"
        self.show_diff = False
        self._bindings: dict[str, Callable] = {}
        self._picture_requests: dict[str, str] = {}
        if speaker:
            self.editor.load(speaker)
            self.selected_speaker = speaker
            self.stage = 1
        super().__init__(
            title="Spinorama Metadata",
            sidebar_title="Spinorama",
            sidebar_subtitle="Speaker metadata",
            serial_reducer=True,
            required_capabilities=("events", "patches", "effects"),
            miniapp=MiniAppConfig(
                title="Spinorama Metadata",
                app_name="Spinorama Metadata",
                width=1100,
                height=800,
                with_theme=True,
            ),
        )

    def to_spec(self) -> dict:
        self.sections = [section("metadata", "Metadata", self.view())]
        return super().to_spec()

    def _input(
        self,
        data: dict,
        path: tuple[str, ...],
        label: str,
        *,
        prefix: str,
        choices=None,
        numeric=False,
        disabled=False,
        help=None,
    ):
        id = prefix + "-" + "-".join(path)
        value = nested_value(data, path)
        self._bindings[id] = lambda value: set_nested(data, path, value)
        kwargs = dict(
            id=id,
            label=label,
            action="edit",
            disabled=disabled,
            help=help,
            width=600 if path[-1] == "notes" else 220,
        )
        if choices is not None:
            return ui.select(
                value=value,
                options=[("", "Unspecified"), *((choice, choice) for choice in choices)],
                **kwargs,
            )
        if numeric:
            return ui.number_input(value=value, **kwargs)
        return ui.text_input(value=str(value), **kwargs)

    def _check(self, data: dict, path: tuple[str, ...], label: str, *, prefix: str):
        id = prefix + "-" + "-".join(path)
        self._bindings[id] = lambda value: set_nested(data, path, bool(value))
        return ui.checkbox(
            id=id, label=label, value=bool(nested_value(data, path, False)), action="edit"
        )

    def _attribute_input(self, id: str, owner, attribute: str, label: str, **kwargs):
        self._bindings[id] = lambda value: setattr(owner, attribute, str(value))
        kwargs.setdefault("width", 400 if attribute == "url" else 220)
        return ui.text_input(
            id=id, value=str(getattr(owner, attribute)), label=label, action="edit", **kwargs
        )

    def _speaker_options(self):
        names = self.editor.repository.speakers()
        return [
            ("", "Select a speaker"),
            *((name, name) for name in names if self.query.casefold() in name.casefold()),
        ]

    def _select_view(self):
        children = [
            ui.tabs(
                ["Existing speaker", "New speaker"],
                active=self.mode,
                id="speaker-mode",
                action="mode",
            )
        ]
        if self.mode == 0:
            children.extend(
                [
                    self._attribute_input("speaker-search", self, "query", "Search speakers"),
                    ui.select(
                        id="speaker-selection",
                        value=self.selected_speaker,
                        options=self._speaker_options(),
                        label="Speaker",
                        action="select-speaker",
                    ),
                ]
            )
        else:
            children.extend(
                [
                    self._attribute_input("new-brand", self, "new_brand", "Brand"),
                    self._attribute_input("new-model", self, "new_model", "Model"),
                ]
            )
        return ui.vstack(children, gap=16)

    def _edit_view(self):
        editor = self.editor
        speaker = editor.speaker
        row = editor.active
        prefix = row.id
        self._bindings["default-measurement"] = lambda value: setattr(
            editor, "default_id", str(value)
        )
        fields = [
            self._input(speaker, ("brand",), "Brand", prefix="speaker"),
            self._input(speaker, ("model",), "Model", prefix="speaker"),
            self._input(
                speaker, ("type",), "Type", prefix="speaker", choices=("passive", "active")
            ),
            self._input(speaker, ("shape",), "Shape", prefix="speaker", choices=SHAPES),
            self._input(speaker, ("price",), "Price", prefix="speaker"),
            self._input(speaker, ("amount",), "Amount", prefix="speaker", choices=("each", "pair")),
        ]
        measurement = [
            ui.wrap(
                [
                    self._attribute_input(f"{prefix}-key", row, "key", "Version key"),
                    self._input(row.data, ("origin",), "Origin", prefix=prefix),
                    self._input(row.data, ("format",), "Format", prefix=prefix, choices=FORMATS),
                    self._input(
                        row.data,
                        ("quality",),
                        "Quality",
                        prefix=prefix,
                        choices=("low", "medium", "high", "unknown"),
                    ),
                    self._input(
                        row.data,
                        ("symmetry",),
                        "Symmetry",
                        prefix=prefix,
                        choices=("none", "coaxial", "vertical", "horizontal"),
                    ),
                ],
                gap=12,
            ),
            self._input(row.data, ("notes",), "Measurement notes", prefix=prefix),
            ui.wrap(
                [
                    self._input(
                        row.data,
                        ("parameters", "mean_min"),
                        "Mean band: min (Hz)",
                        prefix=prefix,
                        numeric=True,
                    ),
                    self._input(
                        row.data,
                        ("parameters", "mean_max"),
                        "Mean band: max (Hz)",
                        prefix=prefix,
                        numeric=True,
                    ),
                ],
                gap=12,
            ),
        ]
        reviews = [
            self._input(
                row.data,
                ("review_published",),
                "Review date",
                prefix=prefix,
                help="YYYYMMDD; partial dates remain unchanged.",
            )
        ]
        if "review" in row.data:
            reviews.append(self._input(row.data, ("review",), "Review URL", prefix=prefix))
        for review in row.reviews:
            reviews.append(
                ui.wrap(
                    [
                        self._attribute_input(f"{review.id}-key", review, "key", "Review key"),
                        self._attribute_input(f"{review.id}-url", review, "url", "URL"),
                        _button(
                            "Remove review",
                            id=f"remove-{review.id}",
                            action=f"remove-review:{review.id}",
                        ),
                    ],
                    gap=12,
                )
            )
        reviews.append(_button("Add review", action="add-review"))
        active = speaker.get("type") == "active"
        specs = [
            self._input(
                row.data,
                ("specifications", *path),
                label,
                prefix=prefix,
                numeric=True,
                disabled=active and path in (("sensitivity",), ("impedance",)),
            )
            for path, label in SPEC_FIELDS
        ]
        acquisition = [
            self._input(
                row.data,
                ("data_acquisition", "via"),
                "Acquired via",
                prefix=prefix,
                help="For example: Klippel or GLL",
            ),
            self._input(row.data, ("data_acquisition", "signal"), "Signal", prefix=prefix),
            *[
                self._input(row.data, ("data_acquisition", key), label, prefix=prefix, numeric=True)
                for key, label in DA_FIELDS
            ],
            self._check(
                row.data, ("data_acquisition", "air_absorbtion"), "Air absorption", prefix=prefix
            ),
            self._input(
                row.data, ("data_acquisition", "notes"), "Acquisition notes", prefix=prefix
            ),
        ]
        extras = [
            self._check(
                row.data, ("extras", "is_equed"), "Measurement already equalized", prefix=prefix
            ),
            self._input(
                row.data, ("extras", "score_penalty"), "Score penalty", prefix=prefix, numeric=True
            ),
        ]
        measurement.append(
            ui.accordion(
                id=f"{prefix}-details",
                multiple=True,
                expanded=("reviews", "specifications"),
                items=[
                    ("reviews", "Reviews", reviews),
                    ("specifications", "Specifications", [ui.wrap(specs, gap=12)]),
                    ("acquisition", "Data acquisition", [ui.wrap(acquisition, gap=12)]),
                    ("extras", "Extras", extras),
                ],
            )
        )
        children = [
            ui.section_header(editor.name, "Edit speaker metadata and measurement versions."),
            ui.wrap(fields, gap=12),
            self._check(speaker, ("skip",), "Skip generation (metadata only)", prefix="speaker"),
            ui.wrap(
                [
                    _button("Choose picture…", action="choose-picture"),
                    _button("View picture", action="view-picture", disabled=editor.picture is None),
                    ui.text(
                        editor.picture.name if editor.picture else "No picture", tone="secondary"
                    ),
                ],
                gap=12,
            ),
            ui.divider(),
            ui.wrap(
                [
                    ui.select(
                        id="default-measurement",
                        label="Default measurement",
                        value=editor.default_id,
                        options=[
                            (item.id, item.key or "New measurement") for item in editor.measurements
                        ],
                        action="edit",
                    ),
                    ui.select(
                        id="active-measurement",
                        label="Edit measurement",
                        value=editor.active_id,
                        options=[
                            (item.id, item.key or "New measurement") for item in editor.measurements
                        ],
                        action="measurement",
                    ),
                    _button("Add measurement", action="add-measurement"),
                    _button(
                        "Remove measurement",
                        action="remove-measurement",
                        disabled=len(editor.measurements) == 1,
                    ),
                ],
                gap=12,
            ),
            *measurement,
        ]
        return ui.vstack(children, gap=16)

    def view(self):
        self._bindings = {}
        if self.stage == 0:
            body = self._select_view()
        elif self.stage == 1:
            body = self._edit_view()
        else:
            body = ui.vstack(
                [
                    ui.wrap(
                        [
                            _button("TOML", action="preview-toml", selected=not self.show_diff),
                            _button("Show diff", action="preview-diff", selected=self.show_diff),
                            _button("Copy TOML", action="copy"),
                            _button("Save metadata", action="save"),
                        ],
                        gap=12,
                    ),
                    ui.code(self.editor.diff() if self.show_diff else self.editor.preview()),
                ],
                gap=16,
            )
        return ui.vstack(
            [
                ui.section_header(
                    "Speaker metadata", "Select, edit, and review TOML before saving."
                ),
                ui.stepper(
                    id="workflow",
                    steps=["Select speaker", "Edit metadata", "Review and save"],
                    active=self.stage,
                    disabled_steps=(0, 1, 2),
                ),
                ui.alert(self.message, id="status", variant=self.message_variant)
                if self.message
                else ui.text("", id="status"),
                ui.wrap(
                    [
                        _button("Back", action="back", disabled=self.stage == 0),
                        _button(
                            "Next" if self.stage == 0 else "Review TOML",
                            action="next",
                            disabled=self.stage == 2,
                        ),
                        _button("Start over", action="start-over"),
                        _button("Close", action="close"),
                    ],
                    gap=12,
                ),
                body,
            ],
            id="metadata-root",
            gap=20,
        )

    def _publish(self, context: SessionContext, event: Event | None = None):
        context.patch(
            [{"op": "replace", "id": "metadata-root", "node": self.view().to_spec()}],
            request_id=event.id if event else None,
        )

    def _edit(self, event: Event, context: SessionContext):
        update = self._bindings.get(event.node_id)
        if update is None:
            return
        value = (event.payload or {}).get("value", "")
        update(value)
        ops = []
        if event.node_id == "speaker-search":
            options = self._speaker_options()
            if self.selected_speaker not in {key for key, _ in options}:
                self.selected_speaker = ""
            ops = [
                {
                    "op": "set",
                    "id": "speaker-selection",
                    "property": "options",
                    "value": [{"value": key, "label": label} for key, label in options],
                },
                {
                    "op": "set",
                    "id": "speaker-selection",
                    "property": "value",
                    "value": self.selected_speaker,
                },
            ]
        elif self.stage == 1:
            row = self.editor.active
            defaults = {}
            if event.node_id == f"{row.id}-format" and value == "klippel":
                defaults[("quality",)] = "high"
            if event.node_id == f"{row.id}-data_acquisition-via" and str(value).lower() == "gll":
                defaults.update(
                    {
                        ("data_acquisition", key): default
                        for key, default in {
                            "distance": 10,
                            "signal": "aes 20Hz-20kHz",
                            "min_valid_freq": 20,
                            "max_valid_freq": 20000,
                        }.items()
                        if nested_value(row.data, ("data_acquisition", key)) == ""
                    }
                )
            for path, default in defaults.items():
                set_nested(row.data, path, default)
                ops.append(
                    {
                        "op": "set",
                        "id": row.id + "-" + "-".join(path),
                        "property": "value",
                        "value": default,
                    }
                )
            if event.node_id == "speaker-type":
                for key in ("sensitivity", "impedance"):
                    ops.append(
                        {
                            "op": "set",
                            "id": f"{row.id}-specifications-{key}",
                            "property": "disabled",
                            "value": value == "active",
                        }
                    )
            if event.node_id == f"{row.id}-key":
                options = [
                    {"value": item.id, "label": item.key or "New measurement"}
                    for item in self.editor.measurements
                ]
                for id in ("default-measurement", "active-measurement"):
                    ops.append({"op": "set", "id": id, "property": "options", "value": options})
        if ops:
            context.patch(ops, request_id=event.id)

    def on_action(self, event: Event, context: SessionContext):
        context.acknowledge(event)
        payload = event.payload or {}
        try:
            if event.action == "edit":
                self._edit(event, context)
                return
            self.message = ""
            self.message_variant = "info"
            if event.action == "select-speaker":
                self.selected_speaker = str(payload.get("value", ""))
                return
            if event.action == "mode":
                self.mode = int(payload.get("index", payload.get("value", 0)))
            elif event.action == "next":
                if self.stage == 0:
                    if self.mode == 0:
                        self.editor.load(self.selected_speaker)
                    else:
                        self.editor.create(self.new_brand, self.new_model)
                elif self.stage == 1:
                    self.editor.preview()
                self.stage = min(2, self.stage + 1)
                self.show_diff = False
            elif event.action == "back":
                self.stage = max(0, self.stage - 1)
            elif event.action == "start-over":
                self.editor.repository.reload()
                self.stage = 0
                self.selected_speaker = ""
            elif event.action == "measurement":
                self.editor.active_id = str(payload["value"])
            elif event.action == "add-measurement":
                self.editor.add_measurement()
            elif event.action == "remove-measurement":
                self.editor.remove_measurement(self.editor.active_id)
            elif event.action == "add-review":
                self.editor.add_review(self.editor.active)
            elif event.action and event.action.startswith("remove-review:"):
                id = event.action.split(":", 1)[1]
                row = self.editor.active
                row.reviews = [review for review in row.reviews if review.id != id]
            elif event.action == "save":
                path = self.editor.save()
                self.message, self.message_variant = f"Saved {path}", "success"
            elif event.action in {"preview-toml", "preview-diff"}:
                self.show_diff = event.action == "preview-diff"
            elif event.action == "copy":
                effects.clipboard_write(context, f"copy:{event.id}", self.editor.preview())
                return
            elif event.action == "choose-picture":
                request_id = f"picture:{event.id}"
                self._picture_requests[request_id] = self.editor.name
                effects.choose_file(
                    context,
                    request_id,
                    prompt="Choose a speaker picture",
                    filters=("png", "jpg", "jpeg", "webp"),
                )
                return
            elif event.action == "view-picture" and self.editor.picture:
                effects.open_with_system(context, event.id, str(self.editor.picture))
                return
            elif event.action == "close":
                effects.close_window(context, event.id)
                return
            else:
                return
        except (ValueError, OSError) as error:
            self.message, self.message_variant = str(error), "error"
        self._publish(context, event)

    def on_effect_result(self, request_id: str, result: EffectResult, context: SessionContext):
        name = self._picture_requests.pop(request_id, None)
        if result.status is EffectStatus.CANCELLED:
            return
        if not result.ok:
            self.message = result.error or "The native operation failed"
            self.message_variant = "error"
            self._publish(context)
            return
        try:
            if name == self.editor.name:
                paths = result.data.get("paths", [])
                if paths:
                    self.editor.choose_picture(Path(paths[0]))
            elif request_id.startswith("copy:"):
                self.message, self.message_variant = "TOML copied to clipboard", "success"
        except (ValueError, OSError) as error:
            self.message, self.message_variant = str(error), "error"
        self._publish(context)


def main() -> None:
    parser = argparse.ArgumentParser(description="Spinorama Metadata Manager (Python GPUI)")
    parser.add_argument(
        "-s", "--speaker", default=os.environ.get("SPINORAMA_METAEDIT_SPEAKER") or None
    )
    parser.add_argument(
        "--metadata-dir", type=Path, default=os.environ.get("SPINORAMA_METAEDIT_DIR")
    )
    parser.add_argument("--generate-graph", type=Path, default=None)
    args = parser.parse_args()
    try:
        repository = MetadataRepository(args.metadata_dir, args.generate_graph)
        app = MetadataApp(repository, speaker=args.speaker)
    except (ValueError, OSError) as error:
        parser.exit(1, f"{error}\n")
    # The native host restarts this script in session mode without the CLI arguments.
    os.environ["SPINORAMA_METAEDIT_DIR"] = str(repository.directory)
    os.environ["SPINORAMA_GENERATE_GRAPH"] = str(repository.validator)
    os.environ["SPINORAMA_METAEDIT_SPEAKER"] = args.speaker or ""
    os.environ["GPUI_PYTHON"] = sys.executable
    app.run()


if __name__ == "__main__":
    main()
