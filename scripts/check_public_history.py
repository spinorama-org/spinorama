#!/usr/bin/env python3
"""Inventory unapproved measurement blobs reachable from Git refs, without values.

Use a full clone with fetched branch and pull-request refs. This checker cannot
inspect unfetched refs, forks, host caches, or third-party clones.
"""

import argparse
import json
import re
import subprocess
import shutil
from pathlib import Path

# ruff: noqa: S603 - Git receives argument arrays with no shell expansion.

POLICY = json.loads(
    (Path(__file__).resolve().parents[1] / "conf/public-data-policy.json").read_text()
)
APPROVED_BLOBS = POLICY["approved_source_blobs"]

CONFIDENTIAL_PATH = re.compile(
    r"^(?:datas/inroom/.*\.(?:txt|csv)|manual/data/template Spinorama\.tar|"
    r"crates/spinorama-generate/tests/fixtures/.*\.bundle\.json|datas/private-support/.*)$"
)


def inventory(refs):
    result = subprocess.run(
        [
            shutil.which("git"),
            "rev-list",
            "--objects",
            *refs,
            "--",
            "datas/inroom",
            "manual/data/template Spinorama.tar",
            "crates/spinorama-generate/tests/fixtures",
            "datas/private-support",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    exposed = []
    for line in result.stdout.splitlines():
        oid, _, path = line.partition(" ")
        if CONFIDENTIAL_PATH.fullmatch(path) and APPROVED_BLOBS.get(path) != oid:
            size = subprocess.run(
                [shutil.which("git"), "cat-file", "-s", oid],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            exposed.append({"oid": oid, "bytes": int(size), "path": path})
    return sorted(exposed, key=lambda item: (item["path"], item["oid"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("refs", nargs="*", default=["--all"])
    args = parser.parse_args()
    exposed = inventory(args.refs)
    print(json.dumps({"refs": args.refs, "exposed_blobs": exposed}, indent=2))
    return int(bool(exposed))


if __name__ == "__main__":
    raise SystemExit(main())
