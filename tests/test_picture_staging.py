"""Exercise picture staging with isolated sibling repositories."""

import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize("pictures_available", [False, True])
def test_picture_update_uses_sibling_web_images_and_warns_when_missing(
    tmp_path, pictures_available
):
    checkout = tmp_path / "spinorama"
    for folder in ["scripts", "datas/icons", "datas/pictures", "manual/images", "dist/pictures"]:
        (checkout / folder).mkdir(parents=True)
    script = checkout / "scripts/update_pictures.sh"
    shutil.copy2(ROOT / "scripts/update_pictures.sh", script)
    for name in ["public.svg", "icons.ipynb", "private.tar"]:
        (checkout / "datas/icons" / name).write_text("synthetic")
        (checkout / "dist/pictures" / name).write_text("stale copy")
    (checkout / "datas/pictures/Legacy.png").write_text("unused original")
    (checkout / "manual/images/diagram.svg").write_text("manual diagram")
    pictures = tmp_path / "spinorama-pictures/web"
    if pictures_available:
        (pictures / "nested folder").mkdir(parents=True)
        for name in ["Product photo.webp", "Product photo.jpg", "nested folder/Other.png"]:
            (pictures / name).write_bytes(b"prepared website image")
        (pictures / "private.tar").write_text("do not publish")
        (pictures / "notes.ipynb").write_text("do not publish")
        (checkout / "dist/pictures/Product photo.webp").write_text("stale image")
    result = subprocess.run(
        [shutil.which("sh"), str(script)],
        cwd=tmp_path,  # The source path is relative to the checkout, not the caller.
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    expected = ["public.svg"]
    if pictures_available:
        assert "WARNING" not in result.stderr
        for name in ["Product photo.webp", "Product photo.jpg", "nested folder/Other.png"]:
            assert (checkout / "dist/pictures" / name).read_bytes() == b"prepared website image"
        expected += ["Product photo.webp", "Product photo.jpg", "nested folder"]
    else:
        assert "WARNING: Website pictures are unavailable" in result.stderr
        assert "git clone https://github.com/spinorama-org/spinorama-pictures.git" in result.stderr
        assert str(tmp_path / "spinorama-pictures") in result.stderr
    assert sorted(p.name for p in (checkout / "dist/pictures").iterdir()) == sorted(expected)
    assert (checkout / "dist/docs/images/diagram.svg").read_text() == "manual diagram"


def test_picture_update_reports_copy_failure(tmp_path):
    checkout = tmp_path / "spinorama"
    for folder in ["scripts", "manual/images", "datas/icons", "dist/pictures"]:
        (checkout / folder).mkdir(parents=True)
    script = checkout / "scripts/update_pictures.sh"
    shutil.copy2(ROOT / "scripts/update_pictures.sh", script)
    pictures = tmp_path / "spinorama-pictures/web"
    pictures.mkdir(parents=True)
    (pictures / "Product.webp").write_text("website image")
    # A directory where the destination file belongs must fail the update.
    (checkout / "dist/pictures/Product.webp").mkdir()
    (checkout / "dist/pictures/Product.webp/Product.webp").mkdir()
    result = subprocess.run(
        [shutil.which("sh"), str(script)],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0, result.stdout + result.stderr
