#!/usr/bin/env python3
"""Tests of scripts/release-version.py on a scratch tree (#428, #485). Run from the repo root."""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "release-version.py"
_spec = importlib.util.spec_from_file_location("release_version", SCRIPT)
rv = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rv)
CARGO = '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1" }\n'
TAURI = '{\n  "productName": "crank",\n  "version": "0.1.0",\n  "app": { "version": "9" }\n}\n'
PLIST = '<plist version="1.0"><dict>\n<key>CFBundleShortVersionString</key><string>0.1.0</string>\n<key>CFBundleVersion</key><string>10099</string>\n<key>LSMinimumSystemVersion</key><string>13.0</string></dict></plist>\n'
PKG = '{\n  "name": "x",\n  "version": "0.1.0",\n  "dependencies": { "a": "^1" }\n}\n'
CHANGELOG = "# Changelog\n\n## Unreleased\n\n### Added\n\n- thing (#1)\n\n### Fixed\n\n## [0.0.9](u) - 2026-01-01\n\n### Added\n\n- old\n"


def run(root, *args):
    p = subprocess.run([sys.executable, "-I", str(SCRIPT), "--root", str(root), *args], capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


class Parse(unittest.TestCase):
    def test_final_and_rc(self):
        self.assertEqual(rv.parse("1.2.3"), rv.Version(1, 2, 3, None))
        self.assertEqual(rv.parse("0.2.0-rc.4"), rv.Version(0, 2, 0, 4))
        self.assertEqual(str(rv.parse("0.2.0-rc.4")), "0.2.0-rc.4")
        self.assertEqual(rv.parse("0.2.0-rc.4").short(), "0.2.0")

    def test_bundle_version_mapping(self):
        for text, bundle in (("0.1.0", "10099"), ("0.2.0-rc.1", "20001"), ("0.2.0-rc.2", "20002"), ("0.2.0", "20099"),
                             ("1.2.3-rc.98", "1020398"), ("1.0.0", "1000099"), ("12.99.99", "12999999")):
            with self.subTest(text=text):
                self.assertEqual(rv.parse(text).bundle(), bundle)

    def test_bundle_version_orders_like_semver(self):
        ordered = ["0.1.0", "0.1.1-rc.1", "0.1.1-rc.2", "0.1.1-rc.10", "0.1.1", "0.2.0-rc.1", "0.2.0", "0.99.99",
                   "1.0.0-rc.1", "1.0.0", "2.0.0-rc.98", "2.0.0"]
        parsed = [rv.parse(t) for t in ordered]
        self.assertEqual(sorted(parsed, key=rv.Version.order), parsed)
        bundles = [int(v.bundle()) for v in parsed]
        self.assertEqual(bundles, sorted(bundles))
        self.assertEqual(len(set(bundles)), len(bundles))

    def test_malformed_versions_are_refused(self):
        for text in ("", "1.2", "1.2.3.4", "v1.2.3", "01.2.3", "1.02.3", "1.2.3 ", "1.2.3-rc", "1.2.3-rc.0", "1.2.3-rc.01",
                     "1.2.3-RC.1", "1.2.3-beta.1", "1.2.3-rc.1.1", "1.2.3+build.5", "1.100.0", "1.2.100", "1.2.3-rc.99",
                     "0.1.1\u0663", "\u0660.1.0", "0.1.0-rc.\u0661", "0.1.0\n", "0.1.0-rc.1\n", "\n0.1.0"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                rv.parse(text)


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
        for rel, old, new in (("Cargo.toml", "0.1.0", "0.2.0"), ("app/src-tauri/tauri.conf.json", "0.1.0", "0.2.0"),
                              ("app/package.json", "0.1.0", "0.2.0"), ("app/src-tauri/Info.plist", "0.1.0", "0.2.0"),
                              ("app/src-tauri/Info.plist", "10099", "0.1.0"), ("app/src-tauri/Info.plist", "10099", "10001")):
            with self.subTest(rel=rel, new=new):
                path = self.root / rel
                good = path.read_text()
                path.write_text(good.replace(old, new, 1))
                code, _, err = run(self.root, "check")
                self.assertEqual(code, 1)
                self.assertIn(rel, err)
                path.write_text(good)

    def test_a_malformed_cargo_version_fails_check(self):
        path = self.root / "Cargo.toml"
        path.write_text(CARGO.replace('version = "0.1.0"', 'version = "0.1"'))
        code, _, err = run(self.root, "check")
        self.assertEqual(code, 1)
        self.assertIn("Cargo.toml", err)

    def test_version_and_parse_print(self):
        self.assertEqual(run(self.root, "version")[1], "0.1.0\n")
        code, out, _ = run(self.root, "parse", "0.2.0-rc.3")
        self.assertEqual((code, out), (0, "version=0.2.0-rc.3\nkind=rc\nshort=0.2.0\nbundle=20003\n"))
        self.assertEqual(run(self.root, "parse", "0.2.0")[1], "version=0.2.0\nkind=final\nshort=0.2.0\nbundle=20099\n")
        self.assertNotEqual(run(self.root, "parse", "0.2.0-rc.0")[0], 0)

    def test_bump_edits_only_the_version_keys(self):
        self.assertEqual(run(self.root, "bump", "1.2.3")[0], 0)
        self.assertEqual((self.root / "Cargo.toml").read_text(), CARGO.replace('version = "0.1.0"', 'version = "1.2.3"'))
        self.assertEqual((self.root / "app/src-tauri/tauri.conf.json").read_text(), TAURI.replace('"0.1.0"', '"1.2.3"'))
        self.assertEqual((self.root / "app/package.json").read_text(), PKG.replace("0.1.0", "1.2.3"))
        self.assertEqual((self.root / "app/src-tauri/Info.plist").read_text(), PLIST.replace("0.1.0", "1.2.3").replace("10099", "1020399"))
        self.assertIn("7.7.7", (self.root / "node_modules/z/package.json").read_text())

    def test_bump_to_an_rc_maps_the_bundle_versions(self):
        self.assertEqual(run(self.root, "bump", "0.2.0-rc.1")[0], 0)
        plist = (self.root / "app/src-tauri/Info.plist").read_text()
        self.assertIn("<key>CFBundleShortVersionString</key><string>0.2.0</string>", plist)
        self.assertIn("<key>CFBundleVersion</key><string>20001</string>", plist)
        self.assertIn('version = "0.2.0-rc.1"', (self.root / "Cargo.toml").read_text())
        self.assertIn('"version": "0.2.0-rc.1"', (self.root / "app/package.json").read_text())
        self.assertEqual(run(self.root, "version")[1], "0.2.0-rc.1\n")
        self.assertEqual(run(self.root, "bump", "0.2.0-rc.2")[0], 0)
        self.assertEqual(run(self.root, "bump", "0.2.0")[0], 0)
        self.assertIn("<key>CFBundleVersion</key><string>20099</string>", (self.root / "app/src-tauri/Info.plist").read_text())

    def test_bad_version_is_refused(self):
        for bad in ("1.2", "1.2.3-rc.0", "1.2.3-beta.1", "1.100.0", "0.1.1\u0663", "0.2.0\n"):
            with self.subTest(bad=bad):
                self.assertNotEqual(run(self.root, "bump", bad)[0], 0)
                self.assertEqual(run(self.root, "version")[1], "0.1.0\n")

    def test_bump_refuses_a_version_not_later_than_the_current(self):
        for old in ("0.1.0", "0.0.9", "0.1.0-rc.1"):
            with self.subTest(old=old):
                code, _, err = run(self.root, "bump", old)
                self.assertNotEqual(code, 0)
                self.assertIn("not later", err)
                self.assertEqual(run(self.root, "version")[1], "0.1.0\n")

    def test_freeze_moves_unreleased_and_notes_reads_it(self):
        self.assertEqual(run(self.root, "freeze", "1.0.0", "2026-11-02")[0], 0)
        text = (self.root / "CHANGELOG.md").read_text()
        self.assertRegex(text, r"(?s)## Unreleased\n\n### Added\n\n### Changed\n.*### Security\n\n## \[1\.0\.0\]\(https://github\.com/gerchowl/crank/releases/tag/1\.0\.0\) - 2026-11-02\n\n### Added\n\n- thing \(#1\)\n\n## \[0\.0\.9\]")  # the empty "Fixed" subsection is dropped
        self.assertLess(text.index("## [1.0.0]"), text.index("## [0.0.9]"))
        code, out, _ = run(self.root, "notes", "1.0.0")
        self.assertEqual(code, 0)
        self.assertIn("- thing (#1)", out)
        self.assertNotIn("old", out)

    def test_freeze_refuses_an_rc_and_notes_of_an_rc_read_unreleased(self):
        code, _, err = run(self.root, "freeze", "1.0.0-rc.1", "2026-11-02")
        self.assertNotEqual(code, 0)
        self.assertIn("release candidate", err)
        self.assertEqual((self.root / "CHANGELOG.md").read_text(), CHANGELOG)
        code, out, _ = run(self.root, "notes", "1.0.0-rc.2")
        self.assertEqual(code, 0)
        self.assertIn("Release candidate 2 of 1.0.0", out)
        self.assertIn("- thing (#1)", out)
        self.assertNotIn("### Fixed", out)  # empty subsection dropped
        self.assertNotIn("old", out)

    def test_freeze_refuses_an_empty_unreleased_and_a_repeat(self):
        run(self.root, "freeze", "1.0.0", "2026-11-02")
        self.assertNotEqual(run(self.root, "freeze", "1.0.1", "2026-11-03")[0], 0)
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)
        self.assertNotEqual(run(self.root, "freeze", "0.0.9", "2026-11-03")[0], 0)


if __name__ == "__main__":
    unittest.main()
