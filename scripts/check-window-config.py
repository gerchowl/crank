#!/usr/bin/env python3
"""Assert the Tauri window config against docs/design/DESIGN.md §8 (#123).

DESIGN §8 lists the supported window sizes; the first row is the primary size
(the default window) and the smallest row is the minimum. Nothing is
hard-coded here: both are read from the table.
Usage: check-window-config.py [root]
"""
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
design = (root / "docs/design/DESIGN.md").read_text(encoding="utf-8")
section = design.split("## 8. Window sizes", 1)[1].split("\n## ", 1)[0].split("\n---", 1)[0]
sizes = [(int(w), int(h)) for w, h in re.findall(r"^\| \*\*(\d+)×(\d+)\*\*", section, re.M)]
if len(sizes) < 2:
    sys.exit("window-config: could not read sizes from DESIGN §8")
default, minimum = sizes[0], min(sizes, key=lambda s: s[0] * s[1])

conf = json.loads((root / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
windows = conf["app"]["windows"]
expected = {"width": default[0], "height": default[1], "minWidth": minimum[0], "minHeight": minimum[1]}
errors = [
    f"window[{i}].{k}: expected {v}, found {win.get(k)}"
    for i, win in enumerate(windows)
    for k, v in expected.items()
    if win.get(k) != v
]
if errors:
    sys.exit("window-config: tauri.conf.json disagrees with DESIGN §8:\n  " + "\n  ".join(errors))
print(f"window-config: ok (default {default[0]}x{default[1]}, min {minimum[0]}x{minimum[1]})")
