#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585; round 10, the hybrid).

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
from concurrent.futures import ThreadPoolExecutor

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
    ("_continues tolerance 10", "X5",
     "        return abs(y - st.y) < 1e-6 and abs(v - st.v) < 1e-5 and abs(a - st.a) < 1e-4",
     "        return abs(y - st.y) < 10 and abs(v - st.v) < 10 and abs(a - st.a) < 10"),
    ("braking-plan failure without its X5 tag", "X5",
     "                exc = exc or \"X5\"; self.x5_tick = True   # the step stands; X2 below still bounds it",
     "                pass   # the step stands; X2 below still bounds it"),
    ("X4 keeps the slope", "step 8",
     "            nst, exc = _cut(nst, State(0.0, 0.0, 0.0), \"X4\", exc)",
     "            nst, exc = _cut(nst, State(0.0, nst.v, nst.a), \"X4\", exc)"),
    # Round 6's own rules:
    ("no collinear merge", "the line on the tick grid",
     "        if lines and seg.joined and collinear(seg.s, lines[-1].s):",
     "        if False:"),
    ("keys compared exactly", "step 7",
     "        if not same_key(key, self.key) or exc or cut_before:",
     "        if key != self.key or exc or cut_before:"),
    # The round-6 falsifier's survivors (PR #625, round 7):
    ("hold above a rising line at 2 EPS", "step 5",
     "        elif trackable(fs) and fs > 0 and st.y - line(k) > EPS:",
     "        elif trackable(fs) and fs > 0 and st.y - line(k) > 2 * EPS:"),
    ("SLOPE_TOL 1e-3", "the line on the tick grid", "SLOPE_TOL = 1e-9 ", "SLOPE_TOL = 1e-3 "),
    ("SLOPE_TOL 1e-6", "the line on the tick grid", "SLOPE_TOL = 1e-9 ", "SLOPE_TOL = 1e-6 "),
    ("merged slope not recomputed", "the line on the tick grid",
     "            lines[-1] = Seg(q.k0, q.r0, (end - q.r0) / (n * DT), n, q.joined)",
     "            lines[-1] = Seg(q.k0, q.r0, q.s, n, q.joined)"),
    ("same_key tolerance 1e-3", "step 7",
     "        return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))",
     "        return abs(a - b) <= 1e-3 * max(1.0, abs(a), abs(b))"),
    ("continuation not tagged X5", "X5",
     "                nst = State(base + p, v, a) if vel_mode else State(fl(k) + p, v + fl.fs, a)\n                exc = exc or \"X5\"",
     "                nst = State(base + p, v, a) if vel_mode else State(fl(k) + p, v + fl.fs, a)"),
    # Round 7's own rules:
    ("merge across a jump (no VALUE_TOL)", "the line on the tick grid",
     "                and abs(out[-1].at(p.k0) - p.r0) <= VALUE_TOL):",
     "                and True):"),
    ("continuation past the trajectory's end", "X5",
     "        if self.elapsed + DT > self.traj.duration + 1e-9:\n            return False",
     "        if False:\n            return False"),
    ("a clamped jump read as joined", "the line on the tick grid",
     "                joined = line.joined and last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL",
     "                joined = line.joined"),
    # Round 7's falsifier survivors (round 10, where the rule still exists):
    ("snap threshold 1e-12 W/s absolute", "the line on the tick grid",
     "0.0 if collinear(q.s, 0.0) else q.s", "0.0 if abs(q.s) <= 1e-12 else q.s"),
    ("snap threshold 1e-3 W/s absolute", "the line on the tick grid",
     "0.0 if collinear(q.s, 0.0) else q.s", "0.0 if abs(q.s) <= 1e-3 else q.s"),
    ("snap threshold 0.05 W/s absolute", "the line on the tick grid",
     "0.0 if collinear(q.s, 0.0) else q.s", "0.0 if abs(q.s) <= 0.05 else q.s"),
    ("no merge after the clamp", "the line on the tick grid",
     "    return [p for p in merge_collinear(out) if p.k0 + p.n > k]", "    return [p for p in out if p.k0 + p.n > k]"),
    ("raw join tolerance 1e-3", "the line on the tick grid",
     "abs(b.frm * mult - prev_end) < 1e-9", "abs(b.frm * mult - prev_end) < 1e-3"),
    ("raw join tolerance 1.0", "the line on the tick grid",
     "abs(b.frm * mult - prev_end) < 1e-9", "abs(b.frm * mult - prev_end) < 1.0"),
    ("post-clamp join ignores the raw join", "the line on the tick grid",
     "joined = line.joined and last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL",
     "joined = last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL"),
    ("drop before merge (blocker 1: the line depends on the tick that asks)", "the line on the tick grid",
     "            out.append(piece)\n            last, kk = piece, end\n    # Merge first, then drop the pieces that end at or before tick k: dropping first made a merged\n    # piece's start depend on the tick that asked (round-7 falsifier, blocker 1).\n    return [p for p in merge_collinear(out) if p.k0 + p.n > k]",
     "            if end > k:\n                out.append(piece)\n            last, kk = piece, end\n    return merge_collinear(out)"),
    # X5, round 10: track 2's fallback in place of the recovery chain.
    ("N_ERRORS 2", "X5", "N_ERRORS = 3 ", "N_ERRORS = 2 "),
    ("N_ERRORS 4", "X5", "N_ERRORS = 3 ", "N_ERRORS = 4 "),
    ("the trajectory in force for one refusal fewer", "X5",
     "            if self.fail_run <= N_ERRORS and self._continues(st, k - 1):",
     "            if self.fail_run < N_ERRORS and self._continues(st, k - 1):"),
    ("no trajectory in force (the chain at once)", "X5",
     "            if self.fail_run <= N_ERRORS and self._continues(st, k - 1):",
     "            if False:"),
    ("chain order: rate, acceleration, jerk", "X5 (the clamp chain)",
     'order=("jerk", "acc", "vel")) -> tuple[float, bool]:', 'order=("vel", "acc", "jerk")) -> tuple[float, bool]:'),
    ("chain order: acceleration, jerk, rate", "X5 (the clamp chain)",
     'order=("jerk", "acc", "vel")) -> tuple[float, bool]:', 'order=("acc", "jerk", "vel")) -> tuple[float, bool]:'),
    ("chain jerk as the reference's 2·j·dt²", "X5 (the clamp chain)",
     "            da = min(max(dv - dv0, -t.j_max * dt * dt), t.j_max * dt * dt)",
     "            da = min(max(dv - dv0, -t.j_max * 2 * dt * dt), t.j_max * 2 * dt * dt)"),
    ("no rate clamp in the chain", "X5 (the clamp chain)",
     "        else:\n            v = min(max(v, -t.r_dn), t.r_up)\n        if step",
     "        else:\n            v = v\n        if step"),
    ("chain linear zone over τ (rings)", "X5 (the chain's reference)",
     "min(v_stop, e / (2 * t.ease))", "min(v_stop, e / t.ease)"),
    ("no jerk-aware aim in the chain", "X5 (the chain's reference)",
     "    return v_ref - a0 * abs(a0) / (2 * t.j_max)", "    return v_ref"),
    ("no curvature clip at the hand-off", "X5 (the hand-off)",
     "    if a > 0 and v + a * a / (2 * t.j_max) > t.r_up:", "    if False:"),
    ("no history restart after a cut", "X5 (the chain's rates)",
     "        if exc in CUTS:\n            ez.hist = hist_from(st)", "        if False:\n            ez.hist = hist_from(st)"),
    ("no return to Ruckig after the chain", "X5",
     "        injected = (bool(self.inject) and traj is not self.probe and self.k >= self.inject_from",
     "        injected = (self.chain_ticks > 0 and traj is not self.probe) or (bool(self.inject) and traj is not self.probe and self.k >= self.inject_from"),
    ("a slower Ruckig answer accepted", "X5 (answers not accepted)",
     "                and traj.duration > landing_bound(self.t, -rel, st.v, st.a)):",
     "                and False):"),
    ("target velocity exactly at its limit allowed", "Ruckig's segfault guard",
     "        tv = min(max(tv, lo + (1 - TARGET_SCALE) * max(1.0, abs(lo))), hi - (1 - TARGET_SCALE) * max(1.0, abs(hi)))",
     "        tv = min(max(tv, lo), hi)"),
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
