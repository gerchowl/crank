#!/usr/bin/env python3
"""Fail on a Rust citation under app/ that is not `file.rs symbol (~line)` (#391).

The convention (AGENTS.md "Citing Rust from app/"): after `<file>.rs` comes the symbol (an identifier, optionally in
backticks, `::` paths allowed), then an optional `(~line)` hint; `file.rs` alone is fine. A bare line number goes stale
when the Rust file moves. Flagged:
- a line hint straight after the file name, with no symbol: `.rs:123`, `.rs 123`, `.rs (~123)`, `.rs (123)`,
  `.rs#L123`, `.rs L123`, `.rs, line 123`;
- a line hint straight after the symbol that is not `(~N)`: `.rs sym:123`, `.rs sym 123`, `.rs see 123`;
- in a file that names a `.rs` file anywhere, a continuation ` :123` / `(:123` (as in `core.rs `x`, :1288`);
  formatted code never has a space before a colon, so this only matches citations.
Usage: check-rust-citations.py [root]
"""
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
SKIP_DIRS = {"node_modules", "target", "dist", "e2e-out", ".vite"}
EXTS = {".ts", ".tsx", ".rs", ".md", ".css", ".html"}
FILE = re.compile(r"\.rs`?")
SYMBOL = re.compile(r"\s+`?(?!L\d)[A-Za-z_][\w:]*(?:\(\))?`?")
CANONICAL_HINT = re.compile(r"\s*\(~\d+\)")
HINT = re.compile(r"[\s,;:]*(?:line\s*)?(?:#?L|~|:|\()*\d{2,}")
CONTINUATION = re.compile(r"(?<![\w.`/])(?:\s|\(|,):\d{2,}")


def bad_citation(line: str) -> bool:
    for m in FILE.finditer(line):
        tail = line[m.end():]
        sym = SYMBOL.match(tail)
        if not sym:
            if HINT.match(tail):
                return True
            continue
        rest = tail[sym.end():]
        if not CANONICAL_HINT.match(rest) and HINT.match(rest):
            return True
    return False


errors = []
for path in sorted((root / "app").rglob("*")):
    if path.suffix not in EXTS or not path.is_file() or SKIP_DIRS & set(path.relative_to(root).parts):
        continue
    text = path.read_text(encoding="utf-8", errors="replace")
    cites_rust = ".rs" in text
    for n, line in enumerate(text.splitlines(), 1):
        if bad_citation(line) or (cites_rust and CONTINUATION.search(line)):
            errors.append(f"{path.relative_to(root)}:{n}: {line.strip()[:100]}")
if errors:
    sys.exit("rust-citations: cite `file.rs symbol (~line)`, not a bare line number (AGENTS.md):\n" + "\n".join(errors))
print("rust-citations: ok")
