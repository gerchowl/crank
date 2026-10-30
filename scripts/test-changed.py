#!/usr/bin/env python3
"""Plan `just test-changed` (#371): which workspace crates a diff affects.

changed files -> owning workspace crates -> plus their reverse dependencies
(from `cargo metadata`) -> the `-p` list. A workspace-level file (Cargo.toml,
Cargo.lock, flake.nix, flake.lock, rust-toolchain.toml, clippy.toml, .cargo/,
.config/) selects the whole workspace. A diff that touches no crate and no
workspace-level file selects nothing.

Usage: test-changed.py [base]   (default origin/dev; compares base...HEAD plus
the working tree and untracked files). Prints one line:
  all                   whole workspace
  none                  nothing to test
  crates a b c          the affected crates, sorted
"""
import json
import subprocess
import sys
from pathlib import Path

WORKSPACE_FILES = {
    "Cargo.toml", "Cargo.lock", "flake.nix", "flake.lock",
    "rust-toolchain.toml", "clippy.toml",
}
WORKSPACE_DIRS = (".cargo/", ".config/")
# Files outside a crate directory that a crate's tests read (include_str!).
EXTRA_OWNERS = {"docs/hil/captures/": "crank-ble"}


def crates_and_rdeps(metadata):
    """Return ({crate dir relative to the root: name}, {name: set of workspace dependents})."""
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
    return dirs, rdeps


def plan(changed, metadata):
    """Return "all", "none" or a sorted list of crate names for the changed paths."""
    dirs, rdeps = crates_and_rdeps(metadata)
    owners = set()
    for f in changed:
        if f in WORKSPACE_FILES or f.startswith(WORKSPACE_DIRS):
            return "all"
        # Longest crate directory wins (a crate may nest in another's tree).
        hits = [d for d in dirs if f.startswith(d + "/")]
        if hits:
            owners.add(dirs[max(hits, key=len)])
        for prefix, crate in EXTRA_OWNERS.items():
            if f.startswith(prefix):
                owners.add(crate)
    if not owners:
        return "none"
    affected, pending = set(), list(owners)
    while pending:
        c = pending.pop()
        if c not in affected:
            affected.add(c)
            pending.extend(rdeps[c])
    return sorted(affected)


def git_lines(*args):
    out = subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout
    return [line for line in out.splitlines() if line]


def changed_files(base):
    """Paths changed since the merge-base with `base`, in the working tree, or untracked."""
    return sorted(set(git_lines("diff", "--name-only", f"{base}...HEAD"))
                  | set(git_lines("diff", "--name-only", "HEAD"))
                  | set(git_lines("ls-files", "--others", "--exclude-standard")))


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "origin/dev"
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"],
        capture_output=True, text=True, check=True).stdout)
    result = plan(changed_files(base), meta)
    print(result if isinstance(result, str) else "crates " + " ".join(result))


if __name__ == "__main__":
    main()
