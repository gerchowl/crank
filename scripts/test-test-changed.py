#!/usr/bin/env python3
"""Tests for scripts/test-changed.py (#371): the file -> crate mapping and reverse dependencies.

Uses a synthetic `cargo metadata` document (domain <- store <- core <- desktop,
fec <- ble <- core), so it needs no cargo.
Usage: test-test-changed.py
"""
import importlib.util
import sys
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
}
DIRS = {"crank-desktop": "app/src-tauri"}
ids = {n: f"{n} 0.1.0 (path+file://{ROOT})" for n in DEPS}
META = {
    "workspace_root": ROOT,
    "workspace_members": list(ids.values()),
    "packages": [
        {"id": ids[n], "name": n,
         "manifest_path": f"{ROOT}/{DIRS.get(n, 'crates/' + n)}/Cargo.toml",
         "dependencies": [{"name": d} for d in deps]}
        for n, deps in DEPS.items()
    ],
}

failed = []


def expect(name, changed, want):
    got = tc.plan(changed, META)
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
expect("crate dir prefix is a path, not a string prefix", ["crates/crank-store-extra/src/lib.rs"], "none")
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
expect("docs, app TS and scripts only", ["docs/x.md", "app/src/a.tsx", "scripts/y.sh", "justfile"], "none")
expect("empty diff", [], "none")

if failed:
    sys.exit("test-changed: " + "\n".join(failed))
