#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585, track 2).

Each mutant deletes or breaks one of the rules of §7.10.2 in `erg_slew.py` (a copy in a
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
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
SRC = HERE / "erg_slew.py"

# (name, rule of §7.10.2 it breaks, exact text, replacement). Track 2 (2026-11-15): the rules are
# the target, the limits, the hard clamp, the refusal path (N_ERRORS) and the clamp chain.
MUTANTS = [
    # The clamp chain's order (jerk, then acceleration, then rate).
    ("chain order: rate, acceleration, jerk", "the clamp chain",
     'order=("jerk", "acc", "vel")) -> tuple[float, bool]:', 'order=("vel", "acc", "jerk")) -> tuple[float, bool]:'),
    ("chain order: acceleration, jerk, rate", "the clamp chain",
     'order=("jerk", "acc", "vel")) -> tuple[float, bool]:', 'order=("acc", "jerk", "vel")) -> tuple[float, bool]:'),
    ("chain jerk as the reference's 2·j·dt²", "the clamp chain",
     "            da = min(max(dv - dv0, -t.j_max * dt * dt), t.j_max * dt * dt)",
     "            da = min(max(dv - dv0, -t.j_max * 2 * dt * dt), t.j_max * 2 * dt * dt)"),
    ("no rate clamp in the chain", "the clamp chain",
     "        else:\n            v = min(max(v, -t.r_dn), t.r_up)\n        if step",
     "        else:\n            v = v\n        if step"),
    ("chain linear zone over τ (rings)", "the chain's reference",
     "min(v_stop, e / (2 * t.ease))", "min(v_stop, e / t.ease)"),
    ("no jerk-aware aim in the chain", "the chain's reference",
     "    return v_ref - a0 * abs(a0) / (2 * t.j_max)", "    return v_ref"),
    ("no curvature clip at the hand-off", "the hand-off",
     "    if a > 0 and v + a * a / (2 * t.j_max) > t.r_up:", "    if False:"),
    ("chain history not restarted at the clamp", "the hard clamp",
     "            self.traj = None\n            self.hist = []", "            self.traj = None"),
    # N: refusals answered from the trajectory in force.
    ("N_ERRORS 2", "the refusal path", "N_ERRORS = 3 ", "N_ERRORS = 2 "),
    ("N_ERRORS 4", "the refusal path", "N_ERRORS = 3 ", "N_ERRORS = 4 "),
    ("the trajectory in force for N + 1 refusals", "the refusal path",
     "            if self.errors <= self.n_errors and self.traj is not None:",
     "            if self.errors <= self.n_errors + 1 and self.traj is not None:"),
    ("the chain at once (no trajectory in force)", "the refusal path",
     "            if self.errors <= self.n_errors and self.traj is not None:",
     "            if False:"),
    # The target.
    ("target velocity 0", "the target", "    return r, min(max(s, -t.r_dn * TARGET_SCALE), t.r_up * TARGET_SCALE)", "    return r, 0.0"),
    ("target velocity not clamped to the rates", "the target", "    return r, min(max(s, -t.r_dn * TARGET_SCALE), t.r_up * TARGET_SCALE)", "    return r, s"),
    ("target velocity clamped exactly at the rates", "the target", "TARGET_SCALE = 1 - 2 ** -20 ", "TARGET_SCALE = 1 "),
    ("target not clamped to [0, M]", "the target", "    if r <= 0.0 or r >= m:\n        return clamp(r, m), 0.0", "    if False:\n        return clamp(r, m), 0.0"),
    ("look-ahead 1", "the target", "LOOKAHEAD = 2 ", "LOOKAHEAD = 1 "),
    ("no minimum duration", "the target", "MIN_DURATION = True ", "MIN_DURATION = False "),
    # The limits handed to Ruckig.
    ("rates × 1.05", "the limits", "        i.max_velocity, i.min_velocity = [t.r_up], [-t.r_dn]",
     "        i.max_velocity, i.min_velocity = [t.r_up * 1.05], [-t.r_dn * 1.05]"),
    ("acceleration × 1.05", "the limits", "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max]",
     "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max * 1.05], [-t.a_max * 1.05], [t.j_max]"),
    ("jerk × 1.05", "the limits", "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max]",
     "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max * 1.05]"),
    ("acceleration × 0.95", "the limits", "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max]",
     "        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max * 0.95], [-t.a_max * 0.95], [t.j_max]"),
    ("falling rate as the rising", "the limits", "        i.max_velocity, i.min_velocity = [t.r_up], [-t.r_dn]",
     "        i.max_velocity, i.min_velocity = [t.r_up], [-t.r_up]"),
    # The hard clamp.
    ("no hard clamp", "the hard clamp", "        if nst.y > m or nst.y < 0.0:", "        if False:"),
    ("hard clamp at M + 1", "the hard clamp", "        if nst.y > m or nst.y < 0.0:", "        if nst.y > m + 1 or nst.y < 0.0:"),
    ("no clamp at 0 W", "the hard clamp", "        if nst.y > m or nst.y < 0.0:", "        if nst.y > m:"),
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
        jobs = []
        for i, (name, rule, old, new) in enumerate(MUTANTS):
            if src.count(old) != 1:
                jobs.append((name, rule, None))
                continue
            path = pathlib.Path(tmp) / f"m{i}" / "erg_slew.py"
            path.parent.mkdir()
            path.write_text(src.replace(old, new), encoding="utf-8")
            jobs.append((name, rule, path))
        with ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(lambda j: run(j[2], n) if j[2] else None, jobs))
        for (name, rule, path), res in zip(jobs, results):
            if res is None:
                rows.append((name, rule, "-", "NOT APPLIED (text not found once)", ""))
                ok = False
                continue
            code, first = res
            killed = code != 0
            ok &= killed
            rows.append((name, rule, code, "killed" if killed else "SURVIVED", first))
    print("| Mutant | Rule | Exit | Result | First line |\n|---|---|---|---|---|")
    for r in rows:
        print("| " + " | ".join(str(x) for x in r) + " |")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
