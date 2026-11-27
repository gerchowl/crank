#!/usr/bin/env python3
"""Mutation check of the eased-ERG model (ADR-0002 §7.10, #585; round 19, simple easing: the closed-form landing,
the meet, the frame, the cuts, the input layer).

Each mutant deletes or breaks one of crank's rules in `erg_slew.py` (a copy in a
temporary directory; the file in the repo is never touched), then runs the model's whole suite
with a reduced random-plan count. A mutant is KILLED when the suite exits non-zero. The runner
exits 0 only if every mutant is killed and the unmutated model passes.

Run with any Python ≥ 3.10 (the standard library only):
    python3 docs/adr/models/erg_slew_mutants.py [--random N] [--jobs J] [--only TEXT]
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
    ('no meet above a rising line (the ordinary landing from above)', 'step 5',
     '        elif tracked and fs > 0 and st.y - line(k) > EPS:',
     '        elif False:'),
    ('braking may pass the cap by EPS', 'step 6',
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + ROUND_SNAP):',
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
     '            leads = (nxt.joined and track(nxt)\n',
     '            leads = False and (nxt.joined and track(nxt)\n'),
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
    ('a line at the rate landed on, not chased', 'step 4',
     '        track = lambda q: -t.r_dn < q.s < t.r_up\n',
     '        track = lambda q: -t.r_dn <= q.s <= t.r_up\n'),
    ('the lead and the line keyed apart (rounds 4-17: a re-plan at the handover with an unchanged frame)', 'step 7 (fix round 1)',
     '                ("frame", c, fs)),\n',
     '                ("lead", c, fs) if lead else ("frame", c, fs)),\n'),
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
     '        elif tracked and fs > 0 and st.y - line(k) > EPS:',
     '        elif tracked and fs > 0 and st.y - line(k) > 2 * EPS:'),
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
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + ROUND_SNAP):',
     '            if y > min(line_at(segs, k + i, m) + EPS, m + ROUND_SNAP):'),
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
    ('X5: an instant stop (a hold, no ease-out)', 'X5',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y, 0.0, 0.0)'),
    ('X5: the slope kept (no stop)', 'X5',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y + st.v * DT, st.v, 0.0)'),
    ('X5: the stop without the jerk limit (a_max at once)', 'X5',
     '        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))',
     '        return State(st.y + st.v * DT - math.copysign(min(abs(st.v), self.t.a_max * DT), st.v) * DT / 2, st.v - math.copysign(min(abs(st.v), self.t.a_max * DT), st.v), -math.copysign(self.t.a_max, st.v) if st.v else 0.0)'),
    ('X5: the trajectory in force kept after the hold', 'X5',
     '            self.key, self.braking, self.chasing, self.prev_piece = None, False, False, None\n            return self._after(st, self._stop(st)',
     '            self.braking, self.chasing, self.prev_piece = False, False, None\n            return self._after(st, self._stop(st)'),
    ('X5: the hold not reported (the status follows it, P8)', 'X5 (the status)',
     '            self.held = True\n',
     ''),
    ('recovery landing bound from the recovery state ignores its slope', 'the check (recovery)',
     '            bound = landing_bound(t, delta, v0, a0)\n        elif',
     '            bound = landing_bound(t, delta, 0.0, 0.0)\n        elif'),
    ('X5 path tolerance 1 W', 'the check (X5)',
     '            if abs(tr.y[j] - want) > 1e-6:',
     '            if abs(tr.y[j] - want) > 1.0:'),
    ('step 5 holds above a rising line (rounds 1-13)', 'step 5',
     '            vel_target = None\n            meet = True\n',
     '            vel_target = 0.0\n'),
    ("the meet lands with the line's slope, not at rest", 'step 5',
     '        elif meet:\n            self.traj = meet_profile(',
     '        elif False:\n            self.traj = meet_profile('),
    ('after the meet ends the command rides the line', 'step 5',
     '            if self.meet:   # met at rest',
     '            if False:'),
    ('a target ±inf before the clamp is not a fault', 'not a number',
     '        ez.plan_invalid = not finite',
     '        ez.plan_invalid = False'),
    ("the line's cap not floored at 0 W", 'the cap below 0 W',
     '                    m_line, cap_bad = max(e.value, 0.0), False',
     '                    m_line, cap_bad = e.value, False'),
    ('the hold never cleared (f625i)', 'X5',
     '        self.held = False\n        # The step from tick k-1',
     '        # The step from tick k-1'),
    ('ramp_bound: delta > 0 for the gap left after the stop (f625i)', 'the check (P7, D5)',
     '    if lo_g > 0:\n        r = rate_up',
     '    if delta > 0:\n        r = rate_up'),
    ("ramp_bound: round 13's rates (the line's own slope)", 'the check (P7, D5)',
     '    rate_up, rate_dn = up, dn',
     '    rate_up, rate_dn = (min(up, -s) if s < 0 else up), (s if s > 0 else dn)'),
    ('the sent state not checked against the reported one', 'the check (state)',
     '    check_state(tr, t, rep, name)\n',
     ''),
    ('a state that is not a number passed on', 'not a number',
     '        if not state_ok:\n            st = State(',
     '        if False:\n            st = State('),
    ('an infinite origin clamped to M (not a fault)', 'not a number',
     '                st, reset = State(clamp(origin, me) if math.isfinite(origin) else origin), True',
     '                st, reset = State(clamp(origin, me)), True'),
    ('X3 before the step keeps the falling slope (rounds 11-14)', 'X3',
     '            st, exc = _cut(st, self._at_cap(st, m, clamp(segs[0].at(k), m)), "X3", exc)',
     '            st, exc = _cut(st, State(m, min(st.v, 0.0), min(st.a, 0.0)), "X3", exc)'),
    ('X3 bound r↑ + a_max·Δt (rounds 10-14)', 'the check (X3)',
     '    return {"X2": t.r_up + step, "X3": max(t.r_up, t.r_dn) + step, "X4": t.r_dn + step}',
     '    return {"X2": t.r_up + step, "X3": t.r_up + step, "X4": t.r_dn + step}'),
    ("the line's cap NaN while the cap is not a number", 'the cap',
     '                if cap_ok(e.value):\n                    m_line, cap_bad',
     '                if True:\n                    m_line, cap_bad'),
    ('safe braking brakes to rest below a falling line (no meet from below, rounds 1-15)', 'step 6',
     '            meet_nst = self._meet_below(st, nst, k, segs, m)',
     '            meet_nst = None'),
    ('the fault branch applies no cap (round 14: it applied the previous one)', 'the cap',
     '            return self._after(st, self._stop(st), k, segs if known else [], m, r_k, "X5", brake=True)',
     '            return self._after(st, self._stop(st), k, segs if known else [], math.inf, r_k, "X5", brake=True)'),
    ('X5 without X2 (the last line known ignored during the hold)', 'X5 (D3)',
     '            known = bool(segs) and all(',
     '            known = False and all('),
    ('same tick: the easer loses the valid cap to a later invalid one (round 15)', 'the input layer',
     '        st, exc = ez.step(st, k, segs, me, reset or disc)',
     '        st, exc = ez.step(st, k, segs, math.nan if tick_bad else me, reset or disc)'),
    ('an invalid value on a tick raises no fault', 'the input layer',
     '        ez.plan_invalid = not finite or cap_bad or off_bad or tick_bad',
     '        ez.plan_invalid = not finite or cap_bad or off_bad'),
    ('braking may pass the cap by 1e-6 W (rounds 6-17; more than step 8 snaps)', 'step 6',
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + ROUND_SNAP):',
     '            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):'),
    ('the fault not kept while the latest cap is invalid', 'the input layer',
     '                    m_line, cap_bad = max(e.value, 0.0), False\n                else:\n                    cap_bad = tick_bad = True',
     '                    m_line, cap_bad = max(e.value, 0.0), False\n                else:\n                    tick_bad = True'),
    ('X3 keeps a negative curvature after reducing the slope (f625k K12)', 'X3',
     '        return State(m, hi, 0.0)',
     '        return State(m, hi, a)'),
    ('the last command never recorded (f625k K6)', 'not a number',
     '        if math.isfinite(nst.y):\n            self.y_ok = nst.y',
     '        if False:\n            self.y_ok = nst.y'),
    ('X3 bound doubled (f625k K11)', 'the check (X3)',
     '    return {"X2": t.r_up + step, "X3": max(t.r_up, t.r_dn) + step, "X4": t.r_dn + step}',
     '    return {"X2": t.r_up + step, "X3": 2 * (max(t.r_up, t.r_dn) + step), "X4": t.r_dn + step}'),
    ('X3 undershoot not checked', 'the check (X3)',
     '    check_cap_floor(tr, t, rep, name)\n',
     ''),
    ('the meet from below meets the line itself (round 16)', 'step 6',
     '        early = fs * DT\n',
     '        early = 0.0\n'),
    ("X3's floor checked only until the command is within ε of the line (round 16)", 'the check (X3)',
     '            floor = min(tr.r[k], tr.m[k], min(tr.r[j:j + ahead + 1]))\n',
     '            if j > k and abs(tr.y[j - 1] - tr.r[j - 1]) <= EPS:\n                break\n            floor = min(tr.r[k], tr.m[k], min(tr.r[j:j + ahead + 1]))\n'),
    ('an invalid initial cap is not a fault (f625l L12)', 'the input layer',
     '    cap_bad, off_bad = not cap_ok(m0), False             # the latest cap / offset was invalid',
     '    cap_bad, off_bad = False, False             # the latest cap / offset was invalid'),
    ('P7 not checked on ramps', 'the check (P7)',
     '    check_above(tr, t, rep, name)\n',
     ''),
    ('landing: no cruise at the rate (a long move ends short)', 'the landing',
     '        return prof(rate, (d - x_r) / (sgn * rate))',
     '        return prof(rate)'),
    ("landing: the bisection's direction reversed (the peak converges to a bound)", 'the landing',
     '        lo, hi = (mid, hi) if sgn * (end_of(prof(mid), v, a)[0] - d) < 0 else (lo, mid)',
     '        lo, hi = (lo, mid) if sgn * (end_of(prof(mid), v, a)[0] - d) < 0 else (mid, hi)'),
    ("landing: the frame's cap ignored (the rider's rates in a lead)", 'the landing (step 3)',
     'landing(st.v - fs, st.a, -rel, vmax - fs, fs - vmin, t))',
     'landing(st.v - fs, st.a, -rel, t.r_up - fs, fs + t.r_dn, t))'),
    ('landing: the rates not relative to the line', 'the landing',
     'landing(st.v - fs, st.a, -rel, vmax - fs, fs - vmin, t))',
     'landing(st.v - fs, st.a, -rel, vmax, -vmin, t))'),
    ("landing: the stop's own displacement ignored (the peak's direction from d alone)", 'the landing',
     '    g = d - x_s\n',
     '    g = d\n'),
    ("meet: the line's motion ignored (at rest where the line was)", 'the meet',
     '    h = lambda D, T: rel + D - fs * T\n',
     '    h = lambda D, T: rel + D\n'),
    ('meet: the stop alone, always (no descent onto the line)', 'the meet',
     '    if (h(x_s, t_s) <= 0) if above else (h(x_s, t_s) >= 0):',
     '    if True:'),
]


# Mutants shown equivalent, not run (each with its evidence; ADR-0002 §7.10.7 item 9).
EQUIVALENT: list = []

def run(path: pathlib.Path, n: int) -> tuple[int, str]:
    p = subprocess.run([sys.executable, str(path), "--random", str(n)], capture_output=True, text=True)
    out = (p.stdout + p.stderr).strip().splitlines()
    return p.returncode, (out[0] if out else "")[:120]


def main(argv) -> int:
    n = int(argv[argv.index("--random") + 1]) if "--random" in argv else 400
    # --jobs: mutants run at once (8 by default; 1 on a shared host); --only: mutants whose name contains one of the
    # comma-separated texts.
    jobs_n = int(argv[argv.index("--jobs") + 1]) if "--jobs" in argv else 8
    only = argv[argv.index("--only") + 1] if "--only" in argv else None
    src = SRC.read_text(encoding="utf-8")
    rows, ok = [], True
    code, first = run(SRC, n)
    rows.append(("(none: the model as committed)", "-", code, "passes" if code == 0 else "FAILS", first))
    ok &= code == 0
    with tempfile.TemporaryDirectory() as tmp:
        jobs = []
        for i, (name, rule, old, new) in enumerate(MUTANTS):
            if only is not None and not any(o in name for o in only.split(",")):
                continue
            if src.count(old) != 1:
                jobs.append((name, rule, None))
                continue
            path = pathlib.Path(tmp) / f"m{i}" / "erg_slew.py"
            path.parent.mkdir()
            path.write_text(src.replace(old, new), encoding="utf-8")
            jobs.append((name, rule, path))
        with ThreadPoolExecutor(max_workers=jobs_n) as pool:
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
