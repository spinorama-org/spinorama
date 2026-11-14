"""Exercise environment activation without installing packages or changing SSH state."""

# ruff: noqa: S603 - subprocesses run test-owned scripts and resolved local shells.

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

pytestmark = pytest.mark.skipif(os.name == "nt", reason="env.sh targets Bash and Zsh on Unix")


@pytest.fixture
def checkout(tmp_path):
    root = tmp_path / "checkout with spaces"
    root.mkdir()
    shutil.copyfile(Path(__file__).parents[1] / "env.sh", root / "env.sh")
    (root / "requirements.txt").write_text("gpui-toolkit~=0.11.1\n")
    commands = tmp_path / "commands"
    commands.mkdir()
    stub = commands / "python3.12"
    stub.write_text(
        f"#!{sys.executable}\n"
        """import json, os, pathlib, shlex, shutil, sys
with open(os.environ["ENV_TEST_LOG"], "a") as stream:
    stream.write(json.dumps([pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]) + "\\n")
if sys.argv[1:3] == ["-m", "venv"]:
    target = pathlib.Path(sys.argv[3])
    (target / "bin").mkdir(parents=True)
    shutil.copyfile(__file__, target / "bin/python")
    (target / "bin/python").chmod(0o755)
    (target / "bin/activate").write_text("export VIRTUAL_ENV=" + shlex.quote(str(target)) + '\\nexport PATH="$VIRTUAL_ENV/bin:$PATH"\\n')
elif sys.argv[1:3] == ["-m", "pip"]:
    sys.exit(0)
elif pathlib.Path(sys.argv[0]).name == "npm":
    (pathlib.Path(os.environ["SPIN"]) / "node_modules").mkdir(exist_ok=True)
elif pathlib.Path(sys.argv[0]).name in ("ssh-agent", "ssh-add"):
    sys.exit(42)
elif os.environ.get("ENV_TEST_BAD_VERSION") and sys.argv[1] == "-c":
    sys.exit(1)
else:
    os.execv(sys.executable, [sys.executable, *sys.argv[1:]])
"""
    )
    stub.chmod(0o755)
    for name in ("npm", "ssh-agent", "ssh-add"):
        shutil.copyfile(stub, commands / name)
        (commands / name).chmod(0o755)
    env = os.environ.copy()
    env["PATH"] = f"{commands}:{env['PATH']}"
    env["ENV_TEST_LOG"] = str(tmp_path / "commands.jsonl")
    env["SSH_AUTH_SOCK"] = "existing-agent"
    env["PIP_NO_INDEX"] = "1"
    env["PIP_DISABLE_PIP_VERSION_CHECK"] = "1"
    return root, env


@pytest.mark.parametrize("shell", ["bash", "zsh"])
def test_source_environment_from_another_directory_twice(checkout, tmp_path, shell):
    executable = shutil.which(shell)
    if executable is None:
        pytest.skip(f"{shell} is not installed")
    root, env = checkout
    script = """
source "$1/env.sh" || exit
source "$1/env.sh" || exit
python -c 'import json, os; print(json.dumps({key: os.environ[key] for key in ("SPIN", "VIRTUAL_ENV", "PYTHONPATH", "PATH", "SSH_AUTH_SOCK")}))'
pwd
"""
    result = subprocess.run(
        [executable, "-f", "-c", script, "env-test", str(root)],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    lines = result.stdout.splitlines()
    values = json.loads(lines[-2])
    assert values["SPIN"] == str(root)
    assert values["VIRTUAL_ENV"] == str(root / ".venv")
    assert str(root / "scripts") in values["PYTHONPATH"].split(":")
    assert values["PATH"].split(":").count(str(root / "node_modules/.bin")) == 1
    assert values["SSH_AUTH_SOCK"] == "existing-agent"
    assert lines[-1] == str(tmp_path)
    commands = [json.loads(line) for line in Path(env["ENV_TEST_LOG"]).read_text().splitlines()]
    installs = [command for command in commands if command[1:3] == ["-m", "pip"]]
    assert (
        installs
        == [["python", "-m", "pip", "install", "--quiet", "-r", str(root / "requirements.txt")]] * 2
    )
    assert [command for command in commands if command[0] == "npm"] == [
        ["npm", "--prefix", str(root), "install"]
    ]
    assert not any(command[0].startswith("ssh-") for command in commands)


def test_incompatible_environment_is_preserved(checkout, tmp_path):
    shell = shutil.which("bash")
    if shell is None:
        pytest.skip("Bash is not installed")
    root, env = checkout
    subprocess.run(
        [str(tmp_path / "commands/python3.12"), "-m", "venv", str(root / ".venv")],
        env=env,
        check=True,
    )
    marker = root / ".venv/keep-me"
    marker.write_text("existing environment")
    env["ENV_TEST_BAD_VERSION"] = "1"
    result = subprocess.run(
        [shell, "-c", 'source "$1/env.sh"', "env-test", str(root)],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "must use Python 3.12" in result.stderr
    assert marker.read_text() == "existing environment"
