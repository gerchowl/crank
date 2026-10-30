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


STUBS = {
    "crates/crank-core/src/core.rs": "fn shared_name() {}\npub struct Core;\nimpl Core {\n    pub async fn end_ride(&self) {}\n}\nconst REQUEST_TIMEOUT: u8 = 1;\n",
    "crates/crank-core/tests/core.rs": "fn only_in_tests() {}\nfn end_ride() {}\n",
    "crates/crank-devices/src/sim/core.rs": "fn sim_only() {}\nfn shared_name() {}\n",
    "crates/crank-core/src/other.rs": "fn x() {}\n",
    "crates/crank-engine/src/erg/policy.rs": "pub enum Mode {\n    Erg,\n}\nimpl TargetPolicy for ErgPolicy {\n    fn desired(&mut self) {}\n}\n",
}


def tree_with(base: Path, name: str, line: str, extra: str = "") -> Path:
    t = base / name
    (t / "app/src").mkdir(parents=True)
    (t / "app/src/x.ts").write_text(f"// {line}\n{extra}")
    for rel, body in STUBS.items():
        (t / rel).parent.mkdir(parents=True, exist_ok=True)
        (t / rel).write_text(body)
    return t


with tempfile.TemporaryDirectory() as tmp:
    base = Path(tmp)
    expect("real tree passes", root, True)
    good = [
        "core.rs `Core::end_ride` (~1742)",
        "core.rs end_ride",
        "crank-core/tests/core.rs `only_in_tests`",
        "core.rs `sim_only`",
        "erg/policy.rs `ErgPolicy::desired` (~172) and erg/policy.rs Mode",
        "core.rs `REQUEST_TIMEOUT` (~81), core.rs `Core::end_ride` (~9)",
        "it lives in crates/crank-core/src/core.rs (a path mention, no symbol)",
        "see core.rs.",
        "(tao 0.37.1, macos/window_delegate.rs, the handler), #330 and 10:30",
        "https://github.com/o/r/blob/abc123/crates/crank-core/src/core.rs",
        "no citation at all",
    ]
    for i, g in enumerate(good):
        expect(f"good {g!r}", tree_with(base, f"ok{i}", g), True)
    bad = [
        "core.rs:1667",
        "core.rs:7",
        "`core.rs`:81",
        "core.rs 1667",
        "core.rs ~1667",
        "core.rs (~1742)",
        "core.rs (1742)",
        "core.rs#L1742",
        "core.rs L1742",
        "core.rs, line 1742",
        "core.rs see 1742",
        "core.rs @1742",
        "core.rs - 1742",
        "core.RS `Core::end_ride`:1742",
        "core.rs `x`:1742",
        "core.rs end_ride:1742",
        "core.rs `end_ride:1742`",
        "core.rs end_ride::1742",
        "core.rs x1742",
        "core.rs see (~12)",
        "core.rs end_ride is called later (~1742)",
        "core.rs end_ride (line 1742)",
        "core.rs `Core::end_ride` (~1742) and `Core::end_ride` (~9)",
        "core.rs `no_such_fn` (~1742)",
        "core.rs for the details",
        "nofile.rs `end_ride` (~1742)",
        "core.rs `only_in_tests` (~3)",
        "core.rs `shared_name`",
        "https://github.com/o/r/blob/abc/crates/crank-core/src/core.rs#L1742",
        "core.rs `x` (library.rs:75-79, via `y` :1288)",
        "(a.rs `f`, :1632)",
    ]
    for i, b in enumerate(bad):
        expect(f"bad {b!r}", tree_with(base, f"bad{i}", b), False)
    expect("a citation split across lines", tree_with(base, "split", "core.rs", "// `Core::end_ride` (~1742)\n"), False)
print("test-check-rust-citations: ok")
