#!/usr/bin/env python3
"""Verify that a Git history rewrite removed only datas/pictures/."""

import argparse
import json
from pathlib import Path
import subprocess


def git(repository, *arguments):
    return subprocess.check_output(["git", "-C", str(repository), *arguments])


def tree(repository, commit):
    return {
        path: entry
        for record in git(repository, "ls-tree", "-rz", commit).split(b"\0")
        if record
        for entry, path in [record.split(b"\t", 1)]
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--commit-map", type=Path, required=True)
    args = parser.parse_args()
    mapping = dict(line.split() for line in args.commit_map.read_text().splitlines()[1:])
    if not mapping:
        raise SystemExit("No commits in the rewrite map")
    removed_blobs = set()
    for old, new in mapping.items():
        if new == "0" * 40:
            raise SystemExit(f"Commit was dropped: {old}")
        before = tree(args.before, old)
        expected = {
            path: entry for path, entry in before.items() if not path.startswith(b"datas/pictures/")
        }
        if tree(args.after, new) != expected:
            raise SystemExit(f"Unexpected tree change: {old} -> {new}")
        removed_blobs.update(
            entry.split()[2]
            for path, entry in before.items()
            if path.startswith(b"datas/pictures/")
        )
        old_parents = git(args.before, "show", "-s", "--format=%P", old).decode().split()
        new_parents = git(args.after, "show", "-s", "--format=%P", new).decode().split()
        if new_parents != [mapping[parent] for parent in old_parents]:
            raise SystemExit(f"Parent topology changed: {old}")
        metadata_format = "--format=%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%B"
        if git(args.before, "show", "-s", metadata_format, old) != git(
            args.after, "show", "-s", metadata_format, new
        ):
            raise SystemExit(f"Commit metadata changed: {old}")
    refs = (
        git(args.after, "for-each-ref", "--format=%(refname)", "refs/heads", "refs/tags")
        .decode()
        .split()
    )
    for ref in refs:
        for commit in git(args.after, "rev-list", ref).decode().split():
            if any(path.startswith(b"datas/pictures/") for path in tree(args.after, commit)):
                raise SystemExit(f"Pictures remain reachable from {ref}: {commit}")
    print(
        json.dumps(
            {
                "verified_rewritten_commits": len(mapping),
                "removed_unique_picture_blobs": len(removed_blobs),
                "verified_refs": refs,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
