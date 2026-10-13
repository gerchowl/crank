#!/usr/bin/env python3
"""Regression test for scripts/check-window-config.py (#170).

The real tree must pass; each deliberate mismatch (sizes.json vs DESIGN §8,
sizes.json vs tauri.conf.json, a missing DESIGN row) must fail loudly.
Usage: test-window-config.py [root [check-window-config.py]]
"""
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
check = Path(sys.argv[2] if len(sys.argv) > 2 else Path(__file__).resolve().parent / "check-window-config.py").resolve()
FILES = ["docs/design/DESIGN.md", "docs/design/sizes.json", "app/src-tauri/tauri.conf.json"]


def run(tree: Path) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, "-I", str(check), str(tree)], capture_output=True, text=True)


def copy_tree(dest: Path) -> Path:
    for f in FILES:
        (dest / f).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / f, dest / f)  # not copy(): store files are read-only
    return dest


def expect(name: str, tree: Path, ok: bool) -> None:
    r = run(tree)
    if (r.returncode == 0) != ok:
        sys.exit(f"test-window-config: {name}: expected {'pass' if ok else 'failure'}, got exit {r.returncode}\n{r.stdout}{r.stderr}")
    print(f"test-window-config: {name}: ok")


with tempfile.TemporaryDirectory() as tmp:
    base = Path(tmp)
    expect("real tree passes", copy_tree(base / "real"), True)

    t = copy_tree(base / "sizes-vs-design")
    p = t / "docs/design/sizes.json"
    data = json.loads(p.read_text())
    data["sizes"][1]["width"] -= 24
    p.write_text(json.dumps(data))
    expect("sizes.json disagrees with DESIGN and tauri.conf", t, False)

    t = copy_tree(base / "design-row-missing")
    p = t / "docs/design/DESIGN.md"
    text = p.read_text()
    assert "| **1024×768**" in text
    p.write_text(text.replace("| **1024×768**", "| 1024×768", 1))
    expect("DESIGN row no longer matches sizes.json", t, False)

    t = copy_tree(base / "conf-min")
    p = t / "app/src-tauri/tauri.conf.json"
    conf = json.loads(p.read_text())
    conf["app"]["windows"][0]["minWidth"] += 1
    p.write_text(json.dumps(conf))
    expect("tauri.conf.json minWidth drift", t, False)
print("test-window-config: ok")
