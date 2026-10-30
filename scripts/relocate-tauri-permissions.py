"""Relocate Tauri's absolute ACL paths when restoring crane artifacts.

Cargo relocates build-script metadata using root-output, but cannot relocate
paths embedded inside Tauri's JSON permission lists. Preserve cached code and
rebase those generated paths to the restored OUT_DIR before Cargo consumes it.

Artifacts can be restored more than once in a chain (deps-only build, then the shared workspace
build, then the checks, #371). Cargo's `root-output` keeps naming the directory the build script
ran in, but this script has already moved the manifest to the later restore. `relocated-from`
(next to `root-output`) records where the manifest points now; it wins over `root-output`.
"""
import json
import os
from pathlib import Path

for marker in Path(os.environ.get("CARGO_TARGET_DIR", "target")).glob("*/build/tauri-*/root-output"):
    moved = marker.parent / "relocated-from"
    previous = Path((moved if moved.exists() else marker).read_text().strip())
    current = (marker.parent / "out").resolve()
    for manifest in current.glob("*-permission-files"):
        paths = [Path(p) for p in json.loads(manifest.read_text())]
        relocated = [current / p.relative_to(previous) if p.is_relative_to(previous) else p for p in paths]
        assert all(p.is_file() for p in relocated), f"Missing cached Tauri permission: {manifest}"
        manifest.write_text(json.dumps([str(p) for p in relocated]))
    moved.write_text(str(current))
