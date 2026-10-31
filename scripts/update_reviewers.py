"""Generate reviewer options from the measurement metadata used by the site."""

import html
import json
import re
import sys

LABELS = {
    "ASR": "ASR",
    "Princeton": "Princeton 3D3A",
    "sr": "Sound & Recording",
    "pp": "Production Partner",
    "audioxpress": "Audio Xpress",
    "avnirvana": "AV Nirvana",
    "sausalitoaudio": "Sausalito Audio",
    "soundstageultra": "Sound Stage Ultra",
    "tomvg": "TomVG",
}


def reviewer_options(metadata):
    reviewers = set()
    for speaker in metadata.values():
        for version, measurement in speaker.get("measurements", {}).items():
            origin = measurement.get("origin", "")
            if origin == "Misc" and version.startswith("misc-"):
                origin = re.sub(r"-(horizontal|vertical|sealed|ported)", "", version[5:])
            if origin:
                reviewers.add(origin)
    for origin in sorted(reviewers, key=str.casefold):
        label = LABELS.get(origin, origin if not origin.islower() else origin.capitalize())
        yield f'<option value="{html.escape(origin, quote=True)}">{html.escape(label)}</option>'


if __name__ == "__main__":
    with open(sys.argv[1], encoding="utf-8") as source:
        print("\n".join(reviewer_options(json.load(source))))
