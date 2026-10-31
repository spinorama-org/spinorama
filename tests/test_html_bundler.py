#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# A library to display spinorama charts
#
# Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

from contextlib import chdir
import logging
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from mako.template import Template

import spinorama.constant_paths as cpaths

import generate_html
from generate_html import (
    CACHE_VERSION,
    SITEDEV,
    SITEPROD,
    WEBSITE_JS_FILES,
    adapt_imports,
    copy_if_different,
    get_files,
    get_versions,
)

def test_measurement_pages_follow_graph_origin_directory(tmp_path, monkeypatch):
    monkeypatch.setattr(cpaths, "CPATH_DIST_SPEAKERS", str(tmp_path))
    monkeypatch.setattr(generate_html, "logger", logging.getLogger(__name__), raising=False)
    monkeypatch.setattr(generate_html, "flag_optim", False, raising=False)
    monkeypatch.setattr(generate_html, "find_metadata_file", lambda: ("meta", "eq"))
    monkeypatch.setattr(generate_html, "find_metadata_chunks", lambda: {})
    monkeypatch.setattr(generate_html, "need_update", lambda *_: True)
    template = Template("test page")
    for origin in ("SpeakerLab", "Different Vendor"):
        generate_html.generate_measurement(
            {}, {"Test Speaker": {"brand": "Test"}}, "", True, {}, "Test Speaker",
            {}, template, template, origin, {}, "v1", {"CEA2034": object()},
        )
        assert (tmp_path / "Test Speaker" / origin / "index_v1.html").is_file()
        assert (tmp_path / "Test Speaker" / origin / "v1" / "CEA2034.html").is_file()


