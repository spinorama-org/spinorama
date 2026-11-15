"""Check source staging and build failures without contacting deployment hosts."""

# ruff: noqa: S603 - subprocesses execute test-owned commands.

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest


@pytest.fixture
def deployment(tmp_path):
    root = tmp_path / "checkout"
    (root / "scripts").mkdir(parents=True)
    source = Path(__file__).parents[1]
    shutil.copy2(source / "scripts/deploy_binaries.sh", root / "scripts/deploy_binaries.sh")
    commands = tmp_path / "commands"
    commands.mkdir()
    remote = tmp_path / "remote"
    remote.mkdir()
    stub = commands / "ssh"
    stub.write_text(
        f"#!{sys.executable}\n"
        """import json, os, pathlib, shlex, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['DEPLOY_TEST_LOG'], 'a') as stream:
    stream.write(json.dumps([name, *sys.argv[1:]]) + '\\n')
if name == 'cargo':
    sys.exit(int(os.environ.get('DEPLOY_BUILD_STATUS', '0')))
if name == 'ssh':
    command = sys.argv[2]
    if 'cargo build' in command:
        command = command.replace('~/spinorama-workspace-build', shlex.quote(os.environ['DEPLOY_TEST_REMOTE']))
        sys.exit(subprocess.run(['sh', '-c', command]).returncode)
    if 'objdump' in command: print('GLIBC_2.39')
    elif 'ldd --version' in command: print('2.39')
"""
    )
    stub.chmod(0o755)
    for name in ("cargo", "rsync"):
        shutil.copy2(stub, commands / name)
    env = dict(
        os.environ,
        PATH=f"{commands}:{os.environ['PATH']}",
        DEPLOY_TEST_LOG=str(tmp_path / "commands.jsonl"),
        DEPLOY_TEST_REMOTE=str(remote),
    )
    return root, env


def run_deploy(deployment, target, status=0):
    root, env = deployment
    result = subprocess.run(
        [shutil.which("sh"), str(root / "scripts/deploy_binaries.sh"), target],
        env=dict(env, DEPLOY_BUILD_STATUS=str(status)),
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=10,
        check=False,
    )
    calls = [json.loads(line) for line in Path(env["DEPLOY_TEST_LOG"]).read_text().splitlines()]
    return result, calls


def test_stages_the_workspace_crates_before_building(deployment):
    result, calls = run_deploy(deployment, "dev")
    assert result.returncode == 0, result.stdout + result.stderr
    build = next(i for i, call in enumerate(calls) if call[0] == "cargo")
    assert any(
        call[0] == "rsync"
        and call[-2].endswith("/crates/")
        and call[-1].endswith(":spinorama-workspace-build/crates/")
        for call in calls[:build]
    )


@pytest.mark.parametrize("target", ["dev", "prod", "all"])
def test_failed_remote_build_stops_before_installing_or_checking_glibc(deployment, target):
    result, calls = run_deploy(deployment, target, status=42)
    assert result.returncode != 0
    assert "glibc check OK" not in result.stdout
    assert not any(
        "objdump" in arg or "mv -f" in arg or "cp ~" in arg for call in calls for arg in call
    )


@pytest.mark.parametrize("data_dir", ["/var/www/html/private", "relative/path"])
def test_dev_graph_data_refuses_public_or_relative_destinations(tmp_path, data_dir):
    script = Path(__file__).parents[1] / "scripts/update_graph_data_dev.sh"
    result = subprocess.run(
        [shutil.which("sh"), str(script)],
        env=dict(os.environ, SPINORAMA_DATA_DIR=data_dir),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2


def test_dev_graph_sync_copies_bundles_without_plotly_payloads(tmp_path):
    rsync = shutil.which("rsync")
    if rsync is None:
        pytest.skip("rsync is not installed")
    source = Path(__file__).parents[1]
    root = tmp_path / "checkout"
    (root / "scripts").mkdir(parents=True)
    for name in ("update_graph_data_dev.sh", "spinorama-api-start.sh"):
        shutil.copy2(source / "scripts" / name, root / "scripts" / name)
    graph = root / "dist-private/speakers/Example Speaker/Lab/test"
    (graph / "traces").mkdir(parents=True)
    (graph / "traces/CEA2034.json").write_text('{"traces": []}')
    (graph / "CEA2034.json").write_text('{"data": []}')
    assets = root / "dist/json"
    assets.mkdir(parents=True)
    (assets / "metadata.json").write_text('{"Example Speaker": {}}')
    (assets / "headphone.json").write_text("{}")
    private = tmp_path / "private-api"
    (private / "speakers").mkdir(parents=True)
    (private / "assets").mkdir()
    run_dir = tmp_path / "run"
    run_dir.mkdir()
    commands = tmp_path / "commands"
    commands.mkdir()
    shim = commands / "rsync"
    shim.write_text(
        f"#!{sys.executable}\n"
        "import os, sys\n"
        "args = [arg.replace('web01.spin:', '').replace('/home/spin/run/spin-api/', os.environ['DEV_TEST_RUN'] + '/') for arg in sys.argv[1:]]\n"
        "os.execv(os.environ['DEV_TEST_RSYNC'], [os.environ['DEV_TEST_RSYNC'], *args])\n"
    )
    shim.chmod(0o755)
    (commands / "ssh").write_text("#!/bin/sh\nexit 0\n")
    (commands / "ssh").chmod(0o755)
    result = subprocess.run(
        [shutil.which("sh"), str(root / "scripts/update_graph_data_dev.sh")],
        env=dict(
            os.environ,
            PATH=f"{commands}:{os.environ['PATH']}",
            SPINORAMA_DATA_DIR=str(private),
            DEV_TEST_RSYNC=rsync,
            DEV_TEST_RUN=str(run_dir),
            DEV_API_HOST="web01.spin",
        ),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert (private / "speakers/Example Speaker/Lab/test/traces/CEA2034.json").is_file()
    assert not (private / "speakers/Example Speaker/Lab/test/CEA2034.json").exists()
    assert (private / "assets/metadata.json").is_file()
    assert (private / "assets/headphone.json").is_file()
    assert (run_dir / "spinorama-api-start.sh").is_file()
