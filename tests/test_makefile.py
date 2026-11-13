"""Exercise the image build pipeline without compiling or deploying anything."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


MAKEFILE = Path(__file__).resolve().parents[1] / "Makefile"


class ImageBuildTests(unittest.TestCase):
    def test_website_rebuild_does_not_require_annotation_bindings(self):
        result = subprocess.run(
            ["make", "--no-print-directory", "-n", "rebuild",
             "MATURIN=UNWANTED_MATURIN", "WASM_PACK=UNWANTED_WASM_PACK"],
            cwd=MAKEFILE.parent, capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("UNWANTED_MATURIN", result.stdout)
        self.assertNotIn("UNWANTED_WASM_PACK", result.stdout)
        self.assertIn("--render-webp", result.stdout)
        self.assertIn("scripts/generate_html.py", result.stdout)

    def run_images(self, fail_renderer=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "commands.jsonl"
            stub = root / "command"
            stub.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "with open(os.environ['MAKE_TEST_LOG'], 'a') as output:\n"
                "    output.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                "if os.environ.get('MAKE_TEST_FAIL') and 'gpui-spinorama' in sys.argv:\n"
                "    sys.exit(9)\n"
            )
            stub.chmod(0o755)
            target = root / "shared-target"
            generator = target / "release/generate-graph"
            generator.parent.mkdir(parents=True)
            generator.write_bytes(stub.read_bytes())
            generator.chmod(0o755)
            # An existing file must not suppress the image-generation command.
            (root / "images").touch()
            env = dict(os.environ, MAKE_TEST_LOG=str(log))
            env.pop("MAKEFLAGS", None)
            if fail_renderer:
                env["MAKE_TEST_FAIL"] = "1"
            result = subprocess.run(
                [
                    "make", "--no-print-directory", "-j2", "-f", str(MAKEFILE),
                    "images", f"CARGO={stub}", f"CARGO_TARGET_DIR={target}",
                    "GPUI_TOOLKIT_DIR=/tmp/toolkit with spaces",
                    "SPEAKER=Topping MA4", "VERSION=asr",
                ],
                cwd=root, env=env, capture_output=True, text=True, check=False,
            )
            commands = [json.loads(line) for line in log.read_text().splitlines()]
            return result, commands

    def test_images_rebuilds_both_tools_before_rendering(self):
        result, commands = self.run_images()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(commands), 3)
        builds, render = commands[:2], commands[2]
        self.assertEqual({cmd[cmd.index("-p") + 1] for cmd in builds},
                         {"spinorama-generate", "gpui-spinorama"})
        self.assertTrue(all("--release" in cmd for cmd in builds))
        self.assertEqual(render[render.index("--speaker") + 1], "Topping MA4")
        self.assertEqual(render[render.index("--mversion") + 1], "asr")
        self.assertEqual(render[render.index("--render-images-bin") + 1],
                         "/tmp/toolkit with spaces/target/release/spinorama-render-images")
        for flag in ["--render-png", "--render-webp", "--force"]:
            self.assertIn(flag, render)

    def test_failed_renderer_build_prevents_image_generation(self):
        result, commands = self.run_images(fail_renderer=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("--render-webp" in cmd for cmd in commands))
