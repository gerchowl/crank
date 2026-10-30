#!/usr/bin/env python3
"""Plan `just test-changed` (#371): which workspace crates a diff affects.

changed files -> owning workspace crates -> plus their reverse dependencies
(from `cargo metadata --no-deps`, dev-dependency edges included on purpose) -> the
`-p` list. The default for a path is the SAFE one: a path that is not inside a
workspace crate and not on the explicit non-Rust list below selects the whole
workspace (Cargo.toml, Cargo.lock, flake.*, rust-toolchain.toml, clippy.toml,
.cargo/, .config/, and anything new nobody classified). A diff made only of
non-Rust paths selects nothing.

Non-Rust paths (never affect a cargo test): docs/ (except docs/hil/captures/, which
crank-ble's tests include), app/ (the frontend; app/src-tauri/ is the crank-desktop
crate), design/, shared/, assets/, issues/, scripts/, .github/, any *.md, and the root
files justfile, LICENSE, NOTICE, .gitignore, .editorconfig, package.json, pnpm-*.yaml,
deny.toml, deny-fuzz.toml. Keep AGENTS.md in step with this list.

Usage: test-changed.py [base]   (default origin/dev; compares base...HEAD plus
the working tree and untracked files; a rename counts as its old and new path).
Prints one line, `all` (whole workspace) or `none` (nothing to test) or
`crates a b c` (the affected crates, sorted); after a `crates` line, a second line
`doc x y` lists those of them with a library target (doctests need one; may be `doc`).
"""
import json
import subprocess
import sys
from pathlib import Path

NON_RUST_DIRS = ("docs/", "design/", "shared/", "assets/", "issues/", "scripts/", ".github/")
NON_RUST_ROOT = {
    "justfile", "LICENSE", "NOTICE", ".gitignore", ".editorconfig", "package.json",
    "deny.toml", "deny-fuzz.toml",
}
# Files outside a crate directory that a crate's tests read (include_str!).
EXTRA_OWNERS = {"docs/hil/captures/": "crank-ble"}


def crates_and_rdeps(metadata):
    """Return ({crate dir relative to the root: name}, {name: set of workspace dependents}, {crates with a lib target})."""
    root = Path(metadata["workspace_root"])
    members = set(metadata["workspace_members"])
    by_id = {p["id"]: p for p in metadata["packages"] if p["id"] in members}
    dirs = {str(Path(p["manifest_path"]).parent.relative_to(root)): p["name"] for p in by_id.values()}
    names = set(dirs.values())
    rdeps = {n: set() for n in names}
    for p in by_id.values():
        for d in p["dependencies"]:
            if d["name"] in names:  # dev-dependencies count: their tests build against the dependency
                rdeps[d["name"]].add(p["name"])
    libs = {p["name"] for p in by_id.values()
            if any(k in ("lib", "rlib", "proc-macro") for t in p["targets"] for k in t["kind"])}
    return dirs, rdeps, libs


def non_rust(f):
    """True for a path outside every crate that no cargo test reads."""
    if any(f.startswith(d) for d in NON_RUST_DIRS) or f.endswith(".md"):
        return True
    if f.startswith("app/") and f.split("/")[1] != "src-tauri":
        return True
    return f in NON_RUST_ROOT or (f.startswith("pnpm-") and f.endswith(".yaml") and "/" not in f)


def plan(changed, metadata):
    """Return ("all"|"none"|sorted crate list, crates with a lib target among them)."""
    dirs, rdeps, libs = crates_and_rdeps(metadata)
    owners = set()
    for f in changed:
        # Longest crate directory wins (a crate may nest in another's tree).
        hits = [d for d in dirs if f.startswith(d + "/")]
        if hits:
            owners.add(dirs[max(hits, key=len)])
            continue
        mapped = [c for prefix, c in EXTRA_OWNERS.items() if f.startswith(prefix)]
        if mapped:
            owners.update(mapped)
        elif not non_rust(f):
            return "all", []
    if not owners:
        return "none", []
    affected, pending = set(), list(owners)
    while pending:
        c = pending.pop()
        if c not in affected:
            affected.add(c)
            pending.extend(rdeps[c])
    return sorted(affected), sorted(affected & libs)


def git_lines(*args):
    out = subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout
    return [line for line in out.splitlines() if line]


def changed_files(base):
    """Paths changed since the merge-base with `base`, in the working tree, or untracked."""
    # --no-renames: a moved file must report its old path too (its old crate changed as well).
    return sorted(set(git_lines("diff", "--name-only", "--no-renames", f"{base}...HEAD"))
                  | set(git_lines("diff", "--name-only", "--no-renames", "HEAD"))
                  | set(git_lines("ls-files", "--others", "--exclude-standard")))


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "origin/dev"
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        capture_output=True, text=True, check=True).stdout)
    result, doc = plan(changed_files(base), meta)
    if isinstance(result, str):
        print(result)
    else:
        print("crates " + " ".join(result))
        print("doc " + " ".join(doc))


if __name__ == "__main__":
    main()
