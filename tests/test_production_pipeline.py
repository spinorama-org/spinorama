"""Run the actual release scripts with isolated commands and synthetic files."""

# ruff: noqa: S603 - Subprocesses execute test-owned release scripts and shims.

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]


def executable(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)


@pytest.mark.parametrize(
    "failure", ["", "generate-headphone-graph", "generate-peq-headphone", "missing-eq"]
)
def test_full_website_requires_headphone_outputs_and_orders_scores(tmp_path, failure):
    shutil.copy2(ROOT / "update_website.sh", tmp_path)
    log = tmp_path / "commands.jsonl"
    shim = (
        f"#!{sys.executable}\n"
        """import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['PIPELINE_LOG'], 'a') as out: out.write(json.dumps([name, *sys.argv[1:]]) + '\\n')
if name == os.environ['PIPELINE_FAIL']: sys.exit(42)
if name == 'generate-peq-headphone': pathlib.Path('score-ready').touch()
if name == 'generate-headphone-meta' and not pathlib.Path('score-ready').exists(): sys.exit(43)
"""
    )
    bins = tmp_path / "target/release"
    for name in [
        "generate-graph",
        "generate-meta",
        "generate-radar",
        "generate-eq-compare",
        "generate-headphone-meta",
        "generate-headphone-graph",
        "generate-peq-headphone",
    ]:
        if failure != "missing-eq" or name != "generate-peq-headphone":
            executable(bins / name, shim)
    for path in [
        ".venv/bin/python",
        "renderer",
        "scripts/update_pictures.sh",
        "scripts/check_html.sh",
        "scripts/update_dev.sh",
        "scripts/update_prod_ovh.sh",
        "scripts/update_brands.sh",
        "scripts/update_reviewers.sh",
        "commands/quarto",
    ]:
        executable(tmp_path / path, shim)
    result = subprocess.run(
        [shutil.which("bash"), "update_website.sh", "--skip-build"],
        cwd=tmp_path,
        env=dict(
            os.environ,
            PATH=f"{tmp_path / 'commands'}:{os.environ['PATH']}",
            PIPELINE_LOG=str(log),
            PIPELINE_FAIL=failure,
            CARGO_TARGET_DIR=str(bins.parent),
            SPINORAMA_RENDER_IMAGES=str(tmp_path / "renderer"),
        ),
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    calls = [json.loads(line) for line in log.read_text().splitlines()]
    names = [call[0] for call in calls]
    if failure:
        assert result.returncode != 0
        assert "Update complete" not in result.stdout
        assert "update_dev.sh" not in names
    else:
        assert result.returncode == 0, result.stdout + result.stderr
        assert (
            names.index("generate-peq-headphone")
            < names.index("generate-headphone-meta")
            < names.index("generate-headphone-graph")
        )
        assert names[-1] == "update_dev.sh"


@pytest.mark.parametrize("failure", ["ssh", "rsync", "optional-transfer", "manifest", ""])
def test_api_staging_propagates_transport_and_manifest_failures(tmp_path, failure):
    for tree in ["scripts", "conf/etc"]:
        shutil.copytree(ROOT / tree, tmp_path / tree)
    for name in [
        "metadata.json",
        "headphone_metadata.json",
        "eqdata.json",
        "headphone_eqdata.json",
    ]:
        path = tmp_path / "dist/json" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("{}")
    for folder in ["datas/headphones", "dist-private/speakers", "dist-private/headphones"]:
        path = tmp_path / folder
        path.mkdir(parents=True)
        (path / "synthetic.txt").write_text("synthetic")
    shutil.copy2(ROOT / "datas/origins.toml", tmp_path / "datas/origins.toml")
    shim = (
        f"#!{sys.executable}\n"
        """import os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
failure = os.environ['STAGING_FAIL']
if failure == name: sys.exit(42)
if failure == 'manifest' and name == 'ssh' and 'sha256sum -c' in ' '.join(sys.argv): sys.exit(1)
if failure == 'optional-transfer' and name == 'rsync' and './dist-private/headphones/' in sys.argv: sys.exit(23)
"""
    )
    for name in ["ssh", "rsync"]:
        executable(tmp_path / "commands" / name, shim)
    result = subprocess.run(
        [shutil.which("bash"), "scripts/update_api_ovh.sh"],
        cwd=tmp_path,
        env=dict(
            os.environ, PATH=f"{tmp_path / 'commands'}:{os.environ['PATH']}", STAGING_FAIL=failure
        ),
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert (result.returncode == 0) == (not failure), result.stdout + result.stderr
    assert ("1. Deploy the Rust API binary" in result.stdout) == (not failure)


@pytest.mark.parametrize("tampered", [False, True])
def test_public_sync_deletes_removed_top_level_entries(tmp_path, tampered):
    real_rsync = shutil.which("rsync")
    if not real_rsync:
        pytest.skip("rsync unavailable")
    for name in ["update_prod_ovh.sh", "check_public_dist.py"]:
        path = tmp_path / "scripts" / name
        path.parent.mkdir(exist_ok=True)
        shutil.copy2(ROOT / "scripts" / name, path)
    (tmp_path / "conf").mkdir()
    shutil.copy2(ROOT / "conf/public-data-policy.json", tmp_path / "conf/public-data-policy.json")
    (tmp_path / "dist/js").mkdir(parents=True)
    (tmp_path / "dist/index.html").write_text("synthetic public page")
    (tmp_path / "dist/js/meta-test.js").write_text("/* synthetic */")
    remote = tmp_path / "remote"
    (remote / "obsolete").mkdir(parents=True)
    (remote / "obsolete/old.json").write_text("private stale data")
    (remote / ".stale").write_text("stale")
    executable(
        tmp_path / "commands/rsync",
        f"#!{sys.executable}\n"
        """import os, sys, pathlib
args = sys.argv[1:]
args[-1] = os.environ['SYNC_DEST'] + '/'
if '--itemize-changes' in args and os.environ['SYNC_TAMPER'] == '1':
    target = pathlib.Path(os.environ['SYNC_DEST'], 'index.html')
    previous = target.stat()
    target.write_text('x' * len(target.read_text()))
    os.utime(target, ns=(previous.st_atime_ns, previous.st_mtime_ns))
os.execv(os.environ['REAL_RSYNC'], [os.environ['REAL_RSYNC'], *args])
""",
    )
    result = subprocess.run(
        [shutil.which("sh"), "scripts/update_prod_ovh.sh"],
        cwd=tmp_path,
        env=dict(
            os.environ,
            PATH=f"{tmp_path / 'commands'}:{os.environ['PATH']}",
            SYNC_DEST=str(remote),
            REAL_RSYNC=real_rsync,
            SYNC_TAMPER="1" if tampered else "0",
        ),
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert (result.returncode == 0) == (not tampered), result.stdout + result.stderr
    assert ("destination contents verified" in result.stdout) == (not tampered)
    assert sorted(p.name for p in remote.iterdir()) == ["index.html", "js"]


@pytest.mark.parametrize("data_case", ["synthetic", "approved", "modified-approved"])
def test_history_check_finds_deleted_measurements(tmp_path, data_case):
    git = shutil.which("git")
    if not git:
        pytest.skip("git unavailable")

    def run(*args):
        return subprocess.run([git, *args], cwd=tmp_path, check=True, capture_output=True)

    run("init", "--quiet")
    run("config", "user.email", "synthetic@example.org")
    run("config", "user.name", "Synthetic Test")
    path = tmp_path / "datas/inroom/m001/L.txt"
    path.parent.mkdir(parents=True)
    if data_case == "synthetic":
        path.write_text("100 80\n200 81\n")
    else:
        path.write_bytes((ROOT / "datas/inroom/m001/L.txt").read_bytes())
    run("add", ".")
    run("commit", "--quiet", "-m", "Synthetic measurement")
    if data_case == "modified-approved":
        path.write_bytes(path.read_bytes() + b"changed")
        run("add", ".")
        run("commit", "--quiet", "-m", "Changed example requires approval")
    run("rm", "datas/inroom/m001/L.txt")
    run("commit", "--quiet", "-m", "Remove synthetic measurement")
    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts/check_public_history.py"), "HEAD"],
        cwd=tmp_path,
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == (0 if data_case == "approved" else 1)
    inventory = json.loads(result.stdout)["exposed_blobs"]
    if data_case == "approved":
        assert inventory == []
        return
    assert len(inventory) == 1
    assert inventory[0]["path"] == "datas/inroom/m001/L.txt"
    assert "100 80" not in result.stdout


@pytest.mark.parametrize("invalid", [False, True])
def test_dev_nginx_installs_all_policies_and_rolls_back_without_touching_bans(tmp_path, invalid):
    script = tmp_path / "scripts/deploy_nginx_dev.sh"
    script.parent.mkdir()
    shutil.copy2(ROOT / "scripts/deploy_nginx_dev.sh", script)
    shutil.copytree(ROOT / "conf/etc/nginx", tmp_path / "conf/etc/nginx")
    destination = tmp_path / "installed"
    relatives = [
        "sites-available/spinorama-dev",
        "snippets/spinorama-dev-cache.conf",
        "snippets/bot-protection-server.conf",
        "snippets/deny-repository-metadata.conf",
        "conf.d/bot-protection.conf",
        "snippets/spinorama-banned.conf",
    ]
    for relative in relatives:
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("original " + relative)
    stage = tmp_path / "stage"
    stage.mkdir()
    shim = (
        f"#!{sys.executable}\n"
        """import os, pathlib, shutil, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
args = [arg for arg in sys.argv[1:] if arg not in ['-o', 'BatchMode=yes', 'ConnectTimeout=10']]
if name == 'ssh':
    if args[1].startswith('mktemp '): print(os.environ['NGINX_STAGE'])
    elif args[1] == 'sh':
        script = sys.stdin.read().replace('/etc/nginx', os.environ['NGINX_DEST'])
        sys.exit(subprocess.run(['sh', '-s', '--', args[-1]], input=script, text=True).returncode)
    # Cleanup is owned by the pytest temporary directory.
elif name == 'scp':
    target = args[-1].split(':', 1)[1]
    for source in args[:-1]: shutil.copy2(source, target)
"""
    )
    for name in ["ssh", "scp"]:
        executable(tmp_path / "commands" / name, shim)
    executable(tmp_path / "commands/sudo", '#!/bin/sh\nshift\nexec "$@"\n')
    executable(tmp_path / "commands/nginx", '#!/bin/sh\nexit "$NGINX_STATUS"\n')
    executable(tmp_path / "commands/systemctl", "#!/bin/sh\nexit 0\n")
    result = subprocess.run(
        [shutil.which("sh"), str(script)],
        check=False,
        env=dict(
            os.environ,
            PATH=f"{tmp_path / 'commands'}:{os.environ['PATH']}",
            NGINX_STAGE=str(stage),
            NGINX_DEST=str(destination),
            NGINX_STATUS="42" if invalid else "0",
        ),
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert (result.returncode == 0) == (not invalid), result.stdout + result.stderr
    for relative in relatives:
        actual = (destination / relative).read_text()
        if invalid or relative.endswith("spinorama-banned.conf"):
            assert actual == "original " + relative
        else:
            assert actual == (ROOT / "conf/etc/nginx" / relative).read_text()


@pytest.mark.parametrize("conversion_fails", [False, True])
def test_picture_update_keeps_notebooks_and_archives_out_of_public_build(
    tmp_path, conversion_fails
):
    for folder in ["datas/icons", "datas/pictures", "manual/images", "dist/pictures"]:
        (tmp_path / folder).mkdir(parents=True)
    for name in ["public.svg", "icons.ipynb", "private.tar"]:
        (tmp_path / "datas/icons" / name).write_text("synthetic")
        (tmp_path / "dist/pictures" / name).write_text("stale copy")
    executable(tmp_path / "commands/uname", "#!/bin/sh\necho Linux\n")
    executable(tmp_path / "commands/convert", "#!/bin/sh\nexit 42\n")
    if conversion_fails:
        (tmp_path / "datas/pictures/Synthetic.png").write_text("synthetic source")
    result = subprocess.run(
        [shutil.which("sh"), str(ROOT / "scripts/update_pictures.sh")],
        cwd=tmp_path,
        env=dict(os.environ, PATH=f"{tmp_path / 'commands'}:{os.environ['PATH']}"),
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert (result.returncode == 0) == (not conversion_fails), result.stdout + result.stderr
    if conversion_fails:
        return
    assert sorted(p.name for p in (tmp_path / "dist/pictures").iterdir()) == ["public.svg"]
