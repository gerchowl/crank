#!/usr/bin/env python3
"""Unit tests for the SPDX handling in check-npm-prod-licenses.py (#233)."""
import importlib.util
import pathlib
import sys

spec = importlib.util.spec_from_file_location(
    "check_npm", pathlib.Path(__file__).with_name("check-npm-prod-licenses.py"))
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
ok = mod.licence_ok

CASES = [
    ("MIT", True),
    ("Apache-2.0 OR MIT", True),
    ("(GPL-3.0 AND MIT) OR Apache-2.0", True),   # passes only via Apache-2.0
    ("GPL-3.0 AND MIT", False),
    ("MIT AND Apache-2.0", True),
    ("GPL-3.0 OR LGPL-3.0", False),
    ("GPL-3.0 OR MIT AND ISC", True),             # AND binds tighter: GPL-3.0 OR (MIT AND ISC)
    ("(GPL-3.0 OR MIT) AND ISC", True),
    ("(GPL-3.0 OR MIT) AND GPL-2.0", False),
    ("GPL-3.0 AND MIT OR GPL-2.0", False),        # (GPL-3.0 AND MIT) OR GPL-2.0
    ("MIT WITH Classpath-exception-2.0", True),
    ("GPL-3.0 WITH Classpath-exception-2.0", False),
    ("mit OR gpl-3.0", False),                    # identifiers are case-sensitive; operators are not
    ("Apache-2.0 or GPL-3.0", True),
    ("mit", False),                               # a deliberate safe-side policy, see check-npm-prod-licenses.py
    ("UNKNOWN", False),
    ("SEE LICENSE IN LICENSE.md", False),
    ("MIT OR", False),
    ("(MIT", False),
    ("MIT)", False),
    ("", False),
    ("MIT MIT", False),
]
failed = [f"{expr!r}: want {want}" for expr, want in CASES if ok(expr) is not want]
if failed:
    sys.exit("npm-licenses test failures:\n  " + "\n  ".join(failed))
print(f"npm-licenses test: ok ({len(CASES)} cases)")
