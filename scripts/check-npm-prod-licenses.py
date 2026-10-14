#!/usr/bin/env python3
"""Every production npm dependency of `app` must carry an allow-listed licence (#233).

Usage: pnpm -C app licenses list --prod --json | scripts/check-npm-prod-licenses.py
Needs installed node_modules (`pnpm install --frozen-lockfile`). Development-only packages are not
distributed and are not checked (AGENTS.md).
"""
import json
import re
import sys

ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "0BSD", "BlueOak-1.0.0"}

groups = json.load(sys.stdin)
bad = []
count = 0
for expr, pkgs in groups.items():
    # An SPDX expression passes if every OR-alternative group has an allowed member.
    ok = all(
        any(part.strip("() ") in ALLOWED for part in re.split(r"\s+OR\s+", clause))
        for clause in re.split(r"\s+AND\s+", expr)
    )
    for pkg in pkgs:
        count += 1
        if not ok:
            bad.append(f"{pkg['name']}@{','.join(pkg['versions'])}: {expr}")
if not count:
    sys.exit("npm-licenses: no production packages found (is node_modules installed?)")
if bad:
    sys.exit("npm-licenses: licence not on the allow-list:\n  " + "\n  ".join(bad))
print(f"npm-licenses: ok ({count} production packages)")
