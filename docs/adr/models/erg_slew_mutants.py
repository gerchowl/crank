#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585).

Each mutant deletes or breaks one of crank's rules around Ruckig in `erg_slew.py` (a copy in a
temporary directory; the file in the repo is never touched), then runs the model's whole suite
with a reduced random-plan count. A mutant is KILLED when the suite exits non-zero. The runner
exits 0 only if every mutant is killed and the unmutated model passes.

Run with the interpreter that has Ruckig (see erg_slew.py):
    .venv/bin/python docs/adr/models/erg_slew_mutants.py [--random N]
`just slew-mutants` does the same.
"""
from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
SRC = HERE / "erg_slew.py"

# (name, rule of §7.10.2 it removes, exact text, replacement)
MUTANTS = [
    ("no safe braking", "step 6",
     "        if (not brake and ((will_rise and not self._brake_ok(nst, k, segs, m))",
     "        if False and (not brake and ((will_rise and not self._brake_ok(nst, k, segs, m))"),
    ("no floor braking", "step 6 (0 W)",
     "                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):",
     "                           or (False and nst.v < -1e-9 and not self._floor_ok(nst)))):"),
    ("no hold above a rising line", "step 5",
     "        elif trackable(fs) and fs > 0 and st.y - line(k) > EPS:",
     "        elif False:"),
    ("braking may pass the cap by EPS", "step 6",
     "            if y > min(line_at(segs, k + i, m) + EPS / 2, m):",
     "            if y > line_at(segs, k + i, m) + EPS / 2:"),
    ("no X2 after the step", "step 8",
     "        if nst.y > r_k + EPS and (nst.v > 1e-9 or nst.y > st.y + 1e-7):",
     "        if False:"),
    ("no X3 before the step", "step 1",
     "        if st.y > m:                                     # X3: the cap is never eased",
     "        if False:"),
    ("no X3 after the step", "step 8",
     "        if nst.y > m:\n            nst, exc = _cut(nst, State(m, 0.0, 0.0), \"X3\", exc)",
     "        if False:\n            nst, exc = _cut(nst, State(m, 0.0, 0.0), \"X3\", exc)"),
    ("no 0 W floor (X4)", "step 8",
     "        if nst.y < 0:                                    # X4: nothing is below 0 W",
     "        if False:"),
    ("lead also from above the line", "step 3",
     "                     and st.y <= r_k - s_cur * DT + EPS)   # the state is at tick k-1: compare with the line there",
     "                     and True)   # the state is at tick k-1: compare with the line there"),
    ("no lead into vertices", "step 3",
     "            leads = (nxt.joined and trackable(nxt.s) and",
     "            leads = False and (nxt.joined and trackable(nxt.s) and"),
    ("new piece extended back at a vertex", "step 2",
     "                and abs(self.prev_piece.at(k) - segs[0].r0) < 1e-9):",
     "                and False):"),
    ("chase gap from the line extended back", "step 4",
     "            gap = line(k) - st.y",
     "            gap = line(k - 1) - st.y"),
    ("re-plan every tick", "step 7",
     "        if key != self.key or exc or cut_before:",
     "        if True:"),
    ("X5 holds instead of continuing", "X5",
     "            if self._continues(st, k - 1):",
     "            if False:"),
    ("one-tick-late braking predicate (v > 0 only)", "step 6",
     "        will_rise = nst.v > 1e-9 or (nst.a > 1e-9 and nst.v + nst.a * nst.a / (2 * t.j_max) > 1e-9)",
     "        will_rise = nst.v > 1e-9"),
    ("acceleration and jerk from the falling rate", "the mapping",
     "        return 2 * self.r_up / self.ease",
     "        return 2 * self.r_dn / self.ease"),
]


def run(path: pathlib.Path, n: int) -> tuple[int, str]:
    p = subprocess.run([sys.executable, str(path), "--random", str(n)], capture_output=True, text=True)
    out = (p.stdout + p.stderr).strip().splitlines()
    return p.returncode, (out[0] if out else "")[:120]


def main(argv) -> int:
    n = int(argv[argv.index("--random") + 1]) if "--random" in argv else 400
    src = SRC.read_text(encoding="utf-8")
    rows, ok = [], True
    code, first = run(SRC, n)
    rows.append(("(none: the model as committed)", "-", code, "passes" if code == 0 else "FAILS", first))
    ok &= code == 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, rule, old, new in MUTANTS:
            if src.count(old) != 1:
                rows.append((name, rule, "-", "NOT APPLIED (text not found once)", ""))
                ok = False
                continue
            path = pathlib.Path(tmp) / "erg_slew.py"
            path.write_text(src.replace(old, new), encoding="utf-8")
            code, first = run(path, n)
            killed = code != 0
            ok &= killed
            rows.append((name, rule, code, "killed" if killed else "SURVIVED", first))
    print(f"| Mutant | Rule | Exit | Result | First line |\n|---|---|---|---|---|")
    for r in rows:
        print("| " + " | ".join(str(x) for x in r) + " |")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
