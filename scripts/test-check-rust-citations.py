#!/usr/bin/env python3
"""Regression test for scripts/check-rust-citations.py (#391).

The real tree must pass; each bare-line citation form must fail; symbol citations must pass.
Usage: test-check-rust-citations.py [root [check-rust-citations.py]]
"""
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
check = Path(sys.argv[2] if len(sys.argv) > 2 else Path(__file__).resolve().parent / "check-rust-citations.py").resolve()


def run(tree: Path) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, "-I", str(check), str(tree)], capture_output=True, text=True)


def expect(name: str, tree: Path, ok: bool) -> None:
    r = run(tree)
    if (r.returncode == 0) != ok:
        sys.exit(f"test-check-rust-citations: {name}: expected {'pass' if ok else 'fail'}, got {r.returncode}\n{r.stdout}{r.stderr}")


def tree_with(base: Path, name: str, line: str) -> Path:
    t = base / name
    (t / "app/src").mkdir(parents=True)
    (t / "app/src/x.ts").write_text(f"// {line}\n")
    return t


with tempfile.TemporaryDirectory() as tmp:
    base = Path(tmp)
    expect("real tree passes", root, True)
    for i, ok_line in enumerate(["core.rs `end_ride` (~1742)", "core.rs end_ride", "see engine.rs `Pauses::reason` (~336) and :", "no citation"]):
        expect(f"symbol form {ok_line!r}", tree_with(base, f"ok{i}", ok_line), True)
    for i, bad in enumerate(["core.rs:1667", "`core.rs`:81", "core.rs 1667", "core.rs ~1667", "core.rs `x` (library.rs:75-79, via `y` :1288)", "(a.rs `f`, :1632)"]):
        expect(f"bare line {bad!r}", tree_with(base, f"bad{i}", bad), False)
print("test-check-rust-citations: ok")
