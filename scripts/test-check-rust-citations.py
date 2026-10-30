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
    "crates/x/src/ghost.rs": (
        "// fn ghost_fn is gone\n/// struct Phantom was removed\nconst S: &str = \"type Bogus = 1\";\n/* const OLDCONST: u8 = 1; /* nested fn inner_gone */ still comment fn after_nested */\n"
        "let raw = r#\"fn in_raw\"#;\nlet c = '\"'; // fn after_char\n"
        "pub(crate) async unsafe fn qualified() {}\npub const fn const_fn() {}\nextern \"C\" fn ffi() {}\nstatic mut COUNTER: u8 = 0;\nmacro_rules! mac { () => {} }\n"
        "pub enum Kind {\n    #[allow(dead_code)]\n    First,\n    Second(u8),\n    Third { inner: u8 },\n}\n"
        "pub struct Cfg<T: Fn(u8, u8) -> u8> {\n    pub(crate) field_a: Vec<(u8, u8)>,\n    /// doc\n    field_b: u8,\n}\n"
        "impl<T> Trait<Arg> for Target<T> where T: Bound {\n    fn m(&self) { Ok(self.release(crate::Result)) }\n}\n"
        "pub fn caller() {\n    release(\n        arg_word,\n    );\n    let s = S {\n        lit_field: 1 };\n}\n"
        "impl<F: Fn(Frame) -> Result<(), Closed> + Send + 'static> FrameSink for F {}\n"
        "impl<F: Fn() -> u8> Wrap<F> for Holder {}\n"
        "impl Display for &'a RefTarget {}\n"
        "#[derive(Debug)] pub struct Attr;\n"
        "pub enum Ev {\n    Started {\n        inner_v: u8,\n        pub(crate) other_v: Vec<(u8, u8)>,\n    },\n    Stopped,\n    Tuple(u8),\n}\n"
        "pub enum Flags {\n    A = 1 << 2,\n    B,\n    C = 1 >> 1,\n}\n"
        "mod inline_mod { fn inline_fn() {} }\npub fn first_fn() {} pub fn same_line_fn() {}\n#[a(b[0])] fn nested_attr_fn() {}\n"
        "fn outer() { struct LocalStruct; let _ = 1; const LOCAL_CONST: u8 = 1; }\n"
    ),
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
    good += [
        "ghost.rs `qualified` (~3), ghost.rs const_fn, ghost.rs ffi, ghost.rs COUNTER, ghost.rs mac",
        "ghost.rs `Kind::First` and ghost.rs Second and ghost.rs Third and ghost.rs `Cfg::field_a` and ghost.rs field_b",
        "ghost.rs `Target::m` (~7) and ghost.rs Trait (an impl target is a definition)",
        "ghost.rs qualified #330 (an issue reference next to a citation is fine)",
        "ghost.rs `qualified` (~12) and 5 s",
        "ghost.rs FrameSink and ghost.rs Wrap and ghost.rs Holder and ghost.rs RefTarget and ghost.rs Display",
        "ghost.rs Attr (an attribute on the same line as the item)",
        "ghost.rs `Flags::B` and ghost.rs C (a variant after `A = 1 << 2,`)",
        "ghost.rs `Ev::Started` and ghost.rs inner_v and ghost.rs other_v and ghost.rs Stopped (struct-variant fields)",
        "ghost.rs `qualified` (see #330) and ghost.rs qualified, then 5 s",
    ]
    good += [
        "ghost.rs `qualified` at 50 Hz and ghost.rs qualified, 3 call sites",
        "ghost.rs `qualified` takes 12 ms; ghost.rs qualified (~12) is 100% sure",
        "ghost.rs inline_mod and ghost.rs inline_fn (an item after `{`)",
        "ghost.rs first_fn and ghost.rs same_line_fn (an item after `}`)",
        "ghost.rs nested_attr_fn (an attribute with a nested `]`)",
        "ghost.rs LocalStruct and ghost.rs LOCAL_CONST (items after `{` and `;`)",
        "core.rs `Core::end_ride` (~9): `Erg { 0 }` until a sample",
    ]
    good += [f"ghost.rs `qualified` {u}" for u in (
        "10 ms", "10 us", "3 min", "1 h", "2 kW", "5 J", "12 V", "5 mA", "3 dB", "16 B", "4 KiB", "2 MiB", "20 bytes", "8 bits",
        "4 frames", "2 packets", "9 ticks", "3 retries", "2 attempts", "5 calls", "2 tests", "4 cases", "3 files", "6 variants",
        "2 fields", "7 entries", "5 items", "3 call-sites",
    )]
    for i, g in enumerate(good[-46:]):
        expect(f"good {g!r}", tree_with(base, f"okx{i}", g), True)
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
        # #398 item 1: a mention in a comment, doc comment, string, nested block comment, raw string or char literal is no definition
        "ghost.rs ghost_fn",
        "ghost.rs Phantom",
        "ghost.rs Bogus",
        "ghost.rs OLDCONST",
        "ghost.rs inner_gone",
        "ghost.rs after_nested",
        "ghost.rs in_raw",
        "ghost.rs after_char",
        # #398 item 2: words in an impl header, a call, an argument or a struct literal are no definition
        "ghost.rs self",
        "ghost.rs Result",
        "ghost.rs release",
        "ghost.rs Ok",
        "ghost.rs crate",
        "ghost.rs Bound",
        "ghost.rs Arg",
        "ghost.rs `Cfg::u8`",
        "ghost.rs inner_field",
        # multi-line call arguments and struct literals, and the type parameters / generic arguments of the real
        # `impl<F: Fn(StreamFrame) -> Result<(), SinkClosed> + ..> FrameSink for F` header (core.rs `FrameSink`)
        "ghost.rs arg_word",
        "ghost.rs lit_field",
        "ghost.rs F",
        "ghost.rs Closed",
        "ghost.rs Frame",
        # #398 item 3: only ` (~N)`, N >= 1, directly after the symbol is a line hint
        "ghost.rs qualified 1742",
        "ghost.rs qualified (1742)",
        "ghost.rs qualified, 1742",
        "ghost.rs qualified (~ 1742)",
        "ghost.rs qualified (~-1)",
        "ghost.rs qualified (~0)",
        "ghost.rs qualified (~1742):3",
        "ghost.rs qualified L1742",
        # #398 item 4: prose after a file name is not a symbol; the message says so
        "see core.rs for x",
    ]
    # #398 round 3: the class "separators and labels, then a number" after a file name or a symbol, in every spelling
    spellings = [
        "[1742]", "[1742-1750]", "[ 1742 ]", ", ln 1742", "ln 1742", "ln.1742", "(l. 1742)", "l.1742", "L1742", ":1742",
        "#L1742", "line 1742", "lines 1742-1750", "( line 1742 )", "at 1742", "around 1742", "@1742", "~1742", "(1742)",
        "- 1742", "(~ 1742)", "1742",
    ]
    # #401: the bare-number class closed by rule: separators and up to two label words, then a number with no unit
    spellings += [
        "| 1742", "/ 1742", "— 1742", "– 1742", "→ 1742", "'line' 1742", "(Zeile 1742)", "at 1742", "at 1742.", "on 1742 and",
        "// 1742", "· 1742", "(see 1742)", "ab cd 1742", "1742 to 1750", "1742 or so", "& 1742", "= 1742",
    ]
    bad += [f"ghost.rs `qualified` {sp}" for sp in spellings]
    bad += [f"ghost.rs {sp}" for sp in spellings]
    bad += ["ghost.rs inner_zz (a struct-variant field that does not exist)", "ghost.rs inline_gone", "ghost.rs `qualified` at 50"]
    # #402 round 2: anything but a listed unit or count noun after the number is a line hint
    bad += [f"ghost.rs `qualified` {sp}" for sp in (
        "1742 is wrong", "1742 in", "1742 ff", "1742 x", "1742 L", "1742 etc", "1742 und", "1742 bis 1750", "1742 thru 1750",
        "1742 y 1750", "1742 ish", "1742f", "0x6D6", "(1742 lines)", "1742 lines", "1742 or 1743", "1742 and", "1742 to 1750",
        "1742 sorry", "1742 sx",
        # false positives the rule implies; they are rejected on purpose: rephrase
        "in 2024", "x 3", "× 3", '"1742"', "*1742*", "since v1.2", "50-60 Hz", "at 50", "(50)",
    )]
    for i, b in enumerate(bad):
        expect(f"bad {b!r}", tree_with(base, f"bad{i}", b), False)
    r = run(tree_with(base, "kw", "see core.rs for x"))
    if "keyword, not a symbol" not in r.stderr + r.stdout:
        sys.exit(f"test-check-rust-citations: a keyword after a file name must be reported as not a symbol:\n{r.stdout}{r.stderr}")
    expect("a citation split across lines", tree_with(base, "split", "core.rs", "// `Core::end_ride` (~1742)\n"), False)
print("test-check-rust-citations: ok")
