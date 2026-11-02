#!/usr/bin/env python3
"""Version sources and changelog of a release (#428, RELEASING.md).

Usage: release-version.py [--root DIR] check
       release-version.py [--root DIR] bump X.Y.Z
       release-version.py [--root DIR] freeze X.Y.Z YYYY-MM-DD
       release-version.py [--root DIR] notes X.Y.Z

`check` fails unless every version source agrees: `[workspace.package] version` of Cargo.toml,
`version` of app/src-tauri/tauri.conf.json, both bundle versions of app/src-tauri/Info.plist, and `version` of every package.json that has one
(the `check` gate of CI). `bump` rewrites them in place with textual edits, so formatting and
key order are kept; Cargo.lock follows with `cargo update --workspace` (prepare-release.yml).
`freeze` moves the body of `## Unreleased` into a dated `## [X.Y.Z]` section and leaves a fresh
empty `## Unreleased` above it (devkit's changelog format). `notes` prints that section.
"""
import argparse
import json
import re
import sys
from pathlib import Path

SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
CARGO = re.compile(r'(\[workspace\.package\][^\[]*?^version\s*=\s*")([^"]+)(")', re.M | re.S)
PLIST = re.compile(r"(<key>CFBundle(?:Short)?Version(?:String)?</key><string>)([^<]+)(</string>)")
JSON = re.compile(r'^(\s{2}"version"\s*:\s*")([^"]+)(")', re.M)
SECTIONS = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]


def sources(root):
    """Yield (relative path, regex, text) for each file that carries a version (every match counts)."""
    cargo = root / "Cargo.toml"
    yield "Cargo.toml", CARGO, cargo.read_text()
    tauri = root / "app/src-tauri/tauri.conf.json"
    yield "app/src-tauri/tauri.conf.json", JSON, tauri.read_text()
    plist = root / "app/src-tauri/Info.plist"
    yield "app/src-tauri/Info.plist", PLIST, plist.read_text()
    for pkg in sorted(root.glob("**/package.json")):
        rel = pkg.relative_to(root)
        if "node_modules" in rel.parts:
            continue
        text = pkg.read_text()
        if "version" in json.loads(text):
            yield str(rel), JSON, text


def versions(root):
    """Map each version source to the version it declares."""
    out = {}
    for rel, rx, text in sources(root):
        found = [m.group(2) for m in rx.finditer(text)]
        if not found:
            raise SystemExit(f"{rel}: no version found")
        out[rel] = found[0] if len(set(found)) == 1 else " vs ".join(found)
    return out


def check(root):
    found = versions(root)
    if len(set(found.values())) == 1:
        print(f"version sources agree: {next(iter(found.values()))} ({len(found)} files)")
        return 0
    for rel, ver in found.items():
        print(f"{rel}: {ver}", file=sys.stderr)
    print("version sources disagree: bump them together with release-version.py bump", file=sys.stderr)
    return 1


def bump(root, version):
    for rel, rx, text in sources(root):
        (root / rel).write_text(rx.sub(lambda m: m.group(1) + version + m.group(3), text))
    return check(root)


def changelog_parts(text):
    """Split into (head, unreleased body, rest) around the `## Unreleased` section."""
    m = re.search(r"^## Unreleased[ \t]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    if not m:
        raise SystemExit("CHANGELOG.md has no '## Unreleased' section")
    return text[: m.start()], m.group(1), text[m.end():]


def has_content(body):
    return any(ln.strip() and not ln.startswith("### ") for ln in body.splitlines())


def freeze(root, version, date):
    path = root / "CHANGELOG.md"
    head, body, rest = changelog_parts(path.read_text())
    if not has_content(body):
        raise SystemExit("CHANGELOG.md: '## Unreleased' has no entries to release")
    if re.search(rf"^## \[{re.escape(version)}\]", rest, re.M):
        raise SystemExit(f"CHANGELOG.md already has a section for {version}")
    empty = "## Unreleased\n\n" + "".join(f"### {s}\n\n" for s in SECTIONS)
    body = re.sub(r"^### [^\n]*\n\s*(?=^### |\Z)", "", body.rstrip() + "\n", flags=re.M)  # drop empty subsections
    url = f"https://github.com/gerchowl/crank/releases/tag/{version}"
    path.write_text(f"{head}{empty}## [{version}]({url}) - {date}\n{body.rstrip()}\n\n{rest}")
    return 0


def notes(root, version):
    text = (root / "CHANGELOG.md").read_text()
    m = re.search(rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    if not m:
        raise SystemExit(f"CHANGELOG.md has no section for {version}")
    print(m.group(1).strip())
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    for name in ("bump", "freeze", "notes"):
        p = sub.add_parser(name)
        p.add_argument("version")
        if name == "freeze":
            p.add_argument("date")
    args = ap.parse_args(argv)
    if args.cmd != "check" and not SEMVER.match(args.version):
        raise SystemExit(f"'{args.version}' is not MAJOR.MINOR.PATCH")
    root = args.root
    return {
        "check": lambda: check(root),
        "bump": lambda: bump(root, args.version),
        "freeze": lambda: freeze(root, args.version, args.date),
        "notes": lambda: notes(root, args.version),
    }[args.cmd]()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
