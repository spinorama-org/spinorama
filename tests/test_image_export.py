"""Public image export: hardening, trace bundles, private/public split."""

import base64
import io
import json
import pathlib
import struct

import plotly.graph_objects as go
import plotly.io as pio
from PIL import Image, ImageDraw

import spinorama.speaker as speaker_module
from spinorama.image_export import (
    bundle_y_values,
    extract_trace_bundle,
    harden_png_to_webp,
    source_label,
    watermark_text,
    write_public_webp_batch,
)


def test_source_label_mapping():
    assert source_label("ASR") == "Audio Science Review"
    assert source_label("", "asr") == "Audio Science Review"
    assert source_label("", "asr_eq") == "Audio Science Review"
    assert source_label("ErinsAudioCorner") == "Erin's Audio Corner"
    assert source_label("", "eac") == "Erin's Audio Corner"
    assert source_label("ErinsAudioCorner", "eac") == "Erin's Audio Corner"
    assert source_label("Princeton") == "Princeton 3D3A"
    assert source_label("Vendors-Genelec") == "Genelec"
    assert source_label("Vendors-Meyer Sound") == "Meyer Sound"
    assert source_label("Misc", "douk-sr") == "Sound & Recording"
    assert source_label("Misc", "xxx-pp") == "Production Partners"
    assert source_label("Misc", "misc-archimago") == "Archimago"
    assert source_label("") == ""
    assert watermark_text() == "spinorama.org  CC BY-NC-SA 4.0"
    assert watermark_text("Audio Science Review") == (
        "spinorama.org  CC BY-NC-SA 4.0  |  Audio Science Review"
    )


def _png_bytes() -> bytes:
    img = Image.new("RGB", (1200, 800), "white")
    draw = ImageDraw.Draw(img)
    draw.line([(80, 700), (600, 200), (1120, 400)], fill=(92, 119, 165), width=4)
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return buf.getvalue()


def test_harden_is_deterministic_per_seed_and_changes_pixels():
    png = _png_bytes()
    a = harden_png_to_webp(png, seed_text="Speaker/ASR/v1/CEA2034")
    b = harden_png_to_webp(png, seed_text="Speaker/ASR/v1/CEA2034")
    c = harden_png_to_webp(png, seed_text="Other/ASR/v1/CEA2034")
    assert a == b
    assert a != c
    out = Image.open(io.BytesIO(a))
    assert out.format == "WEBP"
    assert out.size == (1200, 800)
    original = Image.open(io.BytesIO(png)).convert("RGB")
    hardened = out.convert("RGB")
    # Corner pixel: white originally, textured/watermarked after hardening.
    assert original.getpixel((5, 5)) == (255, 255, 255)
    assert hardened.getpixel((600, 400)) != (255, 255, 255)


def _figure():
    freqs = [20.0 * (1000.0 ** (i / 100.0)) for i in range(101)]
    return go.Figure(
        data=[
            go.Scatter(x=freqs, y=[85.0 - i * 0.05 for i in range(101)], mode="lines", name="On Axis",
                       line={"color": "#5c77a5", "width": 2}),
            go.Scatter(x=freqs, y=[84.0 - i * 0.04 for i in range(101)], mode="lines", name="Listening Window",
                       line={"color": "#dc842a", "dash": "dash"}),
            go.Heatmap(z=[[1.0, 2.0]], name="ignored"),
        ],
        layout={"width": 1200, "height": 800, "title": {"text": "CEA2034 for Test"}},
    )


def test_trace_bundle_quantizes_and_resamples():
    bundle = extract_trace_bundle(_figure(), graph_name="CEA2034", points=128)
    assert bundle is not None
    assert bundle["graph"] == "CEA2034"
    assert bundle["points"] == 128
    assert [t["name"] for t in bundle["traces"]] == ["On Axis", "Listening Window"]
    ys = bundle_y_values(bundle["traces"][0])
    assert len(ys) == 128
    assert abs(ys[0] - 85.0) <= 0.011
    assert -32768 not in struct.unpack(
        "<128h", base64.b64decode(bundle["traces"][0]["y_int16_b64"])
    )[:10]


def test_minus6_directivity_crossings():
    contour = go.Figure(
        data=[
            go.Heatmap(
                x=[100, 1000, 10000],
                y=[-60, -30, 0, 30, 60],
                z=[
                    [-12, -12, -12],
                    [-3, -3, -3],
                    [0, 0, 0],
                    [-3, -3, -3],
                    [-12, -12, -12],
                ],
            )
        ]
    )
    fig = speaker_module._minus6_directivity_figure(contour, "Horizontal Directivity (-6dB)")
    assert fig is not None
    # -6 dB crossing: one third of the way from -3 dB at 30° to -12 dB at 60°.
    assert list(fig.data[0].y) == [40.0, 40.0, 40.0]
    assert list(fig.data[1].y) == [-40.0, -40.0, -40.0]


def test_write_public_webp_batch(tmp_path, monkeypatch):
    def fake_write_images(figs, paths, format=None, width=None, height=None):
        for path in paths:
            pathlib.Path(path).write_bytes(_png_bytes())

    monkeypatch.setattr(pio, "write_images", fake_write_images)
    targets = [str(tmp_path / f"graph{i}.webp") for i in range(3)]
    written = write_public_webp_batch(
        [(_figure(), targets[i], f"seed{i}", "Audio Science Review") for i in range(3)]
    )
    assert written == 3
    for target in targets:
        assert Image.open(target).format == "WEBP"
    assert not list(tmp_path.glob(".kaleido-tmp-*"))


def test_emit_figures_splits_private_json_and_public_webp(tmp_path, monkeypatch):
    public_root = tmp_path / "dist" / "speakers"
    private_root = tmp_path / "dist-private" / "speakers"
    speaker_module.set_dist_speakers_root(str(public_root))
    speaker_module.set_private_speakers_root(str(private_root))

    written_sources = []

    def fake_write_batch(items):
        for _fig, filename, _seed, source in items:
            written_sources.append(source)
            pathlib.Path(filename).write_bytes(b"WEBP")
        return len(items)

    monkeypatch.setattr(speaker_module, "write_public_webp_batch", fake_write_batch)
    try:
        updated = speaker_module.emit_figures(
            {"CEA2034": _figure()}, "Test Speaker", "ASR", "v1", True
        )
    finally:
        speaker_module.set_dist_speakers_root(None)
        speaker_module.set_private_speakers_root(None)

    public_dir = public_root / "Test Speaker" / "ASR" / "v1"
    private_dir = private_root / "Test Speaker" / "ASR" / "v1"
    assert (public_dir / "CEA2034.webp").exists()
    assert not (public_dir / "CEA2034.json").exists()
    assert (private_dir / "CEA2034.json").exists()
    bundle_path = private_dir / "traces" / "CEA2034.json"
    assert bundle_path.exists()
    assert json.loads(bundle_path.read_text())["graph"] == "CEA2034"
    assert updated >= 1
    assert written_sources == ["Audio Science Review"]
