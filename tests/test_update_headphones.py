"""Exercise the headphone update pipeline without building or deploying."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "update_headphones.sh"


class HeadphoneUpdateTests(unittest.TestCase):
    def run_update(self, args=(), fail="", locked=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copyfile(SCRIPT, root / SCRIPT.name)
            log = root / "commands.jsonl"
            binary_dir = root / "Rust target" / "release"
            command = (
                f"#!{sys.executable}\n"
                "import json, os, pathlib, sys\n"
                "name = pathlib.Path(sys.argv[0]).name\n"
                "with open(os.environ['HEADPHONE_TEST_LOG'], 'a') as output:\n"
                "    output.write(json.dumps([name, *sys.argv[1:]]) + '\\n')\n"
                "if name == os.environ.get('HEADPHONE_TEST_FAIL'):\n"
                "    sys.exit(9)\n"
            )
            for path in [
                root / "bin/make",
                root / "bin/python3.12",
                root / "bin/python",
                root / "scripts/check_html.sh",
                root / "scripts/update_dev.sh",
                root / "scripts/update_prod_ovh.sh",
                binary_dir / "generate-headphone-meta",
                binary_dir / "generate-headphone-graph",
                binary_dir / "generate-peq-headphone",
                root / "render images",
            ]:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(command)
                path.chmod(0o755)
            if locked:
                (root / ".cache/update_website.lock").mkdir(parents=True)
            env = dict(
                os.environ,
                PATH=f"{root / 'bin'}:{os.environ['PATH']}",
                THEPYTHON=str(root / "bin/python"),
                CARGO_TARGET_DIR=str(binary_dir.parent),
                SPINORAMA_RENDER_IMAGES=str(root / "render images"),
                HEADPHONE_TEST_LOG=str(log),
                HEADPHONE_TEST_FAIL=fail,
            )
            result = subprocess.run(
                ["bash", str(root / SCRIPT.name), *args],
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            calls = (
                [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            )
            return result, calls, (root / ".cache/update_website.lock").exists()

    def test_rust_eq_precedes_metadata_and_graphs(self):
        result, calls, locked = self.run_update(
            [
                "--headphone",
                "Example In Ear",
                "--force",
                "--jobs",
                "2",
                "--skip-build",
                "--skip-deploy",
                "--skip-catalogue-refresh",
                "--skip-check-html",
            ]
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        binaries = [call for call in calls if call[0].startswith("generate-")]
        self.assertEqual(
            [call[0] for call in binaries],
            [
                "generate-peq-headphone",
                "generate-headphone-meta",
                "generate-headphone-graph",
            ],
        )
        eq, meta, graph = binaries
        self.assertEqual(eq[eq.index("--only") + 1], "Example In Ear")
        self.assertEqual(graph[graph.index("--speaker") + 1], "Example In Ear")
        for call in (eq, graph):
            self.assertIn("--force", call)
            self.assertEqual(call[call.index("--jobs") + 1], "2")
        self.assertNotIn("--headphone", meta, "selected rebuild must publish the full catalogue")
        self.assertFalse(
            any(call[0] in ("make", "update_dev.sh", "update_prod_ovh.sh") for call in calls)
        )
        self.assertFalse(
            any(
                "generate_graphs.py" in arg or "generate_headphone_meta.py" in arg
                for call in calls
                for arg in call
            )
        )
        self.assertFalse(locked)

    def test_builds_then_updates_and_deploys(self):
        result, calls, _ = self.run_update()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls[0], ["make", "generate-build", "renderer-build"])
        self.assertTrue(any("./scripts/generate_headphone_datas.py" in call for call in calls))
        self.assertEqual(calls[-1], ["update_dev.sh"])

    def test_eq_failure_stops_before_graphs_and_deploy(self):
        result, calls, locked = self.run_update(
            ["--skip-build", "--skip-catalogue-refresh"], fail="generate-peq-headphone"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(
            any(call[0] in ("generate-headphone-graph", "update_dev.sh") for call in calls)
        )
        self.assertFalse(locked)

    def test_missing_option_value_fails_before_running_steps(self):
        result, calls, _ = self.run_update(["--headphone"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])

    def test_shared_update_lock_prevents_overlapping_runs(self):
        result, calls, locked = self.run_update(locked=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])
        self.assertTrue(locked)
