#!/usr/bin/env python3
"""Every production npm dependency of `app` must carry an allow-listed licence (#233).

Usage: pnpm -C app licenses list --prod --json | scripts/check-npm-prod-licenses.py
Needs installed node_modules (`pnpm install --frozen-lockfile`). Development-only packages are not
distributed and are not checked (AGENTS.md).

Licence strings are SPDX expressions. `allowed(expr)` parses them with the standard precedence
(`WITH` binds tightest, then `AND`, then `OR`; parentheses group) and is true when the licensee can
satisfy it using only allow-listed licences: an `OR` needs one allowed side, an `AND` needs both.
Anything that does not parse (`UNKNOWN`, `SEE LICENSE IN ...`, free text) is not allowed.
"""
import json
import re
import sys

ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "0BSD", "BlueOak-1.0.0"}

_TOKEN = re.compile(r"\s*(\(|\)|[A-Za-z0-9.+\-_]+)")


def _tokens(expr):
    pos, out = 0, []
    expr = expr.strip()
    while pos < len(expr):
        m = _TOKEN.match(expr, pos)
        if not m:
            raise ValueError(f"unexpected character at {pos} in {expr!r}")
        out.append(m.group(1))
        pos = m.end()
    return out


def allowed(expr, allow=frozenset(ALLOWED)):
    """True when the SPDX expression can be satisfied using only licences in `allow`."""
    toks = _tokens(expr)
    pos = 0

    def peek():
        return toks[pos].upper() if pos < len(toks) else None

    def take():
        nonlocal pos
        tok = toks[pos]
        pos += 1
        return tok

    def or_expr():
        value = and_expr()
        while peek() == "OR":
            take()
            rhs = and_expr()  # evaluate both sides so a parse error is never short-circuited away
            value = value or rhs
        return value

    def and_expr():
        value = atom()
        while peek() == "AND":
            take()
            rhs = atom()
            value = value and rhs
        return value

    def atom():
        if pos >= len(toks):
            raise ValueError(f"unexpected end of {expr!r}")
        tok = take()
        if tok == "(":
            value = or_expr()
            if pos >= len(toks) or take() != ")":
                raise ValueError(f"missing ')' in {expr!r}")
            return value
        if tok == ")" or tok.upper() in ("AND", "OR", "WITH"):
            raise ValueError(f"unexpected {tok!r} in {expr!r}")
        lic = tok.rstrip("+")  # `X+` (or later): allowed when X is
        if peek() == "WITH":  # an exception only adds permissions to the licence
            take()
            if pos >= len(toks) or toks[pos] in "()" or toks[pos].upper() in ("AND", "OR", "WITH"):
                raise ValueError(f"missing exception after WITH in {expr!r}")
            take()
        # Deliberately case-sensitive, unlike SPDX matching (which ignores case): `mit` is not
        # on the list. A package that spells its licence unusually fails the gate and gets a
        # human look, the safe side for a deny-by-default allow-list; the operators AND/OR/WITH
        # are matched case-insensitively. Pinned by the `mit OR gpl-3.0` case of the unit test.
        return lic in allow

    value = or_expr()
    if pos != len(toks):
        raise ValueError(f"trailing {toks[pos]!r} in {expr!r}")
    return value


def licence_ok(expr, allow=frozenset(ALLOWED)):
    """`allowed`, with a malformed expression counted as not allowed."""
    try:
        return allowed(expr, allow)
    except ValueError:
        return False


def main():
    groups = json.load(sys.stdin)
    bad, count = [], 0
    for expr, pkgs in groups.items():
        ok = licence_ok(expr)
        for pkg in pkgs:
            count += 1
            if not ok:
                bad.append(f"{pkg['name']}@{','.join(pkg['versions'])}: {expr}")
    if not count:
        sys.exit("npm-licenses: no production packages found (is node_modules installed?)")
    if bad:
        sys.exit("npm-licenses: licence not on the allow-list:\n  " + "\n  ".join(bad))
    print(f"npm-licenses: ok ({count} production packages)")


if __name__ == "__main__":
    main()
