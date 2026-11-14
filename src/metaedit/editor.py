"""Editable metadata drafts independent of the desktop renderer."""

from dataclasses import dataclass, field
import copy
import difflib
from itertools import count
from pathlib import Path
import shutil

from metaedit.models import SpeakerMetadata, export_speaker_metadata, prune_empty
from metaedit.storage import MetadataRepository, ROOT, sanitize_filename


@dataclass
class ReviewDraft:
    id: str
    key: str = ""
    url: str = ""


@dataclass
class MeasurementDraft:
    id: str
    key: str
    data: dict = field(default_factory=dict)
    reviews: list[ReviewDraft] = field(default_factory=list)


def nested_value(data: dict, path: tuple[str, ...], default=""):
    for key in path:
        if not isinstance(data, dict) or key not in data:
            return default
        data = data[key]
    return data


def set_nested(data: dict, path: tuple[str, ...], value) -> None:
    for key in path[:-1]:
        data = data.setdefault(key, {})
    data[path[-1]] = value


class EditorState:
    def __init__(self, repository: MetadataRepository, pictures_dir: Path | None = None):
        self.repository = repository
        self.pictures_dir = pictures_dir or ROOT / "datas/pictures"
        self._ids = count()
        self.original_name: str | None = None
        self.speaker: dict = {}
        self.measurements: list[MeasurementDraft] = []
        self.default_id = ""
        self.active_id = ""
        self.picture: Path | None = None

    def load(self, name: str) -> None:
        data = self.repository.load(name)
        if data is None:
            message = f"Speaker {name!r} not found"
            raise ValueError(message)
        self.original_name = name
        self.speaker = data
        self.measurements = []
        self.default_id = ""
        for key, measurement in self.speaker.pop("measurements").items():
            row = self.add_measurement(key, measurement)
            if key == self.speaker.get("default_measurement"):
                self.default_id = row.id
        if not self.measurements:
            self.add_measurement()
        self.active_id = self.default_id or self.measurements[0].id
        self.picture = self.find_picture()

    def create(self, brand: str, model: str) -> None:
        if not brand.strip() or not model.strip():
            message = "Enter a brand and model before creating a speaker"
            raise ValueError(message)
        self.original_name = None
        self.speaker = {
            "brand": brand.strip(),
            "model": model.strip(),
            "type": "passive",
            "shape": "bookshelves",
            "price": "",
        }
        self.measurements = []
        self.default_id = ""
        self.add_measurement()
        self.picture = self.find_picture()

    @property
    def name(self) -> str:
        return (
            self.original_name
            or f"{self.speaker.get('brand', '')} {self.speaker.get('model', '')}".strip()
        )

    @property
    def active(self) -> MeasurementDraft:
        return next(row for row in self.measurements if row.id == self.active_id)

    def add_measurement(self, key: str = "", data: dict | None = None) -> MeasurementDraft:
        values = copy.deepcopy(data or {})
        row = MeasurementDraft(f"m{next(self._ids)}", key, values)
        for review_key, url in values.pop("reviews", {}).items():
            self.add_review(row, review_key, url)
        self.measurements.append(row)
        self.active_id = row.id
        if not self.default_id:
            self.default_id = row.id
        return row

    def remove_measurement(self, id: str) -> None:
        if len(self.measurements) == 1:
            message = "Keep at least one measurement"
            raise ValueError(message)
        self.measurements = [row for row in self.measurements if row.id != id]
        if self.default_id == id:
            self.default_id = self.measurements[0].id
        if self.active_id == id:
            self.active_id = self.measurements[0].id

    def add_review(self, row: MeasurementDraft, key: str = "", url: str = "") -> ReviewDraft:
        review = ReviewDraft(f"r{next(self._ids)}", key, url)
        row.reviews.append(review)
        return review

    def export(self) -> dict:
        measurements = {}
        for row in self.measurements:
            key = row.key.strip()
            if not key or key in measurements:
                message = "Each measurement needs a unique, nonempty version key"
                raise ValueError(message)
            values = copy.deepcopy(row.data)
            reviews = {}
            for review in row.reviews:
                if not review.key.strip() and not review.url.strip():
                    continue
                if (
                    not review.key.strip()
                    or not review.url.strip()
                    or review.key.strip() in reviews
                ):
                    message = "Each review needs a unique key and a URL"
                    raise ValueError(message)
                reviews[review.key.strip()] = review.url.strip()
            values["reviews"] = reviews
            measurements[key] = values
        data = copy.deepcopy(self.speaker)
        data["name"] = self.name
        data["measurements"] = measurements
        data["default_measurement"] = next(
            row.key.strip() for row in self.measurements if row.id == self.default_id
        )
        return export_speaker_metadata(SpeakerMetadata(**prune_empty(data)))

    def preview(self) -> str:
        return self.repository.render(self.export(), self.original_name)

    def diff(self) -> str:
        before = self.repository.sources.get(self.original_name, "")
        return (
            "\n".join(
                difflib.unified_diff(
                    before.splitlines(),
                    self.preview().splitlines(),
                    fromfile="original/meta.toml",
                    tofile="edited/meta.toml",
                    lineterm="",
                )
            )
            or "No changes."
        )

    def save(self) -> Path:
        data = self.export()
        path = self.repository.save(data, original_name=self.original_name)
        self.original_name = data["name"]
        return path

    def find_picture(self) -> Path | None:
        for directory in (self.pictures_dir, ROOT / "datas/icons"):
            for extension in (".png", ".jpg", ".jpeg", ".webp"):
                path = directory / (sanitize_filename(self.name) + extension)
                if path.is_file():
                    return path
        return None

    def choose_picture(self, source: Path) -> Path:
        if source.suffix.lower() not in {".png", ".jpg", ".jpeg", ".webp"} or not source.is_file():
            message = "Choose a PNG, JPEG, or WebP picture"
            raise ValueError(message)
        self.pictures_dir.mkdir(parents=True, exist_ok=True)
        destination = self.pictures_dir / (sanitize_filename(self.name) + source.suffix.lower())
        if source.resolve() != destination.resolve():
            shutil.copy2(source, destination)
        self.picture = destination
        return destination
