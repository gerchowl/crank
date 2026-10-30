#!/usr/bin/env python3
"""Test scripts/relocate-tauri-permissions.py across a chain of restores (#371).

A build script ran in /a/out; the artifacts are restored to /b (checks of one derivation), and the
result is restored again to /c (the next derivation). Each restore must rebase the manifest.
Usage: test-relocate-tauri-permissions.py [relocate-tauri-permissions.py]
"""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

script = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parent / "relocate-tauri-permissions.py")


def restore(target: Path) -> None:
    subprocess.run([sys.executable, "-I", str(script)], check=True, env={**os.environ, "CARGO_TARGET_DIR": str(target)})


with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp).resolve()
    first = root / "a" / "target"
    out = first / "release" / "build" / "tauri-x" / "out"
    out.mkdir(parents=True)
    (out / "perm.toml").write_text("")
    (out / "tauri-core-permission-files").write_text(json.dumps([str(out / "perm.toml")]))
    (out.parent / "root-output").write_text(str(out))
    for name in ("b", "c"):
        moved = root / name / "target"
        moved.parent.mkdir()
        (first if name == "b" else root / "b" / "target").rename(moved)
        restore(moved)
        got = json.loads((moved / "release/build/tauri-x/out/tauri-core-permission-files").read_text())
        want = [str(moved / "release/build/tauri-x/out/perm.toml")]
        if got != want:
            sys.exit(f"relocate-tauri-permissions: restore {name}: {got}, want {want}")
        print(f"relocate-tauri-permissions: restore {name}: ok")
