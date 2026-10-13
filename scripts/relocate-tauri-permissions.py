"""Relocate Tauri's absolute ACL paths when restoring crane artifacts.

Cargo relocates build-script metadata using root-output, but cannot relocate
paths embedded inside Tauri's JSON permission lists. Preserve cached code and
rebase those generated paths to the restored OUT_DIR before Cargo consumes it.
"""
import json
import os
from pathlib import Path

for marker in Path(os.environ.get("CARGO_TARGET_DIR", "target")).glob("*/build/tauri-*/root-output"):
    previous = Path(marker.read_text().strip())
    current = (marker.parent / "out").resolve()
    for manifest in current.glob("*-permission-files"):
        paths = [Path(p) for p in json.loads(manifest.read_text())]
        relocated = [current / p.relative_to(previous) if p.is_relative_to(previous) else p for p in paths]
        assert all(p.is_file() for p in relocated), f"Missing cached Tauri permission: {manifest}"
        manifest.write_text(json.dumps([str(p) for p in relocated]))
