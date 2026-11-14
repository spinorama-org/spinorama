"""Exercise the Python batch runners through subprocess and filesystem boundaries."""

import json
from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import eqs_compute
import eqs_process
from eqs_common import catalogue, eq_digest, read_eq, write_manifest


def apo(score=6.0, baseline=4.0, filters=1):
    lines = [f"Preference Score {baseline:.2f} with EQ {score:.2f}", "Preamp: -2.0 dB"]
    lines.extend(
        f"Filter {index + 1}: ON PK Fc 1000 Hz Gain -2.0 dB Q 1.0" for index in range(filters)
    )
    return "\n".join(lines) + "\n"


def metadata(root, *, name="Example Speaker", skip=False):
    speaker = root / name
    speaker.mkdir(parents=True)
    (speaker / "meta.toml").write_text(f"""name = "{name}"
default_measurement = "lab"
[measurements.lab]
origin = "Lab"
format = "klippel"
skip = {str(skip).lower()}
""")
    source = speaker / "lab"
    source.mkdir()
    (source / "source.txt").write_text("measurement input")
    return speaker


def optimizer(path, *, fail=False, exit_code=0):
    path.write_text(f"""#!{sys.executable}
from pathlib import Path
import json, os, sys
args = sys.argv[1:]
output = Path(args[args.index('--output-dir') + 1])
with (output / 'calls.txt').open('a') as stream: stream.write('called\\n')
(output / 'invocation.json').write_text(json.dumps({{'args': args, 'threads': os.environ['RAYON_NUM_THREADS']}}))
(output / 'iir-autoeq.txt').write_text({("incomplete" if fail else apo())!r})
(output / 'conf-autoeq.json').write_text('{{}}')
sys.exit({exit_code})
""")
    path.chmod(0o755)
    return path


def test_compute_runs_isolated_cmaes_seeds_and_resumes_completed_outputs(tmp_path):
    catalogue_dir = tmp_path / "measurements"
    metadata(catalogue_dir)
    binary = optimizer(tmp_path / "generate-peq")
    output = tmp_path / "candidates"
    args = [
        "Example Speaker",
        "--metadata-dir",
        str(catalogue_dir),
        "--generate-peq",
        str(binary),
        "--output-dir",
        str(output),
        "--fitness",
        "Score",
        "--peq-counts",
        "3",
        "--starts",
        "2",
        "--jobs",
        "2",
    ]
    assert eqs_compute.main(args) == 0
    candidates = sorted(output.glob("Example Speaker/*"))
    assert len(candidates) == 2
    seeds = set()
    for candidate in candidates:
        invocation = json.loads((candidate / "invocation.json").read_text())
        command = invocation["args"]
        assert command[command.index("--algo") + 1] == "autoeq:cmaes"
        assert command[command.index("--max-iter") + 1] == "100000"
        assert invocation["threads"] == "1"
        seeds.add(command[command.index("--seed") + 1])
        assert json.loads((candidate / "run.json").read_text())["status"] == "complete"
    assert seeds == {"42", "43"}
    assert eqs_compute.main(args) == 0
    assert all((path / "calls.txt").read_text() == "called\n" for path in candidates)
    # A source change invalidates the saved candidate signature.
    (catalogue_dir / "Example Speaker/lab/source.txt").write_text("changed measurement input")
    assert eqs_compute.main(args) == 0
    assert all((path / "calls.txt").read_text() == "called\ncalled\n" for path in candidates)


@pytest.mark.parametrize("exit_code", [0, 7])
def test_compute_reports_failed_or_incomplete_output(tmp_path, exit_code):
    catalogue_dir = tmp_path / "measurements"
    metadata(catalogue_dir)
    binary = optimizer(tmp_path / "generate-peq", fail=True, exit_code=exit_code)
    output = tmp_path / "candidates"
    assert (
        eqs_compute.main(
            [
                "Example Speaker",
                "--metadata-dir",
                str(catalogue_dir),
                "--generate-peq",
                str(binary),
                "--output-dir",
                str(output),
                "--fitness",
                "Score",
                "--peq-counts",
                "3",
                "--starts",
                "1",
            ]
        )
        == 1
    )
    manifest = next(output.glob("*/*/run.json"))
    assert json.loads(manifest.read_text())["status"] == "failed"


