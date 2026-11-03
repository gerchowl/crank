#!/usr/bin/env python3
"""Tests for scripts/test-changed.py (#371): the file -> crate mapping and reverse dependencies.

Uses a synthetic `cargo metadata` document (domain <- store <- core <- desktop,
fec <- ble <- core), so it needs no cargo.
`changed_files` is tested against a temporary git repository (renames, deletion, untracked).
`feature_flags` (CI feature flags per selection) uses the same synthetic metadata.
Usage: test-test-changed.py
"""
import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path

spec = importlib.util.spec_from_file_location("test_changed", Path(__file__).resolve().parent / "test-changed.py")
tc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tc)

ROOT = "/w"
DEPS = {
    "crank-domain": [],
    "crank-store": ["crank-domain"],
    "crank-fec": [],
    "crank-ble": ["crank-fec", "crank-domain"],
    "crank-core": ["crank-store", "crank-ble", "serde"],
    "crank-desktop": ["crank-core"],
    "crank-cli": [],  # isolated: a binary-only crate with no dependents
}
DIRS = {"crank-desktop": "app/src-tauri"}
ids = {n: f"{n} 0.1.0 (path+file://{ROOT})" for n in DEPS}
META = {
    "workspace_root": ROOT,
    "workspace_members": list(ids.values()),
    "packages": [
        {"id": ids[n], "name": n,
         "targets": [{"kind": ["bin" if n == "crank-cli" else "lib"]}],
         "manifest_path": f"{ROOT}/{DIRS.get(n, 'crates/' + n)}/Cargo.toml",
         "dependencies": [{"name": d} for d in deps]}
        for n, deps in DEPS.items()
    ],
}

failed = []


def expect(name, changed, want, doc=None):
    got, got_doc = tc.plan(changed, META)
    if doc is not None and got_doc != doc:
        failed.append(f"{name}: doc crates {got_doc!r}, want {doc!r}")
    if got != want:
        failed.append(f"{name}: {changed} -> {got!r}, want {want!r}")
    else:
        print(f"test-changed: {name}: ok")


expect("leaf crate only touches itself and dependents", ["crates/crank-store/src/lib.rs"],
       ["crank-core", "crank-desktop", "crank-store"])
expect("top crate touches only itself", ["crates/crank-core/src/x.rs"], ["crank-core", "crank-desktop"])
expect("desktop under app/src-tauri", ["app/src-tauri/src/main.rs"], ["crank-desktop"])
expect("domain reaches every dependent transitively", ["crates/crank-domain/src/lib.rs"],
       ["crank-ble", "crank-core", "crank-desktop", "crank-domain", "crank-store"])
expect("non-source file inside a crate counts (fixtures, migrations)",
       ["crates/crank-store/migrations/0001.sql"], ["crank-core", "crank-desktop", "crank-store"])
expect("crate dir prefix is a path, not a string prefix (so this is unclassified)",
       ["crates/crank-store-extra/src/lib.rs"], "all")
expect("capture read by crank-ble tests", ["docs/hil/captures/check4.fecap"],
       ["crank-ble", "crank-core", "crank-desktop"])
expect("two crates are unioned", ["crates/crank-fec/a.rs", "crates/crank-store/b.rs"],
       ["crank-ble", "crank-core", "crank-desktop", "crank-fec", "crank-store"])
for f in ["Cargo.lock", "Cargo.toml", "flake.nix", "flake.lock", "rust-toolchain.toml", "clippy.toml",
          ".cargo/config.toml", ".config/nextest.toml"]:
    expect(f"workspace-level {f}", [f], "all")
expect("workspace-level file wins over a crate file", ["crates/crank-fec/a.rs", "Cargo.lock"], "all")
expect("a crate's own Cargo.toml is not workspace-level", ["crates/crank-fec/Cargo.toml"],
       ["crank-ble", "crank-core", "crank-desktop", "crank-fec"])
expect("non-Rust paths only", ["docs/x.md", "app/src/a.tsx", "scripts/y.sh", "justfile", "README.md",
                                ".github/workflows/ci.yml", "pnpm-lock.yaml", "design/a.png"], "none")
for f in ["somenew.toml", "nix/x.nix", "rustfmt.toml", "spec/knowledge-pack/rules.bin"]:
    expect(f"unclassified path {f} selects the whole workspace", [f], "all")
