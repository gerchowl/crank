#!/usr/bin/env python3
"""Version sources and changelog of a release (#428, #485, RELEASING.md, ADR-0001 §9.6).

Usage: release-version.py [--root DIR] check
       release-version.py [--root DIR] version
       release-version.py parse VERSION
       release-version.py [--root DIR] bump VERSION
       release-version.py [--root DIR] freeze X.Y.Z YYYY-MM-DD
       release-version.py [--root DIR] notes VERSION

A VERSION is `X.Y.Z` (a final release) or `X.Y.Z-rc.N` (release candidate N of X.Y.Z). macOS bundle
versions must be numeric, so app/src-tauri/Info.plist carries a mapping: `CFBundleShortVersionString` is
`X.Y.Z` and `CFBundleVersion` the integer X*1000000 + Y*10000 + Z*100 + B, B = N for rc.N and 99 for the
final release. It orders like semver precedence, which bounds Y and Z to 0..99 and N to 1..98.

`check` fails unless every version source agrees with `[workspace.package] version` of Cargo.toml:
`version` of app/src-tauri/tauri.conf.json, both bundle versions of Info.plist (mapped as above) and
`version` of every package.json that has one (the `release-versions` gate of CI). `version` prints the
agreed version. `parse` validates a version and prints `key=value` lines for $GITHUB_OUTPUT. `bump`
rewrites every source in place with textual edits, so formatting and key order are kept, and refuses a
version that is not later than the current one; Cargo.lock follows with `cargo update --workspace`
(prepare-release.yml). `freeze` (final releases only) moves the body of `## Unreleased` into a dated
`## [X.Y.Z]` section and leaves a fresh empty `## Unreleased` above it (devkit's changelog format).
`notes` prints that section, or for a release candidate the current `## Unreleased` entries.
"""
import argparse
import json
import re
import sys
from pathlib import Path
from typing import NamedTuple

# ASCII digits only, matched whole: `\d` would take any Unicode digit and `$` a trailing newline.
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-rc\.([1-9][0-9]*))?")
FINAL_BUILD = 99  # the B of a final release; rc numbers stay below it
CARGO = re.compile(r'(\[workspace\.package\][^\[]*?^version\s*=\s*")([^"]+)(")', re.M | re.S)
PLIST_SHORT = re.compile(r"(<key>CFBundleShortVersionString</key><string>)([^<]+)(</string>)")
PLIST_BUNDLE = re.compile(r"(<key>CFBundleVersion</key><string>)([^<]+)(</string>)")
JSON = re.compile(r'^(\s{2}"version"\s*:\s*")([^"]+)(")', re.M)
SECTIONS = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]


class Version(NamedTuple):
    """A parsed release version; `rc` is None for a final release."""

    major: int
    minor: int
    patch: int
    rc: int | None

    def __str__(self):
        return self.short() + (f"-rc.{self.rc}" if self.rc is not None else "")

    def short(self):
        """`CFBundleShortVersionString`: the release this build is, or is a candidate for."""
        return f"{self.major}.{self.minor}.{self.patch}"

    def bundle(self):
        """`CFBundleVersion`: one integer that orders like semver precedence (ADR-0001 §9.6)."""
        build = FINAL_BUILD if self.rc is None else self.rc
        return str(self.major * 1_000_000 + self.minor * 10_000 + self.patch * 100 + build)

    def order(self):
        """Sort key: semver precedence (a candidate sorts before its final release)."""
        return (self.major, self.minor, self.patch, FINAL_BUILD if self.rc is None else self.rc)


def parse(text):
    """Parse `X.Y.Z` or `X.Y.Z-rc.N`; raise ValueError for anything the bundle mapping cannot carry."""
    m = VERSION.fullmatch(text)
    if not m:
        raise ValueError(f"'{text}' is not X.Y.Z or X.Y.Z-rc.N")
    major, minor, patch = (int(g) for g in m.group(1, 2, 3))
    rc = int(m.group(4)) if m.group(4) else None
    if minor > 99 or patch > 99:
        raise ValueError(f"'{text}': minor and patch must be 0..99 for the numeric bundle version")
    if rc is not None and rc >= FINAL_BUILD:
        raise ValueError(f"'{text}': rc must be 1..{FINAL_BUILD - 1} for the numeric bundle version")
    return Version(major, minor, patch, rc)


def sources(root):
    """Yield (name, path, regex, render) per version source; render maps a Version to the text it must hold."""
    yield "Cargo.toml", "Cargo.toml", CARGO, str
    yield "app/src-tauri/tauri.conf.json", "app/src-tauri/tauri.conf.json", JSON, str
    plist = "app/src-tauri/Info.plist"
    yield f"{plist} CFBundleShortVersionString", plist, PLIST_SHORT, Version.short
    yield f"{plist} CFBundleVersion", plist, PLIST_BUNDLE, Version.bundle
    for pkg in sorted(root.glob("**/package.json")):
        rel = pkg.relative_to(root)
        if "node_modules" in rel.parts:
            continue
        if "version" in json.loads(pkg.read_text()):
            yield str(rel), str(rel), JSON, str


