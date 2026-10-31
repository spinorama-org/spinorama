#!/usr/bin/env python3
"""Regression tests for the EQ image stage cache."""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from autoeq.auto_plot import graph_eq_compare


sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "scripts"))

import generate_peqs


class TestSmoothingOptions(unittest.TestCase):
    def test_smoothing_order_reaches_result_plot(self):
        for extra_args, expected_order in (([], 3), (["--smooth-order=2"], 2)):
            with self.subTest(extra_args=extra_args):
                args = generate_peqs.get_argument_parser().parse_args(
                    [
                        "--speaker=Test speaker",
                        "--force",
                        "--min-freq=80",
                        "--target-min-freq=100",
                        "--optimisation=global",
                        "--smooth-measurements=5",
                        "--fitness=Score",
                        *extra_args,
                    ]
                )
                with (
                    patch.object(generate_peqs, "args", args, create=True),
                    patch.object(generate_peqs, "level", 30, create=True),
                    patch.object(generate_peqs, "logger", create=True),
                    patch.object(generate_peqs, "cache_load", return_value={}),
                    patch.object(generate_peqs, "compute_eqs", return_value=[]) as compute,
                    patch.object(generate_peqs, "compute_stats"),
                    self.assertRaises(SystemExit) as exit_result,
                ):
                    generate_peqs.main()
                self.assertEqual(exit_result.exception.code, 0)
                config = compute.call_args.args[1]
                self.assertEqual(config["smooth_order"], expected_order)
                config["curve_names"] = ["Listening Window"]
                freq = np.geomspace(20, 20000, 31)
                target = np.sin(np.log(freq))
                traces = graph_eq_compare(freq, [], [target], target, config)
                self.assertIn("smoothed LW", [trace.name for trace in traces])


class TestEqImageStageCache(unittest.TestCase):
    def test_requires_the_recorded_outputs_to_remain_present(self):
        with tempfile.TemporaryDirectory() as temporary_dir:
            root = Path(temporary_dir)
            output = root / "eq.json"
            output.write_text("{}", encoding="utf-8")
            manifest = root / "stage-manifest.json"
            manifest.write_text(
                json.dumps({"fingerprint": "current", "outputs": [str(output)]}),
                encoding="utf-8",
            )

            with (
                patch.object(generate_peqs, "EQ_IMAGE_STAGE_CACHE_MANIFEST", manifest),
                patch.object(generate_peqs, "eq_image_stage_fingerprint", return_value="current"),
            ):
                self.assertTrue(generate_peqs.eq_image_stage_cache_is_valid({}))
                output.unlink()
                self.assertFalse(generate_peqs.eq_image_stage_cache_is_valid({}))


if __name__ == "__main__":
    unittest.main()
