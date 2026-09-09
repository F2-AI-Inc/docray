#!/usr/bin/env python3
"""Fail when a release tag does not match the Rust workspace version."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

RELEASE_TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+)$")


def verify_release_version(tag: str, manifest_path: Path) -> str:
    match = RELEASE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"release tag must be exactly vX.Y.Z; got {tag!r}")

    with manifest_path.open("rb") as manifest:
        workspace_version = tomllib.load(manifest)["workspace"]["package"]["version"]

    tag_version = match.group("version")
    if tag_version != workspace_version:
        raise ValueError(
            f"release tag {tag!r} does not match workspace package version "
            f"{workspace_version!r}"
        )
    return workspace_version


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag", help="Git release tag, exactly vX.Y.Z")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path("Cargo.toml"),
        help="workspace Cargo.toml (default: ./Cargo.toml)",
    )
    args = parser.parse_args()

    try:
        version = verify_release_version(args.tag, args.manifest)
    except (KeyError, OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"release version check failed: {error}", file=sys.stderr)
        return 1

    print(f"release tag {args.tag} matches workspace package version {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
