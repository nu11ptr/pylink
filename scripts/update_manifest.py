#!/usr/bin/env python3
"""Refresh the checked-in Astral distribution checksums using only Python's stdlib.

Example:
    python3 scripts/update_manifest.py --release 20260901 \
        --versions 3.14.7

Review the resulting diff and update the crate's default/version aliases separately.
This is a maintainer tool; Cargo builds never query GitHub's latest-release API.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


REPOSITORY = "astral-sh/python-build-standalone"
TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
)


def fetch(url):
    request = Request(url, headers={"User-Agent": "pylink-manifest-maintainer"})
    with urlopen(request, timeout=60) as response:
        return response.read()


def parse_checksums(contents):
    checksums = {}
    for number, line in enumerate(contents.decode("utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](\S+)", line)
        if match is None:
            raise ValueError(f"Invalid SHA256SUMS line {number}")
        digest, name = match.groups()
        if name in checksums:
            raise ValueError(f"Duplicate SHA256SUMS entry: {name}")
        checksums[name] = digest.lower()
    return checksums


def verify_api_digest(asset, digest):
    api_digest = asset.get("digest")
    if api_digest is not None and api_digest != f"sha256:{digest}":
        raise ValueError(f"GitHub API digest does not match SHA256SUMS: {asset['name']}")


def update(release, versions, output):
    metadata = json.loads(
        fetch(f"https://api.github.com/repos/{REPOSITORY}/releases/tags/{release}")
    )
    if metadata.get("tag_name") != release:
        raise ValueError("GitHub returned a different release than requested")
    if metadata.get("draft") or metadata.get("prerelease"):
        raise ValueError("Only published, non-prerelease Astral releases are supported")
    assets = {asset["name"]: asset for asset in metadata["assets"]}
    sums_asset = assets.get("SHA256SUMS")
    if sums_asset is None:
        raise ValueError(f"Release {release} has no SHA256SUMS asset")
    sums = fetch(sums_asset["browser_download_url"])
    verify_api_digest(sums_asset, hashlib.sha256(sums).hexdigest())
    checksums = parse_checksums(sums)

    rows = []
    for version in versions:
        for target in TARGETS:
            name = f"cpython-{version}+{release}-{target}-install_only_stripped.tar.gz"
            asset = assets.get(name)
            if asset is None or name not in checksums:
                raise ValueError(f"Missing release asset or checksum: {name}")
            digest = checksums[name]
            verify_api_digest(asset, digest)
            rows.append(f"{version}\t{release}\t{target}\t{digest}\n")

    # Validate every requested asset before changing the checked-in manifest.
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(rows), encoding="utf-8")
    print(f"Wrote {len(rows)} pinned distributions to {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", required=True, help="Exact Astral release, e.g. 20260901")
    parser.add_argument(
        "--versions", nargs="+", required=True, help="Exact CPython patch versions"
    )
    parser.add_argument(
        "--output", type=Path,
        default=Path(__file__).resolve().parents[1] / "build_support" / "distributions.tsv",
    )
    args = parser.parse_args()
    if not re.fullmatch(r"\d{8}", args.release):
        parser.error("--release must be an eight-digit Astral release date")
    if any(not re.fullmatch(r"3\.\d+\.\d+", version) for version in args.versions):
        parser.error("--versions must contain exact stable Python 3 versions, e.g. 3.14.7")
    if any(int(version.split(".")[1]) < 14 for version in args.versions):
        parser.error("pylink requires Python 3.14 or newer for the opaque PyInitConfig API")
    if len(set(args.versions)) != len(args.versions):
        parser.error("--versions must not contain duplicates")
    try:
        update(args.release, args.versions, args.output)
    except (HTTPError, URLError, OSError, ValueError, KeyError) as error:
        print(f"Could not update manifest: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
