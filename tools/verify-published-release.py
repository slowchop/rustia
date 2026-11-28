#!/usr/bin/env python3
"""Verify the published crate set without using write credentials."""

import argparse
import hashlib
import io
import json
import re
import tarfile
from urllib.request import urlopen


CRATES = (
    "rustia-lexer",
    "rustia-parser",
    "rustia-vm",
    "rustia-compiler",
    "rustia",
    "rustia-cli",
)
NUMBER = r"(?:0|[1-9][0-9]*)"
IDENTIFIER = r"[0-9A-Za-z-]+"
VERSION = re.compile(
    rf"{NUMBER}\.{NUMBER}\.{NUMBER}"
    rf"(?:-(?P<prerelease>{IDENTIFIER}(?:\.{IDENTIFIER})*))?"
    rf"(?:\+{IDENTIFIER}(?:\.{IDENTIFIER})*)?"
)


def validate_inputs(version, commit):
    match = VERSION.fullmatch(version)
    if not match or any(
        part.isdigit() and len(part) > 1 and part.startswith("0")
        for part in (match.group("prerelease") or "").split(".")
    ):
        raise ValueError("version must be a SemVer version without the v prefix")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("commit must be a full lowercase Git SHA")


def download(url):
    with urlopen(url, timeout=20) as response:
        return response.read()


def verify_published_release(version, commit, fetch=download):
    validate_inputs(version, commit)
    for crate in CRATES:
        stem = f"{crate}-{version}"
        try:
            index = fetch(f"https://index.crates.io/{crate[:2]}/{crate[2:4]}/{crate}")
            entries = [json.loads(line) for line in index.splitlines()]
            records = [entry for entry in entries if entry["vers"] == version]
            if len(records) != 1 or records[0]["name"] != crate:
                raise ValueError("expected one matching registry index entry")
            if records[0]["yanked"] is not False:
                raise ValueError("version is yanked")
            archive = fetch(f"https://static.crates.io/crates/{crate}/{stem}.crate")
            if hashlib.sha256(archive).hexdigest() != records[0]["cksum"]:
                raise ValueError("archive checksum differs from the registry index")
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as package:
                member = package.getmember(f"{stem}/.cargo_vcs_info.json")
                if not member.isfile():
                    raise ValueError("Git metadata is not a regular file")
                with package.extractfile(member) as metadata:
                    git = json.load(metadata)["git"]
            if git["sha1"] != commit or git.get("dirty", False) is not False:
                raise ValueError("Git metadata does not identify the requested clean commit")
        except (KeyError, ValueError, OSError, tarfile.TarError) as error:
            raise ValueError(f"{stem}: {error}") from error
        print(f"Verified crates.io archive and Git metadata for {stem}: {commit}", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("commit")
    options = parser.parse_args()
    try:
        verify_published_release(options.version, options.commit)
    except ValueError as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
