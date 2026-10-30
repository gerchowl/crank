#!/usr/bin/env python3
"""Fail on a Rust citation under app/ that is not `file.rs symbol (~line)`, or whose symbol does not exist (#391).

The canonical form (AGENTS.md "Citing Rust from app/"): `<name>.rs <symbol>`, then optionally ` (~N)` right after the
symbol. `symbol` is `Ident` or `Ident::Ident...` (backticks around it are allowed). A bare `file.rs` mention, with no
symbol, is fine. A bare line number goes stale when the Rust file moves, so the checker is strict by construction:
- every `<name>.rs <symbol>` is parsed with that grammar (case-insensitive on `.rs`), and the symbol's LAST segment
  must be defined in the cited file under crates/ or app/src-tauri/ (fn, struct, enum, trait, type, const, static,
  mod, union, macro_rules!, an `impl` header naming it, or an enum variant / field line starting with it). English
  words and stale symbols fail here. A base name shared by several files (`core.rs`) resolves, outside tests/, when exactly one of
  them defines the symbol; give the path (`crank-core/tests/core.rs`) if that is not enough;
- the symbol must be followed by whitespace, punctuation or ` (~N)`: `sym:12`, `sym::12`, `sym@12` fail;
- any other line hint on a line that mentions a `.rs` file fails: `(~N)` not directly after a resolved symbol, `~N`,
  `:N`, `#LN` (a GitHub `#L12` URL fragment is not allowed either), `@N`, `LN`, `line N`, a number right after the
  file name;
- a `(~N)` anywhere that does not directly follow a resolved symbol fails, so a citation split across lines fails.
Usage: check-rust-citations.py [root]
"""
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
SKIP_DIRS = {"node_modules", "target", "dist", "e2e-out", ".vite"}
EXTS = {".ts", ".tsx", ".md", ".css", ".html"}

MENTION = re.compile(r"(?P<name>\w[\w./-]*?)\.rs\b`?", re.I)
IDENT = r"[A-Za-z_]\w*"
SYMBOL = re.compile(rf"[ ]+(?P<tick>`?)(?P<sym>{IDENT}(?:::{IDENT})*)(?P=tick)(?![\w:@#~(<`])")
SYMBOL_TICKED_END = re.compile(rf"[ ]+`(?P<sym>{IDENT}(?:::{IDENT})*)`")
LINE_HINT = re.compile(r"[ ]\(~\d+\)")
HINT_AFTER_FILE = re.compile(r"[\s,;:#@-]*(?:line\s*)?L?~?\(?\d", re.I)
HINT_ANY = re.compile(r"\(~\d+\)|~\d+|(?<!\w)(?:[:@]L?|#L)\d+|\bline\s*\d+|\bL\d+\b", re.I)
STRAY_TILDE = re.compile(r"\(~\d+\)")

_rust: list[Path] | None = None
_defs: dict[tuple[Path, str], bool] = {}


def rust_files() -> list[Path]:
    global _rust
    if _rust is None:
        _rust = [
            p
            for base in (root / "crates", root / "app/src-tauri")
            if base.is_dir()
            for p in base.rglob("*.rs")
            if not SKIP_DIRS & set(p.relative_to(root).parts)
        ]
    return _rust


def defined(path: Path, name: str) -> bool:
    key = (path, name)
    if key not in _defs:
        n = re.escape(name)
        pat = re.compile(
            rf"\b(?:fn|struct|enum|trait|type|const|static|mod|union)\s+{n}\b"
            rf"|\bmacro_rules!\s*{n}\b"
            rf"|^\s*impl\b[^{{;]*\b{n}\b"
            rf"|^\s*(?:pub(?:\([^)]*\))?\s+)?{n}\s*(?:[:,({{=]|$)",
            re.M,
        )
        _defs[key] = bool(pat.search(path.read_text(encoding="utf-8", errors="replace")))
    return _defs[key]


def resolve(name: str, sym: str) -> str | None:
    """None when `sym` is defined in the file `name` (unambiguously); else the reason."""
    want = name + ".rs"
    files = [p for p in rust_files() if p.as_posix().lower().endswith("/" + want.lower())]
    if not files:
        return f"no file {want} under crates/ or app/src-tauri/"
    if len(files) > 1 and "/" not in name:
        files = [p for p in files if "tests" not in p.parts] or files  # a bare name means the library source
    last = sym.split("::")[-1]
    hits = [p for p in files if defined(p, last)]
    if len(hits) == 1:
        return None
    if not hits:
        return f"`{last}` is not defined in {want}"
    return f"`{last}` is defined in several files named {want} ({len(hits)}); cite the path"


def check_line(line: str) -> list[str]:
    problems: list[str] = []
    consumed: list[tuple[int, int]] = []
    for m in MENTION.finditer(line):
        tail = line[m.end():]
        sym = SYMBOL.match(tail)
        if sym is None:
            if re.match(r"[ ]+`?" + IDENT, tail):
                problems.append(f"{m.group(0).strip('`')}: the symbol must be an identifier path with no suffix")
            elif HINT_AFTER_FILE.match(tail):
                problems.append(f"{m.group(0).strip('`')}: a line number without a symbol")
            continue
        why = resolve(m.group("name"), sym.group("sym"))
        if why:
            problems.append(why)
            continue
        hint = LINE_HINT.match(tail, sym.end())
        if hint:
            consumed.append((m.end() + hint.start(), m.end() + hint.end()))
    rest = line
    for a, b in reversed(consumed):
        rest = rest[:a] + rest[b:]
    if STRAY_TILDE.search(rest):
        problems.append("a (~N) hint that does not directly follow a resolved symbol")
    elif MENTION.search(line) and HINT_ANY.search(MENTION.sub(" ", rest)):
        problems.append("a line hint that is not `(~N)` right after a symbol")
    return problems


errors = []
for path in sorted((root / "app").rglob("*")):
    if path.suffix not in EXTS or not path.is_file() or SKIP_DIRS & set(path.relative_to(root).parts):
        continue
    for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        for why in check_line(line):
            errors.append(f"{path.relative_to(root)}:{n}: {why}: {line.strip()[:90]}")
if errors:
    sys.exit("rust-citations: cite `file.rs symbol (~line)` with an existing symbol (AGENTS.md):\n" + "\n".join(errors))
print("rust-citations: ok")