def found(root, path, rx):
    """The value a source holds, or `a vs b` when its matches differ."""
    values = [m.group(2) for m in rx.finditer((root / path).read_text())]
    if not values:
        raise SystemExit(f"{path}: no version found")
    return values[0] if len(set(values)) == 1 else " vs ".join(values)


def current(root):
    """The version of Cargo.toml, parsed."""
    text = found(root, "Cargo.toml", CARGO)
    try:
        return parse(text)
    except ValueError as e:
        raise SystemExit(f"Cargo.toml: {e}") from None


def check(root, quiet=False):
    ref = current(root)
    bad = []
    count = 0
    for name, path, rx, render in sources(root):
        count += 1
        have, want = found(root, path, rx), render(ref)
        if have != want:
            bad.append(f"{name}: {have} (expected {want})")
    if not bad:
        if not quiet:
            print(f"version sources agree: {ref} ({count} sources)")
        return 0
    for line in bad:
        print(line, file=sys.stderr)
    print(f"version sources disagree with Cargo.toml {ref}: bump them together with release-version.py bump", file=sys.stderr)
    return 1


def version(root):
    code = check(root, quiet=True)
    if code == 0:
        print(current(root))
    return code


def describe(ver):
    print(f"version={ver}\nkind={'final' if ver.rc is None else 'rc'}\nshort={ver.short()}\nbundle={ver.bundle()}")
    return 0


def bump(root, ver):
    old = current(root)
    if ver.order() <= old.order():
        raise SystemExit(f"{ver} is not later than the current version {old}")
    for _, path, rx, render in sources(root):
        text = (root / path).read_text()
        (root / path).write_text(rx.sub(lambda m: m.group(1) + render(ver) + m.group(3), text))
    return check(root)


def changelog_parts(text):
    """Split into (head, unreleased body, rest) around the `## Unreleased` section."""
    m = re.search(r"^## Unreleased[ \t]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S)
    if not m:
        raise SystemExit("CHANGELOG.md has no '## Unreleased' section")
    return text[: m.start()], m.group(1), text[m.end():]


def has_content(body):
    return any(ln.strip() and not ln.startswith("### ") for ln in body.splitlines())


def drop_empty_subsections(body):
    return re.sub(r"^### [^\n]*\n\s*(?=^### |\Z)", "", body.rstrip() + "\n", flags=re.M)


def freeze(root, ver, date):
    if ver.rc is not None:
        raise SystemExit(f"{ver}: a release candidate keeps its entries in '## Unreleased'; only a final release freezes")
    version = str(ver)
    path = root / "CHANGELOG.md"
    head, body, rest = changelog_parts(path.read_text())
    if not has_content(body):
        raise SystemExit("CHANGELOG.md: '## Unreleased' has no entries to release")
    if re.search(rf"^## \[{re.escape(version)}\]", rest, re.M):
        raise SystemExit(f"CHANGELOG.md already has a section for {version}")
    empty = "## Unreleased\n\n" + "".join(f"### {s}\n\n" for s in SECTIONS)
    body = drop_empty_subsections(body)
    url = f"https://github.com/gerchowl/crank/releases/tag/{version}"
    path.write_text(f"{head}{empty}## [{version}]({url}) - {date}\n{body.rstrip()}\n\n{rest}")
    return 0


def notes(root, ver):
    text = (root / "CHANGELOG.md").read_text()
    if ver.rc is not None:
        body = drop_empty_subsections(changelog_parts(text)[1]).strip()
        print(f"Release candidate {ver.rc} of {ver.short()}. Changes not yet in a final release:\n\n{body or '(none listed)'}")
        return 0
    version = str(ver)
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
    sub.add_parser("version")
    for name in ("parse", "bump", "freeze", "notes"):
        p = sub.add_parser(name)
        p.add_argument("version")
        if name == "freeze":
            p.add_argument("date")
    args = ap.parse_args(argv)
    root = args.root
    if args.cmd == "check":
        return check(root)
    if args.cmd == "version":
        return version(root)
    try:
        ver = parse(args.version)
    except ValueError as e:
        raise SystemExit(str(e)) from None
    return {
        "parse": lambda: describe(ver),
        "bump": lambda: bump(root, ver),
        "freeze": lambda: freeze(root, ver, args.date),
        "notes": lambda: notes(root, ver),
    }[args.cmd]()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
