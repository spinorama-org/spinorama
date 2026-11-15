"""Public website trees must not contain private measurement data."""

import bz2
import gzip
import json
import zipfile
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
                if relative.endswith(".bz2"):
                    path.write_bytes(bz2.compress(b"{}"))
                elif relative == "docs/search.json":
                    path.write_text("[]")
                elif relative.endswith(".txt") and relative in public:
                    path.write_text("Measurement license")
                else:
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

    def test_rejects_archives_compressed_data_and_unrecognized_catalogues(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            names = [
                "docs/data/template.tar",
                "json/private.bundle.json",
                "speakers/Test/response.txt",
                "speakers/Test/data.csv.gz",
                "json/metadata.json.br",
                "json/metadata.json.bz2",
            ]
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"100 80\n200 82\n")
            self.assertEqual(len(check_public_dist.find_private_data(root)), len(names))

    def test_checks_payload_schema_inside_every_supported_container(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            folder = root / "json"
            folder.mkdir()
            valid = {
                "Example": {
                    "brand": "Example",
                    "model": "Synthetic",
                    "measurements": {"test": {"origin": "Lab"}},
                }
            }
            for payload, count in [
                (valid, 0),
                (
                    {
                        "Example": {
                            **valid["Example"],
                            "measurements": {"test": {"x": [1, 2], "y": [80, 81]}},
                        }
                    },
                    4,
                ),
            ]:
                raw = json.dumps(payload).encode()
                (folder / "metadata.json").write_bytes(raw)
                (folder / "metadata.json.bz2").write_bytes(bz2.compress(raw))
                (folder / "metadata.json.gz").write_bytes(gzip.compress(raw))
                with zipfile.ZipFile(folder / "metadata.json.zip", "w") as archive:
                    archive.writestr("dist/json/metadata.json", raw)
                self.assertEqual(len(check_public_dist.find_private_data(root)), count)
            with zipfile.ZipFile(folder / "metadata.json.zip", "w") as archive:
                archive.writestr("metadata.json", json.dumps(valid))
                archive.writestr("hidden.csv", "100,80")
            self.assertIn(folder / "metadata.json.zip", check_public_dist.find_private_data(root))

    def test_accepts_only_the_exact_approved_documentation_archive(self):
        source = _SCRIPT.parents[1] / "manual/data/template Spinorama.tar"
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            archive = root / "docs/data/template Spinorama.tar"
            archive.parent.mkdir(parents=True)
            approved = source.read_bytes()
            archive.write_bytes(approved)
            self.assertEqual(check_public_dist.find_private_data(root), [])
            archive.write_bytes(approved + b"changed")
            self.assertEqual(check_public_dist.find_private_data(root), [archive])
            archive.write_bytes(approved)
            unapproved = archive.with_name("unapproved.tar")
            unapproved.write_bytes(approved)
            self.assertEqual(check_public_dist.find_private_data(root), [unapproved])

    def test_missing_root_is_an_error(self) -> None:
        self.assertEqual(check_public_dist.main(["/definitely/missing"]), 2)


if __name__ == "__main__":
    unittest.main()
