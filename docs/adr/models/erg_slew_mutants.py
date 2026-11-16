#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585; round 10, the hybrid; round 12).

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
     "        if (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))",
     "        if False and (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))"),
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
     "    return st.v > 1e-9 or (st.a > 1e-9 and st.v + st.a * st.a / (2 * t.j_max) > 1e-9)",
     "    return st.v > 1e-9"),
    ("acceleration and jerk from the falling rate", "the mapping",
     "        return 2 * self.r_up / self.ease",
     "        return 2 * self.r_dn / self.ease"),
    # The round-5 falsifier's survivors (PR #625), each killed since round 6:
    ("lead from up to 3 EPS off the line", "step 3",
     "(off <= EPS if self.off_plan else abs(off) <= EPS)",
     "(off <= EPS if self.off_plan else abs(off) <= 3 * EPS)"),
    ("lead velocity not capped", "step 3",
     "            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))",
     "            cap_v = lambda: (t.r_up, -t.r_dn)"),
    ("lead one tick late", "step 3",
     "<= s_dur / 2 + DT + 1e-9:",
     "<= s_dur / 2 + 1e-9:"),
    ("lead two ticks early", "step 3",
     "<= s_dur / 2 + DT + 1e-9:",
     "<= s_dur / 2 + 3 * DT + 1e-9:"),
    ("will-rise with a²/(4j)", "step 6",
     "st.v + st.a * st.a / (2 * t.j_max) > 1e-9)",
     "st.v + st.a * st.a / (4 * t.j_max) > 1e-9)"),
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
     "                exc = exc or \"X5b\"; self.x5_tick = True",
     "                pass"),
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
     "    if a > 0 and v + a * a / (2 * t.j_max) > up:", "    if False:"),
    ("no history restart after a cut", "X5 (the chain's rates)",
     "        if exc in CUTS:\n            ez.hist = hist_from(st)", "        if False:\n            ez.hist = hist_from(st)"),
    ("no return to Ruckig after the chain", "X5",
     "        injected = (self.k in self.refuse_ticks) or (",
     "        injected = (self.chain_ticks > 0 and traj is not self.probe) or (self.k in self.refuse_ticks) or ("),
    ("a slower Ruckig answer accepted", "X5 (answers not accepted)",
     "                and traj.duration > landing_bound(self.t, -rel, st.v, st.a)):",
     "                and False):"),
    ("target velocity exactly at its limit allowed", "Ruckig's segfault guard",
     "        tv = min(max(tv, lo + (1 - TARGET_SCALE) * max(1.0, abs(lo))), hi - (1 - TARGET_SCALE) * max(1.0, abs(hi)))",
     "        tv = min(max(tv, lo), hi)"),
    # Round 11 (falsifier at f135b8c): the seam between the planner and X5.
    ("no braking check in the chain", "X5 (the chain keeps the planner's promises)",
     "                if (((rises(nst, t) or nst.y > st.y) and not self._brake_ok(nst, k, segs, m))\n                        or (nst.v < -1e-9 and not self._floor_ok(nst))):",
     "                if False:"),
    ("chain aims at the raw line, not the planner's frame", "X5 (the chain's reference)",
     "                lo_v, hi_v = max(vmin, -t.r_dn), min(vmax, t.r_up)\n                s_t = min(max(fs, lo_v), hi_v)",
     "                lo_v, hi_v = -t.r_dn, t.r_up\n                s_t = min(max(s_cur, lo_v), hi_v)"),
    ("chain gap taken against the line at tick k", "X5 (the chain's reference)",
     "chain_rate(clamp(line(k - 1), m), s_t, st.y, t, (v0 - v1) / DT)", "chain_rate(clamp(line(k), m), s_t, st.y, t, (v0 - v1) / DT)"),
    # Round 12 (falsifier at 45be306): the lead kept off the plan, and the round-11 survivors.
    ("the lead's EPS gate kept off the plan (the chain drops the lead)", "step 3 / X5",
     "(off <= EPS if self.off_plan else abs(off) <= EPS)", "abs(off) <= EPS"),
    ("off the plan never set by the chain", "step 3 / X5",
     "                self.key, self.x5_frame, self.braking = None, None, False\n                self.off_plan = True",
     "                self.key, self.x5_frame, self.braking = None, None, False"),
    ("chain braking check only on a rising state, not a rising step", "X5 (the chain keeps the planner's promises)",
     "if (((rises(nst, t) or nst.y > st.y) and not", "if (((rises(nst, t)) and not"),
    ("no rounding snap at M and 0 W (a 1-ulp landing is cut)", "step 8",
     "        if m < nst.y <= m + ROUND_SNAP:", "        if False:"),
    ("no rounding snap at 0 W (a landing 4e-14 W below is cut)", "step 8",
     "        elif -ROUND_SNAP <= nst.y < 0:", "        elif False:"),
    ("chain step not kept inside [0, M]", "X5 (the clamp chain)",
     "                if y_new > m or y_new < 0:", "                if False:"),
    ("cut domains tested only outside the band", "the check",
     "        if is_cut and not tr.unforeseen[k]:", "        if is_cut and not tr.unforeseen[k] and False:"),
    ("profile_extrema: one root of the slope only (N7)", "safe braking (closed form)",
     "                for sgn in (1, -1):", "                for sgn in (1,):"),
    ("profile_at: no constant-slope tail (N6)", "safe braking (closed form)",
     "    return y + v * tt, v, 0.0", "    return y, v, 0.0"),
    ("lead timing ignores the curvature (N10)", "step 3",
     "s_dur = sum(d for d, _ in slope_change(st.v, st.a, nxt.s, t))", "s_dur = sum(d for d, _ in slope_change(st.v, 0.0, nxt.s, t))"),
    ("chain drops the lead's rate caps (N11)", "X5 (the chain's reference)",
     "                lo_v, hi_v = max(vmin, -t.r_dn), min(vmax, t.r_up)", "                lo_v, hi_v = -t.r_dn, t.r_up"),
    ("chain rate not clamped to the lead's caps (N21)", "X5 (the chain's reference)",
     "v_ref = min(max(chain_rate(clamp(line(k - 1), m), s_t, st.y, t, (v0 - v1) / DT), lo_v), hi_v)",
     "v_ref = chain_rate(clamp(line(k - 1), m), s_t, st.y, t, (v0 - v1) / DT)"),
    ("chain aims at the unclamped line (N13/E6)", "X5 (the chain's reference)",
     "chain_rate(clamp(line(k - 1), m), s_t", "chain_rate(line(k - 1), s_t"),
    ("hand-off curvature clip against the rate, not the margin (N18)", "X5 (the hand-off)",
     "    if a > 0 and v + a * a / (2 * t.j_max) > up:", "    if a > 0 and v + a * a / (2 * t.j_max) > t.r_up:"),
    ("safe braking slack ε, not ε/2 (N20)", "step 6",
     "            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):", "            if y > min(line_at(segs, k + i, m) + EPS, m + 1e-6):"),
    ("hand-off on the rate limits", "X5 (the hand-off)",
     "    up = t.r_up - (1 - TARGET_SCALE) * max(1.0, t.r_up)\n    dn = t.r_dn - (1 - TARGET_SCALE) * max(1.0, t.r_dn)",
     "    up = t.r_up\n    dn = t.r_dn"),
    ("hand-off slope without the half-tick midpoint (E1)", "X5 (the hand-off)",
     "nst = State(y_new, *handoff(v_new + a_new * DT / 2, a_new, t))",
     "nst = State(y_new, *handoff(v_new, a_new, t))"),
    ("hand-off curvature dropped (E2)", "X5 (the hand-off)",
     "nst = State(y_new, *handoff(v_new + a_new * DT / 2, a_new, t))",
     "nst = State(y_new, *handoff(v_new + a_new * DT / 2, 0.0, t))"),
    ("fail_run not reset on success (E4)", "X5",
     "                self.x5_frame = None\n                self.fail_run = 0", "                self.x5_frame = None"),
    ("continuation checked against tick k (E5)", "X5",
     "            if self.fail_run <= N_ERRORS and self._continues(st, k - 1):",
     "            if self.fail_run <= N_ERRORS and self._continues(st, k):"),
    ("chain brakes at a_max, not a_max/2 (E8)", "X5 (the chain's reference)",
     "    ab = 0.5 * t.a_max", "    ab = t.a_max"),
    ("continuation one tick past the trajectory's end (E16)", "X5",
     "        if self.elapsed + DT > self.traj.duration + 1e-9:", "        if self.elapsed > self.traj.duration + 1e-9:"),
    ("chain bound tightened by τ/4 (E19's constant)", "X5 (the fallback's landing bound)",
     "CHAIN_C = 0.0 ", "CHAIN_C = -0.25 "),
    ("pre-step X3 skipped on chain ticks (E21)", "X3",
     "        if st.y > m:                                     # X3: the cap is never eased",
     "        if st.y > m and self.chain_ticks == 0:                                     # X3: the cap is never eased"),
    ("chain ignores the line's slope (E7)", "X5 (the chain's reference)",
     "                s_t = min(max(fs, lo_v), hi_v)", "                s_t = 0.0"),
    ("jerk-aware aim with the wrong sign (E9)", "X5 (the chain's reference)",
     "    return v_ref - a0 * abs(a0) / (2 * t.j_max)", "    return v_ref + a0 * abs(a0) / (2 * t.j_max)"),
    ("drop pieces ending at k too (E13)", "the line on the tick grid",
     "    return [p for p in merge_collinear(out) if p.k0 + p.n > k]", "    return [p for p in merge_collinear(out) if p.k0 + p.n >= k]"),
    ("hand-off clip with a²/(4j) (E17)", "X5 (the hand-off)",
     "    if a > 0 and v + a * a / (2 * t.j_max) > up:\n        a = math.sqrt(2 * t.j_max * (up - v))",
     "    if a > 0 and v + a * a / (4 * t.j_max) > up:\n        a = math.sqrt(4 * t.j_max * (up - v))"),
    ("fallback_bound without the N_ERRORS + 1 ticks and the stop (E18)", "X5 (the fallback's landing bound)",
     "    return chain_bound(t, math.copysign(d, delta)) + t_stop + (N_ERRORS + 1) * DT",
     "    return chain_bound(t, math.copysign(abs(delta), delta))"),
    ("closed-form stop without the jerk ramp-down", "safe braking (closed form)",
     "    return [(t1, s * j), (t2, 0.0), (t3, -s * j)]", "    return [(t1, s * j), (t2 + t3, 0.0)]"),
    ("a refused braking plan lets the step stand", "X5 (the braking plan)",
     "                nst = State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, t), DT))\n                self.key, self.x5_frame = None, None",
     "                self.key, self.x5_frame = None, None"),
    ("X5f jerk bound 1.95·j_max (below the worst measured, 1.979)", "X5f", "X5F_JERK = 2.25 ", "X5F_JERK = 1.95 "),
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
