#!/usr/bin/env python3
"""Fail on a Rust citation under app/ that is not `file.rs symbol (~line)`, or whose symbol does not exist (#391).

The canonical form (AGENTS.md "Citing Rust from app/"): `<name>.rs <symbol>`, then optionally ` (~N)` right after the
symbol. `symbol` is `Ident` or `Ident::Ident...` (backticks around it are allowed). A bare `file.rs` mention, with no
symbol, is fine. A bare line number goes stale when the Rust file moves, so the checker is strict by construction:
- every `<name>.rs <symbol>` is parsed with that grammar (case-insensitive on `.rs`), and the symbol's LAST segment
  must be DEFINED in the cited file under crates/ or app/src-tauri/. The file is first stripped of comments and string
  and char literals (a deleted item still named in a comment or string does not count), then a definition is: an item
  at the start of a line (`[pub(..)] [default|const|async|unsafe|extern "x"]* fn|struct|enum|trait|type|const|static
  [mut]|mod|union NAME`, `macro_rules! NAME`), the TYPE or TRAIT an `impl` header names (`impl Tr for Ty`: `Tr` and
  `Ty`; neither generic arguments such as the `X` of `From<X>` nor the impl's own type parameters count), or an enum variant or named struct field,
  parsed from the braces of its item. Words in a header, call or argument do not count. English words and stale
  symbols fail here, and a keyword such as `for` is reported as not a symbol. A base name shared by several files (`core.rs`) resolves, outside tests/, when exactly one of
  them defines the symbol; give the path (`crank-core/tests/core.rs`) if that is not enough;
- directly after the symbol only ` (~N)` with N >= 1 is a line hint: `sym:12`, `sym::12`, `sym@12`, `sym 12`, `sym (12)`,
  `sym, 12`, `sym (~ 12)`, `sym (~0)`, `sym (~-1)` and `sym (~12):3` fail. An issue reference `#123` next to a citation is
  allowed (`#` + digits is not a line hint; only `#L12` is);
- any other line hint on a line that mentions a `.rs` file fails: `(~N)` not directly after a resolved symbol, `~N`,
  `:N`, `#LN` (a GitHub `#L12` URL fragment is not allowed either), `@N`, `LN`, `line N`, `ln N`, `l. N`, `[N]`, `(N)`.
  After a file name or a symbol the whole class is closed, not a list of spellings: separators/brackets and label words
  (`line`, `ln`, `l.`, `at`, `around`, ...), optionally `L`/`#L`, then a digit;
- a `(~N)` anywhere that does not directly follow a resolved symbol fails, so a citation split across lines fails.
Known limits (#401): an item must start its line (after optional same-line attributes without a nested `]`), so items
inside a macro body, one-line `mod m { fn f() {} }` and tuple-struct fields cannot be cited. Fields of struct-like enum variants (`Started { inner_v: u8 }`) can.
A bare number right after a symbol is rejected even when it is prose (`core.rs `host`, 3 call sites`): rephrase it.
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
LINE_HINT = re.compile(r"[ ]\(~[1-9]\d*\)")
# What may sit between a file name or symbol and a number for the number to read as a line hint: punctuation and
# brackets, label words (`line`, `ln`, `l.`, `at`, ...), then `L`/`#L`. The class is "separators and labels, then a digit",
# so `:12`, `[12]`, `(12)`, `, 12`, `ln 12`, `(l. 12)`, `#L12`, `L12`, `@12`, `~12`, `- 12` are all caught; `#12` is an
# issue reference and is not (`#` only counts before `L`).
NUMBER_AFTER = re.compile(
    r"(?:[\s,;:.\[\](){}<>@~=+\-]|(?:lines?|ln|l|at|on|near|around|about|pos|offset|row)\b\.?)*(?:#?L\s*)?\d", re.I
)
# A hint anywhere on a line that mentions a `.rs` file (after the canonical ones are removed).
HINT_ANY = re.compile(
    r"~\s*\d|(?<!\w)(?:[:@]L?|#L)\d+|\b(?:lines?|ln|l)\.?\s*\d|\bL\d+\b|[\[(]\s*\d{2,}(?:\s*[-,]\s*\d+)*\s*[\])]", re.I
)
STRAY_TILDE = re.compile(r"\(\s*~")
SYMBOL_WORD = re.compile(rf"[ ]+`?{IDENT}")
GENERIC_PARAM = re.compile(rf"\s*(?:const\s+)?({IDENT})")
FOR = re.compile(r"\bfor\b")
ATTRIBUTE = re.compile(r"#\[[^\]]*\]")
PUB = re.compile(r"^pub(?:\([^)]*\))?\s+")
FIELD = re.compile(rf"({IDENT})\s*(:(?!:))?")

RAW_STRING = re.compile(r'b?r(#*)"')
CHAR_LITERAL = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^\\'])'")
WHERE = re.compile(r"\bwhere\b")
IMPL_SIDE = re.compile(rf"\s*(?:[&!*]\s*(?:'\w+\s+)?(?:mut\s+|const\s+)?)*(?:dyn\s+)?((?:{IDENT}::)*{IDENT})")

_rust: list[Path] | None = None
_defs: dict[Path, set[str]] = {}
KEYWORDS = {
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
}


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


def strip_rust(src: str) -> str:
    """`src` without comments (nested block comments included) and with string and char literals emptied.

    Newlines are kept so that line anchors still work.
    """
    out: list[str] = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        two = src[i : i + 2]
        if two == "//":
            while i < n and src[i] != "\n":
                i += 1
        elif two == "/*":
            depth = 0
            while i < n:
                if src.startswith("/*", i):
                    depth += 1
                    i += 2
                elif src.startswith("*/", i):
                    depth -= 1
                    i += 2
                    if depth == 0:
                        break
                else:
                    if src[i] == "\n":
                        out.append("\n")
                    i += 1
        elif (m := RAW_STRING.match(src, i)) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            end = src.find('"' + m.group(1), m.end())
            end = n if end < 0 else end + 1 + len(m.group(1))
            out.append('""' + "\n" * src.count("\n", i, end))
            i = end
        elif c == '"' or (c == "b" and two == 'b"' and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_"))):
            i += 1 if c == '"' else 2
            start = i
            while i < n and src[i] != '"':
                i += 2 if src[i] == "\\" else 1
            out.append('""' + "\n" * src.count("\n", start, i))
            i += 1
        elif c == "'":
            m = CHAR_LITERAL.match(src, i)
            if m:  # a char literal; otherwise a lifetime or label
                out.append("''")
                i = m.end()
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def matching(text: str, open_at: int) -> int:
    """Index just past the bracket that closes the one at `open_at` (or the end of `text`)."""
    pairs = {"{": "}", "(": ")", "[": "]"}
    stack = [pairs[text[open_at]]]
    i = open_at + 1
    while i < len(text) and stack:
        ch = text[i]
        if ch in pairs:
            stack.append(pairs[ch])
        elif ch == stack[-1]:
            stack.pop()
        i += 1
    return i


def angle_delta(text: str, i: int) -> int:
    """+1 for a `<` that opens generics, -1 for a `>` that closes them, else 0.

    A `<` opens generics only right after an identifier or `::` (`Vec<u8>`), so `1 << 2` and `a < b` do not; a `>` that
    is part of `->`, `=>`, `>=` or `>>=` closes nothing.
    """
    ch = text[i]
    if ch == "<":
        return 1 if i > 0 and (text[i - 1].isalpha() or text[i - 1] in "_:") else 0
    if ch == ">":
        return 0 if i > 0 and text[i - 1] in "-=" else -1
    return 0


def top_level_split(body: str) -> list[str]:
    parts, depth, angle, cur, i = [], 0, 0, [], 0
    while i < len(body):
        ch = body[i]
        if body.startswith(("->", "=>"), i):
            cur.append(body[i : i + 2])
            i += 2
            continue
        depth += ch in "({["
        depth -= ch in ")}]"
        angle = max(0, angle + angle_delta(body, i))
        if ch == "," and depth == 0 and angle == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
        i += 1
    parts.append("".join(cur))
    return parts


def impl_names(header: str) -> set[str]:
    """The type and trait names of an `impl` header (text between `impl` and its `{`), minus its own generic parameters."""
    header = WHERE.split(header)[0].strip()
    params: set[str] = set()
    if header.startswith("<"):
        depth, end = 0, 0
        for i, ch in enumerate(header):
            depth += angle_delta(header, i) if i else 1
            if depth == 0:
                end = i + 1
                break
        for entry in top_level_split(header[1 : end - 1]):
            pm = GENERIC_PARAM.match(entry)
            if pm:
                params.add(pm.group(1))
        header = header[end:]
    names = set()
    for side in FOR.split(header):
        m = IMPL_SIDE.match(side)
        if m:
            names.add(m.group(1).split("::")[-1])
    return names - params


ITEM = re.compile(
    r"^[ \t]*(?:#\[[^\]]*\][ \t]*)*(?:pub(?:\([^)]*\))?[ \t]+)?(?:(?:default|const|async|unsafe|extern(?:[ \t]+\"\")?)[ \t]+)*"
    rf"(?:(?P<kw>fn|struct|enum|trait|type|const|mod|union|static(?:[ \t]+mut)?)[ \t]+(?P<name>{IDENT})"
    rf"|macro_rules[ \t]*![ \t]*(?P<mac>{IDENT})"
    r"|impl\b(?P<impl>))",
    re.M,
)


def body_brace(text: str, start: int) -> int:
    """Index of the `{` that opens a braced struct/enum/union body, or -1 for a tuple or unit item."""
    angle, i = 0, start
    while i < len(text):
        ch = text[i]
        if text.startswith("->", i):
            i += 2
            continue
        angle = max(0, angle + angle_delta(text, i))
        if angle == 0 and ch in "{;":
            return i if ch == "{" else -1
        if angle == 0 and ch == "(" and not WHERE.search(text[start:i]):
            return -1
        i += 1
    return -1


def member_names(body: str, is_enum: bool) -> set[str]:
    """Variants (enum) and named fields of a braced body; the fields of a struct-like variant are members too."""
    names: set[str] = set()
    for entry in top_level_split(body):
        entry = PUB.sub("", ATTRIBUTE.sub("", entry).strip())
        em = FIELD.match(entry)
        if not em or not (is_enum or em.group(2)):
            continue
        names.add(em.group(1))
        rest = entry[em.end() :].lstrip()
        if is_enum and rest.startswith("{"):
            names |= member_names(rest[1 : matching(rest, 0) - 1], False)
    return names


def definitions(path: Path) -> set[str]:
    if path not in _defs:
        text = strip_rust(path.read_text(encoding="utf-8", errors="replace"))
        names: set[str] = set()
        for m in ITEM.finditer(text):
            if m.group("impl") is not None:
                brace = text.find("{", m.end())
                semi = text.find(";", m.end())
                if brace >= 0 and (semi < 0 or brace < semi):
                    names |= impl_names(text[m.end() : brace])
                continue
            if m.group("mac"):
                names.add(m.group("mac"))
                continue
            names.add(m.group("name"))
            kw = m.group("kw")
            if kw in ("enum", "struct", "union"):
                brace = body_brace(text, m.end())
                if brace < 0:
                    continue
                names |= member_names(text[brace + 1 : matching(text, brace) - 1], kw == "enum")
        _defs[path] = names
    return _defs[path]


def defined(path: Path, name: str) -> bool:
    return name in definitions(path)


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
        if last in KEYWORDS:
            return f"`{last}` is a Rust keyword, not a symbol (a citation is `{want} symbol`; prose after a file name is read as a symbol)"
        return f"`{last}` is not defined in {want}"
    return f"`{last}` is defined in several files named {want} ({len(hits)}); cite the path"


def check_line(line: str) -> list[str]:
    problems: list[str] = []
    consumed: list[tuple[int, int]] = []
    for m in MENTION.finditer(line):
        tail = line[m.end():]
        sym = SYMBOL.match(tail)
        if sym is None:
            if SYMBOL_WORD.match(tail):
                problems.append(f"{m.group(0).strip('`')}: the symbol must be an identifier path with no suffix")
            elif NUMBER_AFTER.match(tail):
                problems.append(f"{m.group(0).strip('`')}: a line number without a symbol")
            continue
        why = resolve(m.group("name"), sym.group("sym"))
        if why:
            problems.append(why)
            continue
        hint = LINE_HINT.match(tail, sym.end())
        if hint:
            consumed.append((m.end() + hint.start(), m.end() + hint.end()))
        if NUMBER_AFTER.match(tail, hint.end() if hint else sym.end()):
            problems.append("a bare number after the symbol; only ` (~N)` with N >= 1 directly after it is a line hint")
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
