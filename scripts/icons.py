#!/usr/bin/env python3
"""App icon provenance (#121): the Tauri icon set is generated from the /C/ mark.

`icons.py record` writes app/src-tauri/icons/MANIFEST.sha256: the SHA-256 of the
1024 px source followed by one line per generated file. `icons.py check` (the
`icons-drift` flake check) fails when the source or any generated icon no longer
matches, i.e. when one was edited without running `just icons`.
"""
import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = Path("assets/icon/icon-1024.png")
ICONS = Path("app/src-tauri/icons")
MANIFEST = ICONS / "MANIFEST.sha256"


def digest(path: Path) -> str:
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def tracked() -> list[Path]:
    generated = sorted(p.relative_to(ROOT) for p in (ROOT / ICONS).iterdir() if p.is_file() and p.name != MANIFEST.name)
    return [SOURCE, *generated]


def main(mode: str) -> int:
    if mode == "record":
        lines = [f"{digest(p)}  {p}\n" for p in tracked()]
        (ROOT / MANIFEST).write_text("".join(lines))
        return 0
    if mode == "check":
        recorded = {}
        for line in (ROOT / MANIFEST).read_text().splitlines():
            h, _, name = line.partition("  ")
            recorded[Path(name)] = h
        errors = []
        for p in tracked():
            if p not in recorded:
                errors.append(f"{p}: not in {MANIFEST}")
            elif recorded[p] != digest(p):
                errors.append(f"{p}: differs from the recorded hash")
        errors += [f"{p}: recorded but missing" for p in recorded if p not in tracked()]
        if errors:
            print("icon drift:", *errors, "Run `just icons` and commit the result.", sep="\n  ")
            return 1
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else ""))
