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
     "            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):",
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
    ("no lead into vertices", "step 3",
     "            leads = (nxt.joined and trackable(nxt.s)\n",
     "            leads = False and (nxt.joined and trackable(nxt.s)\n"),
    ("new piece extended back at a vertex", "step 2",
     '        if (segs[0].k0 == k and segs[0].joined and self.prev_piece is not None and not cut_before',
     '        if (False and segs[0].k0 == k and segs[0].joined and self.prev_piece is not None and not cut_before'),
    ("chase gap from the line extended back", "step 4",
     "            gap = line(k) - st.y",
     "            gap = line(k - 1) - st.y"),
    ("re-plan every tick", "step 7",
     '        if not same_key(key, self.key) or exc or cut_before:',
     '        if True:'),
    ("X5 holds instead of continuing", "X5",
     "            if self._continues(st, k - 1):",
     "            if False:"),
    ("one-tick-late braking predicate (v > 0 only)", "step 6",
     "        will_rise = nst.v > 1e-9 or (nst.a > 1e-9 and nst.v + nst.a * nst.a / (2 * t.j_max) > 1e-9)",
     "        will_rise = nst.v > 1e-9"),
    ("acceleration and jerk from the falling rate", "the mapping",
     "        return 2 * self.r_up / self.ease",
     "        return 2 * self.r_dn / self.ease"),
    # The round-5 falsifier's survivors (PR #625), each killed since round 6:
    ("lead from up to 3 EPS off the line", "step 3",
     "                     and ((nxt.s < s_cur and abs(st.y - (r_k - s_cur * DT)) <= EPS)",
     "                     and ((nxt.s < s_cur and abs(st.y - (r_k - s_cur * DT)) <= 3 * EPS)"),
    ("lead velocity not capped", "step 3",
     "            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))",
     "            cap_v = lambda: (t.r_up, -t.r_dn)"),
    ("lead one tick late", "step 3",
     "<= self.probe.duration / 2 + DT + 1e-9:",
     "<= self.probe.duration / 2 + 1e-9:"),
    ("lead two ticks early", "step 3",
     "<= self.probe.duration / 2 + DT + 1e-9:",
     "<= self.probe.duration / 2 + 3 * DT + 1e-9:"),
    ("will-rise with a²/(4j)", "step 6",
     "nst.v + nst.a * nst.a / (2 * t.j_max) > 1e-9)",
     "nst.v + nst.a * nst.a / (4 * t.j_max) > 1e-9)"),
    ("chase only strictly behind the line (round 5's rule)", "step 4",
     '            vel_target = t.r_up if (gap > -EPS and fs > 0) else (-t.r_dn if (gap < EPS and fs < 0) else 0.0)',
     '            vel_target = t.r_up if (gap > 0 and fs > 0) else (-t.r_dn if (gap < 0 and fs < 0) else 0.0)'),
    ("trackable at exactly the rate", "step 4",
     "        trackable = lambda s: -t.r_dn < s < t.r_up",
     "        trackable = lambda s: -t.r_dn <= s <= t.r_up"),
    ("_continues tolerance 10", "X5 step 2",
     "        return abs(y - st.y) < 1e-6 and abs(v - st.v) < 1e-5 and abs(a - st.a) < 1e-4",
     "        return abs(y - st.y) < 10 and abs(v - st.v) < 10 and abs(a - st.a) < 10"),
    ("braking-plan failure without its X5 tag", "X5",
     "                exc = exc or \"X5\"; self.x5_tick = True   # the step stands; X2 below still bounds it",
     "                pass   # the step stands; X2 below still bounds it"),
    ("X4 keeps the slope", "step 8",
     "            nst, exc = _cut(nst, State(0.0, 0.0, 0.0), \"X4\", exc)",
     "            nst, exc = _cut(nst, State(0.0, nst.v, nst.a), \"X4\", exc)"),
    # Round 6's own rules:
    ("no tightened twin calculation (X5 step 1)", "X5 step 1",
     "        ok_t = self._one(self.alt, st, rel, fs, vmax, vmin, vel_target, RETRY_SCALE, injected)",
     "        ok_t = False"),
    ("keep the nominal answer even when slower", "X5 step 1",
     "        if not ok_n or d_t < d_n - (1e-6 * d_n + 1e-9):   # refused, or materially slower: use the tightened one",
     "        if not ok_n:"),
    ("no closed-form fallback (X5 step 3)", "X5 step 3",
     "            elif at_rest and vel_target is None and fs == 0.0 and not exc:",
     "            elif False:"),
    ("no collinear merge", "the line on the tick grid",
     "        if lines and seg.joined and collinear(seg.s, lines[-1].s):",
     "        if False:"),
    ("keys compared exactly", "step 7",
     "        if not same_key(key, self.key) or exc or cut_before:",
     "        if key != self.key or exc or cut_before:"),
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
