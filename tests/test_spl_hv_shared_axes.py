"""Shared front and rear directions in SPL H/V text exports."""

import pytest

from spinorama.loaders.spl_hv_txt import parse_graph_spl_hv_txt


def write_sweep(directory, orientation, angle, level, named):
    name = f"Speaker _{orientation} {angle}.txt" if named else f"{angle}_{orientation}.txt"
    (directory / name).write_text(f"100,{level},10\n1000,{level + 1},20\n")


@pytest.mark.parametrize("orientation,other", [("H", "V"), ("V", "H")])
@pytest.mark.parametrize("rear", [180, -180])
@pytest.mark.parametrize("named", [False, True])
def test_missing_shared_directions(tmp_path, orientation, other, rear, named):
    write_sweep(tmp_path, orientation, 10, 70, named)
    write_sweep(tmp_path, other, 0, 80, named)
    write_sweep(tmp_path, other, rear, 60, named)
    write_sweep(tmp_path, other, 20, 50, named)

    status, spl = parse_graph_spl_hv_txt(str(tmp_path), orientation, "none")

    assert status
    assert spl["Freq"].tolist() == [100, 1000]
    assert spl["On Axis"].tolist() == [80, 81]
    assert spl["Phase On Axis"].tolist() == [10, 20]
    assert spl["180°"].tolist() == [60, 61]
    assert "-180°" not in spl
    assert "20°" not in spl
    assert not spl.isna().any().any()


@pytest.mark.parametrize("rear", [180, -180])
def test_local_shared_directions_take_precedence(tmp_path, rear):
    for orientation, level in [("H", 80), ("V", 90)]:
        write_sweep(tmp_path, orientation, 0, level, False)
        write_sweep(tmp_path, orientation, rear, level - 10, False)

    for orientation, level in [("H", 80), ("V", 90)]:
        status, spl = parse_graph_spl_hv_txt(str(tmp_path), orientation, "none")
        assert status
        assert spl["On Axis"].tolist() == [level, level + 1]
        assert spl["180°"].tolist() == [level - 10, level - 9]


def test_positive_rear_endpoint_takes_precedence(tmp_path):
    write_sweep(tmp_path, "H", 0, 80, False)
    write_sweep(tmp_path, "H", -180, 60, False)
    write_sweep(tmp_path, "H", 180, 70, False)

    status, spl = parse_graph_spl_hv_txt(str(tmp_path), "H")

    assert status
    assert spl["180°"].tolist() == [70, 71]


def test_borrowed_negative_rear_does_not_disable_mirroring(tmp_path):
    write_sweep(tmp_path, "H", 0, 80, False)
    write_sweep(tmp_path, "H", 10, 70, False)
    write_sweep(tmp_path, "V", -180, 60, False)

    status, spl = parse_graph_spl_hv_txt(str(tmp_path), "H")

    assert status
    assert spl["10°"].equals(spl["-10°"])
    assert spl["180°"].tolist() == [60, 61]
