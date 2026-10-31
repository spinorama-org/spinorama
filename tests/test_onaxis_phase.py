"""Phase on the on-axis plot uses the measured degree reference."""

import numpy as np
import pandas as pd
import pytest

from spinorama.loaders.spl_hv_txt import parse_graph_spl_hv_txt
from spinorama.plot.spl import plot_graph_onaxis

PARAMS = {"width": 1000, "height": 700, "ymin": -40, "ymax": 10}


def phase_trace(h_spl, **options):
    onaxis = h_spl[["Freq", "On Axis"]]
    figure = plot_graph_onaxis(onaxis, h_spl, PARAMS | options, (20, 20000))
    trace = next(trace for trace in figure.data if trace.name == "Phase (deg)")
    return figure, trace


def test_wrapped_phase_preserves_reference_and_breaks_at_wraps():
    h_spl = pd.DataFrame(
        {"Freq": [100, 500, 1000, 2000, 5000],
         "On Axis": [0, 1, 0, 0, -1],
         "Phase On Axis": [-170, -190, -350, -530, -550]}
    )
    original = h_spl.copy()
    figure, trace = phase_trace(h_spl)

    assert list(figure.layout.yaxis2.range) == [-180, 360]
    assert list(trace.y) == [-170, None, 170, 10, -170, None, 170]
    assert list(trace.x) == [100, 500, 500, 1000, 2000, 5000, 5000]
    assert trace.connectgaps is False
    pd.testing.assert_frame_equal(h_spl, original)


def test_small_degree_values_are_not_mistaken_for_radians():
    h_spl = pd.DataFrame(
        {"Freq": [100, 1000, 10000], "On Axis": [0, 1, 0], "Phase On Axis": [-1, 0, 1]}
    )
    _, trace = phase_trace(h_spl, phase_remove_delay=False)
    assert list(trace.y) == [-1, 0, 1]


def test_aalto_measurement_phase_matches_original_export():
    status, h_spl = parse_graph_spl_hv_txt(
        "datas/measurements/Aalto Speakers Aalto 7 Passive/vendor", "H"
    )
    assert status
    _, trace = phase_trace(h_spl, phase_remove_delay=False)
    plotted = {float(f): float(p) for f, p in zip(trace.x, trace.y, strict=True) if p is not None}
    for frequency, expected in [(20, -131.695), (100, 7.695), (1000, 129.364), (2000, 10.006)]:
        row = h_spl.iloc[(h_spl.Freq - frequency).abs().argmin()]
        assert plotted[row.Freq] == pytest.approx(expected, abs=0.01)
        assert np.isclose((plotted[row.Freq] - row["Phase On Axis"]) % 360, 0)


@pytest.mark.parametrize("wrapped_input", [False, True])
def test_default_removes_high_frequency_delay(wrapped_input):
    freq = np.array([100, 500, 1000, 12000, 12500, 13000, 13500, 14000, 14500, 15000, 15500, 16000])
    phase = 37.0 - 360.0 * freq * 0.0002
    if wrapped_input:
        phase = (phase + 180) % 360 - 180
    data = pd.DataFrame({"Freq": freq, "On Axis": np.zeros(len(freq)), "Phase On Axis": phase})
    original = data.copy()
    figure, trace = phase_trace(data)
    assert list(trace.y) == pytest.approx([37] * len(freq), abs=1e-8)
    assert "0.20 ms delay removed" in figure.layout.yaxis2.title.text
    serialized = figure.to_plotly_json()
    info = next(t for t in serialized["data"] if t.get("name") == "Phase (deg)")["meta"]["spinorama_phase"]
    assert info["delay_seconds"] == pytest.approx(0.0002)
    np.testing.assert_allclose(
        np.exp(1j * np.deg2rad(info["measured"])), np.exp(1j * np.deg2rad(phase)), atol=1e-10
    )
    np.testing.assert_allclose(
        np.exp(1j * np.deg2rad(info["corrected"])), np.exp(1j * np.deg2rad(37)), atol=1e-10
    )
    pd.testing.assert_frame_equal(data, original)


def test_unwrapped_and_no_delay_options_and_insufficient_band():
    data = pd.DataFrame({"Freq": [100, 1000, 10000], "On Axis": [0, 1, 0],
                         "Phase On Axis": [-170, -190, -350]})
    figure, trace = phase_trace(data, phase_wrap=False, phase_remove_delay=False)
    assert list(trace.y) == [-170, -190, -350]
    assert figure.layout.yaxis2.range is None
    figure, _ = phase_trace(data)
    assert "delay not removed" in figure.layout.yaxis2.title.text


@pytest.mark.parametrize("melted", [False, True])
def test_spl_level_normalization_preserves_phase(melted):
    from spinorama.load import shift_spl, normalize_spl
    from spinorama.misc import graph_melt

    status, raw = parse_graph_spl_hv_txt(
        "datas/measurements/Aalto Speakers Aalto 7 Passive/vendor", "H"
    )
    assert status
    data = graph_melt(raw) if melted else raw
    shifted = shift_spl(data, 91.72545283018865)
    normalized = normalize_spl(data)
    np.testing.assert_allclose(shifted["Phase On Axis"], raw["Phase On Axis"])
    np.testing.assert_allclose(normalized["Phase On Axis"], raw["Phase On Axis"])
    np.testing.assert_allclose(shifted["On Axis"], raw["On Axis"] - 91.72545283018865)
    np.testing.assert_allclose(normalized["On Axis"], 0)
    _, trace = phase_trace(shifted, phase_remove_delay=False)
    index = list(trace.x).index(20)
    assert trace.y[index] == pytest.approx(-131.695, abs=0.01)
