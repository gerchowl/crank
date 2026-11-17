#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585; round 10, the hybrid; round 13, degraded mode).

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
    ('no safe braking', 'step 6',
     '        if (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))',
     '        if False and (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))'),
    ('no floor braking', 'step 6 (0 W)',
     '                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):',
     '                           or (False and nst.v < -1e-9 and not self._floor_ok(nst)))):'),
    ('no hold above a rising line', 'step 5',
     '        elif trackable(fs) and fs > 0 and st.y - line(k) > EPS:',
     '        elif False:'),
    ('braking may pass the cap by EPS', 'step 6',
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):',
     '            if y > line_at(segs, k + i, m) + EPS / 2:'),
    ('no X2 after the step', 'step 8',
     '        if nst.y > r_k + EPS and (nst.v > 1e-9 or nst.y > st.y + 1e-7):',
     '        if False:'),
    ('no X3 before the step', 'step 1',
     '        if st.y > m:                                     # X3: the cap is never eased',
     '        if False:'),
    ('no X3 after the step', 'step 8',
     '        if nst.y > m:\n            nst, exc = _cut(nst, State(m, 0.0, 0.0), "X3", exc)',
     '        if False:\n            nst, exc = _cut(nst, State(m, 0.0, 0.0), "X3", exc)'),
    ('no 0 W floor (X4)', 'step 8',
     '        if nst.y < 0:                                    # X4: nothing is below 0 W',
     '        if False:'),
    ('no lead into vertices', 'step 3',
     '            leads = (nxt.joined and trackable(nxt.s)\n',
     '            leads = False and (nxt.joined and trackable(nxt.s)\n'),
    ('new piece extended back at a vertex', 'step 2',
     '        if (segs[0].k0 == k and segs[0].joined and self.prev_piece is not None and not cut_before',
     '        if (False and segs[0].k0 == k and segs[0].joined and self.prev_piece is not None and not cut_before'),
    ('chase gap from the line extended back', 'step 4',
     '            gap = line(k) - st.y',
     '            gap = line(k - 1) - st.y'),
    ('re-plan every tick', 'step 7',
     '        if not same_key(key, self.key) or exc or cut_before:',
     '        if True:'),
    ('one-tick-late braking predicate (v > 0 only)', 'step 6',
     '    return st.v > 1e-9 or (st.a > 1e-9 and st.v + st.a * st.a / (2 * t.j_max) > 1e-9)',
     '    return st.v > 1e-9'),
    ('acceleration and jerk from the falling rate', 'the mapping',
     '        return 2 * self.r_up / self.ease',
     '        return 2 * self.r_dn / self.ease'),
    ('lead velocity not capped', 'step 3',
     '            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))',
     '            cap_v = lambda: (t.r_up, -t.r_dn)'),
    ('lead one tick late', 'step 3',
     '<= s_dur / 2 + DT + 1e-9:',
     '<= s_dur / 2 + 1e-9:'),
    ('lead two ticks early', 'step 3',
     '<= s_dur / 2 + DT + 1e-9:',
     '<= s_dur / 2 + 3 * DT + 1e-9:'),
    ('will-rise with a²/(4j)', 'step 6',
     'st.v + st.a * st.a / (2 * t.j_max) > 1e-9)',
     'st.v + st.a * st.a / (4 * t.j_max) > 1e-9)'),
    ("chase only strictly behind the line (round 5's rule)", 'step 4',
     '            vel_target = t.r_up if (gap > -EPS and fs > 0) else (-t.r_dn if (gap < EPS and fs < 0) else 0.0)',
     '            vel_target = t.r_up if (gap > 0 and fs > 0) else (-t.r_dn if (gap < 0 and fs < 0) else 0.0)'),
    ('trackable at exactly the rate', 'step 4',
     '        trackable = lambda s: -t.r_dn < s < t.r_up',
     '        trackable = lambda s: -t.r_dn <= s <= t.r_up'),
    ('X4 keeps the slope', 'step 8',
     '            nst, exc = _cut(nst, State(0.0, 0.0, 0.0), "X4", exc)',
     '            nst, exc = _cut(nst, State(0.0, nst.v, nst.a), "X4", exc)'),
    ('no collinear merge', 'the line on the tick grid',
     '        if lines and seg.joined and collinear(seg.s, lines[-1].s):',
     '        if False:'),
    ('keys compared exactly', 'step 7',
     '        if not same_key(key, self.key) or exc or cut_before:',
     '        if key != self.key or exc or cut_before:'),
    ('hold above a rising line at 2 EPS', 'step 5',
     '        elif trackable(fs) and fs > 0 and st.y - line(k) > EPS:',
     '        elif trackable(fs) and fs > 0 and st.y - line(k) > 2 * EPS:'),
    ('SLOPE_TOL 1e-3', 'the line on the tick grid',
     'SLOPE_TOL = 1e-9 ',
     'SLOPE_TOL = 1e-3 '),
    ('SLOPE_TOL 1e-6', 'the line on the tick grid',
     'SLOPE_TOL = 1e-9 ',
     'SLOPE_TOL = 1e-6 '),
    ('merged slope not recomputed', 'the line on the tick grid',
     '            lines[-1] = Seg(q.k0, q.r0, (end - q.r0) / (n * DT), n, q.joined)',
     '            lines[-1] = Seg(q.k0, q.r0, q.s, n, q.joined)'),
    ('same_key tolerance 1e-3', 'step 7',
     '        return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))',
     '        return abs(a - b) <= 1e-3 * max(1.0, abs(a), abs(b))'),
    ('merge across a jump (no VALUE_TOL)', 'the line on the tick grid',
     '                and abs(out[-1].at(p.k0) - p.r0) <= VALUE_TOL):',
     '                and True):'),
    ('a clamped jump read as joined', 'the line on the tick grid',
     '                joined = line.joined and last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL',
     '                joined = line.joined'),
    ('snap threshold 1e-12 W/s absolute', 'the line on the tick grid',
     '0.0 if collinear(q.s, 0.0) else q.s',
     '0.0 if abs(q.s) <= 1e-12 else q.s'),
    ('snap threshold 1e-3 W/s absolute', 'the line on the tick grid',
     '0.0 if collinear(q.s, 0.0) else q.s',
     '0.0 if abs(q.s) <= 1e-3 else q.s'),
    ('snap threshold 0.05 W/s absolute', 'the line on the tick grid',
     '0.0 if collinear(q.s, 0.0) else q.s',
     '0.0 if abs(q.s) <= 0.05 else q.s'),
    ('no merge after the clamp', 'the line on the tick grid',
     '    return [p for p in merge_collinear(out) if p.k0 + p.n > k]',
     '    return [p for p in out if p.k0 + p.n > k]'),
    ('raw join tolerance 1e-3', 'the line on the tick grid',
     'abs(b.frm * mult - prev_end) < 1e-9',
     'abs(b.frm * mult - prev_end) < 1e-3'),
    ('raw join tolerance 1.0', 'the line on the tick grid',
     'abs(b.frm * mult - prev_end) < 1e-9',
     'abs(b.frm * mult - prev_end) < 1.0'),
    ('post-clamp join ignores the raw join', 'the line on the tick grid',
     'joined = line.joined and last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL',
     'joined = last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL'),
    ('drop before merge (blocker 1: the line depends on the tick that asks)', 'the line on the tick grid',
     "            out.append(piece)\n            last, kk = piece, end\n    # Merge first, then drop the pieces that end at or before tick k: dropping first made a merged\n    # piece's start depend on the tick that asked (round-7 falsifier, blocker 1).\n    return [p for p in merge_collinear(out) if p.k0 + p.n > k]",
     '            if end > k:\n                out.append(piece)\n            last, kk = piece, end\n    return merge_collinear(out)'),
    ('a slower Ruckig answer accepted', 'X5 (answers not accepted)',
     '                and traj.duration > landing_bound(self.t, -rel, st.v, st.a)):',
     '                and False):'),
    ('target velocity exactly at its limit allowed', "Ruckig's segfault guard",
     '        tv = min(max(tv, lo + (1 - TARGET_SCALE) * max(1.0, abs(lo))), hi - (1 - TARGET_SCALE) * max(1.0, abs(hi)))',
     '        tv = min(max(tv, lo), hi)'),
    ('no rounding snap at M and 0 W (a 1-ulp landing is cut)', 'step 8',
     '        if m < nst.y <= m + ROUND_SNAP:',
     '        if False:'),
    ('no rounding snap at 0 W (a landing 4e-14 W below is cut)', 'step 8',
     '        elif -ROUND_SNAP <= nst.y < 0:',
     '        elif False:'),
    ('cut domains tested only outside the band', 'the check',
     '        if is_cut and not tr.unforeseen[k]:',
     '        if is_cut and not tr.unforeseen[k] and False:'),
    ('profile_extrema: one root of the slope only (N7)', 'safe braking (closed form)',
     '                for sgn in (1, -1):',
     '                for sgn in (1,):'),
    ('profile_at: no constant-slope tail (N6)', 'safe braking (closed form)',
     '    return y + v * tt, v, 0.0',
     '    return y, v, 0.0'),
    ('lead timing ignores the curvature (N10)', 'step 3',
     's_dur = sum(d for d, _ in slope_change(st.v, st.a, nxt.s, t))',
     's_dur = sum(d for d, _ in slope_change(st.v, 0.0, nxt.s, t))'),
    ('safe braking slack ε, not ε/2 (N20)', 'step 6',
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):',
     '            if y > min(line_at(segs, k + i, m) + EPS, m + 1e-6):'),
    ('drop pieces ending at k too (E13)', 'the line on the tick grid',
     '    return [p for p in merge_collinear(out) if p.k0 + p.n > k]',
     '    return [p for p in merge_collinear(out) if p.k0 + p.n >= k]'),
    ('closed-form stop without the jerk ramp-down', 'safe braking (closed form)',
     '    return [(t1, s * j), (t2, 0.0), (t3, -s * j)]',
     '    return [(t1, s * j), (t2 + t3, 0.0)]'),
    ('lead from up to 3 EPS off the line', 'step 3',
     'and ((nxt.s < s_cur and abs(off) <= EPS) or',
     'and ((nxt.s < s_cur and abs(off) <= 3 * EPS) or'),
    ("lead from any distance below the line (round 12's off_plan gate, always on)", 'step 3',
     'and ((nxt.s < s_cur and abs(off) <= EPS) or',
     'and ((nxt.s < s_cur and off <= EPS) or'),
    ('pre-step X3 skipped while degraded (E21)', 'X3',
     '        if st.y > m:                                     # X3: the cap is never eased',
     '        if st.y > m and not self.degraded:                                     # X3: the cap is never eased'),
    ('no twin', 'X5a',
     '        for twin in (False, True):',
     '        for twin in (False,):'),
    ("the twin's limits not tightened", 'X5a',
     '        tight = (lambda x: x - math.copysign(TWIN * max(1.0, abs(x)), x)) if twin else (lambda x: x)',
     '        tight = lambda x: x'),
    ("the twin's limits widened", 'X5a',
     '        tight = (lambda x: x - math.copysign(TWIN * max(1.0, abs(x)), x)) if twin else (lambda x: x)',
     '        tight = (lambda x: x + math.copysign(TWIN * max(1.0, abs(x)), x)) if twin else (lambda x: x)'),
    ("the twin's jerk not tightened", 'X5a',
     'vmax, vmin, amax, jmax = tight(vmax), tight(vmin), tight(t.a_max), tight(t.j_max)',
     'vmax, vmin, amax, jmax = tight(vmax), tight(vmin), tight(t.a_max), t.j_max'),
    ('TWIN 2^-10', 'X5a',
     'TWIN = 2 ** -20 ',
     'TWIN = 2 ** -10 '),
    ('a slower twin answer accepted', 'X5a (answers not accepted)',
     '            if (ok and vel_target is None and fs == 0.0',
     '            if (ok and not twin and vel_target is None and fs == 0.0'),
    ('injected refusals spare the twin (degraded mode never tested)', 'fault injection',
     '                           injected and not (twin and self.twin_answers))',
     '                           injected and not twin)'),
    ("a twin's answer not counted", 'X5a (diagnostics)',
     '                self.twins += twin\n',
     ''),
    ('degraded: an instant stop (a hold, no ease-out)', 'X5 (degraded)',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y, 0.0, 0.0)'),
    ('degraded: the slope kept (no stop)', 'X5 (degraded)',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y + st.v * DT, st.v, 0.0)'),
    ('degraded: the stop without the jerk limit (a_max at once)', 'X5 (degraded)',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y + st.v * DT - math.copysign(min(abs(st.v), self.t.a_max * DT), st.v) * DT / 2, st.v - math.copysign(min(abs(st.v), self.t.a_max * DT), st.v), -math.copysign(self.t.a_max, st.v) if st.v else 0.0)'),
    ('degraded: the trajectory in force kept (no Ruckig retry)', 'X5 (degraded)',
     '                self.key, self.braking, self.chasing = None, False, False\n                return self._after(st, self._stop(st)',
     '                self.braking, self.chasing = False, False\n                return self._after(st, self._stop(st)'),
    ('degraded: status not raised', 'X5 (degraded)',
     '                self.x5_tick = self.degraded = True\n                self.key, self.braking, self.chasing = None, False, False',
     '                self.degraded = True\n                self.key, self.braking, self.chasing = None, False, False'),
    ('degraded: no return to Ruckig', 'X5 (recovery)',
     '        injected = (self.k in self.refuse_ticks) or (',
     '        injected = self.degraded or (self.k in self.refuse_ticks) or ('),
    ('degraded: the stop passed through safe braking', 'X5 (degraded)',
     'return self._after(st, self._stop(st), k, segs, m, r_k, exc or "X5", brake=True)',
     'return self._after(st, self._stop(st), k, segs, m, r_k, exc or "X5", brake=False)'),
    ('a refused braking plan lets the step stand', 'X5 (the braking plan)',
     '                nst = self._stop(st)\n                self.key = None\n                exc = exc or "X5"',
     '                self.key = None\n                exc = exc or "X5"'),
    ('a refused braking plan not degraded', 'X5 (the braking plan)',
     '                exc = exc or "X5"\n                self.x5_tick = self.degraded = True',
     '                exc = exc or "X5"'),
    ('recovery landing bound from the recovery state ignores its slope', 'the check (recovery)',
     '            bound = landing_bound(t, delta, v0, a0)\n        elif',
     '            bound = landing_bound(t, delta, 0.0, 0.0)\n        elif'),
    ('degraded path tolerance 1 W', 'the check (degraded)',
     '            if abs(tr.y[j] - want) > 1e-6:',
     '            if abs(tr.y[j] - want) > 1.0:'),
]


# Mutants shown equivalent, not run (each with its evidence; ADR-0002 §7.10.7 item 9).
EQUIVALENT = [
    ('a refused braking plan keeps braking', 'X5 (the braking plan)',
     'equivalent: self.braking is already False in that branch (it runs only for brake=False, which the '
     'normal path passes as self.braking; only the other branch sets it True). Measured before the '
     'assignment was deleted: 1869 entries over 2881 fault-injected plans, braking True on 0, traces '
     'identical to 0 W (round 13b).'),
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
    for name, rule, why in EQUIVALENT:
        rows.append((name, rule, "-", "equivalent (not run)", why))
    for r in rows:
        print("| " + " | ".join(str(x) for x in r) + " |")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