expect("an unclassified path wins over a crate file", ["crates/crank-fec/a.rs", "rustfmt.toml"], "all")
expect("empty diff", [], "none")
expect("doctests only for crates with a lib target", ["crates/crank-cli/src/main.rs"], ["crank-cli"], doc=[])
expect("lib crates are listed", ["crates/crank-fec/a.rs"],
       ["crank-ble", "crank-core", "crank-desktop", "crank-fec"],
       doc=["crank-ble", "crank-core", "crank-desktop", "crank-fec"])


def expect_features(name, selected, want):
    got = tc.feature_flags(selected, META)
    if got != want:
        failed.append(f"{name}: {selected} -> {got!r}, want {want!r}")
    else:
        print(f"test-changed: {name}: ok")


BLE = ["crank-ble/replay", "crank-ble/hil"]
expect_features("features: crank-ble selected", ["crank-ble"], BLE)
expect_features("features: a direct dependent of crank-ble keeps them (CI parity)", ["crank-core"], BLE)
expect_features("features: a transitive dependent has no direct edge, cargo would reject them",
                ["crank-desktop"], [])
expect_features("features: crates unrelated to crank-ble get none", ["crank-store", "crank-cli"], [])


def git(*args):
    subprocess.run(["git", *args], check=True, capture_output=True, text=True)


def test_changed_files():
    """A rename, a move between crates, a deletion and an untracked file all report every path involved."""
    cwd = os.getcwd()
    with tempfile.TemporaryDirectory() as tmp:
        os.chdir(tmp)
        try:
            git("init", "-q", "-b", "dev")
            git("config", "user.email", "t@example.invalid")
            git("config", "user.name", "t")
            for f in ["crates/a/src/one.rs", "crates/a/src/two.rs", "crates/a/src/gone.rs", "crates/b/src/x.rs"]:
                os.makedirs(os.path.dirname(f), exist_ok=True)
                with open(f, "w") as fh:
                    fh.write(f"// {f}\n" * 20)
            git("add", "-A")
            git("commit", "-q", "-m", "base")
            git("checkout", "-q", "-b", "feature")
            os.makedirs("docs", exist_ok=True)
            git("mv", "crates/a/src/one.rs", "docs/one.rs")        # out of a crate
            git("mv", "crates/a/src/two.rs", "crates/b/src/two.rs")  # between crates
            git("rm", "-q", "crates/a/src/gone.rs")                # deletion
            git("commit", "-q", "-m", "change")
            with open("crates/b/src/new.rs", "w") as fh:           # untracked
                fh.write("// new\n")
            got = tc.changed_files("dev")
        finally:
            os.chdir(cwd)
    want = ["crates/a/src/gone.rs", "crates/a/src/one.rs", "crates/a/src/two.rs",
            "crates/b/src/new.rs", "crates/b/src/two.rs", "docs/one.rs"]
    if got != want:
        failed.append(f"changed_files: {got!r}, want {want!r}")
    else:
        print("test-changed: changed_files (rename out, rename between, deletion, untracked): ok")


test_changed_files()


def test_changed_files_staged_rename():
    """A staged, uncommitted `git mv` reports the old path too (the `git diff HEAD` call needs --no-renames)."""
    cwd = os.getcwd()
    with tempfile.TemporaryDirectory() as tmp:
        os.chdir(tmp)
        try:
            git("init", "-q", "-b", "dev")
            git("config", "user.email", "t@example.invalid")
            git("config", "user.name", "t")
            os.makedirs("crates/a/src")
            os.makedirs("crates/b/src")
            with open("crates/a/src/one.rs", "w") as fh:
                fh.write("// one\n" * 20)
            git("add", "-A")
            git("commit", "-q", "-m", "base")
            git("mv", "crates/a/src/one.rs", "crates/b/src/one.rs")  # staged, not committed
            got = tc.changed_files("dev")
        finally:
            os.chdir(cwd)
    want = ["crates/a/src/one.rs", "crates/b/src/one.rs"]
    if got != want:
        failed.append(f"changed_files (staged rename): {got!r}, want {want!r}")
    else:
        print("test-changed: changed_files (staged uncommitted rename): ok")


test_changed_files_staged_rename()

if failed:
    sys.exit("test-changed: " + "\n".join(failed))
