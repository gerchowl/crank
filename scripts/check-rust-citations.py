#!/usr/bin/env python3
"""Fail on a Rust `file.rs:<line>` citation without a symbol under app/ (#391).

The convention (AGENTS.md "Citing Rust from app/"): `file.rs` + the symbol, with an optional `~line` hint,
e.g. `core.rs end_ride (~1742)`. A bare line number goes stale when the Rust file moves. Flagged, per line:
- `.rs:123` / `.rs`:123` / `.rs 123` / `.rs ~123`: a line number straight after the file name, no symbol;
- in a file that names a `.rs` file anywhere, a continuation ` :123` / `(:123` (as in `core.rs `x`, :1288`); formatted
  code never has a space before a colon, so this only matches citations.
Usage: check-rust-citations.py [root]
"""
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
SKIP_DIRS = {"node_modules", "target", "dist", "e2e-out", ".vite"}
EXTS = {".ts", ".tsx", ".rs", ".md", ".css", ".html"}
DIRECT = re.compile(r"\.rs`?(?::|\s+~?)\d")
CONTINUATION = re.compile(r"(?<![\w.`/])(?:\s|\(|,):\d{2,}")

errors = []
for path in sorted((root / "app").rglob("*")):
    if path.suffix not in EXTS or not path.is_file() or SKIP_DIRS & set(path.relative_to(root).parts):
        continue
    text = path.read_text(encoding="utf-8", errors="replace")
    cites_rust = ".rs" in text
    for n, line in enumerate(text.splitlines(), 1):
        if DIRECT.search(line) or (cites_rust and CONTINUATION.search(line)):
            errors.append(f"{path.relative_to(root)}:{n}: {line.strip()[:100]}")
if errors:
    sys.exit("rust-citations: cite `file.rs symbol (~line)`, not a bare line number (AGENTS.md):\n" + "\n".join(errors))
print("rust-citations: ok")
