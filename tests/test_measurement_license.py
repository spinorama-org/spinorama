"""Speaker notices use the license of the selected measurement version."""

import logging

import pytest

import spinorama.constant_paths as cpaths
import generate_html


def test_measurement_license_is_selected_by_version_and_copied(tmp_path, monkeypatch):
    measurements = tmp_path / "measurements"
    monkeypatch.setattr(cpaths, "CPATH_DATAS_SPEAKERS", str(measurements))
    source = measurements / "Test Speaker"
    (source / "asr").mkdir(parents=True)
    (source / "other").mkdir()
    (source / "unknown").mkdir()
    (source / "asr" / "LICENSE.txt").write_text(
        "ASR terms: https://creativecommons.org/licenses/by-nc-sa/4.0/"
    )
    (source / "other" / "LICENSE.md").write_text("Custom terms")
    output = tmp_path / "site"

    info, _, path = generate_html.measurement_license("Test Speaker", "asr", str(output))
    assert info == {"label": "CC BY-NC-SA 4.0", "url": "asr/measurement-license.txt"}
    assert (output / info["url"]).read_text() == (source / "asr" / "LICENSE.txt").read_text()
    assert path.endswith("LICENSE.txt")

    info, _, _ = generate_html.measurement_license("Test Speaker", "other", str(output))
    assert info == {"label": "View license", "url": "other/measurement-license.txt"}
    assert (output / info["url"]).read_text() == "Custom terms"
    assert generate_html.measurement_license("Test Speaker", "unknown", str(output))[0] is None


@pytest.mark.parametrize("has_eq", [False, True])
def test_equalized_page_uses_base_measurement_license(tmp_path, monkeypatch, has_eq):
    measurements = tmp_path / "measurements"
    source = measurements / "Test Speaker" / "asr"
    source.mkdir(parents=True)
    (source / "LICENSE.txt").write_text("https://creativecommons.org/licenses/by-nc-sa/4.0/")
    monkeypatch.setattr(cpaths, "CPATH_DATAS_SPEAKERS", str(measurements))
    monkeypatch.setattr(cpaths, "CPATH_DIST_SPEAKERS", str(tmp_path / "site"))
    monkeypatch.setattr(generate_html, "logger", logging.getLogger(__name__), raising=False)
    monkeypatch.setattr(generate_html, "flag_optim", False, raising=False)
    monkeypatch.setattr(generate_html, "find_metadata_file", lambda: ("meta", "eq"))
    monkeypatch.setattr(generate_html, "find_metadata_chunks", lambda: {})
    monkeypatch.setattr(generate_html, "need_update", lambda *_: True)

    class Capture:
        def render(self, **kwargs):
            self.context = kwargs
            return "rendered"

    template = Capture()
    generate_html.generate_measurement(
        {}, {"Test Speaker": {"measurements": {"asr": {"origin": "ASR"}}}},
        "", True, {}, "Test Speaker", {}, template, template, "ASR",
        {"asr_eq": {"On Axis": {}}} if has_eq else {}, "asr_eq", {},
    )
    # EQ graph availability does not depend on a preference score.
    assert template.context["has_eq_graphs"] is has_eq
    from mako.template import Template
    tabs = Template(filename="src/website/tabs_header.html").render(has_eq_graphs=has_eq)
    assert ('data-target="pane-3"' in tabs) is has_eq
    assert ('data-target="pane-4"' in tabs) is has_eq
    assert template.context["measurement_license"]["label"] == "CC BY-NC-SA 4.0"
    assert template.context["measurement_license"]["url"] == "asr/measurement-license.txt"


def test_source_banner_uses_original_review_and_asr_name():
    from website.utils import measurement_source

    source = measurement_source(None, {
        "origin": "ASR",
        "reviews": {"asr": "https://example.org/original", "other": "https://example.org/other"},
    }, "asr")
    assert source["name"] == "AudioScienceReview"
    assert source["url"] == "https://example.org/original"
    assert source["logo"].startswith("/pictures/")


def test_source_banner_handles_missing_review_and_logo():
    from website.utils import measurement_source

    assert measurement_source(None, {"origin": "Vendors-Aalto Speakers"}, "vendor") == {
        "name": "Aalto Speakers", "url": "", "logo": "",
    }


def test_source_banner_supports_erin_and_vendor_icon_names():
    from website.utils import measurement_source

    cases = {
        "ErinsAudioCorner": ("Erin's Audio Corner", "icon-eac-32x32.webp"),
        "Vendors-Ascend Acoustics": ("Ascend Acoustics", "icon-ascendacoustics-32x32.webp"),
        "Vendors-Audio First Design": ("Audio First Design", "icon-afd-32x32.webp"),
        "Vendors-Buchardt Audio": ("Buchardt Audio", "Buchardt%20Audio.png"),
        "Vendors-Meyer Sound": ("Meyer Sound", "meyersound.png"),
        "Vendors-Revel": ("Revel", "revel.png"),
        "Vendors-Bose": ("Bose", "bose.png"),
    }
    for origin, (name, icon) in cases.items():
        result = measurement_source(None, {"origin": origin, "reviews": {"original": "https://example.org/review"}}, "vendor")
        assert result == {"name": name, "logo": "/pictures/" + icon, "url": "https://example.org/review"}
    # Do not inherit an unrelated vendor's logo from old origin metadata.
    assert measurement_source(None, {"origin": "Vendors-Bowers & Wilkins"}, "vendor")["logo"] == ""
