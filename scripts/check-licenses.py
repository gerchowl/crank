#!/usr/bin/env python3
"""Every tracked package.json must declare "license": "Apache-2.0" (#125).

Usage: check-licenses.py [root]  (tracked files via git, else a tree walk)
"""
import json
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
try:
    names = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True,
                           capture_output=True, text=True).stdout.split("\0")
    files = [root / n for n in names if n == "package.json" or n.endswith("/package.json")]
except (OSError, subprocess.CalledProcessError):
    files = [p for p in root.rglob("package.json") if "node_modules" not in p.parts]
bad = [str(p) for p in files if json.loads(p.read_text(encoding="utf-8")).get("license") != "Apache-2.0"]
if not files:
    sys.exit("licenses: no package.json found")
if bad:
    sys.exit("licenses: missing \"license\": \"Apache-2.0\" in:\n  " + "\n  ".join(bad))
print(f"licenses: ok ({len(files)} package.json files)")