def test_catalogue_discovers_all_defaults_and_honors_skips(tmp_path):
    metadata(tmp_path, name="A")
    metadata(tmp_path, name="B", skip=True)
    assert list(catalogue(tmp_path)) == ["A"]
    with pytest.raises(SystemExit) as error:
        eqs_compute.main(["Unknown", "--metadata-dir", str(tmp_path), "--dry-run"])
    assert error.value.code == 2


def candidate(root, configuration, score, *, filters=1, status="complete"):
    directory = root / configuration
    directory.mkdir(parents=True)
    output = directory / "iir-autoeq.txt"
    output.write_text(apo(score, filters=filters))
    (directory / "conf-autoeq.json").write_text(json.dumps({"configuration": configuration}))
    write_manifest(directory / "run.json", {"status": status, "eq_sha256": eq_digest(output)})
    return output


def test_process_selects_best_complete_candidate_and_keeps_better_existing_eq(tmp_path):
    inputs = tmp_path / "candidates/Example Speaker"
    candidate(inputs, "lower", 5.0)
    best = candidate(inputs, "best", 6.0)
    candidate(inputs, "failed", 9.0, status="failed")
    tampered = candidate(inputs, "tampered", 8.0)
    tampered.write_text(apo(10.0))
    output = tmp_path / "eq"
    assert eqs_process.process(inputs, output, dry_run=True) == "selected"
    assert not output.exists()
    assert eqs_process.process(inputs, output) == "updated"
    destination = output / "Example Speaker"
    assert (destination / "iir-autoeq.txt").read_text() == best.read_text()
    assert (destination / "iir.txt").read_text() == best.read_text()
    assert json.loads((destination / "conf-autoeq.json").read_text()) == {"configuration": "best"}
    assert eqs_process.process(inputs, output) == "skipped"
    (destination / "iir-autoeq.txt").write_text(apo(7.0))
    assert eqs_process.process(inputs, output) == "skipped"
    assert read_eq(destination / "iir-autoeq.txt").score == 7.0


def test_process_preserves_manual_active_eq_and_auto_alias(tmp_path):
    inputs = tmp_path / "candidates/Example Speaker"
    best = candidate(inputs, "best", 6.0)
    output = tmp_path / "eq"
    destination = output / "Example Speaker"
    destination.mkdir(parents=True)
    manual = destination / "iir.txt"
    manual.write_text("manual selection\n")
    assert eqs_process.process(inputs, output) == "updated"
    assert manual.read_text() == "manual selection\n"
    eqs_process.promote(read_eq(best), destination, activate=True)
    assert manual.read_text() == best.read_text()
    manual.unlink()
    manual.symlink_to("iir-autoeq.txt")
    eqs_process.promote(read_eq(best), destination)
    assert manual.is_symlink()
    assert manual.read_text() == best.read_text()


def test_process_rejects_candidates_that_degrade_the_uncorrected_score(tmp_path):
    inputs = tmp_path / "candidates/Example Speaker"
    candidate(inputs, "worse", 3.0)
    assert eqs_process.process(inputs, tmp_path / "eq") == "skipped"
    assert not (tmp_path / "eq").exists()


def test_score_headers_require_complete_finite_filters(tmp_path):
    path = tmp_path / "eq.txt"
    for text in [
        "Preference Score 4 with EQ nan",
        apo().replace("Q 1.0", "Q -1.0"),
        apo().replace("Gain -2.0", "Gain inf"),
    ]:
        path.write_text(text)
        with pytest.raises(ValueError):
            read_eq(path)
