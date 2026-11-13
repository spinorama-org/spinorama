"""Public website trees must not contain private measurement data."""

import importlib.util
import pathlib
import tempfile
import unittest

_SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "check_public_dist.py"
_SPEC = importlib.util.spec_from_file_location("check_public_dist", _SCRIPT)
assert _SPEC is not None and _SPEC.loader is not None
check_public_dist = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(check_public_dist)


class PublicDistTests(unittest.TestCase):
    def test_rejects_graph_json_bundles_and_csv(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            private = [
                "speakers/Test Speaker/ASR/asr/CEA2034.json",
                "speakers/Test Speaker/ASR/asr/traces/CEA2034.json",
                "speakers/Test Speaker/ASR/asr/traces/CEA2034.webp",
                "headphones/Test/asr/frequency_response.csv",
                "headphones/Test/asr/Frequency Response.JSON.GZ",
                "unexpected/measurement.json",
            ]
            public = [
                "speakers/Test Speaker/ASR/asr/CEA2034.webp",
                "speakers/Test Speaker/ASR/asr/CEA2034.html",
                "speakers/Test Speaker/ASR/asr/measurement-license.txt",
                "json/metadata.json",
                "json/metadata.json.bz2",
                "manifest.json",
                "docs/search.json",
            ]
            for relative in private + public:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("{}\n", encoding="utf-8")

            found = {
                path.relative_to(root).as_posix()
                for path in check_public_dist.find_private_data(root)
            }
            self.assertEqual(found, set(private))
            self.assertEqual(check_public_dist.main([str(root)]), 1)

    def test_accepts_webp_and_catalogue_only_tree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for relative in (
                "speakers/Test Speaker/ASR/asr/CEA2034.webp",
                "json/headphone_metadata.json",
                "manifest.json",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("{}\n", encoding="utf-8")

            self.assertEqual(check_public_dist.find_private_data(root), [])
            self.assertEqual(check_public_dist.main([str(root)]), 0)

    def test_rejects_symlink_escaping_public_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            root = base / "dist"
            private = base / "private" / "CEA2034.json"
            private.parent.mkdir(parents=True)
            private.write_text("{}\n", encoding="utf-8")
            link = root / "speakers" / "Test Speaker" / "CEA2034.webp"
            link.parent.mkdir(parents=True)
            link.symlink_to(private)

            self.assertEqual(check_public_dist.find_private_data(root), [link])

    def test_missing_root_is_an_error(self) -> None:
        self.assertEqual(check_public_dist.main(["/definitely/missing"]), 2)


if __name__ == "__main__":
    unittest.main()