class JSImportTests(unittest.TestCase):
    def setUp(self):
        self.versions = get_versions("{}/update_3rdparties.sh".format(cpaths.CPATH_SCRIPTS))
        self.jsfiles = get_files(cpaths.CPATH_WEBSITE, "js")
        self.mini = ".min"

    def test_import_misc(self):
        code = "import { show } from './misc.js';"
        transformed = adapt_imports(code, self.versions, self.jsfiles, self.mini)
        self.assertIn("/js", transformed)

    def test_import_fuse(self):
        code = "import Fuse from 'fuse.js';"
        transformed = adapt_imports(code, self.versions, self.jsfiles, self.mini)
        self.assertIn("/js3rd", transformed)
        self.assertIn(self.versions["FUSE"], transformed)

    def test_import_plotly(self):
        code = "import Plotly from 'plotly.js-dist-min';"
        transformed = adapt_imports(code, self.versions, self.jsfiles, self.mini)
        self.assertNotIn("/js3rd", transformed)
        self.assertNotIn("/js", transformed)
        self.assertNotIn("Plotly", transformed)

    def test_import_multi(self):
        code = """
// import Fuse from 'fuse.js';

import {
    urlSite
} from './meta.js';
import {
    getMetadata,
    assignOptions,
    getAllSpeakers,
    getSpeakerData
} from './download.js';
import {
    knownMeasurements,
    setContour,
    setGlobe,
    setGraph,
    setCEA2034,
    setRadar,
    setSurface,
} from './plot.js';
        """

        minimized = adapt_imports(code, self.versions, self.jsfiles, self.mini)
        self.assertIn("/js3rd", minimized)
        self.assertIn("/js/download-{}.min.js".format(CACHE_VERSION), minimized)
        self.assertIn("/js/meta-{}.min.js".format(CACHE_VERSION), minimized)
        self.assertIn("/js/plot-{}.min.js".format(CACHE_VERSION), minimized)
        self.assertIn(self.versions["FUSE"], minimized)

        transformed = adapt_imports(code, self.versions, self.jsfiles, "")
        self.assertIn("/js3rd", transformed)
        self.assertIn("/js/download-{}.js".format(CACHE_VERSION), transformed)
        self.assertIn("/js/meta-{}.js".format(CACHE_VERSION), transformed)
        self.assertIn("/js/plot-{}.js".format(CACHE_VERSION), transformed)
        self.assertIn(self.versions["FUSE"], transformed)

    def test_meta_template_compiles(self):
        template = Template(filename=f"{cpaths.CPATH_WEBSITE}/meta.js")
        rendered = template.render(
            site="https://www.spinorama.org",
            metadata_filename_head="json/metadata.json",
            metadata_filename_chunks='["json/metadata-0.json"]',
            eqdata_filename="json/eqdata.json",
        )

        self.assertIn("startsWith('$' + '{')", rendered)
        self.assertIn('JSON.parse(metadataFilenameChunksValue)', rendered)

    def test_annotation_layout_is_bundled(self):
        self.assertIn("annotation-layout", WEBSITE_JS_FILES)

    def test_copy_if_different_preserves_mtime_for_identical_output(self):
        with tempfile.TemporaryDirectory() as temporary_dir:
            source = Path(temporary_dir) / "source.js"
            destination = Path(temporary_dir) / "destination.js"
            source.write_text("const value = 1;\n", encoding="utf-8")

            self.assertTrue(copy_if_different(str(source), str(destination)))
            stats = destination.stat()
            os.utime(
                destination,
                ns=(stats.st_atime_ns, stats.st_mtime_ns + 1_000_000),
            )
            unchanged_mtime = destination.stat().st_mtime_ns

            self.assertFalse(copy_if_different(str(source), str(destination)))
            self.assertEqual(destination.stat().st_mtime_ns, unchanged_mtime)

            source.write_text("const value = 2;\n", encoding="utf-8")
            self.assertTrue(copy_if_different(str(source), str(destination)))

    def test_cached_bundles_follow_site_changes(self):
        meta_source = Path(cpaths.CPATH_WEBSITE, "meta.js").resolve()
        versions_source = Path(cpaths.CPATH_SCRIPTS, "update_3rdparties.sh").resolve()

        # Exercise the optimized output path without requiring Node/terser in Python tests.
        def copy_minifier_input(command, *, stdout, **kwargs):
            stdout.write(Path(command[-1]).read_text(encoding="utf-8"))
            return subprocess.CompletedProcess(command, 0)

        for optimized in (False, True):
            with self.subTest(optimized=optimized), tempfile.TemporaryDirectory() as temporary_dir:
                root = Path(temporary_dir)
                for directory in ("src/website", "scripts", "build/website", "dist/json"):
                    (root / directory).mkdir(parents=True)
                shutil.copyfile(meta_source, root / "src/website/meta.js")
                shutil.copyfile(versions_source, root / "scripts/update_3rdparties.sh")
                (root / "src/website/misc.js").write_text("export const unchanged = true;\n")
                for name in ("metadata", "metadata-head", "metadata-0000", "eqdata"):
                    (root / "dist/json" / f"{name}.json").write_text("{}")
                for name in (
                    "index.html",
                    "eqs.html",
                    "compare.html",
                    "customization.html",
                    "scores.html",
                    "similar.html",
                    "statistics.html",
                    "robots.txt",
                    "sitemap.xml",
                    "spinorama.css",
                    "manifest.json",
                ):
                    (root / "src/website" / name).write_text("${site}")

                with (
                    chdir(root),
                    patch.multiple(
                        generate_html,
                        flag_optim=optimized,
                        skip_speakers=True,
                        site=SITEDEV,
                        logger=logging.getLogger(__name__),
                        WEBSITE_JS_FILES=("meta", "misc"),
                        create=True,
                    ),
                    patch.object(generate_html, "generate_annotations_wasm"),
                    patch.object(generate_html.subprocess, "run", side_effect=copy_minifier_input),
                ):
                    suffix = ".min" if optimized else ""
                    output = Path(f"dist/js/meta-{CACHE_VERSION}{suffix}.js")
                    static_output = Path(f"dist/js/misc-{CACHE_VERSION}{suffix}.js")
                    static_mtime = None
                    for site in (SITEDEV, SITEPROD, "http://localhost:8888", SITEPROD):
                        generate_html.site = site
                        with self.assertRaises(SystemExit) as exit_status:
                            generate_html.main()
                        self.assertEqual(exit_status.exception.code, 0)
                        self.assertIn(f"export const urlSite = '{site}' + '/';", output.read_text())
                        if static_mtime is None:
                            static_mtime = static_output.stat().st_mtime_ns
                        self.assertEqual(static_output.stat().st_mtime_ns, static_mtime)

                    unchanged_mtime = output.stat().st_mtime_ns
                    with self.assertRaises(SystemExit) as exit_status:
                        generate_html.main()
                    self.assertEqual(exit_status.exception.code, 0)
                    self.assertEqual(output.stat().st_mtime_ns, unchanged_mtime)


if __name__ == "__main__":
    unittest.main()
