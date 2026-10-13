#!/usr/bin/env python3
"""Assert the Tauri window config against docs/design/sizes.json (#123, #170).

sizes.json is the structured source of the supported window sizes; the first
entry is the primary size (the default window) and the smallest is the minimum.
DESIGN.md §8 renders the same list as a table and must agree with it exactly;
tauri.conf.json must match the default and the minimum. Nothing is hard-coded.
Usage: check-window-config.py [root]
"""
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
errors = []

entries = json.loads((root / "docs/design/sizes.json").read_text(encoding="utf-8")).get("sizes", [])
sizes = [(e["width"], e["height"]) for e in entries]
if len(sizes) < 2:
    sys.exit("window-config: docs/design/sizes.json needs at least two sizes")
default, minimum = sizes[0], min(sizes, key=lambda s: s[0] * s[1])

design = (root / "docs/design/DESIGN.md").read_text(encoding="utf-8")
if "## 8. Window sizes" not in design:
    sys.exit("window-config: DESIGN.md has no '## 8. Window sizes' section")
section = design.split("## 8. Window sizes", 1)[1].split("\n## ", 1)[0].split("\n---", 1)[0]
documented = [(int(w), int(h)) for w, h in re.findall(r"^\| \*\*(\d+)×(\d+)\*\*", section, re.M)]
if documented != sizes:
    errors.append(f"DESIGN §8 lists {documented}, docs/design/sizes.json has {sizes}")

conf = json.loads((root / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
expected = {"width": default[0], "height": default[1], "minWidth": minimum[0], "minHeight": minimum[1]}
errors += [
    f"window[{i}].{k}: expected {v}, found {win.get(k)}"
    for i, win in enumerate(conf["app"]["windows"])
    for k, v in expected.items()
    if win.get(k) != v
]
if errors:
    sys.exit("window-config: sizes disagree:\n  " + "\n  ".join(errors))
print(f"window-config: ok (default {default[0]}x{default[1]}, min {minimum[0]}x{minimum[1]})")
