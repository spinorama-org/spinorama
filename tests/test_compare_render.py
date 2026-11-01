"""Pillow comparison renderer over private trace bundles."""

import io

import plotly.graph_objects as go
from PIL import Image

from spinorama.compare_render import render_compare
from spinorama.image_export import bundle_y_values, extract_trace_bundle


def _bundle(offset: float):
    freqs = [20.0 * (1000.0 ** (i / 100.0)) for i in range(101)]
    fig = go.Figure(
        data=[
            go.Scatter(x=freqs, y=[85.0 + offset - i * 0.05 for i in range(101)], mode="lines",
                       name="On Axis", line={"color": "#5c77a5"}),
            go.Scatter(x=freqs, y=[5.0 + offset for _ in range(101)], mode="lines",
                       name="Sound Power DI", line={"color": "#bab0ac"}, yaxis="y2"),
        ],
        layout={
            "width": 1200, "height": 800,
            "title": {"text": "CEA2034"},
            "yaxis": {"range": [40, 95]},
            "yaxis2": {"range": [-10, 20]},
        },
    )
    bundle = extract_trace_bundle(fig, "CEA2034", points=128)
    assert bundle is not None
    return bundle


def test_render_compare_produces_webp():
    out = render_compare(
        _bundle(0.0), _bundle(3.0),
        name_a="Speaker A", name_b="Speaker B",
        graph_name="CEA2034", width=900,
    )
    img = Image.open(io.BytesIO(out))
    assert img.format == "WEBP"
    assert img.size[0] == 900
    assert 500 < img.size[1] < 1000
    colors = img.convert("RGB").getcolors(maxcolors=2_000_000)
    assert colors is not None and len(colors) > 50  # not a blank canvas


def test_bundle_records_axes():
    bundle = _bundle(0.0)
    assert bundle["v"] == 2
    assert bundle["y_range"] == [40.0, 95.0]
    assert bundle["y2_range"] == [-10.0, 20.0]
    assert [t["axis"] for t in bundle["traces"]] == ["y", "y2"]
    di_values = [v for v in bundle_y_values(bundle["traces"][1]) if v == v]
    assert di_values and abs(di_values[0] - 5.0) <= 0.011
