#!/usr/bin/env python3
"""Tests of scripts/release-version.py on a scratch tree (#428). Run from the repo root."""
import contextlib
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "release-version.py"
CARGO = '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1" }\n'
TAURI = '{\n  "productName": "crank",\n  "version": "0.1.0",\n  "app": { "version": "9" }\n}\n'
PLIST = '<plist version="1.0"><dict>\n<key>CFBundleShortVersionString</key><string>0.1.0</string>\n<key>CFBundleVersion</key><string>0.1.0</string>\n<key>LSMinimumSystemVersion</key><string>13.0</string></dict></plist>\n'
PKG = '{\n  "name": "x",\n  "version": "0.1.0",\n  "dependencies": { "a": "^1" }\n}\n'
CHANGELOG = "# Changelog\n\n## Unreleased\n\n### Added\n\n- thing (#1)\n\n### Fixed\n\n## [0.0.9](u) - 2026-01-01\n\n### Added\n\n- old\n"


def run(root, *args):
    p = subprocess.run([sys.executable, "-I", str(SCRIPT), "--root", str(root), *args], capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


class ReleaseVersion(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        files = {"Cargo.toml": CARGO, "app/src-tauri/tauri.conf.json": TAURI, "app/package.json": PKG, "app/src-tauri/Info.plist": PLIST,
                 "package.json": '{\n  "name": "root"\n}\n', "node_modules/z/package.json": PKG.replace("0.1.0", "7.7.7"),
                 "CHANGELOG.md": CHANGELOG}
        for rel, text in files.items():
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            (self.root / rel).write_text(text)

    def tearDown(self):
        self.tmp.cleanup()

    def test_agreeing_sources_pass(self):
        self.assertEqual(run(self.root, "check")[0], 0)

    def test_each_source_mismatch_fails(self):
        for rel in ("Cargo.toml", "app/src-tauri/tauri.conf.json", "app/package.json", "app/src-tauri/Info.plist"):
            with self.subTest(rel=rel):
                path = self.root / rel
                good = path.read_text()
                path.write_text(good.replace("0.1.0", "0.2.0", 1))
                code, _, err = run(self.root, "check")
                self.assertEqual(code, 1)
                self.assertIn(rel, err)
                path.write_text(good)

    def test_bump_edits_only_the_version_keys(self):
        self.assertEqual(run(self.root, "bump", "1.2.3")[0], 0)
        self.assertEqual((self.root / "Cargo.toml").read_text(), CARGO.replace('version = "0.1.0"', 'version = "1.2.3"'))
        self.assertEqual((self.root / "app/src-tauri/tauri.conf.json").read_text(), TAURI.replace('"0.1.0"', '"1.2.3"'))
        self.assertEqual((self.root / "app/package.json").read_text(), PKG.replace("0.1.0", "1.2.3"))
        self.assertEqual((self.root / "app/src-tauri/Info.plist").read_text(), PLIST.replace("0.1.0", "1.2.3"))
        self.assertIn("7.7.7", (self.root / "node_modules/z/package.json").read_text())

    def test_bad_version_is_refused(self):
        self.assertNotEqual(run(self.root, "bump", "1.2")[0], 0)
        self.assertEqual(run(self.root, "check")[0], 0)

    def test_freeze_moves_unreleased_and_notes_reads_it(self):
        self.assertEqual(run(self.root, "freeze", "1.0.0", "2026-11-02")[0], 0)
        text = (self.root / "CHANGELOG.md").read_text()
        self.assertRegex(text, r"(?s)## Unreleased\n\n### Added\n\n### Changed\n.*### Security\n\n## \[1\.0\.0\]\(https://github\.com/gerchowl/crank/releases/tag/1\.0\.0\) - 2026-11-02\n\n### Added\n\n- thing \(#1\)\n\n## \[0\.0\.9\]")  # the empty "Fixed" subsection is dropped
        self.assertLess(text.index("## [1.0.0]"), text.index("## [0.0.9]"))
        code, out, _ = run(self.root, "notes", "1.0.0")
        self.assertEqual(code, 0)
        self.assertIn("- thing (#1)", out)
        self.assertNotIn("old", out)

    def test_freeze_refuses_an_empty_unreleased_and_a_repeat(self):
        run(self.root, "freeze", "1.0.0", "2026-11-02")
        self.assertNotEqual(run(self.root, "freeze", "1.0.1", "2026-11-03")[0], 0)
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)
        self.assertNotEqual(run(self.root, "freeze", "0.0.9", "2026-11-03")[0], 0)


if __name__ == "__main__":
    unittest.main()
