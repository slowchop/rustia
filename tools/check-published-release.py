#!/usr/bin/env python3
"""Offline regression tests for the registry verification gate."""

from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import runpy
import tarfile
import unittest


VERIFIER = runpy.run_path(Path(__file__).with_name("verify-published-release.py"))
CRATES = VERIFIER["CRATES"]
VERIFY = VERIFIER["verify_published_release"]
VERSION = "0.2.0"
COMMIT = "a" * 40


class PublishedReleaseTests(unittest.TestCase):
    def verify(self, mutate_last=None):
        fixtures = {}
        inspected = []
        for crate in CRATES:
            stem = f"{crate}-{VERSION}"
            git = {"sha1": COMMIT}
            entry = {"name": crate, "vers": VERSION, "yanked": False}
            filename = f"{stem}/.cargo_vcs_info.json"
            if mutate_last and crate == CRATES[-1]:
                filename = mutate_last(entry, git, filename) or filename
            metadata = json.dumps({"git": git}).encode()
            buffer = io.BytesIO()
            with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
                member = tarfile.TarInfo(filename)
                member.size = len(metadata)
                tar.addfile(member, io.BytesIO(metadata))
            entry.setdefault("cksum", hashlib.sha256(buffer.getvalue()).hexdigest())
            fixtures[f"https://index.crates.io/{crate[:2]}/{crate[2:4]}/{crate}"] = json.dumps(entry).encode()
            fixtures[f"https://static.crates.io/crates/{crate}/{stem}.crate"] = buffer.getvalue()

        def fetch(url):
            inspected.append(url)
            return fixtures[url]

        with redirect_stdout(io.StringIO()):
            VERIFY(VERSION, COMMIT, fetch)
        self.assertEqual(len(inspected), 2 * len(CRATES))

    def test_all_six_matching_archives_pass(self):
        self.verify()

    def test_bad_last_crate_blocks_release(self):
        for problem, mutate in [
            ("commit", lambda entry, git, name: git.update(sha1="b" * 40)),
            ("clean commit", lambda entry, git, name: git.update(dirty=True)),
            ("checksum", lambda entry, git, name: entry.update(cksum="0" * 64)),
            ("yanked", lambda entry, git, name: entry.update(yanked=True)),
            ("registry index", lambda entry, git, name: entry.update(vers="0.1.0")),
            ("vcs_info", lambda entry, git, name: "missing-git-metadata.json"),
        ]:
            with self.subTest(problem=problem), self.assertRaisesRegex(ValueError, f"rustia-cli.*{problem}"):
                self.verify(mutate)

    def test_inputs_are_checked_before_fetching(self):
        for version, commit in [("v0.2.0", COMMIT), ("0.2.0\n", COMMIT), (VERSION, "main"), (VERSION, COMMIT[:7])]:
            with self.subTest(version=version, commit=commit), self.assertRaises(ValueError):
                VERIFY(version, commit, lambda url: self.fail(f"unexpected fetch: {url}"))


if __name__ == "__main__":
    unittest.main()
