#!/usr/bin/env python3
"""Model of the eased ERG target, ADR-0002 §7.10 (#585), round 19: simple easing, no Ruckig.

The owner's ruling of 2026-11-27 ("Simple easing, no Ruckig."): the commanded target is eased by crank's own
closed-form, rate-, acceleration- and jerk-limited profiles. There is no planning library, so nothing can refuse a
calculation: no twin, no degraded mode, no slow-answer rule, no landable band. Every tick crank takes:
- the commanded target's state: position y in W, velocity v in W/s, acceleration a in W/s²;
- the plan line, in a frame that moves with the line (§7.10.2);
- the rider's limits;
and it samples, one tick (0.25 s) later, a trajectory it computed in closed form when its input last changed:
- the landing (`landing`): a slope change to a peak slope, a cruise at it, then the stop. It lands at rest in the
  line's frame, exactly, with the peak found by bisection on a monotone closed-form displacement;
- the chase and the brake: one slope change (`slope_change`) to a target slope or to rest;
- the meet: the landing in the absolute frame onto a moving point of the line (bisection on the rest point).
Rounds 4-18 put Ruckig 0.19.4 in that loop. Their history is in git (ADR-0002 §7.10, before round 19). The model
checks the properties of §7.10.6 on every tick, and `--report` prints the tables the ADR quotes.

Setup: the standard library only (no Ruckig); any Python ≥ 3.10.
    python3 docs/adr/models/erg_slew.py [--random N] [--seeds a,b] [--invalid N] [--report]
`--invalid N` sets the plans of the invalid-input generator (2 000 by default). Exit 0 means every check passed.
"""
from __future__ import annotations

import math
import random
import sys
from dataclasses import dataclass, field

DT = 0.25          # one engine tick, s
EPS = 0.5          # W: "on the line" is |R - y| <= EPS
SETTLE_AFTER_TICKS = 20
TOL = 1e-6         # floating-point tolerance of the checks
ROUND_SNAP = 1e-9  # W: a landing this close outside [0, M] is floating-point rounding, snapped, not cut (round 12)
ROOT_STEPS = 200   # at most this many steps of the root finder (the landing's peak slope, the meet's rest point)


@dataclass(frozen=True)
class Tuning:
    """`Tuning.slew` (ADR-0001 §7.2)."""
    rise: int = 20          # W/s, integer
    factor: float = 2.0     # fall = rise × factor, 0.25 grid
    ease: float = 2.0       # s, 0.25 s grid

    @property
    def r_up(self) -> float:
        return float(self.rise)

    @property
    def r_dn(self) -> float:
        return self.rise * self.factor

    @property
    def a_max(self) -> float:
        """Acceleration limit, both directions: from rest to the rising rate in exactly `ease`
        seconds as one S-curve (a triangle of acceleration), a = 2·rise/τ."""
        return 2 * self.r_up / self.ease

    @property
    def j_max(self) -> float:
        """Jerk limit, both directions: j = 4·rise/τ². One pair of limits for every move, so a
        change of direction never changes the limits under a moving state."""
        return 4 * self.r_up / self.ease ** 2


    def valid(self) -> bool:
        return (5 <= self.rise <= 50 and 1 <= self.factor <= 4 and (self.factor * 4) % 1 == 0
                and 1 <= self.ease <= 4 and (self.ease * 4) % 1 == 0 and self.r_dn >= 20)


DEFAULT = Tuning()
CORNERS = [Tuning(5, 4.0, 4.0), Tuning(50, 1.0, 1.0), Tuning(50, 4.0, 1.0), Tuning(50, 1.0, 4.0),
           Tuning(5, 4.0, 1.0), Tuning(20, 1.0, 4.0), Tuning(50, 4.0, 4.0), Tuning(20, 1.0, 1.0)]
GRID = [Tuning(r, f, e) for r in (5, 8, 10, 15, 20, 30, 40, 50) for f in (1.0, 1.25, 1.5, 2.0, 3.0, 4.0)
        for e in (1.0, 1.5, 2.0, 2.75, 3.0, 4.0) if Tuning(r, f, e).valid()]


# ---------------------------------------------------------------- the plan line (§7.10.2)

@dataclass
class Block:
    ticks: int
    frm: float
    to: float

    @property
    def slope(self) -> float:
        return (self.to - self.frm) / (self.ticks * DT)


@dataclass
class Event:
    tick: int
    kind: str     # "offset" (value = multiplier), "cap" (value = M), "skip", "engage" (value = origin)
    value: float = 0.0


@dataclass
class Seg:
    """A piece of the line: value `r0` at tick `k0`, slope `s` W/s, `n` ticks (multiplier applied).
    `joined`: the piece starts where the line already is (a kink, or where the clamp at 0 W or M
    starts or ends), so its start is a vertex, not a discontinuity."""
    k0: int
    r0: float
    s: float
    n: int
    joined: bool = False

    def at(self, k: float) -> float:
        return self.r0 + self.s * (k - self.k0) * DT


def segments(blocks: list[Block], k: int, mult: float, m: float = math.inf) -> list[Seg]:
    """The clamped plan line from tick k on, as canonical pieces. Built from tick 0 every time, so
    the pieces never depend on the tick they are asked for:
    1. each block's line (multiplier applied), joined if it starts where the previous one ended;
    2. collinear joined lines merged into one, its slope recomputed from the merged endpoints, so a
       ramp split into blocks of one slope is bitwise the single block;
    3. each line split where the clamp to [0, M] starts or ends (a clamped piece is flat), and
       merged after the clamp where it continues;
    4. the pieces that end at or before tick k dropped, last (so the result never depends on k)."""
    raw, k0, prev_end = [], 0, None
    for b in blocks:
        if b.ticks > 0:
            joined = prev_end is not None and abs(b.frm * mult - prev_end) < 1e-9
            raw.append((Seg(k0, b.frm * mult, b.slope * mult, b.ticks, joined), b.to * mult))
            prev_end = b.to * mult
        k0 += b.ticks
    lines: list[Seg] = []
    ends: list[float] = []
    for seg, end in raw:
        if lines and seg.joined and collinear(seg.s, lines[-1].s):
            q = lines[-1]
            n = q.n + seg.n
            lines[-1] = Seg(q.k0, q.r0, (end - q.r0) / (n * DT), n, q.joined)
            ends[-1] = end
        else:
            lines.append(seg)
            ends.append(end)
    # A slope within SLOPE_TOL of 0 is 0: float noise in a flat block's endpoints (a file format's,
    # a split's) must not read as a rising or falling line (round 7: 7.7e-14 W/s read as rising
    # held the command 0.75 W above a 0 W line).
    lines = [Seg(q.k0, q.r0, 0.0 if collinear(q.s, 0.0) else q.s, q.n, q.joined) for q in lines]
    out = []
    last = None   # the previous piece, kept or not
    for line in lines:
        kk = line.k0
        while kk < line.k0 + line.n:
            v = line.at(kk)
            state = 0 if v <= 0 else (2 if v >= m else 1)
            end = kk + 1
            while end < line.k0 + line.n:
                w = line.at(end)
                if (0 if w <= 0 else (2 if w >= m else 1)) != state:
                    break
                end += 1
            piece = (Seg(kk, v, line.s, end - kk) if state == 1
                     else Seg(kk, 0.0 if state == 0 else m, 0.0, end - kk))
            if kk == line.k0:
                # A new block: joined only if the clamped line continues there. A block that starts
                # where the unclamped line ended can still jump after the clamp (a ramp that falls
                # from above M within one tick): that is a discontinuity, not a vertex.
                joined = line.joined and last is not None and abs(last.at(kk) - piece.r0) <= VALUE_TOL
            else:
                joined = True   # where the clamp starts or ends inside one line: a vertex
            piece = Seg(piece.k0, piece.r0, piece.s, piece.n, joined)
            out.append(piece)
            last, kk = piece, end
    # Merge first, then drop the pieces that end at or before tick k: dropping first made a merged
    # piece's start depend on the tick that asked (round-7 falsifier, blocker 1).
    return [p for p in merge_collinear(out) if p.k0 + p.n > k]


SLOPE_TOL = 1e-9   # W/s, relative to max(1, |slope|): slopes this close are one slope


def collinear(s1: float, s2: float) -> bool:
    return abs(s1 - s2) <= SLOPE_TOL * max(1.0, abs(s1), abs(s2))


def same_key(a, b) -> bool:
    """Two frame keys are the same input when equal up to floating point: two representations of
    one line (a ramp split into blocks) must not look like a new input (a re-plan, a dropped lead)."""
    if isinstance(a, tuple) and isinstance(b, tuple):
        return len(a) == len(b) and all(same_key(x, y) for x, y in zip(a, b))
    if isinstance(a, float) and isinstance(b, float):
        # Equal first: a cap of +inf (none) is the same input as +inf (round 19's invalid-input generator found that
        # inf - inf = NaN made every tick under no cap a new input, re-planned from the sampled state: P4 missed).
        return a == b or abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))
    return a == b


VALUE_TOL = 1e-6   # W: a piece continues the previous one if it starts within this of its value


def merge_collinear(pieces: list[Seg]) -> list[Seg]:
    """After the clamp: a piece is merged into the previous one only if it starts at the tick the
    previous one ends, at its value (within VALUE_TOL), with its slope (within SLOPE_TOL). This joins
    clamped pieces at the same value (two blocks both held at the cap M). The value test matters:
    after the clamp, the line can jump where the unclamped line was continuous (a ramp that falls
    from above M within one tick), and a flat piece at M must not swallow a lower flat block (the
    round-6 falsifier's case: 232.9 W above the plan)."""
    out: list[Seg] = []
    for p in pieces:
        if (out and p.k0 == out[-1].k0 + out[-1].n and collinear(p.s, out[-1].s)
                and abs(out[-1].at(p.k0) - p.r0) <= VALUE_TOL):
            q = out[-1]
            out[-1] = Seg(q.k0, q.r0, q.s, q.n + p.n, q.joined)
        else:
            out.append(p)
    return out


def clamp(v: float, m: float) -> float:
    return min(max(v, 0.0), m)


def line_at(segs: list[Seg], k: int, m: float) -> float:
    """The plan line at tick k as known now (the last block's end value after the plan)."""
    for sg in segs:
        if sg.k0 <= k < sg.k0 + sg.n:
            return clamp(sg.at(k), m)
    last = segs[-1]
    return clamp(last.at(last.k0 + last.n), m) if k >= last.k0 + last.n else clamp(segs[0].at(k), m)


# ---------------------------------------------------------------- the eased command (§7.10.2)

@dataclass
class State:
    y: float
    v: float = 0.0
    a: float = 0.0


def slope_change(v: float, a: float, target: float, t: Tuning) -> list[tuple[float, float]]:
    """The time-optimal jerk-limited change of slope from (v, a) to (target, 0) under a_max and
    j_max, in closed form (no Ruckig): (duration, jerk) segments. The curvature ramps to a peak
    a_p ≤ a_max, holds, and ramps back to 0 (Biagiotti and Melchiorri 2008, §3.4, for the slope).
    Used for the probes (safe braking, the floor, the lead's timing), the chase and the brake, X5's
    stop, and the landing's phases (round 19)."""
    j, amax = t.j_max, t.a_max
    vu = v + a * abs(a) / (2 * j)              # the slope once the curvature is unwound now
    s = 1.0 if target - vu > 0 else -1.0       # the sign of the curvature the change needs
    d, a1 = s * (target - v), s * a            # mirrored: a non-negative change, needing a > 0
    if abs(target - vu) <= 1e-12 and abs(a) <= 1e-12:
        return []
    ap2 = j * d + a1 * a1 / 2
    ap = math.sqrt(max(ap2, 0.0))
    t2 = 0.0
    if ap > amax:
        ap = amax
        t2 = max(0.0, (d - (2 * amax * amax - a1 * a1) / (2 * j)) / amax)
    t1 = max(0.0, (ap - a1) / j)
    t3 = ap / j
    return [(t1, s * j), (t2, 0.0), (t3, -s * j)]


def profile_at(y: float, v: float, a: float, segs: list[tuple[float, float]], tt: float) -> tuple[float, float, float]:
    """The state reached after tt seconds of a profile, then at constant slope (curvature 0). A segment of zero jerk
    whose curvature is floating-point residue (below 10⁻⁹ W/s²) is a cruise at curvature 0 (round 19: the landing's
    cruise follows a slope change ending at curvature ~10⁻¹⁵, which over a cruise of 10⁹ s at a near-zero frame rate
    would move the slope by 10⁻⁶ W/s)."""
    for d, jk in segs:
        if jk == 0.0 and abs(a) < 1e-9:
            a = 0.0
        h = min(d, tt)
        y, v, a = y + v * h + a * h * h / 2 + jk * h ** 3 / 6, v + a * h + jk * h * h / 2, a + jk * h
        tt -= h
        if tt <= 0:
            return y, v, a
    return y + v * tt, v, 0.0


def profile_extrema(y: float, v: float, a: float, segs: list[tuple[float, float]]) -> tuple[float, float]:
    """The lowest and highest position over a profile (the slope's roots inside each segment)."""
    lo = hi = y
    for d, jk in segs:
        # roots of v + a·τ + jk·τ²/2 in (0, d)
        cands = []
        if abs(jk) > 1e-15:
            disc = a * a - 2 * jk * v
            if disc >= 0:
                for sgn in (1, -1):
                    cands.append((-a + sgn * math.sqrt(disc)) / jk)
        elif abs(a) > 1e-15:
            cands.append(-v / a)
        for tau in cands:
            if 0 < tau < d:
                yy = y + v * tau + a * tau * tau / 2 + jk * tau ** 3 / 6
                lo, hi = min(lo, yy), max(hi, yy)
        y, v, a = y + v * d + a * d * d / 2 + jk * d ** 3 / 6, v + a * d + jk * d * d / 2, a + jk * d
        lo, hi = min(lo, y), max(hi, y)
    return lo, hi


def rises(st: "State", t: Tuning) -> bool:
    """Safe braking's predicate: the state rises or will rise (v > 0, or a braking fall whose
    curvature will carry it past rest: v + a²/(2j) > 0 with a > 0)."""
    return st.v > 1e-9 or (st.a > 1e-9 and st.v + st.a * st.a / (2 * t.j_max) > 1e-9)


def end_of(segs: list[tuple[float, float]], v: float, a: float) -> tuple[float, float]:
    """The displacement and duration of a profile from slope v and curvature a."""
    dur = sum(d for d, _ in segs)
    return profile_at(0.0, v, a, segs, dur)[0], dur


def root(f, lo: float, hi: float, flo: float, fhi: float, ftol: float) -> float:
    """The root of an increasing continuous f on [lo, hi], f(lo) < 0 < f(hi): the Illinois variant of regula falsi
    (a bracketing secant step, halving the stale end's value when one end repeats; a bisection step if the secant
    leaves the bracket). It never leaves the bracket, so it converges whatever the kinks of f (a slope change's
    profile switches shape where its peak curvature reaches a_max). Returns a point with |f| ≤ ftol, or the bracket's
    upper end once the bracket is a few ulps wide."""
    side = 0
    for _ in range(ROOT_STEPS):
        if hi - lo <= 4e-16 * max(1.0, abs(lo), abs(hi)):
            break
        x = (lo * fhi - hi * flo) / (fhi - flo)
        if not lo < x < hi:
            x = (lo + hi) / 2
        fx = f(x)
        if abs(fx) <= ftol:
            return x
        if fx < 0:
            lo, flo = x, fx
            if side < 0:
                fhi /= 2
            side = -1
        else:
            hi, fhi = x, fx
            if side > 0:
                flo /= 2
            side = 1
    return hi


def landing(v: float, a: float, d: float, up: float, dn: float, t: Tuning) -> list[tuple[float, float]]:
    """Round 19: the landing, in closed form. From slope v and curvature a, a move of displacement d that ends at rest
    (slope 0, curvature 0), its slopes within [-dn, up] (the frame's rates, up, dn ≥ 0), under a_max and j_max:
    1. a slope change (slope_change) from (v, a) to a peak slope p in the direction the move still needs
       (g = d − x_s, x_s the time-optimal stop's own displacement), then 2. a cruise at p, then 3. the stop from p.
    With p = 0 the profile is the stop itself (x_s). The peak that lands exactly is a root of the displacement as a
    function of p, found on a bracket (`root`); a move longer than the profile at the rate cruises at the rate for the
    rest. The bracket:
    - beyond the unwound slope vu = v + a·|a|/(2·j_max) in the move's direction (p ≥ max(0, sgn·vu)), every slope
      change starts with jerk in that direction, so a larger peak gives a pointwise larger slope and the displacement
      is monotone (self-tested): the root is unique. That branch reaches every d past its start: from p = 0 when vu is
      not in the move's direction, else from p = sgn·vu, whose profile is the unwinding and the stop from vu (x_A);
    - only a d between x_s and x_A (the stop's and the unwind-then-stop's displacements, which differ only with a
      curvature carrying the slope onwards) needs a peak between 0 and vu, where slowing to the peak and then stopping
      pauses the curvature at 0 and the displacement is not monotone; there the root exists by continuity, and the
      root finder's fixed bracket makes it deterministic.
    From rest it is the time-optimal rest-to-rest S-curve. No input is refused: where the frame allows no motion in
    the direction needed (a rate of 0), it is the stop."""
    stop = slope_change(v, a, 0.0, t)
    x_s, _ = end_of(stop, v, a)
    g = d - x_s
    if abs(g) <= 1e-12 * max(1.0, abs(d)):
        return stop
    sgn = 1.0 if g > 0 else -1.0
    rate = up if sgn > 0 else dn
    if not rate > 0:
        return stop
    prof = lambda p, cruise=0.0: slope_change(v, a, sgn * p, t) + [(cruise, 0.0)] + slope_change(sgn * p, 0.0, 0.0, t)
    x_r, _ = end_of(prof(rate), v, a)
    if sgn * (d - x_r) >= 0:
        return prof(rate, (d - x_r) / (sgn * rate))
    f = lambda p: sgn * (end_of(prof(p), v, a)[0] - d)
    lo, hi, flo, fhi = 0.0, rate, -abs(g), sgn * (x_r - d)
    pu = sgn * (v + a * abs(a) / (2 * t.j_max))
    if 0.0 < pu < rate:
        fu = f(pu)
        if fu == 0.0:
            return prof(pu)
        lo, flo, hi, fhi = (pu, fu, hi, fhi) if fu < 0 else (lo, flo, pu, fu)
    return prof(root(f, lo, hi, flo, fhi, 1e-12 * max(1.0, abs(d))))


class Profile:
    """A closed-form trajectory: (duration, jerk) segments from (x0, v0, a0), then a constant slope. Positions and
    slopes are reported in a frame moving at `frame_v` relative to the one the profile is computed in (the meet is
    computed in the absolute frame and sampled in the line's). Interface for _sample: `duration` and `at_time`."""

    def __init__(self, x0: float, v0: float, a0: float, segs: list[tuple[float, float]], frame_v: float = 0.0):
        self.x0, self.v0, self.a0, self.segs, self.frame_v = x0, v0, a0, segs, frame_v
        self.duration = sum(d for d, _ in segs)

    def at_time(self, tt: float) -> tuple[float, float, float]:
        y, v, a = profile_at(self.x0, self.v0, self.a0, self.segs, tt)
        return y - self.frame_v * tt, v - self.frame_v, a


def stop_profile(v: float, a: float, t: Tuning, target: float = 0.0) -> Profile:
    """The chase and the brake (step 4, step 6): the closed-form slope change from (v, a) to a target slope (to rest,
    for the brake and step 4's hold), positions relative to the state's. It is the very profile safe braking and the
    floor check probe, under the rider's limits."""
    return Profile(0.0, v, a, slope_change(v, a, target, t))


def meet_profile(rel: float, v: float, a: float, fs: float, up: float, dn: float, t: Tuning) -> Profile | None:
    """The meet (step 5 above a rising line; step 6's meet below a falling one): at rest on the line. In the absolute
    frame it is the landing onto a rest point D away; the line moves at fs meanwhile, so the end lies on it iff
    h(D) = rel + D − fs·T(D) = 0 (rel: the command relative to the line now, T: the landing's duration). From above a
    rising line (rel > 0, fs > 0), h(x_s) > 0 when the stop alone ends above the line, h(−rel) = −fs·T < 0, and h
    is monotone between (a rest point further from the stop takes longer), so `root` finds the meet; if the stop
    alone reaches the line, it is the stop (the line passes the command at rest). Below a falling line (rel < 0,
    fs < 0) the mirror; there a stop that reaches the line is no meet (None: step 6 brakes). Sampled in the line's
    frame (positions relative to the line, slopes minus fs)."""
    stop = slope_change(v, a, 0.0, t)
    x_s, t_s = end_of(stop, v, a)
    h = lambda D, T: rel + D - fs * T
    above = rel > 0
    if (h(x_s, t_s) <= 0) if above else (h(x_s, t_s) >= 0):
        return Profile(rel, v, a, stop, fs) if above else None
    f = lambda D: h(D, end_of(landing(v, a, D, up, dn, t), v, a)[1])
    lo, hi = (-rel, x_s) if above else (x_s, -rel)
    D = root(f, lo, hi, f(lo), f(hi), 1e-12 * max(1.0, abs(rel)))
    if not above:   # at or below the line: the root's lower side (h ≤ 0), within the root finder's tolerance
        D -= 2e-12 * max(1.0, abs(rel))
    return Profile(rel, v, a, landing(v, a, D, up, dn, t), fs)


def _cut(old: State, new: State, name: str, exc: str) -> tuple[State, str]:
    """Apply a cut; it is named (an exception) only if it changes the state by more than floating
    point: a landing that ends 1e-9 W past the line is clamped, not cut."""
    material = abs(old.y - new.y) > 1e-6 or abs(old.v - new.v) > 1e-5 or abs(old.a - new.a) > 1e-4
    return new, (name if material else exc)


class Easer:
    """crank's easer (round 19): the target frame, the limits, the closed-form trajectories and the cuts X2-X4.

    A trajectory is computed when its input changes (a new frame, new limits, a cut, an engagement) and is otherwise
    sampled tick by tick (step 7): re-planning from a sampled state each tick would make the command depend on
    sampling, and the trajectory in force is the one the probes vouched for. Every trajectory is closed form, under
    the rider's limits, so it is never refused: the only hold (X5) is the input layer's, while an input is not a
    number."""

    def __init__(self, t: Tuning):
        self.t = t
        self.k = 0
        self.wasteful = 0          # calculations on an unchanged input (step 7 says: none)
        self.traj = stop_profile(0.0, 0.0, t)
        self.held = False          # X5 on the current tick: the input is not a number, the stop then a hold
        self.invalid = 0           # ticks whose plan, cap or state was not a number (the input layer's fault)
        self.plan_invalid = False  # the target ahead is not a number before the clamp (set by the caller each tick)
        self.y_ok = 0.0            # the last command sent, for a state that is not a number (round 15)
        self.meet = False          # the trajectory in force meets the line at rest (step 5, step 6)
        self.meet_fs = 0.0
        self.frame = None          # the step's frame (c, slope, lead), for safe braking's meet from below
        self.prev_piece = None
        self.lead_id = None        # the vertex the lead in force leads into (round 18, fix round 1)
        self.key = None
        self.elapsed = 0.0
        self.base = 0.0
        self.recalcs = 0
        self.chasing = False
        self.braking = False

    def _plan(self, st: State, rel: float, fs: float, vmax: float, vmin: float, vel_target, meet: bool = False):
        """The trajectory for a new input, in closed form: the chase or the brake (a velocity target: positions from the
        state's), the meet (at rest on the line), or the landing onto the line in its frame (rel: the state's position
        relative to the line, fs its slope, [vmin, vmax] the frame's slope limits)."""
        t = self.t
        if vel_target is not None:
            self.traj = stop_profile(st.v, st.a, t, vel_target)
        elif meet:
            self.traj = meet_profile(rel, st.v, st.a, fs, vmax, -vmin, t)
        else:
            self.traj = Profile(rel, st.v - fs, st.a, landing(st.v - fs, st.a, -rel, vmax - fs, fs - vmin, t))

    def _sample(self, vel_mode: bool):
        dur, tt = self.traj.duration, self.elapsed
        if tt >= dur:
            p, v, a = self.traj.at_time(dur)
            if vel_mode:
                return p + v * (tt - dur), v, 0.0
            if self.meet:   # met at rest: it holds (the line moves on, -fs in its frame)
                return p - self.meet_fs * (tt - dur), -self.meet_fs, 0.0
            return p, v, 0.0
        return self.traj.at_time(tt)

    def _meet_below(self, st: State, nst: State, k: int, segs: list[Seg], m: float):
        """Step 6 below a falling line (round 16, f625k): where safe braking rejects a rising step, the command
        meets the line at rest (the meet, from below) instead of braking to rest. The plan onto a falling line
        passes slope 0 still decelerating, so its peak is below the line, but the probe's stop to rest from the
        same state peaks higher: braking to rest and re-planning climbed in pulses (at (50, x4, 1), 6 W/s against
        r↑ = 50 W/s, and stalled at rest while the line came down: a D5 landing in 6.25 s against 5.29 s).
        Round 17 (f625l): it meets the line one tick of its fall early, at rest |fs|·Δt below it, so the rest point
        is at or below the line at every tick boundary, the probe from the candidate passes on its own terms, and the
        line comes down to the command within one tick. Its step is still checked by safe braking (else the brake).
        It is not kept as a plan: each tick tries the ordinary plan first and meets again where safe braking rejects
        it. Returns the new state, or None (not below a falling line, in a lead, or the stop alone reaches the line, or
        rejected: brake)."""
        t = self.t
        if self.frame is None or not rises(nst, t):
            return None
        c, fs, lead = self.frame
        rel = st.y - (c + fs * (k - 1) * DT)
        if lead or not (-t.r_dn < fs < 0) or rel >= 0:
            return None
        # The target is one tick of the line's fall below it (fs·Δt in the line's frame): planned in a frame shifted
        # by that, the command's position is the plan's plus fs·Δt.
        early = fs * DT
        traj = meet_profile(rel - early, st.v, st.a, fs, t.r_up, t.r_dn, t)
        if traj is None:
            return None
        self.traj = traj
        self.meet, self.meet_fs, self.braking, self.chasing = True, fs, False, False
        self.key, self.elapsed = (("meet", c, fs), t.r_up, -t.r_dn, None, m), DT
        p, v, a = self._sample(False)
        cand = State(c + fs * k * DT + p + early, v + fs, a)
        if (rises(cand, t) and not self._brake_ok(cand, k, segs, m)) or (cand.v < -1e-9 and not self._floor_ok(cand)):
            self.key, self.meet = None, False
            return None
        return cand

    def _at_cap(self, st: State, m: float, line: float) -> State:
        """X3 before the step (round 15, f625j): the command steps down to M and keeps its falling slope only
        as far as it can still stop at or above the line there (closed form), so it never undershoots: a cap
        of 20 W under a command falling at 100 W/s kept the slope at M, fell to 0 W (X4) and climbed back."""
        floor = min(line, m)
        ok = lambda v, a: profile_extrema(m, v, a, slope_change(v, a, 0.0, self.t))[0] >= floor - 1e-9
        v, a = min(st.v, 0.0), min(st.a, 0.0)
        if ok(v, a):
            return State(m, v, a)
        lo, hi = v, 0.0                       # the stop from rest stays at M
        for _ in range(60):
            mid = (lo + hi) / 2
            lo, hi = (lo, mid) if ok(mid, 0.0) else (mid, hi)
        return State(m, hi, 0.0)

    def _stop(self, st: State) -> State:
        """X5's step (the input is not a number): one tick of the closed-form time-optimal stop from the state
        (jerk-limited, within a_max and j_max), and at rest a hold."""
        return State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t), DT))

    def _brake_ok(self, st: State, k: int, segs: list[Seg], m: float) -> bool:
        """True if the braking profile from `st` (the state at tick k: the time-optimal stop, closed
        form) stays at or below the plan line known ahead, tick by tick, until it is at rest."""
        t = self.t
        # The candidate itself first: on its own tick it may not end above the line either.
        if st.y > min(line_at(segs, k, m) + EPS / 2, m + ROUND_SNAP):
            return False
        prof = slope_change(st.v, st.a, 0.0, t)
        dur = sum(d for d, _ in prof)
        n = int(math.ceil(dur / DT))
        for i in range(1, n + 1):
            y = profile_at(st.y, st.v, st.a, prof, min(i * DT, dur))[0]
            # The line may be passed by at most EPS/2, strictly inside X2's EPS, so a braking path
            # that is accepted can never be cut; the cap only by ROUND_SNAP, what step 8 snaps.
            if y > min(line_at(segs, k + i, m) + EPS / 2, m + ROUND_SNAP):
                return False
        return True

    def _floor_ok(self, st: State) -> bool:
        """True if the braking profile from `st` (closed form) stays at or above 0 W."""
        return profile_extrema(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t))[0] >= -1e-9

    def step(self, st: State, k: int, segs: list[Seg], m: float, cut_before: bool) -> tuple[State, str]:
        """The command for tick k from the state at tick k - 1."""
        t = self.t
        self.k = k
        exc = ""
        # m is the cap in force from the input layer (run(), round 16): the last valid cap, a number >= 0 or +inf.
        # It applies on every tick, whatever else is a fault (round 15, f625j: a cap drop during a NaN offset was
        # never applied, 62.5 s above M). An invalid cap is the input layer's fault (plan_invalid).
        assert not math.isnan(m) and m >= 0.0, m
        # A state that is not a number (a NaN or ±inf origin, engagement, initial value or slope; round 15) is
        # replaced, at rest: its value if that is a number, else the last command sent (0 W before the first).
        state_ok = math.isfinite(st.y) and math.isfinite(st.v) and math.isfinite(st.a)
        if not state_ok:
            st = State(min(max(st.y if math.isfinite(st.y) else self.y_ok, 0.0), m), 0.0, 0.0)
        # A plan, cap or state that is not a number (NaN, ±inf in the line; round 14, f625i) is the input layer's fault,
        # not a plan: X5, the closed-form stop, then a hold, the status raised on every such tick, the cuts against the
        # cap in force. Normal planning resumes on the first valid tick.
        if (self.plan_invalid or not state_ok
                or any(not (math.isfinite(q.r0) and math.isfinite(q.s)) for q in segs)):
            self.invalid += 1
            self.held = True
            self.key, self.braking, self.chasing, self.prev_piece = None, False, False, None
            # The cuts against the cap in force, and X2 against the last line known (its pieces finite: an offset or a
            # cap not a number leaves the line of the last valid ones), so an unforeseeable drop on the same tick
            # stops a rise as it does outside X5 (D3). A line not a number gives no X2: nothing to compare with.
            known = bool(segs) and all(math.isfinite(q.r0) and math.isfinite(q.s) for q in segs)
            r_k = clamp(segs[0].at(k), m) if known else math.inf
            return self._after(st, self._stop(st), k, segs if known else [], m, r_k, "X5", brake=True)
        self.held = False
        # The step from tick k-1 to tick k belongs to the piece in force over that interval: at a
        # vertex on tick k (the line continuous there), that is still the previous piece. The new
        # piece takes over from the next step (the backward-slope convention of the line on the
        # tick grid). Extending the new piece back a tick would put the command "ahead" of it.
        in_force = segs[0]
        if (segs[0].k0 == k and segs[0].joined and self.prev_piece is not None and not cut_before
                and abs(self.prev_piece.at(k) - segs[0].r0) < 1e-9):   # a kink, not where the clamp starts
            segs = [self.prev_piece] + segs
        self.prev_piece = in_force
        if st.y > m:                                     # X3: the cap is never eased
            st, exc = _cut(st, self._at_cap(st, m, clamp(segs[0].at(k), m)), "X3", exc)
        cur = segs[0]
        r_k = clamp(cur.at(k), m)
        s_cur = cur.s
        within = lambda s: -t.r_dn <= s <= t.r_up
        # Step 4: a line strictly within the rates is landed on; a line at or beyond a rate cannot be caught after an
        # eased start (that would need a slope beyond the rate), so it is chased.
        track = lambda q: -t.r_dn < q.s < t.r_up
        # The frame: the line in force at tick k (c + fs·k·Δt).
        fs = s_cur
        c = r_k - fs * k * DT
        vmax, vmin = t.r_up, -t.r_dn
        # Lead into a vertex where the slope falls (a rising ramp's end, a falling ramp's start):
        # from the tick the S-curve to the next slope needs, aim at the next line with the velocity
        # capped at the current line's, so the command brakes into the vertex below the line.
        if len(segs) > 1:
            nxt = segs[1]
            nv = clamp(nxt.r0, m)
            # Identified by the lines themselves, not by where a piece's list starts: a split of one slope must not look
            # like a new lead. The lead is identified by its vertex (lead_id, kept beside the key); the key itself is the
            # frame, the input (round 18).
            lead_key = ("lead", nxt.k0, nxt.r0, nxt.s)
            # Lead at a vertex that ends or reverses a rise, or ends a fall without starting a rise;
            # never at one that starts or steepens a rise (that would raise the load before the plan).
            # Where the slope falls, the next line extended back lies above the current one, and the
            # velocity is capped at the current slope: the lead starts only from on the current line
            # (tick k-1's state against tick k-1's line, within EPS). From above it would raise the
            # command above the plan; from below the cap would hold it back from the line. A fall's
            # end aims below the current line: no condition.
            off = st.y - (r_k - s_cur * DT)
            leads = (nxt.joined and track(nxt)
                     and ((nxt.s < s_cur and abs(off) <= EPS) or (s_cur < 0 and s_cur < nxt.s <= 0)))
            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))
            if (self.key is not None and self.key[0][0] == "frame" and self.lead_id is not None
                    and same_key(self.lead_id, lead_key) and within(s_cur)):
                fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                vmax, vmin = cap_v()
            elif leads:
                # From the tick the S-curve to the next slope needs (half of it, plus one tick): not
                # earlier, or a command still catching up the current line would be held back by the
                # velocity cap; not later, or it would overshoot the vertex. The S-curve's duration is
                # closed form.
                s_dur = sum(d for d, _ in slope_change(st.v, st.a, nxt.s, t))
                if (nxt.k0 - (k - 1)) * DT <= s_dur / 2 + DT + 1e-9:   # ties lead, whatever the floats
                    fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                    vmax, vmin = cap_v() if within(s_cur) else (t.r_up, -t.r_dn)
        lead = fs != s_cur or c != r_k - s_cur * k * DT
        tracked = track(segs[1]) if lead else track(cur)   # the piece whose line is the frame
        line = lambda kk: c + fs * kk * DT
        brake = self.braking
        meet = False
        self.frame = (c, fs, lead)
        if brake:
            vel_target = 0.0
        elif tracked and fs > 0 and st.y - line(k) > EPS:
            # Above a rising line (round 14, f625i): the command comes down to it at the descent rate and
            # meets it at rest (on the line, zero slope there), so it never rises above the plan (P5); from
            # on the line, the next tick tracks it from below. Rounds 1-13 held and let the line come up:
            # after a -50 % offset onto a 0.17 W/s ramp it held 151 W high for 280 s.
            vel_target = None
            meet = True
        elif tracked:
            vel_target = None
        else:
            # A line at or beyond the rider's rate: chase it at the full rate, never away from it.
            # The gap is to where the line is now (tick k), not to the line extended back a tick.
            gap = line(k) - st.y
            # On the line (within EPS) counts as behind it: the command chases, it does not hold.
            vel_target = t.r_up if (gap > -EPS and fs > 0) else (-t.r_dn if (gap < EPS and fs < 0) else 0.0)
        key = ((("brake",) if brake else
                ("meet", c, fs) if meet else
                ("frame", c, fs)),
               vmax, vmin, vel_target, m)
        # The key is the frame, the input itself (round 18, fix round 1): the lead's frame is the next line's, so when
        # that line takes over with the same limits nothing changes and nothing is calculated. The lead's vertex is kept
        # apart in lead_id, so a lead continues only into its own vertex (a flat line equal to the next one is not a lead).
        lead_id = ("lead", segs[1].k0, segs[1].r0, segs[1].s) if (lead and not brake and not meet) else None
        unchanged = same_key(key, self.key) and not exc and not cut_before
        if not same_key(key, self.key) or exc or cut_before:
            self.recalcs += 1
            self.wasteful += unchanged
            if vel_target is None:
                self._plan(st, st.y - line(k - 1), fs, vmax, vmin, None, meet)
            else:
                self._plan(st, 0.0, 0.0, t.r_up, -t.r_dn, vel_target)
                self.base = st.y
            self.key, self.elapsed, self.meet, self.meet_fs = key, DT, meet, fs
        else:
            self.elapsed += DT
        self.lead_id = lead_id
        self.chasing = vel_target is not None
        p, v, a = self._sample(vel_target is not None)
        nst = State(line(k) + p, v + fs, a) if vel_target is None else State(self.base + p, v, a)
        return self._after(st, nst, k, segs, m, r_k, exc, brake)

    def _after(self, st: State, nst: State, k: int, segs: list[Seg], m: float, r_k: float, exc: str,
               brake: bool) -> tuple[State, str]:
        """Steps 6 and 8: safe braking, then the cuts."""
        t = self.t
        # Safe braking: a step after which the command rises or will rise (v > 0, or a braking fall
        # whose acceleration will carry it past rest) is taken only if the braking trajectory from it
        # (to rest, within the same limits) stays at or below the plan line known ahead. Otherwise the
        # command eases out from the current state and keeps easing out until it is at rest.
        # The same for a fall towards 0 W: a falling step is taken only if its braking trajectory
        # stays at or above 0 W, nothing being below it.
        if (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))
                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):
            meet_nst = self._meet_below(st, nst, k, segs, m)
            if meet_nst is not None:
                nst = meet_nst
            else:
                # The brake: the closed-form stop from st, the profile the probe accepted st on (its own tick's
                # probe), under the same limits.
                self._plan(st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0)
                self.braking, self.meet = True, False
                self.base, self.elapsed = st.y, DT
                self.key = (("brake",), t.r_up, -t.r_dn, 0.0, m)
                self.chasing = True
                p, v, a = self._sample(True)
                nst = State(self.base + p, v, a)
        if self.braking and abs(nst.v) <= 1e-9:
            self.braking = False
            self.key = None if self.traj.duration > self.elapsed + 1e-9 else self.key
        # X2 again: a step that would rise and end above the line stops instead.
        if nst.y > r_k + EPS and (nst.v > 1e-9 or nst.y > st.y + 1e-7):
            nst, exc = _cut(nst, State(st.y, 0.0, 0.0), "X2", exc)
        # A landing on M or 0 W to within floating point is rounding, not a cut: it is snapped to the bound, a
        # position change far below every tolerance (round 12).
        if m < nst.y <= m + ROUND_SNAP:
            nst = State(m, nst.v, nst.a)
        elif -ROUND_SNAP <= nst.y < 0:
            nst = State(0.0, nst.v, nst.a)
        if nst.y < 0:                                    # X4: nothing is below 0 W
            nst, exc = _cut(nst, State(0.0, 0.0, 0.0), "X4", exc)
        if nst.y > m:
            nst, exc = _cut(nst, State(m, 0.0, 0.0), "X3", exc)
        if exc and exc != "X5":
            self.key = None
        if math.isfinite(nst.y):
            self.y_ok = nst.y
        return nst, exc


# ---------------------------------------------------------------- running a plan

@dataclass
class Trace:
    r: list = field(default_factory=list)
    y: list = field(default_factory=list)
    v: list = field(default_factory=list)
    a: list = field(default_factory=list)
    on: list = field(default_factory=list)
    settle: list = field(default_factory=list)
    exc: list = field(default_factory=list)
    reset: list = field(default_factory=list)
    fell: list = field(default_factory=list)
    m: list = field(default_factory=list)
    unforeseen: list = field(default_factory=list)   # an unforeseeable event's effect is pending
    origin: list = field(default_factory=list)       # the engagement origin on a reset tick
    wasteful: int = 0
    held: list = field(default_factory=list)         # X5 on this tick: the input is not a number, the stop then a hold
    invalid: list = field(default_factory=list)      # the plan, cap or state was not a number: `erg_input_invalid` raised
    flat: list = field(default_factory=list)         # (value, last tick) of a flat piece in force, else None
    piece: list = field(default_factory=list)        # (start tick, slope, last tick, starts with a jump) of the piece in force
    events: list = field(default_factory=list)       # an event happened on this tick
    recalcs: int = 0          # trajectories computed (inputs that changed)
    y_init: float = 0.0      # the state before tick 0 (at rest unless v0 was given)
    v_init: float = 0.0


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, m0=2000.0,
        y0: float | None = None, v0: float = 0.0) -> Trace:
    blocks = [Block(b.ticks, b.frm, b.to) for b in blocks if b.ticks > 0]
    ev = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    ez = Easer(t)
    # The input layer (round 16, f625k), for each input (the cap, the ± offset, the engagement), in the order its
    # events arrive on a tick: the last valid value wins, and any invalid one raises the fault, which stays raised
    # while that input's latest value is invalid. A cap is valid if it is a number or +inf (none), below 0 W it is
    # 0 W; an offset or an engagement if it is a finite number. The easer gets the cap in force and the fault.
    cap_ok = lambda x: not (math.isnan(x) or x == -math.inf)
    mult = 1.0
    m_line = max(m0, 0.0) if cap_ok(m0) else math.inf   # the cap in force
    cap_bad, off_bad = not cap_ok(m0), False             # the latest cap / offset was invalid
    tr = Trace()
    st = None
    pending = origin is not None
    unforeseen = origin is not None      # an engagement is not known before it happens
    settle_left = 0
    prev_r, prev_s = None, 0.0
    line_key, line_full = None, []
    k = 0
    while k < sum(b.ticks for b in blocks):     # (a skip shortens a block)
        disc = reset = False
        tick_bad = False                 # an invalid value arrived on this tick
        if ev.get(k):
            unforeseen = True     # skip, ±/coach, cap or engagement: not known before it happens
        # A cap change applies first, so an engagement on the same tick starts at or below the new cap.
        for e in sorted(ev.get(k, []), key=lambda e: e.kind != "cap"):
            if e.kind == "offset":
                if math.isfinite(e.value):
                    mult, disc, off_bad = e.value, True, False
                else:
                    off_bad = tick_bad = True
            elif e.kind == "cap":
                if cap_ok(e.value):
                    m_line, cap_bad = max(e.value, 0.0), False
                else:
                    cap_bad = tick_bad = True
            elif e.kind == "skip":
                k0 = 0
                for idx, b in enumerate(blocks):
                    if k0 <= k < k0 + b.ticks:
                        done = k - k0
                        if done > 0 and idx + 1 < len(blocks):
                            blocks[idx] = Block(done, b.frm, b.frm + b.slope * done * DT)
                        break
                    k0 += b.ticks
            elif e.kind == "engage" and st is not None:
                if math.isfinite(e.value):
                    st, reset, pending = State(clamp(e.value, m_line)), True, True
                else:
                    tick_bad = True
                    if not reset:        # (no valid engagement earlier on this tick: the easer's fault, at rest)
                        st, reset, pending = State(e.value), True, True
        # The line under the cap in force, max(M, 0) of the last valid cap (round 14; round 15: a NaN or -inf
        # cap leaves the last valid one in force, as in the easer, so the checks never see M = NaN).
        me = m_line
        # The target before the clamp (the multiplier on every piece not yet ended): ±inf would clamp to M or 0 W,
        # NaN to anything; either is a fault, not a target (round 14).
        kk, finite = 0, math.isfinite(mult)
        for b in blocks:
            if kk + b.ticks > k:
                finite = finite and math.isfinite(b.frm) and math.isfinite(b.to)
            kk += b.ticks
        ez.plan_invalid = not finite or cap_bad or off_bad or tick_bad
        # The line is built from tick 0 and its pieces ending at or before k dropped last (segments), so the full line
        # is computed once per (blocks, multiplier, cap) and its tail taken: the same list, without rebuilding it on
        # every tick (a plan of 7 200 ticks rebuilt it 7 200 times).
        lkey = (tuple((b.ticks, b.frm, b.to) for b in blocks), mult, me)
        if lkey != line_key:
            line_key, line_full = lkey, segments(blocks, 0, mult, me)
        segs = [p for p in line_full if p.k0 + p.n > k]
        r = clamp(segs[0].at(k), me)
        if segs[0].k0 == k and k > 0 and not segs[0].joined:
            disc = True
        if st is None:
            if origin is not None:
                st, reset = State(clamp(origin, me) if math.isfinite(origin) else origin), True
            else:
                st = State(r if y0 is None else y0, v0, 0.0)
                tr.y_init, tr.v_init = st.y, st.v
        if any(e.kind == "cap" for e in ev.get(k, [])) and prev_r is not None and r > prev_r + prev_s * DT + 1e-6:
            disc = True    # a raised cap lets the line jump up
        origin_k = st.y if reset else None
        invalid_before = ez.invalid
        st, exc = ez.step(st, k, segs, me, reset or disc)
        invalid_now = ez.invalid > invalid_before
        on = abs(r - st.y) <= EPS
        unforeseen_now = unforeseen
        landed = on and abs(st.v - segs[0].s) <= 1e-6 and abs(st.a) <= 1e-5   # on the line, moving with it
        if landed and not ev.get(k):
            unforeseen = False
        if disc:
            pending = True
        if not on:
            settle_left = 0
        elif pending:
            pending, settle_left = False, SETTLE_AFTER_TICKS
        settle = (not on) or settle_left > 0 or pending
        if on and settle_left > 0:
            settle_left -= 1
        for lst, val in ((tr.r, r), (tr.y, st.y), (tr.v, st.v), (tr.a, st.a), (tr.on, on),
                         (tr.settle, settle), (tr.exc, exc), (tr.reset, reset),
                         (tr.fell, prev_r is not None and r < prev_r - 1e-12), (tr.m, me),
                         (tr.unforeseen, unforeseen_now), (tr.origin, origin_k),
                         (tr.held, ez.held), (tr.invalid, invalid_now),
                         (tr.flat, (r, segs[0].k0 + segs[0].n - 1) if segs[0].s == 0.0 else None),
                         (tr.piece, (segs[0].k0, segs[0].s, segs[0].k0 + segs[0].n - 1, segs[0].k0 == k and not segs[0].joined)),
                         (tr.events, bool(ev.get(k)))):
            lst.append(val)
        prev_r = r
        prev_s = segs[0].s     # the backward slope of tick k+1 is the slope of the block in force at k
        k += 1
    tr.wasteful = ez.wasteful
    tr.recalcs = ez.recalcs
    return tr


def nan_window(k0: int, n: int | None, mult: float = 1.0) -> list[Event]:
    """Invalid input (round 19's generators, replacing the refusal windows of rounds 13-18): the ± offset is not a number
    from tick k0, for n ticks (then `mult` again, a valid offset), or to the end (n None)."""
    return [Event(k0, "offset", math.nan)] + ([] if n is None else [Event(k0 + n, "offset", mult)])


# ---------------------------------------------------------------- checks (§7.10.6)

class Report:
    def __init__(self):
        self.fails: list[str] = []
        self.worst = {"X2": 0.0, "X3": 0.0, "X4": 0.0}
        self.cuts = {"X2": 0, "X3": 0, "X4": 0, "X5": 0}
        self.where = {}
        self.recoveries = 0       # recoveries from X5 (an input not a number) whose landing was checked
        self.held_runs = 0        # X5 runs whose path was checked against the closed-form stop
        self.above_checked = 0    # downward steps whose time above the line was checked (P7, round 14)
        self.plans = 0
        self.ff_plans = 0         # plans with every input valid, held to P8 (never held, the status never raised)
        self.invalid_ticks = 0    # ticks with an input not a number (`erg_input_invalid` raised), every plan

    def fail(self, msg):
        self.nfail = getattr(self, "nfail", 0) + 1
        kind = msg.split(": ", 1)[1].split(" ")[0:2] if ": " in msg else ["?"]
        self.kinds = getattr(self, "kinds", {})
        self.kinds[" ".join(kind)] = self.kinds.get(" ".join(kind), 0) + 1
        if len(self.fails) < 25:
            self.fails.append(msg)


def bounds(t: Tuning) -> dict:
    """The slope change each cut may make on the sent positions (ADR-0002 §7.10.2's table)."""
    step = t.a_max * DT
    # X3 (round 15): a step down to M at rest, or with the falling slope it can still stop on the line, so the
    # slope may change by a fall's full rate (a cap under a command falling at r↓).
    return {"X2": t.r_up + step, "X3": max(t.r_up, t.r_dn) + step, "X4": t.r_dn + step}


CUTS = ("X2", "X3", "X4")


def cap_window(tr: Trace, k: int) -> bool:
    """True if a finite-difference window ending at tick k contains a cap drop's position step."""
    for d in (1, 2):
        j = k - d
        if j >= 1 and tr.m[j] < tr.m[j - 1] - 1e-12 and tr.y[j - 1] > tr.m[j] + 1e-9:
            return True
    return False


def landing_bound(t: Tuning, delta: float, v0: float, a0: float = 0.0) -> float:
    """P4/P7's bound from any state: the time to be on (rise) or at/below (fall) a held line `delta`
    away (signed), starting with slope v0 and curvature a0. From rest it is Δ/r + T_e + 0.75 s,
    T_e = τ rising and τ(1 + f)/2 falling (§7.10.2). A moving state adds easing it out: unwinding
    the curvature (|a0|/j, which adds a0²/(2j) to the slope), then stopping the slope (v/a_max +
    a_max/j), and the distance covered meanwhile."""
    up = delta > 0
    r = t.r_up if up else t.r_dn
    t_e = t.ease if up else t.ease * (1 + t.factor) / 2
    if not v0 and not a0:
        return abs(delta) / r + t_e + 0.75
    v_eff = abs(v0) + a0 * a0 / (2 * t.j_max)
    stop_t = abs(a0) / t.j_max + v_eff / t.a_max + t.a_max / t.j_max
    stop_d = v_eff * stop_t
    return (abs(delta) + stop_d) / r + t_e + stop_t + 0.75


def check_landing(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """P4 and P7 on every plan: wherever a flat piece starts (a step, a ramp's end, an event, an
    engagement), the command reaches it within landing_bound, if the piece and the plan last that
    long without a further event. A command stuck off a held line fails here."""
    for k in range(1, len(tr.y)):
        f = tr.flat[k]
        if f is None or (tr.flat[k - 1] is not None and tr.flat[k - 1][0] == f[0] and not tr.events[k]
                         and not tr.reset[k]):
            continue
        val, last = f
        y0 = tr.origin[k] if tr.reset[k] else tr.y[k - 1]
        v0 = 0.0 if tr.reset[k] else tr.v[k - 1]
        a0 = 0.0 if tr.reset[k] else tr.a[k - 1]
        delta = val - y0
        if not all(map(math.isfinite, (delta, v0, a0))):   # an engagement not a number: X5's (check_held, D5)
            continue
        if abs(delta) <= EPS and abs(v0) < 1e-9:
            continue
        bound = landing_bound(t, delta, v0, a0)
        n = int(math.ceil(bound / DT))
        # Held within the window (X5, an input not a number): P4/P7 are not promised while held; the landing
        # after it is check_recovery's (its bound measured from the recovery tick).
        if any(tr.held[k:k + n + 1]):
            continue
        if k + n > min(last, len(tr.y) - 1) or any(tr.events[k + 1:k + n + 1]) or any(tr.reset[k + 1:k + n + 1]):
            continue
        ok = (lambda y: y <= val + EPS) if delta < 0 else (lambda y: abs(y - val) <= EPS)
        if not any(ok(tr.y[j]) for j in range(k, k + n + 1)):
            rep.fail(f"{name} k={k}: P4/P7 not at the held line {val:.2f} W within {bound:.2f} s"
                     f" (from {y0:.2f} W, slope {v0:.2f} W/s)")


def ramp_bound(t: Tuning, delta: float, v0: float, a0: float, s: float) -> float:
    """P7 (round 14) and D5 onto a line of slope s strictly within the rates: landing_bound in the line's
    frame, with the real closing rates r↑ − s (catching up from below) and r↓ + s (coming down from
    above), the slope taken relative to the line, and the ease to the rider's full falling rate. Both
    stay finite and positive for every s strictly within the rates, so the bound stays finite and tight as s → 0±
    (round 14: rounds 11–13 closed a gap above a rising line at s, the command holding while the line
    came up to it; f625i: at 0.17 W/s the line never reached it).
    - The catch-up's direction is the sign of the gap left once the relative slope is stopped (round
      13b: from rest at a 16.5 W/s ramp's start, r↑ = 20 W/s, the gap opens behind the line while the
      command speeds up); where that sign is not known from the bounds, the slower rate.
    - Where the line comes to a command that waits for it (a falling line above it: safe braking) or
      the command reaches it at rest (a rising line below it: step 5 meets the line at rest), the
      command then changes its slope from rest to s, and the line passes it meanwhile by at most
      |s|·T_s/2 (T_s = |s|/a_max + a_max/j_max bounds the jerk-limited slope change's duration, which
      covers half the line's travel): that overshoot is closed at the opposite relative rate (round
      13b, xs990000#1063: 18.69 W/s down at r↓ = 20 W/s, closed at 1.31 W/s)."""
    up, dn = t.r_up - s, t.r_dn + s
    rate_up, rate_dn = up, dn
    v_rel = v0 - s
    v_eff = abs(v_rel) + a0 * a0 / (2 * t.j_max)
    stop_t = abs(a0) / t.j_max + v_eff / t.a_max + t.a_max / t.j_max if (v_rel or a0) else 0.0
    # At most this far, in the direction of the relative slope: unwinding the curvature (|a0|/j at up to
    # v_eff), then the jerk-limited stop from v_eff, which covers v_eff·T/2 with T ≤ v_eff/a_max + a_max/j.
    stop_d = v_eff * (abs(a0) / t.j_max + (v_eff / t.a_max + t.a_max / t.j_max) / 2) if (v_rel or a0) else 0.0
    # The gap left after the stop lies between delta and delta − sign(v_rel)·stop_d (curvature: either).
    lo_g, hi_g = (delta - stop_d, delta + stop_d) if a0 else (
        (delta - stop_d, delta) if v_rel > 0 else (delta, delta + stop_d) if v_rel < 0 else (delta, delta))
    if lo_g > 0:
        r = rate_up
    elif hi_g < 0:
        r = rate_dn
    else:
        r = min(rate_up, rate_dn)
    t_s = abs(s) / t.a_max + t.a_max / t.j_max
    over = 0.0
    if hi_g > 0 and s < 0:
        over = max(over, abs(s) * t_s / 2 / dn + t_s)
    if lo_g < 0 and s > 0:
        over = max(over, s * t_s / 2 / up + t_s)
    d = abs(delta) + stop_d
    return d / r + over + t.ease * (1 + t.factor) / 2 + stop_t + 0.75


def check_recovery(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """D5, the landing after X5 (round 13; round 19: X5 is the input layer's hold): on the tick the input is valid
    again and normal planning resumes, the command is on a held line (from below), or at or below it (from
    above), within landing_bound from the state it recovers from; on a ramp strictly within the
    rates, within ramp_bound. Measured from the recovery tick, if the piece and the plan last that
    long with no further event, engagement or held tick. (The tick the input is valid again carries its event: the
    valid offset or cap; that is the recovery tick.)"""
    n = len(tr.y)
    for k in range(1, n):
        if not tr.held[k - 1] or tr.held[k] or tr.reset[k] or (tr.events[k] and not tr.invalid[k - 1]):
            continue
        k0, s, last, jumped = tr.piece[k]
        y0, v0, a0 = tr.y[k - 1], tr.v[k - 1], tr.a[k - 1]
        if s == 0.0:
            delta = tr.r[k] - y0
            if abs(delta) <= EPS and abs(v0) < 1e-9:
                continue
            bound = landing_bound(t, delta, v0, a0)
        elif -t.r_dn < s < t.r_up:
            delta = (tr.r[k] - s * DT) - y0
            if abs(delta) <= EPS and abs(v0 - s) < 1e-9:
                continue
            bound = ramp_bound(t, delta, v0, a0, s)
        else:
            continue
        end = k + int(math.ceil(bound / DT))
        if (end > min(last, n - 1) or any(tr.events[k + 1:end + 1]) or any(tr.reset[k + 1:end + 1])
                or any(tr.held[k:end + 1])):
            continue
        rep.recoveries += 1
        ok = (lambda j: tr.y[j] <= tr.r[j] + EPS) if delta < 0 else (lambda j: abs(tr.y[j] - tr.r[j]) <= EPS)
        if not any(ok(j) for j in range(k, end + 1)):
            rep.fail(f"{name} k={k}: X5 recovery not on the line (slope {s:.2f} W/s) within {bound:.2f} s"
                     f" (from {y0:.2f} W, {delta:+.2f} W off, slope {v0:.2f} W/s)")


def check_above(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """P7 on every line (round 14, f625i): after a downward step (a skip, ±/coach, cap or a jump down of
    the line), a command above the line is at or below it within the bound: landing_bound onto a held
    line (P7 as before), ramp_bound onto a line of slope s strictly within the rates, rising or falling.
    The rider or coach lowering the load takes effect in bounded time whatever the line's slope.
    Measured from the step's tick, if the piece and the plan last that long with no further event,
    engagement or held tick."""
    n = len(tr.y)
    for k in range(1, n):
        k0, s, last, jumped = tr.piece[k]
        if tr.reset[k] or not (tr.events[k] or jumped) or not -t.r_dn < s < t.r_up:
            continue
        y0, v0, a0 = tr.y[k - 1], tr.v[k - 1], tr.a[k - 1]
        delta = (tr.r[k] - s * DT) - y0
        if not math.isfinite(delta) or delta >= -EPS:
            continue
        bound = landing_bound(t, delta, v0, a0) if s == 0.0 else ramp_bound(t, delta, v0, a0, s)
        end = k + int(math.ceil(bound / DT))
        if (end > min(last, n - 1) or any(tr.events[k + 1:end + 1]) or any(tr.reset[k + 1:end + 1])
                or any(tr.held[k:end + 1])):
            continue
        rep.above_checked += 1
        if not any(tr.y[j] <= tr.r[j] + EPS for j in range(k, end + 1)):
            rep.fail(f"{name} k={k}: P7 still above the line (slope {s:.2f} W/s) after {bound:.2f} s"
                     f" (from {y0:.2f} W, {delta:+.2f} W off, slope {v0:.2f} W/s)")


def check_cap_floor(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """X3 never undershoots (round 16, f625k K12): after a cap cut at tick k, the command does not fall below the
    line it capped there, min(line_k, M), by more than ε, until it has landed on the line (within ε and moving
    with it: its slope the line's, no curvature), the line moves on to its next piece, an event, an engagement or
    X5; that is the floor the cut keeps its slope for (_at_cap). Round 17 (f625l): it stopped at the
    first tick within ε of the line, so a command falling through the line there went on to 179 W under a 300 W
    line unchecked; passing through is not landing. Near the trace's end the line ahead is unseen, so it stops.
    Falling below it is the failure. A line that rises past the command afterwards does not raise the floor, and a
    lead into the line ahead (step 3: within one slope change, max(r↑, r↓)/a_max + a_max/j_max) lowers it.
    The cut keeps only the falling slope (and curvature) whose stop stays on that line; keeping a negative
    curvature after reducing the slope fell 237.5 W below a flat 300 W line at (50, ×4, 4)."""
    n = len(tr.y)
    ahead = int(math.ceil((max(t.r_up, t.r_dn) / t.a_max + t.a_max / t.j_max) / DT))
    for k in range(n):
        if tr.exc[k] != "X3":
            continue
        for j in range(k, n):
            if j > k and (tr.events[j] or tr.reset[j] or tr.held[j] or tr.exc[j] in CUTS
                          or tr.piece[j][0] != tr.piece[k][0]):
                break
            if j + ahead >= n:      # the line ahead runs past the trace (the plan's end): the lead into it is unseen
                break
            if j > k and abs(tr.y[j - 1] - tr.r[j - 1]) <= EPS and abs(tr.v[j - 1] - tr.piece[j - 1][1]) <= 1e-6 \
                    and abs(tr.a[j - 1]) <= 1e-5:
                break               # landed on the line, moving with it (not merely passing through)
            floor = min(tr.r[k], tr.m[k], min(tr.r[j:j + ahead + 1]))
            if tr.y[j] < floor - EPS and (j == k or tr.y[j] < tr.y[j - 1] - 1e-9):
                rep.fail(f"{name} k={j}: below the capped line after X3 at k={k}: {tr.y[j]:.3f} W < {floor:.3f} W - ε")
                return


def check_state(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """The state the easer plans from is the state it sent (round 14): over a tick at piecewise-constant
    jerk, the position step is Δt·(v0 + v1)/2 − (a1 − a0)·Δt²/12 to within j_max·Δt³/6. Ticks with a cut
    (a position step by definition) or an engagement are not tested. A trajectory sampled past its end
    that moves the position but reports another slope fails here (mutant: the meet's end riding the
    line while reporting rest)."""
    tol = t.j_max * DT ** 3 / 6 + 1e-6
    for k in range(1, len(tr.y)):
        if tr.reset[k] or tr.exc[k] in CUTS:
            continue
        want = DT * (tr.v[k - 1] + tr.v[k]) / 2 - (tr.a[k] - tr.a[k - 1]) * DT * DT / 12
        if abs(tr.y[k] - tr.y[k - 1] - want) > tol:
            rep.fail(f"{name} k={k}: state not the one sent: Δy {tr.y[k] - tr.y[k - 1]:.6f} W, from the slope and"
                     f" curvature {want:.6f} W (tolerance {tol:.2g} W)")
            return


def check_held(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """X5's path (the input is not a number): from its first tick, the command follows the closed-form
    time-optimal stop from the state it had (jerk-limited, within a_max and j_max), then holds at
    rest. Checked tick by tick to 1e-6 W until the input is valid again, an unforeseeable event or an
    engagement changes the state, or a cut does (each judged by its own bound and domain)."""
    n = len(tr.y)
    k = 0
    while k < n:
        if not tr.held[k] or tr.exc[k] in CUTS:
            k += 1
            continue
        if tr.reset[k]:
            y0, v0, a0 = tr.origin[k], 0.0, 0.0
        elif k == 0:
            y0, v0, a0 = tr.y_init, tr.v_init, 0.0
        else:
            y0, v0, a0 = tr.y[k - 1], tr.v[k - 1], tr.a[k - 1]
        if not all(map(math.isfinite, (y0, v0, a0))):   # a state not a number: replaced at rest (step 1)
            k += 1
            continue
        prof = slope_change(v0, a0, 0.0, t)
        rep.held_runs += 1
        j = k
        while (j < n and tr.held[j] and tr.exc[j] not in CUTS
               and (j == k or (not tr.events[j] and not tr.reset[j]))):
            want = profile_at(y0, v0, a0, prof, (j - k + 1) * DT)[0]
            if abs(tr.y[j] - want) > 1e-6:
                rep.fail(f"{name} k={j}: X5 off the closed-form stop from tick {k} "
                         f"(y={tr.y[j]:.6f}, the stop {want:.6f})")
                break
            j += 1
        k = max(j, k + 1)


def check_status(tr: Trace, rep: Report, name: str) -> None:
    """P8 (round 19): the easer never refuses and is never held while every input is valid. X5 runs exactly on the ticks
    whose input is not a number, and `erg_input_invalid` (the status, the trace's `invalid`) is raised on exactly those."""
    bad = [k for k in range(len(tr.y)) if tr.held[k] != tr.invalid[k]]
    if bad:
        rep.fail(f"{name} k={bad[0]}: P8 held {tr.held[bad[0]]} with the input invalid {tr.invalid[bad[0]]}")
    if not any(tr.invalid):
        rep.ff_plans += 1
    rep.invalid_ticks += sum(tr.invalid)


def check(tr: Trace, t: Tuning, rep: Report, name: str, y_start: float) -> None:
    """Every property on the SENT positions y_k: slope, curvature and jerk are their finite
    differences, so a cut cannot hide behind the easer's internal state. A window of finite
    differences that contains a cut is judged by that cut's bound and domain instead."""
    rep.plans += 1
    if tr.wasteful:
        rep.fail(f"{name}: {tr.wasteful} trajectories computed on an unchanged input (step 7)")
    bnd = bounds(t)
    # The limits as ADR-0002 §7.10.2 states them, computed here and not taken from the easer, so a
    # change to the easer's mapping cannot move the check with it.
    a_lim, j_lim = 2 * t.rise / t.ease, 4 * t.rise / t.ease ** 2
    tol_v, tol_a, tol_j = 1e-6, 1e-5, 1e-4
    ys, cut, start = [], [], 0
    for k in range(len(tr.y)):
        y, r, exc, m = tr.y[k], tr.r[k], tr.exc[k], tr.m[k]
        if any(math.isnan(x) or math.isinf(x) for x in (y, tr.v[k], tr.a[k])):
            rep.fail(f"{name} k={k}: NaN")
            return
        if tr.held[k]:
            rep.cuts["X5"] += 1
        if tr.reset[k]:            # an engagement restarts from rest at its origin
            # (an origin not a number is replaced at rest, step 1 of X5: the value sent on that tick)
            o = tr.origin[k] if math.isfinite(tr.origin[k]) else y
            ys, cut, start = [o, o, o], [False, False, False], k
        elif k == 0:
            y0 = tr.y_init
            if tr.v_init == 0.0:
                ys, cut = [y0, y0, y0], [False, False, False]
            else:
                ys, cut = [y0], [False]
        prev = ys[-1]
        ys.append(y)
        is_cut = exc in CUTS
        cut.append(is_cut)
        n = len(ys) - 1
        vs = (ys[n] - ys[n - 1]) / DT
        cap_drop = (exc == "X3" and k > 0 and tr.m[k] < tr.m[k - 1] - 1e-12 and not tr.reset[k]
                    and prev > tr.m[k] + 1e-9)
        # P2 on the sent positions (a cap drop is a position step down, never up).
        if not cap_drop and not -t.r_dn - tol_v <= vs <= t.r_up + tol_v:
            rep.fail(f"{name} k={k}: P2 sent slope {vs:.4f}")
        if n >= 2:
            vs0 = (ys[n - 1] - ys[n - 2]) / DT
            curv = (vs - vs0) / DT
            jerk = None
            if n >= 3:
                vs00 = (ys[n - 2] - ys[n - 3]) / DT
                jerk = ((vs - vs0) - (vs0 - vs00)) / DT / DT
            within = abs(curv) <= a_lim + tol_a and (jerk is None or abs(jerk) <= j_lim + tol_j)
            # A window that contains a cut tick's slope (curvature: the cut tick and the next; jerk:
            # the cut tick and the next two) may exceed the band by that cut's own slope change Δv_c:
            # curvature by Δv_c/Δt, jerk by 2·Δv_c/Δt². The cut tick itself is judged by its bound.
            culprit = tr.exc[k] if cut[n] else None
            if culprit is None and not within:
                for d in (1, 2):
                    if n - d >= 2 and cut[n - d]:
                        kc = k - d
                        dv_c = max(abs((ys[n - d] - ys[n - d - 1]) - (ys[n - d - 1] - ys[n - d - 2])) / DT,
                                   abs(tr.v[kc] - tr.v[kc - 1]) if kc >= 1 else 0.0)
                        curv_ok = abs(curv) <= a_lim + (dv_c / DT if d == 1 else 0.0) + tol_a
                        jerk_ok = jerk is None or abs(jerk) <= j_lim + 2 * dv_c / DT ** 2 + tol_j
                        if curv_ok and jerk_ok:
                            within = True
                        break
            if cap_window(tr, k):
                within = True      # a cap drop's position step: windows containing it are judged by X3
            if cap_drop:
                # A position step down to the new cap; the slope is kept if it was falling (X3 never
                # stops a fall), so the state's own slope change stays within X3's bound.
                dvs = abs(tr.v[k] - tr.v[k - 1])
                if dvs > bnd[exc] + tol_v:   # X3 keeps a falling slope
                    rep.fail(f"{name} k={k}: {exc} at a cap drop changed the slope by {dvs:.3f} > {bnd[exc]:.3f}")
            elif not within and culprit is None:
                rep.fail(f"{name} k={k}: P3 sent curvature {curv:.3f} (<= {a_lim:.3f}) / jerk "
                         f"{jerk if jerk is None else round(jerk, 3)} (<= {j_lim:.3f})")
            elif not within and is_cut:
                prev_cap_drop = k > 1 and tr.m[k - 1] < tr.m[k - 2] - 1e-12 and ys[n - 2] > tr.m[k - 1] + 1e-9
                # After a cap's position step the sent slope has no meaningful previous value: the
                # state's slope change is judged instead.
                dv = abs(tr.v[k] - tr.v[k - 1]) if prev_cap_drop else abs(vs - vs0)
                rep.cuts[exc] += 1
                rep.worst[exc] = max(rep.worst[exc], dv)
                if dv > bnd[exc] + tol_v:
                    rep.fail(f"{name} k={k}: {exc} slope change {dv:.3f} > bound {bnd[exc]:.3f}")
        # Domains: X2, X3 and X4 only while an unforeseeable event is pending (round 11; X5's stop is checked
        # against the line ahead and 0 W by safe braking like any step). On every cut tick, also one whose sent samples stay
        # inside the band (round 12: an X4 at rest was not tested).
        if is_cut and not tr.unforeseen[k]:
            rep.fail(f"{name} k={k}: {exc} outside its domain (no unforeseeable event pending)")
        # P5: a step that rises never ends more than EPS above the line.
        if y > r + EPS + TOL and y > prev + TOL:
            rep.fail(f"{name} k={k}: P5 rose to above the line y={y:.3f} R={r:.3f}")
        # P6: the excess grows only on a tick where the line falls; 0 <= y <= M.
        if k > 0 and not tr.reset[k]:
            e0, e1 = max(0.0, prev - tr.r[k - 1] - EPS), max(0.0, y - r - EPS)
            if e1 > e0 + TOL and not tr.fell[k]:
                rep.fail(f"{name} k={k}: P6 excess grew without a falling line ({e0:.3f}->{e1:.3f})")
        if y < -TOL or y > m + TOL:
            rep.fail(f"{name} k={k}: y={y:.3f} outside [0, M={m}]")
        if tr.held[k] and not exc:
            rep.fail(f"{name} k={k}: X5 without its tag")
        if exc == "X4" and (tr.v[k] < -TOL or tr.a[k] < -TOL):
            rep.fail(f"{name} k={k}: X4 left a falling slope at 0 W")
    check_landing(tr, t, rep, name)
    check_above(tr, t, rep, name)
    check_state(tr, t, rep, name)
    check_cap_floor(tr, t, rep, name)
    check_recovery(tr, t, rep, name)
    check_held(tr, t, rep, name)
    check_status(tr, rep, name)


# ---------------------------------------------------------------- the port's generator (round 18, fix round 1)

M64 = (1 << 64) - 1

class SplitMix:
    """The Rust harness's splitmix64 (#620, slew/tests/harness.rs `Rng`): `random()` is its `next_f64`."""
    def __init__(self, seed): self.s = seed & M64
    def random(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & M64
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        z ^= z >> 31
        return (z >> 11) / float(1 << 53)
    def uniform(self, lo, hi): return lo + (hi - lo) * self.random()
    def below(self, n): return min(int(self.random() * n), n - 1)
    def pick(self, xs): return xs[self.below(len(xs))]

def rround(x):  # Rust f64::round: half away from zero
    return math.floor(abs(x) + 0.5) * (1 if x >= 0 else -1)

def port_plan(seed: int):
    """Round 18 (fix round 1): the port's random plan for a proptest seed (#620, slew/tests/props.rs `plan_from`), so the
    port's own repros run in the model: (tuning, blocks, events, origin, cap). (Rounds 18's fault injection drew SplitMix
    numbers from the same seed; round 19 has no refusals to inject.)"""
    rng = SplitMix(seed)
    grid = [Tuning(r, f, e) for r in (5, 8, 10, 15, 20, 30, 40, 50) for f in (1.0, 1.25, 1.5, 2.0, 3.0, 4.0)
            for e in (1.0, 1.5, 2.0, 2.75, 3.0, 4.0) if r * f >= 20.0]
    t = rng.pick(grid)
    ticks_of = lambda secs: int(rround(secs / DT))
    steady = lambda w, secs: Block(ticks_of(secs), w, w)
    ramp = lambda a, b, secs: Block(ticks_of(secs), a, b)
    rise_ease = t.rise * t.ease
    blocks = []
    w = rround(rng.uniform(0.0, 500.0) * 4.0) / 4.0
    for _ in range(2 + rng.below(7)):
        secs = rng.pick([0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0])
        kind = rng.random()
        if kind < 0.45:
            pick = rng.random()
            if pick < 0.4:
                sign = rng.pick([-1.0, 1.0])
                w = max(w + sign * rise_ease * rng.pick([1.0, 2.0, 3.0, 0.5]), 0.0)
            elif pick < 0.7:
                w = rround(rng.uniform(0.0, 600.0) * 4.0) / 4.0
            else:
                w = rng.uniform(0.0, 600.0)
            blocks.append(steady(w, secs))
        else:
            start = w if rng.random() < 0.6 else rng.uniform(0.0, 600.0)
            slope = rng.uniform(-t.r_dn, t.r_up) if kind < 0.8 else rng.pick([-1.0, 1.0]) * rng.uniform(t.r_up, 3.0 * t.r_dn)
            end = max(start + slope * secs, 0.0)
            n = ramp(start, end, secs).ticks
            if n >= 3 and rng.random() < 0.35:
                cuts = sorted(set([1 + rng.below(n - 1), 1 + rng.below(n - 1)]))
                pts = [0] + cuts + [n]
                vals = []
                for i, p in enumerate(pts):
                    v = start + (end - start) * p / n
                    vals.append(v if i == 0 or i == len(pts) - 1 else v + rng.uniform(-1e-12, 1e-12))
                for i in range(len(pts) - 1):
                    blocks.append(Block(pts[i + 1] - pts[i], vals[i], vals[i + 1]))
            else:
                blocks.append(ramp(start, end, secs))
            w = end
    extra = rng.random()
    if extra < 0.25:
        hi = rng.uniform(800.0, 1500.0)
        low = 0.0 if rng.random() < 0.5 else rng.uniform(0.0, 400.0)
        at = rng.below(len(blocks) + 1)
        ins = [steady(hi, rng.pick([1.0, 2.0, 4.0, 8.0])), ramp(hi, low, rng.pick([0.25, 0.5, 1.0])),
               steady(low, rng.pick([5.0, 15.0, 30.0]))]
        blocks[at:at] = ins
    elif extra < 0.45:
        at = rng.below(len(blocks) + 1)
        top = rng.uniform(50.0, 500.0)
        ins = [steady(0.0, rng.pick([1.0, 5.0])), ramp(0.0, top, rng.pick([0.25, 1.0, 5.0, 20.0])),
               steady(top, rng.pick([5.0, 15.0]))]
        blocks[at:at] = ins
    n = sum(b.ticks for b in blocks)
    events = []
    for _ in range(rng.below(4)):
        kind = rng.pick(["offset", "cap", "skip", "engage"])
        value = {"offset": None, "cap": None, "skip": 0.0, "engage": None}[kind]
        if kind == "offset": value = rng.pick([0.9, 0.95, 1.05, 1.1])
        elif kind == "cap": value = rng.uniform(150.0, 700.0)
        elif kind == "engage": value = rng.uniform(0.0, 400.0)
        events.append(Event(rng.below(max(n, 1)), kind, value))
    origin = rng.uniform(0.0, 300.0) if rng.random() < 0.3 else None
    m0 = 2000.0 if rng.random() < 0.5 else rng.uniform(200.0, 800.0)
    return t, blocks, events, origin, m0


def seeded_plan(seed: int, idx: int):
    """randoms()' plan idx for a seed: (tuning, blocks, events, origin, cap)."""
    rng = random.Random(seed)
    for _ in range(idx + 1):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        m0 = rng.choice([2000.0, rng.uniform(200, 800)])
    return t, blocks, events, origin, m0


def steady(w, secs):
    return Block(int(round(secs / DT)), w, w)


def ramp(a, b, secs):
    return Block(int(round(secs / DT)), a, b)


def land_time(tr: Trace, after: int = 0):
    for k in range(len(tr.on) - 1, -1, -1):
        if not tr.on[k]:
            return (k + 1 - after) * DT if k + 1 < len(tr.on) else None
    return 0.0


def excluded_seconds(tr: Trace, lo: int, hi: int) -> int:
    return sum(1 for sec in range(lo // 4, hi // 4) if any(tr.settle[sec * 4:(sec + 1) * 4]))


def tables(rep: Report, report: bool) -> dict:
    q = {}
    for a, b in ((150, 170), (137, 220), (100, 250), (100, 400)):
        up, dn = run([steady(a, 5), steady(b, 60)]), run([steady(b, 5), steady(a, 60)])
        check(up, DEFAULT, rep, f"rise {a}->{b}", a)
        check(dn, DEFAULT, rep, f"fall {b}->{a}", b)
        q[f"jump {b - a} W rising / falling"] = (land_time(up, 20), land_time(dn, 20))
    q["engagement 0->137 / 0->250 W"] = (land_time(run([steady(137, 60)], origin=0.0)),
                                         land_time(run([steady(250, 60)], origin=0.0)))
    q["owner's case: ramp 100->180/60 s into 220 W, lands after the boundary"] = land_time(
        run([steady(100, 10), ramp(100, 180, 60), steady(220, 60)]), 280)
    for name, p in (("steep 100->400 W over 10 s", [steady(100, 5), ramp(100, 400, 10), steady(400, 30)]),
                    ("steep 400->100 W over 5 s", [steady(400, 5), ramp(400, 100, 5), steady(100, 30)])):
        tr = run(p)
        check(tr, DEFAULT, rep, name, tr.r[0])
        q[name + ": lands after its start, v range"] = (land_time(tr, 20), round(min(tr.v), 3), round(max(tr.v), 3))
    for tt in (DEFAULT, Tuning(50, 1.0, 1.0)):
        q[f"first second of an engagement 0->400 W, rise {tt.rise} ease {tt.ease}"] = round(
            run([steady(400, 30)], t=tt, origin=0.0).y[3], 3)
    ramps = {}
    for a, b, secs in ((100, 400, 60), (50, 150, 30), (300, 100, 60), (100, 400, 120), (100, 300, 10),
                       (300, 100, 5)):
        tr = run([steady(a, 10), ramp(a, b, secs), steady(b, 15)])
        check(tr, DEFAULT, rep, f"ramp {a}->{b}/{secs}", a)
        ramps[f"{a}->{b} W over {secs} s ({abs(b - a) / secs:g} W/s)"] = (
            sum(1 for o in tr.on if not o), excluded_seconds(tr, 0, len(tr.y)),
            round(max(r - y for r, y in zip(tr.r, tr.y)), 2), round(max(y - r for r, y in zip(tr.r, tr.y)), 3))
    cases = {
        "skip 100->112 W": run([steady(100, 10), steady(112, 10), steady(112, 20)], [Event(44, "skip")]),
        "skip 300->250 W": run([steady(300, 10), steady(250, 10), steady(250, 20)], [Event(44, "skip")]),
        "skip mid-rise 100->400 W into 150 W": run([steady(100, 5), steady(400, 20), steady(150, 20)],
                                                  [Event(28, "skip")]),
        "engage at 260 W on a -20 W/s ramp": run([steady(300, 5), ramp(300, 100, 10), steady(100, 30)],
                                                [Event(28, "engage", 260.0)]),
        "5 s at 400 W, then 150 W": run([steady(100, 10), steady(400, 5), steady(150, 30)]),
        "3 s at 400 W, then 100 W": run([steady(100, 10), steady(400, 3), steady(100, 30)]),
        "ramp at exactly 20 W/s, both corners": run([steady(100, 10), ramp(100, 300, 10), steady(300, 20)]),
        "manual -10 % mid-rise": run([steady(100, 5), steady(400, 30)], [Event(30, "offset", 0.9)]),
        "cap drops to 250 W mid-rise": run([steady(100, 5), steady(400, 30)], [Event(34, "cap", 250.0)]),
    }
    for name, tr in cases.items():
        check(tr, DEFAULT, rep, name, tr.r[0])
    short = {}
    for tt, tag in ((DEFAULT, "defaults"), (Tuning(50, 4.0, 1.0), "fastest")):
        for name, on_s in (("30/30", 30), ("15/15", 15)):
            tr = run([b for _ in range(6) for b in (steady(100, on_s), steady(400, on_s))] + [steady(100, 30)], t=tt)
            check(tr, tt, rep, f"{name} {tag}", 100)
            n = int(on_s / DT)
            seg = tr.y[9 * n:10 * n]
            short[f"{tag} {name}"] = (round(max(seg)), round(sum(seg) / len(seg)))
        tr = run([steady(100, 10), steady(600, 5), steady(100, 20)], t=tt)
        check(tr, tt, rep, f"5 s at 600 {tag}", 100)
        short[f"{tag} 5 s at 600 W after 100 W"] = (round(max(tr.y)), None)
    # The figures ADR-0002 §7.10 quotes: a change to any rule that moves one must amend the ADR.
    got = {
        "rise 300 W": q["jump 300 W rising / falling"][0], "fall 300 W": q["jump 300 W rising / falling"][1],
        "rise 83 W": q["jump 83 W rising / falling"][0], "fall 83 W": q["jump 83 W rising / falling"][1],
        "engage 137": q["engagement 0->137 / 0->250 W"][0], "engage 250": q["engagement 0->137 / 0->250 W"][1],
        "owner": q["owner's case: ramp 100->180/60 s into 220 W, lands after the boundary"],
        "steep up": q["steep 100->400 W over 10 s: lands after its start, v range"][0],
        "steep down": q["steep 400->100 W over 5 s: lands after its start, v range"][0],
        "ramp 5 W/s off-line": ramps["100->400 W over 60 s (5 W/s)"][0],
        "ramp 5 W/s excluded": ramps["100->400 W over 60 s (5 W/s)"][1],
        "ramp 3.3 W/s fall off-line": ramps["300->100 W over 60 s (3.33333 W/s)"][0],
        "ramp 2.5 W/s excluded": ramps["100->400 W over 120 s (2.5 W/s)"][1],
        "skip 300->250 lands": land_time(cases["skip 300->250 W"]),
        "hold-above case lands": None,
        "15/15 peak": short["defaults 15/15"][0], "15/15 mean": short["defaults 15/15"][1],
        "30/30 mean": short["defaults 30/30"][1],
    }
    got.pop("hold-above case lands")
    want = {"rise 300 W": 16.25, "fall 300 W": 9.75, "rise 83 W": 5.5, "fall 83 W": 4.5, "engage 137": 8.25,
            "engage 250": 13.75, "owner": 3.25, "steep up": 16.5, "steep down": 10.0,
            "ramp 5 W/s off-line": 6, "ramp 5 W/s excluded": 2, "ramp 3.3 W/s fall off-line": 0,
            "ramp 2.5 W/s excluded": 1, "skip 300->250 lands": 13.75, "15/15 peak": 360, "15/15 mean": 232,
            "30/30 mean": 316}
    for key, w in want.items():
        if got[key] is None or abs(got[key] - w) > 1e-9:
            rep.fail(f"quoted: {key}: model {got[key]}, ADR {w}")
    if report:
        print("Durations and figures (defaults unless named):")
        for k, v in q.items():
            print(f"  {k}: {v}")
        print("Ramps within the rates, entered from an equal target (defaults):"
              " off-line ticks, excluded s, max R - y W, max y - R W")
        for k, v in ramps.items():
            print(f"  {k}: {v}")
        print("Falsifier cases (defaults): max y - R, max R - y, on the line after, cuts, max |v|, max |a|")
        for name, tr in cases.items():
            print(f"  {name}: {max(y - r for y, r in zip(tr.y, tr.r)):.2f} W, {max(r - y for y, r in zip(tr.y, tr.r)):.2f} W,"
                  f" {land_time(tr)} s, {sorted(set(e for e in tr.exc if e))},"
                  f" {max(map(abs, tr.v)):.2f} W/s, {max(map(abs, tr.a)):.2f} W/s²")
        print("Short intervals (the 5th rep's hard block): peak, mean")
        for k, v in short.items():
            print(f"  {k}: {v}")
    return {"q": q, "ramps": ramps, "short": short}


def corners(rep: Report, report: bool):
    plans = [
        [steady(100, 5), steady(400, 60), steady(100, 60)],
        [steady(100, 5), ramp(100, 400, 30), steady(400, 10), ramp(400, 100, 15), steady(100, 30)],
        [steady(100, 5), ramp(100, 400, 5), steady(400, 30), ramp(400, 100, 2), steady(100, 30)],
        [b for _ in range(5) for b in (steady(100, 15), steady(400, 15))],
        [steady(300, 5), ramp(300, 100, 10), steady(100, 20)],
    ]
    n = 0
    for t in CORNERS:
        for i, p in enumerate(plans):
            for origin in (None, 0.0):
                tr = run(p, t=t, origin=origin)
                check(tr, t, rep, f"corner {t} plan {i} origin {origin}", tr.r[0] if origin is None else 0.0)
                n += 1
    if report:
        print(f"Bounds corners: {len(CORNERS)} tunings x {len(plans)} plans x (on the line, engaged from 0 W): {n} runs")


GOLDEN_PLANS = {
    # X5 (round 19: the input layer's hold, rounds 13-18's degraded plans): an offset not a number mid-landing, to the
    # end (the closed-form stop, then a hold), or for 20 ticks (then D5's landing).
    "X5 mid-rise from tick 30, 100 -> 400 W, defaults": (DEFAULT, [steady(100, 5), steady(400, 40)],
                                                         [Event(30, "offset", 1.02)] + nan_window(30, None), None),
    "X5 mid-fall from tick 26, 400 -> 100 W, (50, x4, 4)": (Tuning(50, 4.0, 4.0), [steady(400, 5), steady(100, 40)],
                                                            [Event(26, "offset", 0.98)] + nan_window(26, None), None),
    "X5 ticks 40..59 mid-fall, 600 -> 100 W, (5, x4, 4)": (Tuning(5, 4.0, 4.0), [steady(600, 5), steady(100, 60)],
                                                           [Event(40, "offset", 0.98)] + nan_window(40, 20, 0.98), None),
    # name: (tuning, blocks, events, origin)
    "ramp 100->400 W over 60 s": (DEFAULT, [steady(100, 10), ramp(100, 400, 60), steady(400, 15)], [], None),
    "fall 300->100 W over 60 s": (DEFAULT, [steady(300, 10), ramp(300, 100, 60), steady(100, 15)], [], None),
    "ramp at the rate 100->300 W over 10 s": (DEFAULT, [steady(100, 10), ramp(100, 300, 10), steady(300, 20)], [], None),
    "steep 100->400 W over 10 s": (DEFAULT, [steady(100, 5), ramp(100, 400, 10), steady(400, 30)], [], None),
    "steep 400->100 W over 5 s": (DEFAULT, [steady(400, 5), ramp(400, 100, 5), steady(100, 30)], [], None),
    "steep rise behind a fall": (DEFAULT, [steady(300, 5), steady(250, 2), ramp(250, 450, 5), steady(450, 20)], [], None),
    "jump 100->400 W": (DEFAULT, [steady(100, 5), steady(400, 30)], [], None),
    "jump 400->100 W": (DEFAULT, [steady(400, 5), steady(100, 30)], [], None),
    "5 s at 400 W then 150 W": (DEFAULT, [steady(100, 10), steady(400, 5), steady(150, 30)], [], None),
    "ramp 37 -> 49.5 W/s (50, x2, 1)": (Tuning(50, 2.0, 1.0), [steady(100, 10), ramp(100, 840, 20),
                                       ramp(840, 1087.5, 5), steady(1087.5, 10)], [], None),
    "rise into a falling ramp (15, x2, 2)": (Tuning(15, 2.0, 2.0), [steady(100, 5), ramp(100, 160, 6),
                                             ramp(160, 130, 6), steady(130, 15)], [], None),
    "engagement 0->250 W": (DEFAULT, [steady(250, 30)], [], 0.0),
    "engagement above a rising ramp (50, x1.5, 1)": (Tuning(50, 1.5, 1.0), [ramp(8.8, 182.1, 5), steady(182.1, 10)],
                                                     [], 61.5),
    "cap drop mid-fall": (DEFAULT, [steady(400, 10), steady(100, 30)], [Event(52, "cap", 250.0)], None),
    "skip mid-rise": (DEFAULT, [steady(100, 5), steady(400, 20), steady(150, 20)], [Event(28, "skip")], None),
    # The lead's timing (a lead started early freezes a command still converging on the line):
    "fall -171.8 then -145.7 W/s (50, x4, 1.5)": (Tuning(50, 4.0, 1.5), [steady(500, 5),
        ramp(500, 500 - 171.75976976761186 * 3, 3), ramp(500 - 171.75976976761186 * 3,
        500 - 171.75976976761186 * 3 - 145.65991569843064 * 4, 4),
        steady(500 - 171.75976976761186 * 3 - 145.65991569843064 * 4, 10)], [], None),
    "engaged on a short fall (40, x1.25, 2.75)": (Tuning(40, 1.25, 2.75), [steady(500, 5),
        ramp(500, 500 - 46.284533925877284, 1), steady(500 - 46.284533925877284, 14)], [], 525.9131080243571),
    # The chase counts on-the-line as behind: engaged 0.3 W above a line steeper than the rate.
    "engaged just above a steep rise": (DEFAULT, [ramp(200, 500, 10), steady(500, 20)], [], 200.3),
    "engaged just below a steep fall (50, x4, 1)": (Tuning(50, 4.0, 1.0),
        [ramp(397.2659764981054, 397.2659764981054 - 311.27094022554127 * 2, 2),
         steady(max(0.0, 397.2659764981054 - 311.27094022554127 * 2), 15)], [], 397.2659764981054 - 0.2623860099003067),
    # The hold of step 5 at its threshold: engaged 0.98 W above a gentle rise (between ε and 2ε).
    "engaged 0.98 W above a gentle rise (8, x3, 1)": (Tuning(8, 3.0, 1.0),
        [ramp(341.2837804924229, 341.2837804924229 + 3.1758414224402975 * 5, 5),
         steady(341.2837804924229 + 3.1758414224402975 * 5, 15)], [], 341.2837804924229 + 0.9831520533996803),
    # A real vertex between near-equal slopes (8e-4 relative, far above SLOPE_TOL): it is re-planned.
    "ramp 5 -> 5.004 W/s": (DEFAULT, [steady(100, 5), ramp(100, 250, 30), ramp(250, 250 + 5.004 * 60, 60),
                                      steady(250 + 5.004 * 60, 10)], [], None),
    # A vertex between slopes 8e-4 apart early, at a high value: the frame keys differ by ~4e-5
    # relative, far above the key tolerance (1e-9): it must be re-planned.
    "ramp 5 -> 5.004 W/s at 1000 W": (DEFAULT, [steady(1000, 5), ramp(1000, 1025, 5), ramp(1025, 1025 + 5.004 * 60, 60),
                                                steady(1025 + 5.004 * 60, 10)], [], None),
    # The same near-equal vertex while a landing is still in flight: the frame keys differ by
    # ~1e-4 relative, and only a re-plan follows the new slope from the landing's state.
    "jump onto a ramp that bends 5 -> 5.004 W/s mid-landing": (DEFAULT, [steady(940, 5), ramp(1000, 1002.5, 0.5),
        ramp(1002.5, 1002.5 + 5.004 * 20, 20), steady(1002.5 + 5.004 * 20, 10)], [], None),
    # Clamp, then a drop within one tick: the clamped line jumps (a discontinuity, with its settle).
    "cap 250: 300 W, ramp to 150 W in 0.25 s": (DEFAULT, [steady(100, 5), steady(300, 10), ramp(300, 150, 0.25),
                                                       steady(150, 30)], [], None, dict(m0=250.0)),
    # X5 from a tick on to the end (rounds 13-18: a persistent refusal).
    "X5 from tick 60 mid-fall, then a jump up": (DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 40)],
                                                 nan_window(60, None), None),
    "X5 from tick 100 mid-rise on a ramp": (DEFAULT, [steady(100, 5), ramp(100, 400, 60), steady(400, 20)],
                                           nan_window(100, None), None),
    "X5 from tick 26 while chasing a steep fall": (DEFAULT, [steady(500, 5), ramp(500, 100, 4), steady(100, 30)],
                                                  nan_window(26, None), None),
}
# The findings of the falsifiers of rounds 18 (PR #693) and 18's fix rounds (PR #712) as plain plans (tuning, blocks,
# events, origin, kw). Rounds 18 and later found them against Ruckig in the loop (a refused or slow answer, the twin's
# tighter limits, a degenerate frame, Ruckig's segfault); with no planning library they have nothing to refuse, and
# repros() holds each to every property with no cut where it has no event and never held (P8).
FINDINGS = {
    "PR #693 finding 1: a brake's tick, X2 at round 17 (30, x3, 1.5)": (
        Tuning(30, 3.0, 1.5), [Block(120, 573.1468849073444, 0.0), Block(2, 388.75, 388.75),
                               Block(16, 388.75, 233.2499999999994), Block(7, 233.2499999999994, 165.2187499999991),
                               Block(17, 165.2187499999991, 0.0), Block(4, 0.0, 0.0), Block(20, 0.0, 0.0)],
        [], None, dict(m0=471.0385022477104)),
    "PR #693 finding 1: a brake's tick, X4 at round 17 (15, x4, 3)": (
        Tuning(15, 4.0, 3.0), [Block(40, 224.0, 0.0), Block(4, 0.0, 0.684538790944913),
                               Block(8, 852.4764354272369, 852.4764354272369), Block(4, 852.4764354272369, 0.0),
                               Block(40, 0.0, 0.0)],
        [], None, {}),
    "PR #693 finding 2: a jump up while falling (10, x4, 1.5)": (
        Tuning(10, 4.0, 1.5), [Block(8, 392.25, 392.25), Block(8, 392.25, 374.774), Block(4, 374.774, 340.330),
                               Block(240, 347.830, 347.830), Block(2, 347.830, 299.738)],
        [], None, dict(m0=647.623)),
    # From round 18's 20 000-plan run of the twin answering (seed 18585), trimmed.
    "round 18: a landing onto M, X3 at round 18 (50, x1.5, 2), 18585+12602": (
        Tuning(50, 1.5, 2.0), [Block(8, 75.0, 36.51408651827683), Block(4, 1248.3279485702865, 1248.3279485702865),
                               Block(2, 1248.3279485702865, 0.0), Block(120, 0.0, 0.0), Block(240, 36.51408651827683, 0.0),
                               Block(240, 0.0, 2108.427508249026), Block(2, 2108.427508249026, 2133.0071857237167)],
        [Event(27, "engage", 215.08491013613659)], None, {}),
    "round 18: a ramp near r↑ (40, x1.25, 2), 18585+16175": (
        Tuning(40, 1.25, 2.0), [Block(8, 325.5, 325.5), Block(120, 544.1067856127271, 0.0), Block(60, 0.0, 73.336877797305),
                                Block(32, 1262.598270768813, 1262.598270768813), Block(4, 1262.598270768813, 39.81314370614384),
                                Block(60, 39.81314370614384, 39.81314370614384), Block(9, 423.95614841416784, 513.9160620017757),
                                Block(7, 513.9160620017757, 583.8848836810258), Block(4, 583.8848836810258, 623.8670674977413),
                                Block(40, 153.25, 153.25)],
        [], None, {}),
    "round 18: a ramp near r↓ (40, x1.25, 1), 18585+1882": (
        Tuning(40, 1.25, 1.0), [Block(4, 0.0, 0.0), Block(20, 0.0, 86.69795859499938), Block(20, 86.69795859499938, 86.69795859499938),
                                Block(120, 468.25, 348.6787409977853), Block(120, 563.6259588947523, 563.6259588947523),
                                Block(20, 48.00393773860583, 48.00393773860583), Block(1, 147.4584636016121, 147.4584636016121),
                                Block(2, 187.4584636016121, 187.4584636016121), Block(2, 187.4584636016121, 164.74085082646377),
                                Block(120, 599.9059047739407, 599.9059047739407)],
        [Event(120, "offset", 1.1), Event(235, "engage", 84.87717201898519), Event(295, "skip", 0.0)], None,
        dict(m0=576.8259894897612)),
}
GOLDEN_PLANS.update(FINDINGS)
# From `--golden` (round 19: regenerated, every plan plain or with its invalid input); a change to any rule that moves
# one of these traces fails.
GOLDEN = {
    'X5 mid-rise from tick 30, 100 -> 400 W, defaults': (25575.0, 160, 160),
    'X5 mid-fall from tick 26, 400 -> 100 W, (50, x4, 4)': (59470.3125, 160, 160),
    'X5 ticks 40..59 mid-fall, 600 -> 100 W, (5, x4, 4)': (79770.025, 157, 177),
    'ramp 100->400 W over 60 s': (87839.925566, 6, 6),
    'fall 300->100 W over 60 s': (66100.0, 0, 0),
    'ramp at the rate 100->300 W over 10 s': (35100.0, 45, 45),
    'steep 100->400 W over 10 s': (55650.0, 65, 65),
    'steep 400->100 W over 5 s': (28450.0, 39, 39),
    'steep rise behind a fall': (47653.362647, 59, 79),
    'jump 100->400 W': (39950.0, 65, 85),
    'jump 400->100 W': (26150.0, 39, 59),
    '5 s at 400 W then 150 W': (24675.396732, 27, 47),
    'ramp 37 -> 49.5 W/s (50, x2, 1)': (103768.044493, 29, 29),
    'rise into a falling ramp (15, x2, 2)': (16319.294248, 20, 20),
    'engagement 0->250 W': (22875.0, 55, 75),
    'engagement above a rising ramp (50, x1.5, 1)': (9214.604031, 10, 21),
    'cap drop mid-fall': (32440.0, 30, 50),
    'skip mid-rise': (14775.0, 15, 35),
    'fall -171.8 then -145.7 W/s (50, x4, 1.5)': (14198.054637, 33, 53),
    'engaged on a short fall (40, x1.25, 2.75)': (36451.869461, 43, 49),
    'engaged just above a steep rise': (49669.341, 65, 66),
    'engaged just below a steep fall (50, x4, 1)': (3387.42123, 17, 37),
    'engaged 0.98 W above a gentle rise (8, x3, 1)': (28406.615467, 1, 21),
    'ramp 5 -> 5.004 W/s': (140803.204816, 6, 6),
    'ramp 5 -> 5.004 W/s at 1000 W': (375115.704816, 6, 6),
    'jump onto a ramp that bends 5 -> 5.004 W/s mid-landing': (148302.450899, 22, 42),
    'cap 250: 300 W, ramp to 150 W in 0.25 s': (28541.515139, 55, 81),
    'X5 from tick 60 mid-fall, then a jump up': (69325.094007, 161, 161),
    'X5 from tick 100 mid-rise on a ramp': (62238.675566, 244, 244),
    'X5 from tick 26 while chasing a steep fall': (70884.375, 135, 135),
    "PR #693 finding 1: a brake's tick, X2 at round 17 (30, x3, 1.5)": (36789.373511, 54, 69),
    "PR #693 finding 1: a brake's tick, X4 at round 17 (15, x4, 3)": (5170.582089, 47, 67),
    'PR #693 finding 2: a jump up while falling (10, x4, 1.5)': (90618.30786, 47, 67),
    'round 18: a landing onto M, X3 at round 18 (50, x1.5, 2), 18585+12602': (262416.826924, 66, 110),
    'round 18: a ramp near r↑ (40, x1.25, 2), 18585+16175': (58812.308933, 128, 188),
    'round 18: a ramp near r↓ (40, x1.25, 1), 18585+1882': (172743.879656, 166, 246),
    "PR #693 finding 1, the port's plan (50, x3, 1), seed 1996470693720483841": (61017.418442, 87, 154),
    "PR #693 finding 2, the port's plan (10, x4, 1.5), seed 4795051404215482607": (37052.351013, 47, 67),
    'fix 1: a slope at r↓ plus noise, then a skip (40, x3, 2), seed 105 plan 194': (693.858879, 18, 18),
    'fix 1: a slope at r↓ plus noise (40, x2, 1), seed 108 plan 2789': (35582.120661, 67, 127),
    "fix 1: the lead's handover on the velocity boundary (8, x3, 3), seed 1001 plan 4468": (28815.692363, 200, 260),
    'fix 2: the plan that segfaulted Ruckig 0.19.4 (50, x4, 2.75)': (13011.49736, 47, 48),
    'fix 2: a rising ramp 2.6e-4 W/s inside r↑, Ruckig refused 54 ticks at 047fe9f (50, x3, 4), band 504#1412': (120443.419188, 115, 115),
    'fix 2: a rising ramp 9.6e-5 W/s inside r↑, after a fall (50, x4, 2.75)': (29250.487651, 59, 59),
    'fix 2: a falling ramp 3.9e-4 W/s inside r↓, from an engagement (50, x4, 2.75)': (1582.925134, 12, 12),
    'f712c F1: a ramp 2e-4 W/s inside r↑ to 65 kW (50, x4, 2.75)': (170927856.62911, 5219, 5219),
    'f712c F2: 30 min of 20 s blocks, 300/150 W': (1585650.0, 2620, 4400),
    "f712c F3: the lead's capped velocity off the next slope (50, x1, 3)": (2828.468473, 19, 19),
    'f712c F4: ramps 1e-6 W/s apart, the defaults': (119309.690659, 10, 10),
    'f712c F5: a fall into a slope 1.5e-6 W/s away (5, x4, 4)': (106681.05466, 75, 75),
    'planned fall #29, offset not a number 29..68': (86663.509415, 105, 125),
    'planned fall #202, offset not a number 35..74': (45390.863494, 52, 72),
    'planned fall #358, offset not a number 65..67': (66150.415011, 71, 91),
    'random 56000+87 (the lead from a moving state)': (165144.957611, 118, 158),
}


def constants(rep: Report, report: bool):
    """The tolerances #620 must match, at their boundaries: SLOPE_TOL (collinear lines merge at a
    relative slope difference of 0.5e-9, not at 2e-9) and VALUE_TOL (pieces 1e-7 W apart merge,
    1e-5 W apart do not). And ramp_bound's rules, at the defaults, from its formula (round 14)."""
    # ramp_bound: the real closing rates (finite as s -> 0±, the held line's rates at s = 0), and the slower rate where
    # the catch-up's direction is not known: 2 W below a line falling at 15 W/s, at rest (the relative slope may carry
    # the command past the line, then it comes down at r↓ + s).
    t = DEFAULT
    over = lambda s, r: abs(s) * (abs(s) / t.a_max + t.a_max / t.j_max) / 2 / r + abs(s) / t.a_max + t.a_max / t.j_max
    for s in (1e-9, -1e-9, 0.17, -2.0):
        b = ramp_bound(t, -100.0, s, 0.0, s)
        want = 100.0 / (t.r_dn + s) + (over(s, t.r_up - s) if s > 0 else 0.0) + t.ease * (1 + t.factor) / 2 + 0.75
        if not math.isfinite(b) or abs(b - want) > 1e-9:
            rep.fail(f"constants: ramp_bound from above at slope {s} W/s: {b} s, expected {want} s (r↓ + s)")
        b = ramp_bound(t, 100.0, s, 0.0, s)
        want = 100.0 / (t.r_up - s) + (over(s, t.r_dn + s) if s < 0 else 0.0) + t.ease * (1 + t.factor) / 2 + 0.75
        if not math.isfinite(b) or abs(b - want) > 1e-9:
            rep.fail(f"constants: ramp_bound from below at slope {s} W/s: {b} s, expected {want} s (r↑ − s)")
    # X3's bound (round 15) pinned from above too (f625k K11: doubling it survived): max(r↑, r↓) + a_max·Δt, the worst
    # case measured being exactly r↓ (a cap under a command falling at the full rate).
    for tb in [DEFAULT] + CORNERS:
        if abs(bounds(tb)["X3"] - (max(tb.r_up, tb.r_dn) + tb.a_max * DT)) > 1e-12:
            rep.fail(f"constants: X3's bound {bounds(tb)['X3']} at {tb}, expected max(r↑, r↓) + a_max·Δt")
    s, delta = -15.0, 2.0
    v = -s
    stop_t = v / t.a_max + t.a_max / t.j_max
    stop_d = v * stop_t / 2
    t_s = abs(s) / t.a_max + t.a_max / t.j_max
    want = ((delta + stop_d) / min(t.r_up - s, t.r_dn + s) + abs(s) * t_s / 2 / (t.r_dn + s) + t_s
            + t.ease * (1 + t.factor) / 2 + stop_t + 0.75)
    if abs(ramp_bound(t, delta, 0.0, 0.0, s) - want) > 1e-9:
        rep.fail(f"constants: ramp_bound, direction not known: {ramp_bound(t, delta, 0.0, 0.0, s)} s, expected {want} s"
                 " (the slower rate)")
    base = 7.0
    for rel, n_expected in ((0.5e-9, 1), (2e-9, 2)):
        s2 = base * (1 + rel)
        segs = segments([ramp(100, 100 + base * 10, 10), Block(40, 100 + base * 10, 100 + base * 10 + s2 * 10)],
                        0, 1.0, 1e9)
        if len(segs) != n_expected:
            rep.fail(f"constants: slopes {rel:g} apart give {len(segs)} pieces, expected {n_expected} (SLOPE_TOL 1e-9)")
    for gap, n_expected in ((1e-7, 1), (1e-5, 2)):
        segs = merge_collinear([Seg(0, 100.0, 0.0, 4, False), Seg(4, 100.0 + gap, 0.0, 4, True)])
        if len(segs) != n_expected:
            rep.fail(f"constants: flat pieces {gap:g} W apart give {len(segs)} pieces, expected {n_expected}"
                     f" (VALUE_TOL 1e-6 W)")
    # Round 7's survivors, pinned at their rules: the snap of a near-zero merged slope (SLOPE_TOL,
    # relative 1e-9: 1e-10 W/s snaps, 1e-8 W/s does not), the raw join (1e-9 W: a block 1e-7 W off
    # the previous end is a discontinuity, also after the clamp), and the merge after the clamp (a
    # ramp into the cap and a steeper ramp beyond it are one flat piece at M).
    for slope, flat in ((1e-10, True), (1e-8, False)):
        sg = segments([steady(100, 5), Block(400, 200.0, 200.0 + slope * 100), steady(300, 5)], 0, 1.0, 1e9)
        if len(sg) != 3:          # 100 W, the near-flat block and 300 W are three pieces (jumps between them)
            rep.fail(f"constants: the near-flat block of slope {slope:g} W/s between jumps gives {len(sg)} pieces,"
                     " expected 3 (a jump is never merged: VALUE_TOL)")
            continue
        got = sg[1].s == 0.0
        if got != flat:
            rep.fail(f"constants: a slope of {slope:g} W/s {'is not' if flat else 'is'} snapped to 0 (SLOPE_TOL 1e-9 relative)")
    sg = segments([steady(100, 5), steady(100 + 1e-4, 5)], 0, 1.0, 1e9)
    if len(sg) != 2 or sg[1].joined:
        rep.fail("constants: a block 1e-4 W off the previous end is not a discontinuity (raw join 1e-9 W)")
    sg = segments([steady(100, 5), ramp(100 + 1e-7, 150, 10), steady(150, 5)], 0, 1.0, 1e9)
    if len(sg) != 3 or sg[1].joined:
        rep.fail("constants: a ramp starting 1e-7 W off a held block reads as joined (the post-clamp join keeps the raw join)")
    sg = segments([steady(100, 5), ramp(100, 400, 10), ramp(400, 800, 5), steady(800, 5)], 0, 1.0, 250.0)
    if [(p.k0, p.r0, p.s) for p in sg].count((sg[-1].k0, 250.0, 0.0)) != 1 or len(sg) != 3:
        rep.fail(f"constants: the clamped ramps are not merged after the clamp: {[(p.k0, p.r0, p.s, p.n) for p in sg]}")
    # Round 12: the closed-form helpers on their own. profile_extrema against the profile sampled
    # densely (both roots of the slope in a segment, and the tail at constant slope).
    rng = random.Random(1212)
    for _ in range(3000):
        tt = rng.choice(GRID)
        y, v, a = rng.uniform(0, 500), rng.uniform(-tt.r_dn, tt.r_up), rng.uniform(-tt.a_max, tt.a_max)
        prof = slope_change(v, a, rng.choice([0.0, rng.uniform(-tt.r_dn, tt.r_up)]), tt)
        dur = sum(d for d, _ in prof)
        ys = [profile_at(y, v, a, prof, dur * i / 4000)[0] for i in range(4001)]
        lo, hi = profile_extrema(y, v, a, prof)
        if lo > min(ys) + 1e-6 or hi < max(ys) - 1e-6 or lo < min(ys) - 1e-3 or hi > max(ys) + 1e-3:
            rep.fail(f"constants: profile_extrema {lo:.6f}/{hi:.6f} against sampled {min(ys):.6f}/{max(ys):.6f} {tt} {(y, v, a)}")
            break
        if abs(profile_at(y, v, a, prof, dur)[0] - ys[-1]) > 1e-9 or abs(profile_at(y, v, a, prof, dur + 1)[0]
                                                                          - ys[-1] - profile_at(y, v, a, prof, dur)[1]) > 1e-6:
            rep.fail(f"constants: profile_at does not continue at constant slope after the profile {tt}")
            break
    # Round 19: the landing on its own. From random states the easer reaches (the slope, once the curvature is unwound,
    # within the rates) onto random displacements and frame rates (the rider's, a ramp's r↑ − s and r↓ + s, a lead's
    # cap, near-zero rates): it ends at the displacement, at rest, its slope within the frame's rates (or the state's
    # own unwound slope where that is beyond them), its curvature within a_max; from rest it is the time-optimal
    # rest-to-rest S-curve (Δ/r + r/a_max + a_max/j_max where it reaches the rate). The meet ends at rest on the
    # moving line. And, from the unwound state, the displacement is monotone in the peak beyond max(0, sgn·vu) (the
    # root's premise).
    rng = random.Random(1919)
    worst = 0.0
    for i in range(20000):
        tt = rng.choice(GRID)
        up = rng.choice([tt.r_up, tt.r_up * rng.uniform(1e-6, 1), rng.uniform(0, 2 * tt.r_up), 1e-9])
        dn = rng.choice([tt.r_dn, tt.r_dn * rng.uniform(1e-6, 1), rng.uniform(0, 2 * tt.r_dn), 1e-9])
        v, a = rng.uniform(-dn, up), rng.uniform(-tt.a_max, tt.a_max)
        vu = v + a * abs(a) / (2 * tt.j_max)
        if i % 2 or not -dn <= vu <= up:
            v, a = rng.choice([0.0, rng.uniform(-dn, up)]), 0.0
            vu = v
        d = rng.choice([rng.uniform(-600, 600), rng.uniform(-5, 5), rng.uniform(-1e-3, 1e-3), 0.0])
        segs = landing(v, a, d, up, dn, tt)
        x, dur = end_of(segs, v, a)
        _, ve, ae = profile_at(0.0, v, a, segs, dur)
        vlo, vhi = min(-dn, vu, v), max(up, vu, v)
        # Sampled over the profile, and over its first 2 hours densely (a cruise at a frame rate near 0 is longer than
        # any plan; its end is checked only where it is reached within 10⁶ s, since sampling it at t = 10¹² s keeps
        # no bits for the last phase).
        span = min(dur, 7200.0)
        vs = [profile_at(0.0, v, a, segs, q)[1] for q in [dur * i / 400 for i in range(401)] + [span * i / 400 for i in range(401)]]
        acc = [abs(profile_at(0.0, v, a, segs, q)[2]) for q in [dur * i / 400 for i in range(401)] + [span * i / 400 for i in range(401)]]
        if dur > 1e6:
            x, ve, ae, dur = d, 0.0, 0.0, 0.0
        worst = max(worst, abs(x - d))
        # (A cruise at a near-zero frame rate takes up to |x_s|/rate: at 10⁻⁹ W/s, 10¹⁰ s. Floating point in the slope,
        # about 10⁻¹⁴·|v|, then moves the end by that times the duration; the tolerance says so.)
        # (Sampling at t = dur after a cruise of dur seconds loses dur's last bits, about 4·10⁻¹⁶·dur s, so the end's
        # slope and curvature are off by j_max times that.)
        end_tol = 1e-6 + tt.j_max * 4e-16 * dur
        if (abs(x - d) > 1e-6 * max(1.0, abs(d)) + 1e-14 * max(1.0, abs(v)) * dur or abs(ve) > end_tol
                or abs(ae) > end_tol or min(vs) < vlo - 1e-6
                or max(vs) > vhi + 1e-6 or max(acc) > tt.a_max + 1e-6 or not math.isfinite(dur)):
            rep.fail(f"constants: landing {tt} from {(v, a)} by {d} within [-{dn}, {up}]: ends {x} at {(ve, ae)},"
                     f" slopes {min(vs):.6f}..{max(vs):.6f}, curvature {max(acc):.4f}")
            break
        if v == 0.0 and a == 0.0 and abs(d) > 1e-3:   # (from rest: the time-optimal S-curve)
            r = up if d > 0 else dn
            if abs(d) >= r * (r / tt.a_max + tt.a_max / tt.j_max) and r >= tt.a_max ** 2 / tt.j_max:
                want = abs(d) / r + r / tt.a_max + tt.a_max / tt.j_max
                if abs(dur - want) > 1e-6 * max(1.0, want):
                    rep.fail(f"constants: landing from rest {tt} by {d} at rate {r}: {dur} s, time-optimal {want} s")
                    break
        if i < 4000:
            sg = rng.choice([1.0, -1.0])
            lo_p, hi_p = max(0.0, sg * vu), (up if sg > 0 else dn)
            ps = sorted(rng.uniform(lo_p, hi_p) for _ in range(8)) if lo_p < hi_p else []
            xs = [sg * end_of(slope_change(v, a, sg * q, tt) + slope_change(sg * q, 0.0, 0.0, tt), v, a)[0] for q in ps]
            if any(x2 < x1 - 1e-9 * max(1.0, abs(x1)) for x1, x2 in zip(xs, xs[1:])):
                rep.fail(f"constants: the landing's displacement is not monotone in the peak beyond max(0, sgn·vu) {tt}"
                         f" {(v, a)}: {xs}")
                break
    worst_meet = 0.0
    for i in range(3000):
        tt = rng.choice(GRID)
        fs = rng.choice([1, -1]) * rng.uniform(1e-6, 0.999) * (tt.r_up if rng.random() < 0.5 else tt.r_dn)
        fs = max(min(fs, tt.r_up * 0.999), -tt.r_dn * 0.999)
        rel = (1 if fs > 0 else -1) * rng.choice([rng.uniform(0.5, 400), rng.uniform(0.5, 5)])
        v = rng.uniform(-tt.r_dn, tt.r_up) if rng.random() < 0.5 else 0.0
        a0 = rng.uniform(-tt.a_max, tt.a_max) if i % 2 else 0.0
        if not -tt.r_dn <= v + a0 * abs(a0) / (2 * tt.j_max) <= tt.r_up:
            a0 = 0.0
        pr = meet_profile(rel, v, a0, fs, tt.r_up, tt.r_dn, tt)
        if pr is None:
            continue
        p, ve, ae = pr.at_time(pr.duration)
        x_s, t_s = end_of(slope_change(v, a0, 0.0, tt), v, a0)
        stop_only = (rel + x_s - fs * t_s <= 0) if rel > 0 else False
        # From above it ends on the line (the root finder's 10⁻¹² relative); from below at or under it, by up to 10⁻³ W
        # (the root's lower side; near a fast-falling line the meet is tiny and T(D) steep in D, so the last ulp of D
        # moves the end by more): the line then comes down onto the command at rest.
        off = abs(p) > 1e-6 if rel > 0 else not -1e-3 * max(1.0, abs(rel)) <= p <= 1e-9
        if not stop_only:
            worst_meet = max(worst_meet, abs(p))
        if not stop_only and (off or abs(ve + fs) > 1e-6 or abs(ae) > 1e-6):
            rep.fail(f"constants: the meet {tt} from {rel:+.3f} W at {(v, a0)} onto {fs:.4f} W/s ends {p:.3g} W off"
                     f" the line, slope {ve + fs:.3g}")
            break
    # Round 19: the rules the closed form never needs on its own (its landings end on M and 0 W to well within
    # ROUND_SNAP, where Ruckig's ended 1 ulp outside), pinned directly. Step 8 snaps a state 1e-12 W above M or below
    # 0 W to the bound without a cut; safe braking refuses a stop that passes M by 1e-7 W (more than ROUND_SNAP).
    ez = Easer(DEFAULT)
    segs = segments([steady(400, 60)], 0, 1.0, 250.0)
    # (Moving, so the snap is told apart from X3 and X4, which would also stop it: a cut of the slope.)
    for nst, want in ((State(250.0 + 1e-12, -3.0, 0.0), 250.0), (State(-1e-12, 2.0, 0.0), 0.0)):
        out, exc = ez._after(State(nst.y), nst, 4, segs, 250.0, 250.0, "", True)
        if out.y != want or out.v != nst.v or exc:
            rep.fail(f"constants: step 8 on {nst} gives {out} {exc!r}, expected {want} W snapped with its slope, no cut")
    # same_key: equal values first, so a cap of +inf (none) is the same input every tick; under no cap a held line is
    # landed on with one trajectory, not re-planned each tick from the sampled state.
    if not same_key(math.inf, math.inf):
        rep.fail("constants: same_key(inf, inf) is False (every tick under no cap a new input)")
    tr = run([steady(100, 5), steady(300, 30)], [Event(2, "cap", math.inf)], t=DEFAULT)
    if tr.recalcs > 4:
        rep.fail(f"constants: {tr.recalcs} trajectories for one step under no cap (expected at most 4)")
    v = 5.0
    peak = profile_extrema(0.0, v, 0.0, slope_change(v, 0.0, 0.0, DEFAULT))[1]
    if ez._brake_ok(State(250.0 + 1e-7 - peak, v, 0.0), 1, segs, 250.0):
        rep.fail("constants: a stop 1e-7 W past M is accepted (safe braking's tolerance at M is ROUND_SNAP)")
    # Safe braking's slack: a stop may pass the line by up to ε/2, not more (strictly inside X2's ε).
    ez = Easer(DEFAULT)
    segs = segments([steady(100, 60)], 0, 1.0, 2000.0)
    for over, want in ((0.25 * EPS, True), (0.75 * EPS, False)):
        v = 5.0
        peak = profile_extrema(0.0, v, 0.0, slope_change(v, 0.0, 0.0, DEFAULT))[1]
        got = ez._brake_ok(State(100 + over - peak, v, 0.0), 1, segs, 2000.0)
        if got != want:
            rep.fail(f"constants: a stop {over:.2f} W past the line is {'refused' if want else 'accepted'} (slack ε/2)")
    if report:
        print("Constants SLOPE_TOL and VALUE_TOL pinned at their boundaries; the snap, the raw join, the merge"
              f" after the clamp; the landing on 20 000 random inputs (worst end {worst:.2g} W), the meet on 3 000"
              f" (worst end {worst_meet:.2g} W off the line)")


def line_independence(rep: Report, report: bool, n: int = 2000):
    """Round-7 falsifier, blocker 1: for every plan and every tick k, the line segments() returns is
    the same line whichever tick asks (the pieces that end after k, exactly). And it is canonical:
    no two adjacent pieces that are joined, contiguous and collinear."""
    rng = random.Random(5102)
    bad = 0
    for i in range(n):
        t = rng.choice(GRID)
        blocks, _ = random_plan_v7(rng, t)
        blocks = [b for b in blocks if b.ticks > 0]
        mult = rng.choice([1.0, 1.0, 0.95, 1.1])
        m = rng.choice([1e9, rng.uniform(150, 700)])
        full = segments(blocks, 0, mult, m)
        total = sum(b.ticks for b in blocks)
        for k in range(0, total, max(1, total // 60)):
            got = segments(blocks, k, mult, m)
            want = [p for p in full if p.k0 + p.n > k]
            if got != want:
                bad += 1
                if bad <= 3:
                    rep.fail(f"line independence: plan {i} {t}: the line at tick {k} differs from the line at tick 0's tail")
                break
        for p, q in zip(full, full[1:]):
            if q.joined and q.k0 == p.k0 + p.n and collinear(p.s, q.s) and abs(p.at(q.k0) - q.r0) <= VALUE_TOL:
                rep.fail(f"line independence: plan {i} {t}: two adjacent collinear joined pieces at tick {q.k0} (not merged)")
                break
    if report:
        print(f"Line independence: {n} random plans, the line identical whichever tick asks, and canonical")


def recovery(rep: Report, report: bool):
    """D5 after X5 (round 13; round 19: X5 is the input layer's hold): every grid tuning, steps both ways, an offset not
    a number for 4 or 12 ticks from the step's tick or 6 or 12 ticks into the landing; normal planning resumes when it
    is valid again and lands within landing_bound from the recovery tick's state (check_recovery)."""
    before = rep.recoveries
    n = 0
    for t in GRID:
        for d in (2.0, 20.0, t.rise * t.ease, 300.0):
            for a, b in ((300.0, 300.0 + d), (300.0 + d, 300.0)):
                for o in (0, 6, 12):
                    for L in (4, 12):
                        f = 1.01 if b > a else 0.99
                        ev = [Event(20 + o, "offset", f)] + nan_window(20 + o, L, f)
                        tr = run([steady(a, 5), steady(b, 60)], ev, t=t)
                        check(tr, t, rep, f"recovery {a:g}->{b:g} {t} offset not a number {20 + o}..{20 + o + L - 1}", a)
                        n += 1
    if rep.recoveries - before < n // 2:
        rep.fail(f"recovery: only {rep.recoveries - before} of {n} recoveries had their landing checked")
    if report:
        print(f"D5 after X5: {n} runs over {len(GRID)} tunings, {rep.recoveries - before} landings"
              f" checked against landing_bound from the recovery tick")


def persistent_random(rep: Report, report: bool, n: int = 300):
    """X5 from ANY tick and ANY state to the end: random plans (round 7's generator, caps, events), the offset not a
    number from a random onset tick (rounds 13-18: a persistent refusal). Every property, X5's own (check_held)
    included, must hold. Two schemes: this suite's own (seed 7585) and the round-7 falsifier's q4v7 (seeds 31000 + i)."""
    deg = 0
    for scheme in ("own", "q4v7"):
        rng = random.Random(7585)
        for i in range(n):
            if scheme == "q4v7":
                rng = random.Random(31000 + i)
            t = rng.choice(GRID)
            blocks, events = random_plan_v7(rng, t)
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
            total = sum(b.ticks for b in blocks)
            onset = rng.randrange(max(1, total))
            tr = run(blocks, events + nan_window(onset, None), t=t, m0=m0)
            check(tr, t, rep, f"persistent {scheme}#{i} from tick {onset} {t} {blocks} {events} m0={m0}", tr.y[0])
            deg += sum(tr.held)
    if report:
        print(f"X5 from a random tick to the end: 2 × {n} random plans (incl. the falsifier's q4v7), {deg} held ticks")


def split_equivalence(rep: Report, report: bool, n: int = 300):
    """A ramp split into blocks of one slope (endpoints with float noise) is the single block, tick
    for tick, bitwise: the collinear merge and the key comparison of §7.10.2."""
    rng = random.Random(613)
    worst = 0.0
    for _ in range(n):
        t = rng.choice(GRID)
        s = rng.uniform(-1.5 * t.r_dn, 1.5 * t.r_up)
        w0 = rng.uniform(300, 800)
        d = rng.choice([1, 2, 5, 10, 20])
        nt = int(d / DT)
        pts = [0] + sorted(rng.sample(range(1, nt), min(3, nt - 1))) + [nt]
        split = ([steady(w0, 5)] + [Block(b - a, w0 + s * a * DT + (rng.uniform(-1e-12, 1e-12) if a else 0.0),
                                          w0 + s * b * DT) for a, b in zip(pts, pts[1:])]
                 + [steady(w0 + s * d, 15)])
        for i in range(2, len(split) - 1):    # make each sub-block start where the previous ended
            split[i] = Block(split[i].ticks, split[i - 1].to, split[i].to)
        single = [steady(w0, 5), Block(nt, w0, w0 + s * d), steady(w0 + s * d, 15)]
        a_, b_ = run(split, t=t, m0=4000.0), run(single, t=t, m0=4000.0)
        dev = max(abs(x - y) for x, y in zip(a_.y, b_.y))
        worst = max(worst, dev)
        if dev != 0.0:   # exact at multiplier 1 (the merged slope is recomputed from the endpoints)
            rep.fail(f"split ramp {s:g} W/s {t} {pts}: differs from the single block by {dev:.3g} W")
    if report:
        print(f"Split ramps: {n} plans equal to the single block (largest difference {worst:.3g} W)")


def golden(rep: Report, report: bool, emit: bool = False):
    """Golden traces: Σ y over each plan (to 1e-6 W), its off-line ticks and its settle ticks. They pin behaviour
    the properties allow to vary, such as the lead's timing and cap, the chase, the will-rise
    test, the landing's profile from a moving state and the meet. #620's golden vectors carry the same plans tick by
    tick."""
    got = {}
    for name, spec in {**GOLDEN_PLANS, **golden_random()}.items():
        t, blocks, events, origin = spec[:4]
        kw = spec[4] if len(spec) > 4 else {}
        tr = run(blocks, events, t=t, origin=origin, **kw)
        check(tr, t, rep, "golden: " + name, tr.y[0])
        got[name] = (round(sum(tr.y), 6), sum(1 for o in tr.on if not o), sum(1 for x in tr.settle if x))
    if emit:
        print("GOLDEN = {")
        for k, v in got.items():
            print(f"    {k!r}: {v!r},")
        print("}")
        return
    for k, v in got.items():
        if k not in GOLDEN or abs(GOLDEN[k][0] - v[0]) > 1e-4 or GOLDEN[k][1:] != v[1:]:
            rep.fail(f"golden: {k}: model {v}, pinned {GOLDEN.get(k)}")
    if report:
        print(f"Golden traces: {len(got)} plans match")


def boundary(rep: Report, report: bool, stride: int = 35):
    """Rest-to-rest steps of exactly Δ = rise·ease (and 2·, 3·) up and down, every `stride`-th
    tuning of the full valid set: the input class Ruckig 0.19.4 refused (rounds 5-18). Each lands within P4/P7's
    bound (check_landing)."""
    ts = [Tuning(r, f / 4, e / 4) for r in range(5, 51) for f in range(4, 17) for e in range(4, 17)
          if Tuning(r, f / 4, e / 4).valid()][::stride]
    n = 0
    for t in ts:
        for mul in (1, 2, 3):
            d = t.rise * t.ease * mul
            for a, b in ((300.0, 300.0 - d), (300.0 - d, 300.0)):
                if min(a, b) < 0:
                    continue
                tr = run([steady(a, 5), steady(b, landing_bound(t, b - a, 0.0) + 10)], t=t)
                check(tr, t, rep, f"boundary Δ = {mul}·rise·ease {a:g}->{b:g} {t}", a)
                n += 1
    if report:
        print(f"Δ = rise·ease steps: {n} plans over {len(ts)} tunings")


def pinned(rep: Report, report: bool):
    """Regressions from the safety falsifications of PR #625, exact floats."""
    cases = [
        # Round-5 falsifier, blocker 1: X5 stuck on a plain step (Δ = rise·ease) and its mirror.
        ("stuck step 233 -> 200 W, (11, x2, 3)", Tuning(11, 2.0, 3.0), [steady(233, 10), steady(200, 60)], [], None),
        ("stuck step 200 -> 233 W, (11, x2, 3)", Tuning(11, 2.0, 3.0), [steady(200, 10), steady(233, 60)], [], None),
        ("stuck step 275 -> 200 W, (20, x1.25, 3.75)", Tuning(20, 1.25, 3.75),
         [steady(275, 10), steady(200, 60)], [], None),
        ("stuck mid-plan, (15, x1.5, 2.75)", Tuning(15, 1.5, 2.75),
         [steady(800, 5), ramp(800, 530, 10), ramp(530, 533.75, 5), steady(533.75, 60)], [], None),
        # Round-5 falsifier, blocker 3: near-equal slopes (a split ramp).
        ("split ramp 800 -> 809.7 -> 848.5, defaults", DEFAULT,
         [steady(800, 5), ramp(800, 809.7, 0.5), ramp(809.7, 848.5, 2), steady(848.5, 15)], [], None),
        ("split fall -19.4 W/s (1 s + 0.5 s), (5, x4, 2)", Tuning(5, 4.0, 2.0),
         [steady(800, 5), ramp(800, 780.6, 1), ramp(780.6, 770.9, 0.5), steady(770.9, 15)], [], None),
        # Round-4 falsifier, blocker 1: Ruckig "error in step 1" at the defaults (seed 7, plan 226).
        ("round-4 #269 plan, seed 7 plan 226", DEFAULT,
         [ramp(548.1115998010537, 0, 30), ramp(159.10968483556977, 545.5628042695739, 10),
          steady(65.44805387940211, 5), ramp(65.44805387940211, 0, 5)], [], None),
        # Blocker 2: ramp-to-ramp vertices (rise 50, x2, ease 1; and the defaults).
        ("ramp 37 -> 49.5 W/s, (50, x2, 1)", Tuning(50, 2.0, 1.0),
         [steady(100, 10), ramp(100, 840, 20), ramp(840, 840 + 49.5 * 5, 5), steady(840 + 49.5 * 5, 10)], [], None),
        ("ramp 10 -> 40 W/s, defaults", DEFAULT,
         [ramp(100, 300, 20), ramp(300, 500, 5), steady(500, 10)], [], None),
        ("engagement mid-plan, then a ramp-end vertex, defaults", DEFAULT,
         [steady(150, 5), ramp(150, 100, 10), steady(100, 20)], [Event(30, "engage", 95.0)], None),
        # Nit: X4 in an event-free plan at the defaults.
        ("200 W, 150 W, then 0 W, defaults", DEFAULT,
         [steady(200, 10), steady(150, 5), steady(0, 20)], [], None),
        # Step 6: a fall meets a rising ramp from above (the command must not rise above it).
        ("hold above a rising line, defaults", DEFAULT,
         [steady(300, 10), ramp(200, 400, 20), steady(400, 10)], [], None),
        # Step 5 (hold): an engagement above a fast-rising ramp (seed 585, plan 442).
        ("engagement above a rising ramp, (50, x1.5, 1)", Tuning(50, 1.5, 1.0),
         [ramp(8.7961334282744, 182.11548193531897, 5), steady(182.11548193531897, 10)], [], 61.49644706533412),
        # Safe braking stops at the cap exactly (seed 585, plan 7929).
        ("ramp into the cap, (40, x2, 2)", Tuning(40, 2.0, 2.0),
         [steady(580.7, 5), ramp(580.7, 1131.0, 60)], [], None, 790.99),
        # X3 keeps a falling slope: the cap drops below a command falling at the full rate.
        ("cap drops mid-fall, defaults", DEFAULT,
         [steady(400, 10), steady(100, 30)], [Event(52, "cap", 250.0)], None),
        # Step 7 at 0 W: a falling ramp that ends at 0 W, then a jump up (no lead at a jump).
        ("falling ramp to 0 W, then a jump up, (10, x4, 3)", Tuning(10, 4.0, 3.0),
         [steady(297.26, 1), ramp(297.26, 0, 30), steady(501.46, 30)], [], None),
    ]
    no_cut = {"engagement above a rising ramp, (50, x1.5, 1)", "ramp 37 -> 49.5 W/s, (50, x2, 1)",
              "ramp 10 -> 40 W/s, defaults", "hold above a rising line, defaults", "ramp into the cap, (40, x2, 2)"}
    for case in cases:
        name, t, blocks, events, origin = case[:5]
        m0 = case[5] if len(case) > 5 else 2000.0
        tr = run(blocks, events, t=t, origin=origin, m0=m0)
        check(tr, t, rep, "pinned: " + name, tr.y[0])
        if report:
            print(f"  pinned {name}: tagged {sorted(set(e for e in tr.exc if e))}")
        if name in no_cut and any(e in CUTS for e in tr.exc):
            rep.fail(f"pinned: {name}: a cut {sorted(set(e for e in tr.exc if e in CUTS))} where the design eases")
    # An engagement 2 W off a trajectory at rest, the offset not a number from then on: X5 from the engagement's state
    # (a hold there), never a jump back to the old trajectory.
    tr = run([steady(100, 5), steady(400, 60)], [Event(160, "engage", 402.0)] + nan_window(160, None), t=DEFAULT)
    check(tr, DEFAULT, rep, "pinned: engagement 2 W off a landed trajectory, then the offset not a number", tr.y[0])
    if not all(tr.held[160:]) or abs(tr.y[160] - 402.0) > 1e-9:
        rep.fail("pinned: engagement 2 W off a landed trajectory: not held at the engagement's state")
    # The same while the trajectory is still in flight, at a slow tuning.
    t5 = Tuning(5, 4.0, 4.0)
    ref = run([steady(100, 5), steady(400, 80)], t=t5)
    tr = run([steady(100, 5), steady(400, 80)], [Event(100, "engage", ref.y[99] + 2.0)] + nan_window(100, None), t=t5)
    check(tr, t5, rep, "pinned: engagement 2 W off a rise in flight, then the offset not a number", tr.y[0])
    if not all(tr.held[100:]) or abs(tr.y[100] - ref.y[99] - 2.0) > 1e-9:
        rep.fail("pinned: engagement 2 W off a rise in flight: not held at the engagement's state")
    # A ramp into the cap with X5 from each tick of its approach: the stop never passes M.
    tc = Tuning(40, 2.0, 2.0)
    for k0 in range(36, 52, 3):
        tr = run([steady(580.7, 5), ramp(580.7, 1131.0, 60)], nan_window(k0, None), t=tc, m0=790.99)
        check(tr, tc, rep, f"pinned: ramp into the cap, the offset not a number from tick {k0}", tr.y[0])
    # Random plans the falsifications of PR #625 found, by (seed, plan).
    # (585, 2694), (585, 4693), (7, 2536) need the hold of step 5; (585, 3020), (7, 1773), (7, 5618)
    # need the cap term of step 6 (each fails when that rule is deleted).
    for seed, idx in ((7, 226), (585, 442), (585, 7929), (2, 13093), (2, 19521), (3, 15889),
                      (585, 2694), (585, 4693), (7, 2536), (585, 3020), (7, 1773), (7, 5618)):
        rng = random.Random(seed)
        for i in range(idx + 1):
            t = rng.choice(GRID)
            blocks, events = random_plan(rng, t)
            origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        tr = run(blocks, events, t=t, origin=origin, m0=m0)
        check(tr, t, rep, f"pinned: seed {seed} plan {idx}", tr.y[0])
    # Round 6's generator: (585, 4670) is Ruckig's slower-answer class (v + a²/(2j) on the limit):
    # with the nominal answer kept it lands 23 s late.
    # (1, 2866, v7) and (7, 1525, v7), round 10: a cap drop below a falling command (X3 keeps the falling slope).
    for seed, idx, gen in ((585, 4670, random_plan_v6), (585, 4191, random_plan_v7), (1, 2866, random_plan_v7),
                           (7, 1525, random_plan_v7)):
        # (585, 4191, v7): a fall to 0 W that floor braking refused by float noise and held 0.75 W up.
        rng = random.Random(seed)
        for i in range(idx + 1):
            t = rng.choice(GRID)
            blocks, events = gen(rng, t)
            origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        tr = run(blocks, events, t=t, origin=origin, m0=m0)
        check(tr, t, rep, f"pinned: generator {gen.__name__} seed {seed} plan {idx}", tr.y[0])
        if report:
            cuts = sorted(set(e for e in tr.exc if e))
            print(f"  pinned seed {seed} plan {idx}: tagged {cuts}, max |dv| "
                  f"{max(abs(tr.v[k] - tr.v[k - 1]) for k in range(1, len(tr.v))):.2f} W/s")


# The safety falsifiers' repros of PR #625 (f625b: round 5, f625c: round 6, f625d: round 7) and
# track 2's segfault plan, run as plain plans and held to every property (their refusals as X5 windows, round 19).
REPROS = [
    # f625b (round 5's falsifier)
    ("p10 step 233->200", Tuning(11, 2.0, 3.0), [steady(233, 10), steady(200, 600)], {}),
    ("p10 step 200->233", Tuning(11, 2.0, 3.0), [steady(200, 10), steady(233, 600)], {}),
    ("p10 step 275->200", Tuning(20, 1.25, 3.75), [steady(275, 10), steady(200, 600)], {}),
    ("p10 step 233->200 (12)", Tuning(12, 2.0, 2.75), [steady(233, 10), steady(200, 600)], {}),
    ("p2 split -19.4", Tuning(5, 4.0, 2.0), [steady(800, 5), ramp(800, 780.6, 1), ramp(780.6, 770.9, 0.5), steady(770.9, 15)], {"m0": 4000.0}),
    ("p4 -27 then +0.75", Tuning(15, 1.5, 2.75), [steady(800, 5), ramp(800, 530, 10), ramp(530, 533.75, 5), steady(533.75, 300)], {"m0": 4000.0}),
    ("p8 200->158.75", Tuning(15, 2.0, 2.75), [steady(200, 10), steady(158.75, 120)], {}),
    ("p8 158.75->200", Tuning(15, 2.0, 2.75), [steady(158.75, 10), steady(200, 120)], {}),
    ("p8 300->217.5", Tuning(30, 1.0, 2.75), [steady(300, 10), steady(217.5, 120)], {}),
    ("p8 217.5->300", Tuning(30, 1.0, 2.75), [steady(217.5, 10), steady(300, 120)], {}),
    ("bare #269 input 41.25->0", Tuning(15, 1.5, 2.75), [steady(41.25, 5), steady(0, 30)], {}),
    ("probe1 B1", DEFAULT, [ramp(548.1115998010537, 0, 30), ramp(159.10968483556977, 545.5628042695739, 10), steady(65.44805387940211, 5), ramp(65.44805387940211, 0, 5)], {}),
    ("probe1 B2a", Tuning(50, 2.0, 1.0), [steady(100, 10), ramp(100, 840, 20), ramp(840, 840 + 49.5 * 5, 5), steady(840 + 49.5 * 5, 10)], {}),
    ("probe1 B2b", DEFAULT, [steady(100, 10), ramp(100, 300, 20), ramp(300, 500, 5), steady(500, 10)], {}),
    ("probe1 B2b'", DEFAULT, [ramp(100, 300, 20), ramp(300, 500, 5), steady(500, 10)], {}),
    ("probe1 B2c engage", DEFAULT, [steady(150, 5), ramp(150, 100, 10), steady(100, 20)], {"events": [Event(30, "engage", 95.0)]}),
    # f625c (round 6's falsifier)
    ("q1 fall ramp, offset not a number from 58", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(300, 30)], {"events": nan_window(58, None)}),
    ("q1 steep fall, offset not a number from 40", DEFAULT, [steady(400, 5), ramp(400, 100, 5), steady(100, 30)], {"events": nan_window(40, None)}),
    ("q2 fall, offset not a number from 58, (5, x4, 4)", Tuning(5, 4.0, 4.0), [steady(400, 5), ramp(400, 300, 10), steady(300, 120)], {"events": nan_window(58, None)}),
    ("q3 jump up to 500, offset not a number from 60", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 120)], {"events": nan_window(60, None)}),
    ("q3 jump up to 500, offset not a number from 59", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 120)], {"events": nan_window(59, None)}),
    ("q3 300 held, offset not a number from 58", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(300, 120)], {"events": nan_window(58, None)}),
    ("q6 800->809.7->848.5", DEFAULT, [steady(800, 5), ramp(800, 809.7, 0.5), ramp(809.7, 848.5, 2), steady(848.5, 15)], {}),
    ("q7 cap 250: 300, ramp to 150 in 0.25 s", DEFAULT, [steady(300, 10), ramp(300, 150, 0.25), steady(150, 60)], {"m0": 250.0}),
    ("q7 cap 250: 300, ramp to 240 in 1 s", DEFAULT, [steady(300, 10), ramp(300, 240, 1), steady(240, 60)], {"m0": 250.0}),
    ("q7 0 W, ramp to 400 in 0.25 s", DEFAULT, [steady(0, 10), ramp(0, 400, 0.25), steady(400, 60)], {}),
    ("q8 cap 250: 400 4 s, ramp to 150", DEFAULT, [steady(100, 10), steady(400, 4), ramp(400, 150, 0.25), steady(150, 30)], {"m0": 250.0}),
    ("q8 cap 250: 300 4 s, ramp to 150", DEFAULT, [steady(100, 10), steady(300, 4), ramp(300, 150, 0.25), steady(150, 30)], {"m0": 250.0}),
    ("q9 cap 250: 400 2 s, ramp to 0", DEFAULT, [steady(100, 10), steady(400, 2), ramp(400, 0, 0.25), steady(0, 30)], {"m0": 250.0}),
    # Ruckig 0.19.4 segfaulted on a target velocity exactly at max_velocity from this plan's state at tick 241 (rounds 8-18)
    ("segfault: a ramp beyond the rate, target velocity at the rate (seed 585, plan 19613)", Tuning(30, 4.0, 4.0),
     [steady(391.25, 2), ramp(391.25, 686.9500551790104, 2), steady(494.0, 15), steady(0.0, 1), ramp(0.0, 106.09132761117559, 5),
      steady(106.09132761117559, 5), steady(554.0, 30), steady(614.0, 0.5), ramp(614.0, 776.9755103030757, 0.5)], {}),
    # f625d (round 7's falsifier)
    ("p3 cap 250, 400 then 300", DEFAULT, [steady(100, 10), steady(400, 6), steady(300, 30)], {"m0": 250.0}),
    ("p3 cap 250, 400 then 300, offset not a number from 40", DEFAULT, [steady(100, 10), steady(400, 6), steady(300, 30)], {"m0": 250.0, "events": nan_window(40, None)}),
    ("p4 cap 250 staircase", DEFAULT, [steady(100, 10)] + [steady(400 - 10 * i, 2) for i in range(10)] + [steady(300, 30)], {"m0": 250.0}),
    ("p4 cap 250 staircase, offset not a number from 40", DEFAULT, [steady(100, 10)] + [steady(400 - 10 * i, 2) for i in range(10)] + [steady(300, 30)], {"m0": 250.0, "events": nan_window(40, None)}),
    ("p5 300, 0, 200", DEFAULT, [steady(300, 10), steady(0, 4), steady(200, 40)], {}),
    ("p5 300, 0, 200, offset not a number from 41", DEFAULT, [steady(300, 10), steady(0, 4), steady(200, 40)], {"events": nan_window(41, None)}),
    ("p5 300, 0, 250, offset not a number from 41", DEFAULT, [steady(300, 10), steady(0, 4), steady(250, 40)], {"events": nan_window(41, None)}),
    ("p6 a near-flat block 8000 s, then a ramp", DEFAULT, [steady(100, 10), Block(32000, 200.0, 200.0 + 1e-9 * 8000 * 0.999), ramp(200.0 + 1e-9 * 8000 * 0.999, 260.0, 30), steady(260, 30)], {}),
]


def run_plan(t: Tuning, bl, kw) -> Trace:
    kw = dict(kw)
    ev = kw.pop("events", [])
    return run(bl, ev, t=t, **kw)


def repros(rep: Report, report: bool) -> None:
    for name, t, bl, kw in REPROS:
        tr = run_plan(t, bl, kw)
        check(tr, t, rep, f"repro: {name}", tr.y[0])
    # The findings of rounds 18-18 fix 2 (FINDINGS, also golden plans): every property and never held; a plan with no
    # event makes no cut at all (any would be outside its domain).
    for name, (t, bl, ev, origin, kw) in FINDINGS.items():
        tr = run(bl, ev, t=t, origin=origin, **kw)
        check(tr, t, rep, f"repro: {name}", tr.y[0])
        if (any(e in CUTS for e in tr.exc) and not ev) or any(tr.held):
            rep.fail(f"repro: {name}: cuts {[(k, e) for k, e in enumerate(tr.exc) if e in CUTS]}, held {sum(tr.held)} ticks")
    # The segfault input itself (fix round 2's pin): the meet below a rising line at (50, x4, 2.75) from a state at
    # 50 W/s, frame slope -33.33 W/s (Ruckig 0.19.4 segfaulted on it). In closed form it is the meet: finite, at rest on
    # the line at its end.
    pr = meet_profile(-160.41668156786292, 50.0, 0.0, -33.333333333333336, _T2.r_up, _T2.r_dn, _T2)
    if pr is None:
        pr = Profile(0.0, 0.0, 0.0, [])
    p_end, v_end, _ = pr.at_time(pr.duration)
    if not (math.isfinite(pr.duration) and abs(v_end + -33.333333333333336) < 1e-6 and abs(p_end) < 1e-6) and pr.segs:
        rep.fail(f"repro: the segfault input: the meet ends {p_end:.3g} W off the line, slope {v_end:.3g}")
    # X5's mechanism: an offset not a number on one tick (tick 25, while the step's trajectory is in force) holds the
    # command on exactly that tick (the stop), and the status is raised on exactly that tick; normal planning resumes
    # on the next.
    plan = [steady(100, 5), steady(300, 30)]
    once = run(plan, [Event(25, "offset", 1.1)] + nan_window(25, 1, 1.1), t=DEFAULT)
    check(once, DEFAULT, rep, "repro: an offset not a number on tick 25", 100.0)
    if [k for k, h in enumerate(once.held) if h] != [25] or once.exc[25] != "X5" or once.invalid != once.held:
        rep.fail(f"repro: an offset not a number on tick 25: held on {[k for k, h in enumerate(once.held) if h]},"
                 f" {once.exc[24:28]}")
    # Inputs not a number four times in 60 s (NaN offsets on ticks 20, 40, 60, 80, each cleared by a valid one on the
    # next tick): the status on exactly those ticks (round 18's recurrence rule is gone with the refusals it counted).
    ev_nan = [e for k_ in (20, 40, 60, 80) for e in nan_window(k_, 1)]
    nn = run([steady(200, 80)], ev_nan, t=DEFAULT)
    check(nn, DEFAULT, rep, "repro: NaN offsets four times in 60 s", 200.0)
    if [k_ for k_, f in enumerate(nn.invalid) if f] != [20, 40, 60, 80]:
        rep.fail(f"repro: NaN offsets four times in 60 s: status on {[k_ for k_, f in enumerate(nn.invalid) if f][:8]}")
    # Recovery on the first valid tick (round 13, mutant "the stop passed through safe braking"): injected#7's skip at
    # tick 51 drops the line 700 W below a command rising at 30 W/s; that tick's offset is not a number (X5's stop).
    # On tick 52 the command falls towards the line.
    rng = random.Random(1585)
    for _ in range(8):
        t7 = rng.choice(GRID)
        b7, e7 = random_plan(rng, t7)
        m7 = rng.choice([2000.0, rng.uniform(200, 800)])
    i7 = run(b7, e7 + nan_window(51, 1), t=t7, m0=m7)
    check(i7, t7, rep, "repro: injected#7, recovery after the skip at tick 51", i7.y[0])
    if not (i7.held[51] and not i7.held[52] and i7.y[52] < i7.y[51] - 0.1):
        rep.fail(f"repro: injected#7: held {i7.held[50:54]}, y {[round(y, 3) for y in i7.y[50:54]]}"
                 " (held on tick 51, normal planning and a fall on tick 52)")
    # D5 on ramps (round 13b's sweep): the recovery from rest below a line falling at 18.7 and 19.5 W/s, near r↓ (the
    # line comes down to the command, then passes it while the command speeds up: the overshoot closed at r↓ + s), and
    # inject445000#1027's plan (an offset onto a 16.5 W/s ramp, plain).
    for name, t5, bl5, ev5 in (
            ("inject445000#1027", Tuning(20, 1.25, 2.75),
             [(60, 496.0, 743.8801236256785), (4, 0.0, 0.0), (4, 0.0, 318.46134944356146),
              (60, 318.46134944356146, 318.46134944356146), (20, 279.3309848721474, 279.3309848721474),
              (40, 251.83098487214738, 251.83098487214738), (2, 279.3309848721474, 279.3309848721474)],
             [Event(85, "offset", 1.05)]),
            ("xs990000#1063", Tuning(20, 1.0, 2.0),
             [(25, 332.7558786085489, 588.7235334699823), (2, 588.7235334699823, 588.7235334699823),
              (126, 588.7235334699823, 0.0), (60, 0.0, 0.0)], nan_window(27, 16)),
            ("xs990000#2110", Tuning(5, 4.0, 2.75),
             [(19, 629.943498540935, 629.943498540935), (180, 875.4017817808011, 0.0), (60, 0.0, 0.0)],
             nan_window(16, 25))):
        r5 = Report()
        tr = run([Block(*b) for b in bl5], ev5, t=t5)
        check(tr, t5, r5, f"repro: D5 on a ramp, {name}", tr.y[0])
        for f in r5.fails:
            rep.fail(f)
        # (#2110: r↓ + s = 0.55 W/s, so its bound outlasts the ramp; checked for every other property.)
        if r5.recoveries == 0 and name == "xs990000#1063":
            rep.fail(f"repro: D5 on a ramp, {name}: no recovery checked")
    # X5 under a cap: the closed-form stop from a rise towards the cap ends at or below M, no cut.
    cap = run([steady(100, 5), steady(400, 40)], [Event(28, "offset", 1.05)] + nan_window(28, None), t=DEFAULT, m0=250.0)
    check(cap, DEFAULT, rep, "repro: X5 mid-rise under a cap of 250 W", 100.0)
    if any(e in CUTS for e in cap.exc) or not cap.held[28]:
        rep.fail(f"repro: X5 under a cap of 250 W: cuts {[e for e in cap.exc if e in CUTS]}")
    # X5 windows on a planned 100 -> 400 -> 100 W at the defaults: no cut, never above the plan.
    for k0, n in ((40, 12), (40, 4), (44, 8)):
        tr = run([steady(100, 10), steady(400, 3), steady(100, 30)], nan_window(k0, n), t=DEFAULT)
        check(tr, DEFAULT, rep, f"repro: 100/400/100 W, offset not a number {k0}..{k0 + n - 1}", 100.0)
        up = max([0.0] + [tr.y[k] - tr.r[k] for k in range(1, len(tr.y)) if tr.y[k] > tr.y[k - 1] + 1e-7])
        if any(e in CUTS for e in tr.exc) or up > EPS:
            rep.fail(f"repro: 100/400/100 W, X5 {k0}..{k0 + n - 1}: cuts {[e for e in tr.exc if e in CUTS]},"
                     f" rose {up:.2f} W above the plan")
    for hi in (300, 400):
        tr = run([steady(100, 10), steady(hi, 4), ramp(hi, 150, 0.25), steady(150, 30)], nan_window(40, None), t=DEFAULT,
                 m0=250.0)
        check(tr, DEFAULT, rep, f"repro: round-6 plan at {hi} W, cap 250, X5 from tick 40", 100.0)
    # Round 11's hand-back plan (2100 W, then a -20 W/s ramp), X5 40..61, then D5.
    ho = run([steady(2100, 10), ramp(1800, 0, 90), steady(0, 30)], nan_window(40, 22), t=DEFAULT, m0=4000.0)
    check(ho, DEFAULT, rep, "repro: 2100 W, then a -20 W/s ramp (X5 40..61)", 2100.0)
    # The hard clamp is absolute whatever the trajectory gives (round 11: since the closed-form floor and braking
    # checks, X3 after the step and X4 never act in the suites, so pin them directly).
    for t in (DEFAULT, Tuning(50, 4.0, 1.0)):
        ez = Easer(t)
        # Below 0 W (X4: at rest at 0 W), and just above M within ε of a line held at M (X3, not X2).
        for line_w, st0, nst, want, cut in ((200.0, State(1.0), State(-3.0, -12.0, -5.0), State(0.0, 0.0, 0.0), "X4"),
                                             (400.0, State(249.9), State(250.3, 1.0, 0.0), State(250.0, 0.0, 0.0), "X3")):
            segs = segments([steady(line_w, 20)], 0, 1.0, 250.0)
            out, exc = ez._after(st0, nst, 4, segs, 250.0, min(line_w, 250.0), "", True)
            if (out.y, out.v, out.a) != (want.y, want.v, want.a) or exc != cut:
                rep.fail(f"repro: the hard clamp on {nst} gives {out} {exc!r}, not {want} {cut}")
    # X5 on ramps within the rates, both ways, at the defaults and the corners (an offset mid-ramp not a number): it
    # stops and holds, then lands back on the ramp (D5).
    for t in [DEFAULT] + CORNERS:
        for f in (0.5, -0.5):
            s_ = f * (t.r_up if f > 0 else t.r_dn)
            w = 300.0 + abs(s_) * 70
            for L in (6, 40):
                tr = run([steady(w, 5), ramp(w, w + s_ * 60, 60), steady(w + s_ * 60, 10)],
                         [Event(60, "offset", 1.0001)] + nan_window(60, L, 1.0001), t=t, m0=1e6)
                check(tr, t, rep, f"repro: X5 {L} ticks on a {s_:g} W/s ramp {t}", w)
    # A cap drop below a command falling at the full rate while X5 holds: X3 after the stop, within its bound.
    for t in (DEFAULT, Tuning(20, 4.0, 2.0)):
        plan = [steady(600, 10), steady(100, 40)]
        free = run(plan, t=t)
        kc = next((k for k in range(40, len(free.y)) if free.v[k] <= -0.9 * t.r_dn), None)
        if kc is None:
            rep.fail(f"repro: cap drop during X5 {t}: the fall 600 -> 100 W never reaches 0.9 r↓")
            continue
        tr = run(plan, [Event(kc + 2, "cap", free.y[kc + 1] - 40.0)] + nan_window(kc, None), t=t)
        check(tr, t, rep, f"repro: a cap drop below a held command at the full falling rate {t}", 600.0)
        if not tr.held[kc] or tr.exc[kc + 2] != "X3":
            rep.fail(f"repro: cap drop during X5 {t}: held {tr.held[kc]}, tick {kc + 2} {tr.exc[kc + 2]!r}")
    # Round 12's falsifier plans (at 45be306): X5 windows over the vertex of a planned fall at -39.5 and -30 W/s
    # (defaults), and the random plan 123000+287.
    for slope in (39.5, 30.0):
        plan = [steady(400, 10), ramp(400, 0, 400 / slope), steady(0, 20)]
        for k0 in range(25, 41, 3):
            for n in (3, 12):
                tr = run(plan, nan_window(k0, n), t=DEFAULT)
                check(tr, DEFAULT, rep, f"repro: planned fall at -{slope:g} W/s, X5 {k0}..{k0 + n - 1}", 400.0)
    rng = random.Random(123000 + 287)
    t = rng.choice(GRID)
    blocks, events = random_plan_v7(rng, t)
    m0 = rng.choice([2000.0, rng.uniform(200, 800), rng.uniform(100, 300)])
    tr = run(blocks, events + nan_window(38, 12), t=t, m0=m0)
    check(tr, t, rep, "repro: random 123000+287 with X5 38..49", tr.y[0])
    # A fall to 0 W, X5 120..127 at its end ((10, x2, 1)).
    t = Tuning(10, 2.0, 1.0)
    tr = run([steady(400, 10), ramp(400, 0, 20), steady(0, 30)], nan_window(120, 8), t=t)
    check(tr, t, rep, "repro: fall to 0 W, X5 120..127 (10, x2, 1)", 400.0)
    # The falsifier's scheme with caps of 100-300 W (seeds 56000 + i), X5 windows: 792 and 1227 (the chain onto the
    # cap, round 12) and 656 (the chain's worst jerk, 1..80).
    for i, win in ((792, None), (1227, None), (656, (1, 80))):
        rng = random.Random(56000 + i)
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        m0 = rng.choice([2000.0, rng.uniform(200, 800), rng.uniform(100, 300)])
        k0 = rng.randrange(max(1, sum(b.ticks for b in blocks)))
        L = rng.choice([1, 2, 3, 4, 5, 6, 8, 12, 20, 40, 80])
        if win:
            k0, L, m0 = win[0], win[1], 2000.0
        tr = run(blocks, events + nan_window(k0, L), t=t, m0=m0)
        check(tr, t, rep, f"repro: 56000+{i}, X5 {k0}..{k0 + L - 1}", tr.y[0])
    # Landings on M and 0 W, 1 ulp outside them (random plans of seeds 1, 2 and 7, round 12): snapped, not cut.
    for seed, want in ((1, 3167), (2, 6492), (7, 3875), (7, 5964)):
        rng = random.Random(seed)
        for i in range(want + 1):
            t = rng.choice(GRID)
            blocks, events = random_plan_v7(rng, t)
            origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        tr = run(blocks, events, t=t, origin=origin, m0=m0)
        check(tr, t, rep, f"repro: a landing 1 ulp outside [0, M], seed {seed} plan {want}", tr.y[0])
    # Round 13's falsifier (f625g, at ec2140b): a cap event during X5 (777000 + i; i = 2717 reached 41·j_max at the
    # chain's [0, M] clamp), and the lead from far below the line (125-193 W below the plan).
    for i in (2717,) + tuple(range(0, 120)):
        t, bl, ev, lo = cap_event_plan(i)
        tr = run(bl, ev, t=t)
        check(tr, t, rep, f"repro: cap event during X5 777000+{i}", lo)
    for t in (Tuning(50, 3.0, 3.0), DEFAULT):
        bl = [steady(280, 5), steady(573, 1.25), ramp(573, 0, 10), steady(0, 5)]
        for k0, L in ((16, 80), (16, 20), (20, 12), (22, 8), (16, 8)):
            tr = run(bl, nan_window(k0, L), t=t)
            check(tr, t, rep, f"repro: 280/573 W then a fall to 0 W, X5 {k0}..{k0 + L - 1} {t}", 280.0)
    # The check itself: a cut is tested against its domain on every tick, also inside the band (a cut at rest).
    tr = run([steady(200, 20)], t=DEFAULT)
    tr.exc[40] = "X2"
    self_rep = Report()
    check(tr, DEFAULT, self_rep, "self-test", 200.0)
    if not any("outside its domain" in f for f in self_rep.fails):
        rep.fail("repro: check() does not test a cut's domain inside the band")
    # ... X5 against the closed-form stop: a held tick moved 1e-3 W off it fails.
    tr = run([steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1)] + nan_window(25, None), t=DEFAULT)
    tr.y[30] += 1e-3
    self_rep = Report()
    check_held(tr, DEFAULT, self_rep, "self-test")
    if not any("off the closed-form stop" in f for f in self_rep.fails):
        rep.fail("repro: check_held() does not test X5's path")
    # ... and P8: a held tick whose input was valid fails check_status.
    tr = run([steady(100, 5), steady(300, 30)], t=DEFAULT)
    tr.held[30] = True
    self_rep = Report()
    check_status(tr, self_rep, "self-test")
    if not any("P8" in f for f in self_rep.fails):
        rep.fail("repro: check_status() does not fail a hold with every input valid")
    if report:
        print(f"Repros: {len(REPROS)} falsifier plans (f625b/c/d) and the segfault plan, {len(FINDINGS)} findings of"
              " rounds 18-19's falsifiers (never held, no cut without an event); X5's mechanism pinned; the plans of"
              " rounds 11-13's falsifiers with X5 windows")


def cap_event_plan(i: int):
    """Round 13's falsifier (f625g) scheme, seed 777000 + i: a step up, a cap event at a random tick
    near it (from 20 W below the low level to the high one), an X5 window around the event (rounds 13-18: refusals).
    Returns (tuning, blocks, events, start value)."""
    rng = random.Random(777000 + i)
    t = rng.choice(GRID)
    lo = rng.uniform(50, 300)
    hi = lo + rng.uniform(50, 400)
    bl = [steady(lo, 10), steady(hi, 15), steady(hi, 30)]
    ke = rng.randrange(41, 95)
    cap = rng.uniform(lo - 20, hi)
    k0 = rng.randrange(ke - 8, ke + 3)
    L = rng.choice([1, 2, 3, 4, 6, 8, 12, 20, 40])
    return t, bl, [Event(ke, "cap", cap)] + nan_window(k0, L), lo


def invalid_windows(rep: Report, report: bool, n: int = 300):
    """An X5 window of 1-40 ticks (the offset not a number) at a random place in random plans (round 7's generator, the
    falsifiers' schemes: seeds 56000 + i and 70000 + i; rounds 13-18: refusal windows). Every property holds, P1-P7
    outside X5 and X5's own (check_held, check_recovery) inside and after it."""
    held_ticks = 0
    for base_seed in (56000, 70000):
        for i in range(n):
            rng = random.Random(base_seed + i)
            t = rng.choice(GRID)
            blocks, events = random_plan_v7(rng, t)
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
            total = sum(b.ticks for b in blocks)
            k0 = rng.randrange(max(1, total))
            L = rng.choice([1, 2, 3, 4, 6, 8, 12] if base_seed == 56000 else [4, 5, 6, 8, 12, 20, 40])
            b = run(blocks, events + nan_window(k0, L), t=t, m0=m0)
            check(b, t, rep, f"X5 window {base_seed}+{i} ticks {k0}..{k0 + L - 1} {t}", b.y[0])
            held_ticks += sum(b.held)
    if report:
        print(f"X5 windows: 2 × {n} random plans, {held_ticks} held ticks")


def fall_plan(i: int):
    """invalid_falls' plan i (seed 121000 + i): (tuning, blocks, cap, the X5 window's first tick, its ticks)."""
    rng = random.Random(121000 + i)
    t = rng.choice(GRID)
    w = rng.uniform(50, 800)
    lo = rng.choice([0.0, rng.uniform(0, w)])
    s = rng.uniform(0.2, 1.0) * t.r_dn
    pre = rng.choice([steady(w, 10), ramp(w * 0.6, w, rng.uniform(5, 15))])
    blocks = [pre, ramp(w, lo, (w - lo) / s), steady(lo, 20)]
    v = pre.ticks
    k0 = rng.randrange(max(1, v - 30), v + blocks[1].ticks + 8)
    L = rng.choice([1, 2, 3, 4, 6, 8, 12, 20, 40])
    m0 = rng.choice([2000.0, rng.uniform(lo + 1, w * 1.2)])
    return t, blocks, m0, k0, L


def golden_random() -> dict:
    """Golden plans drawn from the generators (round 12), in GOLDEN_PLANS' format. They pin rules whose change
    moves a trace without breaking a property: the lead's timing from a moving state, and X5 and the landing
    after it on planned falls (rounds 13-18: refusal windows; round 19: the offset not a number)."""
    out = {}
    for i in (29, 202, 358):
        t, blocks, m0, k0, L = fall_plan(i)
        out[f"planned fall #{i}, offset not a number {k0}..{k0 + L - 1}"] = (t, blocks, nan_window(k0, L), None,
                                                                                dict(m0=m0))
    rng = random.Random(56000 + 87)
    t = rng.choice(GRID)
    blocks, events = random_plan_v7(rng, t)
    m0 = rng.choice([2000.0, rng.uniform(200, 800), rng.uniform(100, 300)])
    out["random 56000+87 (the lead from a moving state)"] = (t, blocks, events, None, dict(m0=m0))
    return out


def invalid_falls(rep: Report, report: bool, n: int = 400):
    """X5 windows of 1-40 ticks on planned falls (a held or rising line, then a ramp down at 20-100 % of r↓, possibly
    under a cap), placed from 30 ticks before the fall's vertex to its end: every property holds (round 12's suite, its
    refusals as the offset not a number)."""
    for i in range(n):
        t, blocks, m0, k0, L = fall_plan(i)
        b = run(blocks, nan_window(k0, L), t=t, m0=m0)
        check(b, t, rep, f"X5 on a planned fall #{i} {t}, cap {m0:.1f}, ticks {k0}..{k0 + L - 1}", b.y[0])
    if report:
        print(f"X5 windows on planned falls: {n} plans")


def cap_events(rep: Report, report: bool, n: int = 400):
    """Round 13's falsifier scheme: a cap event during an X5 window (cap_event_plan). Every property holds; the cap's
    position step is X3's (in its domain and bound), X5's stop otherwise."""
    for i in range(n):
        t, bl, ev, lo = cap_event_plan(i)
        tr = run(bl, ev, t=t)
        check(tr, t, rep, f"cap event during X5 777000+{i}", lo)
    if report:
        print(f"Cap events during X5: {n} plans")


def invalid_plan(rng, t, total: int, m0: float) -> list[Event]:
    """The invalid-input generator's events (round 19): 1-4 inputs not a number on random ticks: an offset (NaN or ±inf)
    for 1-80 ticks or to the end, a cap (NaN or -inf) until a valid one (a random cap, or none: +inf), or an
    engagement (NaN or ±inf, at rest at the last command sent)."""
    ev = []
    for _ in range(1 + rng.randrange(4)):
        k0 = rng.randrange(max(1, total))
        kind = rng.choice(["offset", "offset", "cap", "engage"])
        bad = rng.choice([math.nan, math.inf, -math.inf])
        L = rng.choice([1, 2, 4, 8, 20, 80, None])
        if kind == "offset":
            ev += [Event(k0, "offset", bad)] + ([] if L is None else [Event(k0 + L, "offset", rng.choice([1.0, 0.95, 1.05]))])
        elif kind == "cap":
            ev += [Event(k0, "cap", rng.choice([math.nan, -math.inf]))] + (
                [] if L is None else [Event(k0 + L, "cap", rng.choice([m0, math.inf, rng.uniform(150, 700)]))])
        else:
            ev.append(Event(k0, "engage", bad))
    return ev


def invalid_random(rep: Report, report: bool, n: int = 2000, seed: int = 19585):
    """Round 19: random plans (round 7's generator, with engagements and caps, as randoms()) with inputs not a number
    (invalid_plan; rounds 13-18 injected refusals here). Every property holds: X5 exactly on the invalid ticks, its
    path the closed-form stop, the output finite and in [0, max(M, 0)], D5 after it, X2-X4 only in their domain."""
    rng = random.Random(seed)
    held_ticks = 0
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        total = sum(b.ticks for b in blocks)
        bad = invalid_plan(rng, t, total, m0)
        tr = run(blocks, events + bad, t=t, origin=origin, m0=m0)
        if not tr.y:
            continue
        check(tr, t, rep, f"invalid input {seed}#{i} {t} {blocks} {events + bad} origin {origin} m0={m0}", tr.y[0])
        held_ticks += sum(tr.held)
    if report:
        print(f"Inputs not a number (round 19): {n} random plans, {held_ticks} held ticks (X5), every one an invalid"
              " input's (P8)")


def ramp_pairs(rep: Report, report: bool):
    """Ramp-to-ramp vertices (continuous, every slope pair) at the corners of the bounds and the
    defaults: steady, ramp s1 for 10 s, ramp s2 for 5 s, steady."""
    n = 0
    for t in [DEFAULT] + CORNERS:
        slopes = sorted(set([0.0] + [f * t.r_up for f in (0.1, 0.5, 0.74, 0.99, 1.0, 1.5)] +
                            [-f * t.r_dn for f in (0.1, 0.5, 0.74, 0.99, 1.0, 1.5)]))
        for s1 in slopes:
            for s2 in slopes:
                w0 = 1500.0
                w1 = w0 + s1 * 10
                w2 = w1 + s2 * 5
                if min(w1, w2) < 0:
                    continue
                tr = run([steady(w0, 5), ramp(w0, w1, 10), ramp(w1, w2, 5), steady(w2, 15)], t=t, m0=4000.0)
                check(tr, t, rep, f"ramp pair {s1:g} -> {s2:g} W/s {t}", tr.y[0])
                n += 1
    if report:
        print(f"Ramp-to-ramp vertices: {n} slope pairs over the defaults and {len(CORNERS)} bound corners")


def held(rep: Report, report: bool, n: int = 3000):
    """P4 and P7 onto a held line from rest: a rise lands within Δ/r↑ + τ (+ 0.75 s), a fall is at
    or below the line within Δ/r↓ + τ(1 + f)/2 (+ 0.75 s), the time to reach r↓ at the same
    acceleration and jerk limits."""
    rng = random.Random(634)
    worst_up = worst_dn = -1e9
    for _ in range(n):
        t = rng.choice(GRID)
        up = rng.random() < 0.5
        delta = rng.uniform(1, 600)
        a = rng.uniform(0, 600) if up else rng.uniform(delta, 1200)
        b = a + delta if up else a - delta
        r = t.r_up if up else t.r_dn
        ease_t = t.ease if up else t.ease * (1 + t.factor) / 2
        bound = delta / r + ease_t
        tr = run([steady(b, bound + 30)], t=t, y0=a)
        check(tr, t, rep, f"held {a:.1f}->{b:.1f} {t}", a)
        if up:
            lt = land_time(tr)
            worst_up = max(worst_up, lt - bound)
            if lt > bound + 0.75:
                rep.fail(f"P4 held {a:.1f}->{b:.1f} {t}: {lt} s > {bound:.2f} + 0.75")
        else:
            above = next(((k + 1) * DT for k in range(len(tr.y)) if tr.y[k] <= tr.r[k] + EPS), math.inf)
            worst_dn = max(worst_dn, above - bound)
            if above > bound + 0.75:
                rep.fail(f"P7 held {a:.1f}->{b:.1f} {t}: {above} s > {bound:.2f} + 0.75")
    if report:
        print(f"Held lines from rest ({n}): rise lands within Δ/r↑ + τ {worst_up:+.2f} s at worst;"
              f" a fall is at or below the line within Δ/r↓ + τ(1+f)/2 {worst_dn:+.2f} s at worst")


def random_plan_v6(rng, t):
    """Round 6's generator: random_plan's mix, plus the inputs round 5's falsifier showed it lacked.
    - Grid-exact steps: Δ = rise·ease and its multiples (Ruckig's #269-class boundary), and levels
      on the 0.25 W grid.
    - Ramps split into blocks of one slope with float noise in the endpoints (slopes ~1e-13 apart)."""
    blocks, w = [], round(rng.uniform(0, 500) * 4) / 4
    for _ in range(rng.randint(2, 8)):
        secs = rng.choice([0.25, 0.5, 1, 2, 5, 10, 15, 30, 60])
        kind = rng.random()
        if kind < 0.45:
            pick = rng.random()
            if pick < 0.4:
                w = max(0.0, w + rng.choice([-1, 1]) * t.rise * t.ease * rng.choice([1, 2, 3, 0.5]))
            elif pick < 0.7:
                w = round(rng.uniform(0, 600) * 4) / 4
            else:
                w = rng.uniform(0, 600)
            blocks.append(steady(w, secs))
        else:
            start = w if rng.random() < 0.6 else rng.uniform(0, 600)
            slope = (rng.uniform(-t.r_dn, t.r_up) if kind < 0.8
                     else rng.choice([-1, 1]) * rng.uniform(t.r_up, 3 * t.r_dn))
            end = max(0.0, start + slope * secs)
            n = int(round(secs / DT))
            if n >= 3 and rng.random() < 0.35:
                cut = sorted(rng.sample(range(1, n), min(2, n - 1)))
                pts = [0] + cut + [n]
                vals = [start + (end - start) * p / n for p in pts]
                vals = [vals[0]] + [v + rng.uniform(-1e-12, 1e-12) for v in vals[1:-1]] + [vals[-1]]
                for (a, b), (va, vb) in zip(zip(pts, pts[1:]), zip(vals, vals[1:])):
                    blocks.append(Block(b - a, va, vb))
            else:
                blocks.append(ramp(start, end, secs))
            w = end
    total = sum(b.ticks for b in blocks)
    events = []
    for _ in range(rng.randint(0, 3)):
        kind = rng.choice(["offset", "cap", "skip", "engage"])
        val = {"offset": rng.choice([0.9, 0.95, 1.05, 1.1]), "cap": rng.uniform(150, 700), "skip": 0.0,
               "engage": rng.uniform(0, 400)}[kind]
        events.append(Event(rng.randrange(max(1, total)), kind, val))
    return blocks, events


def random_plan_v7(rng, t):
    """Round 7's generator: v6's mix, plus the shapes round 6's falsifier found it never drew.
    - Clamp then drop: a block far above any cap, then a fall within one tick to a lower block
      (the clamped line jumps where the unclamped one was continuous).
    - Ramps that start at exactly 0 W."""
    blocks, events = random_plan_v6(rng, t)
    extra = rng.random()
    if extra < 0.25:
        hi, low = rng.uniform(800, 1500), rng.choice([0.0, rng.uniform(0, 400)])
        at = rng.randrange(len(blocks) + 1)
        blocks[at:at] = [steady(hi, rng.choice([1, 2, 4, 8])), ramp(hi, low, rng.choice([0.25, 0.5, 1])),
                         steady(low, rng.choice([5, 15, 30]))]
    elif extra < 0.45:
        at = rng.randrange(len(blocks) + 1)
        top = rng.uniform(50, 500)
        blocks[at:at] = [steady(0.0, rng.choice([1, 5])), ramp(0.0, top, rng.choice([0.25, 1, 5, 20])),
                         steady(top, rng.choice([5, 15]))]
    return blocks, events


def random_plan(rng, t):
    blocks, w = [], rng.uniform(0, 500)
    for _ in range(rng.randint(2, 8)):
        secs = rng.choice([0.25, 0.5, 1, 2, 5, 10, 15, 30, 60])
        kind = rng.random()
        if kind < 0.4:
            w = rng.uniform(0, 600)
            blocks.append(steady(w, secs))
        else:
            start = w if rng.random() < 0.6 else rng.uniform(0, 600)
            slope = (rng.uniform(-t.r_dn, t.r_up) if kind < 0.75
                     else rng.choice([-1, 1]) * rng.uniform(t.r_up, 3 * t.r_dn))
            end = max(0.0, start + slope * secs)
            blocks.append(ramp(start, end, secs))
            w = end
    total = sum(b.ticks for b in blocks)
    events = []
    for _ in range(rng.randint(0, 3)):
        kind = rng.choice(["offset", "cap", "skip", "engage"])
        val = {"offset": rng.choice([0.9, 0.95, 1.05, 1.1]), "cap": rng.uniform(150, 700), "skip": 0.0,
               "engage": rng.uniform(0, 400)}[kind]
        events.append(Event(rng.randrange(max(1, total)), kind, val))
    return blocks, events


def _pin(spec, **kw):
    t, blocks, events, origin, m0 = spec
    return t, blocks, events, origin, dict(m0=m0, **kw)


# The falsifiers of PR #712 (round 18's fix rounds), as plans: the port's own two findings from its generator
# (port_plan), the fault-free plans the falsifiers found, and f712c's (at 4b958c87). Defined here, after the
# generators they are drawn from. Each degraded, was refused, or segfaulted with Ruckig in the loop.
_T2 = Tuning(50, 4.0, 2.75)
_K = 2 * 2 ** -20 * 50      # two of round 13's twin margins (2⁻²⁰) at r↑ = 50 W/s: fix round 1's band
_S_UP = 50 - 1.01 * _K      # a rising slope just inside fix round 1's band edge
_S_DN = -(200 - 1.01 * 2 * 2 ** -20 * 200)
_S_F1 = 50 - 2.05 * _K      # f712c F1: landable (fix round 2) at M = 65 535 W, degraded 4 ticks
FIX = {
    "PR #693 finding 1, the port's plan (50, x3, 1), seed 1996470693720483841": _pin(port_plan(1996470693720483841)),
    "PR #693 finding 2, the port's plan (10, x4, 1.5), seed 4795051404215482607": _pin(port_plan(4795051404215482607)),
    # A slope at the rate plus noise (-119.9999999999988 W/s, r↓ = 120 W/s): Ruckig degraded fault-free (round 18).
    "fix 1: a slope at r↓ plus noise, then a skip (40, x3, 2), seed 105 plan 194": _pin(seeded_plan(105, 194)),
    "fix 1: a slope at r↓ plus noise (40, x2, 1), seed 108 plan 2789": _pin(seeded_plan(108, 2789)),
    # The lead→line handover re-planned on Ruckig's velocity boundary, a slow answer let through: P4 missed.
    "fix 1: the lead's handover on the velocity boundary (8, x3, 3), seed 1001 plan 4468": _pin(seeded_plan(1001, 4468)),
    # Ruckig 0.19.4 segfaulted (exit 139) on a current velocity exactly at max_velocity, in the meet below a rising line.
    "fix 2: the plan that segfaulted Ruckig 0.19.4 (50, x4, 2.75)": (
        _T2, [Block(8, 100, 233.33333333333334), Block(60, 500, 0.0), Block(40, 0.0, 0.0)], [], None, {}),
    # Ruckig and its twin refused the meet onto a near-rate ramp for 54 ticks at 047fe9f (band.py 504 plan 1412).
    "fix 2: a rising ramp 2.6e-4 W/s inside r↑, Ruckig refused 54 ticks at 047fe9f (50, x3, 4), band 504#1412": (
        Tuning(50, 3.0, 4.0), [Block(3, 1265.0195316450024, 1152.519531645003), Block(9, 1152.519531645003, 815.019531645003),
                              Block(3, 815.019531645003, 702.5195316450024), Block(61, 702.5195316450024, 1465.0155490144605),
                              Block(40, 1465.0155490144605, 1465.0155490144605)], [], None, {}),
    "fix 2: a rising ramp 9.6e-5 W/s inside r↑, after a fall (50, x4, 2.75)": (
        _T2, [Block(6, 300.0, 0.0), Block(40, 0.0, _S_UP * 10), Block(40, _S_UP * 10, _S_UP * 10)], [], None, {}),
    "fix 2: a falling ramp 3.9e-4 W/s inside r↓, from an engagement (50, x4, 2.75)": (
        _T2, [Block(1, 447.9, 447.9), Block(7, 447.9, 447.9 + _S_DN * 1.75),
              Block(4, 447.9 + _S_DN * 1.75, 447.9 + _S_DN * 1.75)], [], 127.8, {}),
    # f712c (the falsifier of PR #712 at 4b958c87). F1: a long ramp 2e-4 W/s inside r↑ at M = 65 535 W (degraded 4
    # ticks: landable at that M). F2: 30 min of 20 s blocks alternating 300/150 W (with 8 refused ticks at each change the
    # fault gate never fired; fault-free here). F3: the lead's capped velocity 2.5e-7 W/s off the next slope (degraded 5
    # ticks). F4: ramps 1e-6 W/s apart at the defaults (on-limit states, the twin refused: the status on 80 ticks). F5:
    # a fall into a slope 1.5e-6 W/s away at (5, x4, 4) (17 degraded ticks, the status on 9).
    "f712c F1: a ramp 2e-4 W/s inside r↑ to 65 kW (50, x4, 2.75)": (
        _T2, [Block(6, 300.0, 0.0), Block(5200, 0.0, _S_F1 * 1300), Block(40, _S_F1 * 1300, _S_F1 * 1300)], [], None,
        dict(m0=65535.0)),
    "f712c F2: 30 min of 20 s blocks, 300/150 W": (
        DEFAULT, [steady(300.0 if i % 2 == 0 else 150.0, 20) for i in range(90)], [], None, {}),
    "f712c F3: the lead's capped velocity off the next slope (50, x1, 3)": (
        Tuning(50, 1.0, 3.0), [Block(4, 205.27759033651068, 155.52759008651068),
                               Block(8, 155.52759008651068, 56.027590086510685), Block(8, 56.03, 56.03)], [], None, {}),
    "f712c F4: ramps 1e-6 W/s apart, the defaults": (
        DEFAULT, [Block(8, 200.0, 200.0), Block(40, 200.0, 300.0), Block(40, 300.0, 399.99999),
                  Block(40, 399.99999, 499.99997), Block(40, 499.99997, 599.99994), Block(40, 599.99994, 699.9999),
                  Block(40, 699.9999, 699.9999)], [], None, {}),
    "f712c F5: a fall into a slope 1.5e-6 W/s away (5, x4, 4)": (
        Tuning(5, 4.0, 4.0), [Block(16, 1500.0, 1500.0 - 15 * (1 + 1e-7) * 4),
                              Block(40, 1500.0 - 15 * (1 + 1e-7) * 4, 1500.0 - 15 * (1 + 1e-7) * 4 - 150.0),
                              Block(20, 1500.0 - 15 * (1 + 1e-7) * 4 - 150.0, 1500.0 - 15 * (1 + 1e-7) * 4 - 150.0)],
        [], None, {}),
}
FINDINGS.update(FIX)
GOLDEN_PLANS.update(FIX)


def randoms(rep: Report, n: int, report: bool, seed: int = 585):
    rng = random.Random(seed)
    ticks = recalcs = 0
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        tr = run(blocks, events, t=t, origin=origin, m0=rng.choice([2000.0, rng.uniform(200, 800)]))
        if tr.y:
            check(tr, t, rep, f"random#{i} {t} {blocks} {events} origin {origin}", tr.y[0])
            ticks += len(tr.y)
            recalcs += tr.recalcs
            # P8 (round 19): every input valid, the easer is never held and the status never raised (check_status).
            if any(tr.held) or any(tr.invalid):
                rep.fail(f"random#{i} seed {seed}: P8 held {sum(tr.held)} ticks with every input valid")
    if report:
        print(f"Random plans, seed {seed}: {n} plans, {ticks} ticks, every property checked on every tick;"
              f" {recalcs} trajectories computed, none refused, none held (P8)")


def cap_valid(x: float) -> bool:
    return not (math.isnan(x) or x == -math.inf)


def above(rep: Report, report: bool):
    """Round 14 (f625i): P7 on every line, the plan or cap not a number, and a cap below 0 W.
    1. The falsifier's repro: a -50 % offset onto a 300 -> 350 W ramp over 300 s at the defaults. Rounds
       1-13 held 303 W for the remaining 280 s, the line at 152 W; the command comes down within P7's bound.
    2. A sweep over slopes, rising and falling, down to s -> 0+ and 0- and up to 0.99 of the rates, over
       the defaults, the bound corners and a sixth of the grid: a downward offset (x0.5, x0.8) or a cap
       drop on the ramp; every property holds, P7 on the ramp included (check_above).
    3. A line or cap that is not a number (NaN, ±inf in a block or an offset; a NaN or -inf cap): X5,
       the status on every such tick, the output finite and in [0, the last valid cap], normal planning
       again once the plan is valid.
    4. A cap below 0 W: the output stays in [0, max(M, 0)] (round 13 sent M: the 0 W cut ran before the cap's).
    5. The check itself: check_above fails a trace that holds above a rising line."""
    tr = run([steady(300, 5), ramp(300, 350, 300)], [Event(100, "offset", 0.5)])
    r1 = Report()
    check(tr, DEFAULT, r1, "f625i: -50 % onto a 300 -> 350 W ramp", tr.y[0])
    for f in r1.fails:
        rep.fail(f)
    rep.above_checked += r1.above_checked
    above_s = sum(1 for k in range(100, len(tr.y)) if tr.y[k] > tr.r[k] + EPS) * DT
    if r1.above_checked != 1 or above_s > 10.0:
        rep.fail(f"f625i: P7 checked {r1.above_checked} times, {above_s:.2f} s above the line (expected 1, under 10 s)")
    # 2. The slope sweep.
    n = checked = 0
    for t in [DEFAULT] + CORNERS + GRID[::6]:
        fr = [1e-4, 1e-3, 0.01, 0.05, 0.2, 0.5, 0.8, 0.99]
        for s in [f * t.r_up for f in fr] + [-f * t.r_dn for f in fr]:
            d = min(120.0, 1200.0 / abs(s))
            bl = [steady(1500, 5), ramp(1500, 1500 + s * d, d), steady(1500 + s * d, 10)]
            ke = 20 + max(1, int(d / DT) // 4)
            for ev in ([Event(ke, "offset", 0.5)], [Event(ke, "offset", 0.8)], [Event(ke, "cap", 1500 + s * ke * DT * 0.5 - 400)]):
                r2 = Report()
                tr = run(bl, ev, t=t, m0=4000.0)
                check(tr, t, r2, f"above sweep {t} s {s:+.4g} W/s {ev}", tr.y[0])
                for f in r2.fails:
                    rep.fail(f)
                n += 1
                checked += r2.above_checked
    rep.above_checked += checked
    if checked < n // 2:
        rep.fail(f"above sweep: P7 checked on {checked} of {n} plans (the sweep tests nothing)")
    # 3. Not a number.
    nan, inf = math.nan, math.inf
    cases = [("a NaN offset", [steady(200, 10), ramp(200, 260, 30)], [Event(40, "offset", nan), Event(60, "offset", 1.0)], 40, 60),
             ("an inf offset", [steady(200, 10), ramp(200, 260, 30)], [Event(40, "offset", inf), Event(60, "offset", 1.0)], 40, 60),
             ("a NaN cap", [steady(200, 10), steady(300, 30)], [Event(30, "cap", nan), Event(70, "cap", 250.0)], 30, 70),
             ("a -inf cap", [steady(200, 10), steady(300, 30)], [Event(30, "cap", -inf), Event(70, "cap", 250.0)], 30, 70),
             ("a NaN block", [steady(200, 10), Block(20, nan, nan), steady(150, 20)], [], 0, 60),
             ("an inf block", [steady(200, 10), Block(20, 200.0, inf), steady(150, 20)], [], 0, 60)]
    for name, bl, ev, k0, k1 in cases:
        tr = run(bl, ev, t=DEFAULT, m0=400.0)
        caps = [400.0] + [e.value for e in ev if e.kind == "cap" and not math.isnan(e.value) and e.value != -inf]
        bad = [k for k in range(len(tr.y)) if not (math.isfinite(tr.y[k]) and 0.0 <= tr.y[k] <= max(caps) + 1e-9)]
        if (bad or not all(tr.held[k0:k1]) or not all(tr.invalid[k0:k1])
                or any(tr.exc[k] != "X5" for k in range(k0, k1))):
            rep.fail(f"not a number, {name}: output out of range at {bad[:3]}, held {sum(tr.held[k0:k1])}/{k1 - k0}"
                     f" ticks {k0}..{k1 - 1}")
        if k1 < len(tr.y) and any(tr.held[k1 + 1:]):
            rep.fail(f"not a number, {name}: still held after the plan is valid again (from tick {k1})")
    # Round 15 (f625j): a valid cap takes effect whatever the fault state (a cap drop during a NaN or inf offset or with
    # a NaN block ahead was never applied: 62.5, 57 and 25 s above M), the output in [0, M] on every tick.
    for name, bl, ev, k0 in (("a NaN offset, cap 100 W", [steady(300, 10), steady(300, 40), steady(150, 20)],
                              [Event(20, "offset", nan), Event(30, "cap", 100.0)], 30),
                             ("an inf offset, cap 50 W", [steady(250, 60)], [Event(8, "offset", inf), Event(12, "cap", 50.0)], 12),
                             ("a NaN block ahead, cap 100 W", [steady(300, 10), Block(20, nan, nan), steady(300, 20)],
                              [Event(20, "cap", 100.0)], 20)):
        tr = run(bl, ev, t=DEFAULT, m0=400.0)
        bad = [k for k in range(len(tr.y)) if not (math.isfinite(tr.y[k]) and 0.0 <= tr.y[k] <= tr.m[k] + 1e-9)]
        if bad or tr.y[k0] > tr.m[k0] + 1e-9 or not tr.held[k0]:
            rep.fail(f"cap during a fault, {name}: output outside [0, M] on ticks {bad[:3]} (y {tr.y[bad[0]] if bad else ''},"
                     f" M {tr.m[bad[0]] if bad else ''}), held at the cap {tr.held[k0]}")
    # A state that is not a number (origin, engagement, initial value or slope; a NaN first block): the output a
    # number in [0, M] on every tick, the status raised.
    for name, bl, ev, kw in (("a NaN y0", [steady(200, 10)], [], dict(y0=nan)), ("a NaN v0", [steady(200, 10)], [], dict(v0=nan)),
                             ("a NaN origin", [steady(200, 10)], [], dict(origin=nan)),
                             ("an inf origin", [steady(200, 10)], [], dict(origin=inf)),
                             ("a NaN engagement", [steady(200, 20)], [Event(10, "engage", nan)], {}),
                             ("an inf engagement", [steady(200, 20)], [Event(10, "engage", -inf)], {}),
                             ("a NaN first block", [Block(8, nan, nan), steady(200, 10)], [], {})):
        tr = run(bl, ev, t=DEFAULT, **kw)
        bad = [k for k in range(len(tr.y)) if not (math.isfinite(tr.y[k]) and math.isfinite(tr.v[k]) and math.isfinite(tr.a[k])
                                                   and 0.0 <= tr.y[k] <= tr.m[k] + 1e-9)]
        if bad or not any(tr.held) or not any(tr.invalid):
            rep.fail(f"state not a number, {name}: output not a number or out of range on ticks {bad[:3]},"
                     f" held {sum(tr.held)} ticks")
    # A cap drop below a command falling fast (f625j, also at 3ace732): the command steps to M and settles there,
    # never below it (it fell to 0 W, X4, and climbed back); X3's slope change within max(r↑, r↓) + a_max·Δt.
    t15 = Tuning(50, 4.0, 4.0)
    for sl, cap in ((-100.0, 20.0), (-100.0, 60.0), (-100.0, 5.0), (-180.0, -300.0), (-180.0, 0.0), (-100.0, 0.0)):
        d = min(200.0, 1500 / abs(sl))
        bl = [steady(1600, 5), ramp(1600, 1600 + sl * d, d), steady(1600 + sl * d, 30)]
        tr = run(bl, [Event(60, "cap", cap)], t=t15, m0=6000.0)
        r8 = Report()
        check(tr, t15, r8, f"cap {cap} W under a {sl} W/s fall {t15}", tr.y[0])
        for f in r8.fails:
            rep.fail(f)
        if min(tr.y[60:]) < max(cap, 0.0) - 1e-9:
            rep.fail(f"cap {cap} W under a {sl} W/s fall: the command undershoots the cap to {min(tr.y[60:]):.2f} W")
    # A cap that is not a number, then a valid one: the checks judge against the cap in force (round 15: P2/X3 were
    # reported against M = NaN), and the output is in range.
    for cap in (0.0, 150.0):
        tr = run([steady(200, 10), steady(300, 30)], [Event(20, "cap", nan), Event(30, "cap", cap)], t=DEFAULT)
        r9 = Report()
        check(tr, DEFAULT, r9, f"a NaN cap, then {cap} W", tr.y[0])
        for f in r9.fails:
            rep.fail(f)
    # Round 16 (f625k). The input layer: several events on one tick, for each input the last valid value wins and any
    # invalid one raises the fault (a NaN cap after a valid one on the same tick kept 300 W against M = 100 W, 27.5 s).
    for name, ev, want_m, deg_after in (
            ("cap 100 then NaN", [Event(10, "cap", 100.0), Event(10, "cap", nan)], 100.0, True),
            ("cap NaN then 100", [Event(10, "cap", nan), Event(10, "cap", 100.0)], 100.0, False),
            ("cap 100 then -inf", [Event(10, "cap", 100.0), Event(10, "cap", -inf)], 100.0, True),
            ("cap -inf then 100", [Event(10, "cap", -inf), Event(10, "cap", 100.0)], 100.0, False),
            ("cap 100 then 50", [Event(10, "cap", 100.0), Event(10, "cap", 50.0)], 50.0, False),
            ("cap 50 then 100", [Event(10, "cap", 50.0), Event(10, "cap", 100.0)], 100.0, False),
            ("cap -5 then NaN", [Event(10, "cap", -5.0), Event(10, "cap", nan)], 0.0, True)):
        tr = run([steady(300, 30)], ev, t=DEFAULT, m0=400.0)
        bad = [k for k in range(len(tr.y)) if not (math.isfinite(tr.y[k]) and 0.0 <= tr.y[k] <= tr.m[k] + 1e-9)]
        fault = any(not cap_valid(e.value) for e in ev)
        if (bad or tr.m[10] != want_m or tr.y[10] > want_m + 1e-9 or tr.held[10] != fault
                or any(tr.held[11:]) != deg_after):
            rep.fail(f"same tick, {name}: M {tr.m[10]} (expected {want_m}), y {tr.y[10]:.2f}, out of [0, M] on {bad[:3]},"
                     f" fault at the tick {tr.held[10]} (expected {fault}), after it {any(tr.held[11:])} (expected {deg_after})")
    for name, ev, want_r, deg_after in (
            ("offset 0.5 then NaN", [Event(10, "offset", 0.5), Event(10, "offset", nan)], 150.0, True),
            ("offset NaN then 0.5", [Event(10, "offset", nan), Event(10, "offset", 0.5)], 150.0, False),
            ("offset 0.5 then inf", [Event(10, "offset", 0.5), Event(10, "offset", inf)], 150.0, True)):
        tr = run([steady(300, 30)], ev, t=DEFAULT, m0=400.0)
        if tr.r[10] != want_r or not tr.held[10] or any(tr.held[11:]) != deg_after or not all(map(math.isfinite, tr.y)):
            rep.fail(f"same tick, {name}: line {tr.r[10]} (expected {want_r}), fault {tr.held[10]}, after {any(tr.held[11:])}"
                     f" (expected {deg_after})")
    for name, ev, want_y in (("engage NaN then 150", [Event(10, "engage", nan), Event(10, "engage", 150.0)], 150.0),
                             ("engage 150 then NaN", [Event(10, "engage", 150.0), Event(10, "engage", nan)], 150.0)):
        tr = run([steady(300, 30)], ev, t=DEFAULT, m0=400.0)
        if abs(tr.origin[10] - want_y) > 1e-9 or not tr.held[10] or not all(map(math.isfinite, tr.y)):
            rep.fail(f"same tick, {name}: engaged at {tr.origin[10]} (expected {want_y}), fault {tr.held[10]}")
    # K6: a state that is not a number takes the last command sent: a NaN engagement while riding 200 W holds 200 W.
    tr = run([steady(200, 20)], [Event(10, "engage", nan)], t=DEFAULT)
    if abs(tr.y[10] - tr.y[9]) > 1e-9 or abs(tr.y[9] - 200.0) > 1e-9:
        rep.fail(f"a NaN engagement at 200 W: {tr.y[9]:.2f} -> {tr.y[10]:.2f} W (the last command, 200 W, expected)")
    # K12: X3 with a non-zero curvature at the cut (the command easing into a fall): it settles on the capped line and
    # never undershoots it (check_cap_floor). The falsifier's case and a grid of cap ticks over the fall's onset.
    n12 = curved = 0
    for t12 in (Tuning(50, 4.0, 4.0), DEFAULT, Tuning(20, 2.0, 2.0), Tuning(50, 1.0, 1.0)):
        for kc in range(21, 45):
            free = run([steady(900, 5), steady(300, 40)], t=t12)
            cap = (free.y[kc - 1] + 300.0) / 2 if kc != 36 or t12 != Tuning(50, 4.0, 4.0) else 541.7
            tr = run([steady(900, 5), steady(300, 40)], [Event(kc, "cap", cap)], t=t12)
            r12 = Report()
            check(tr, t12, r12, f"X3 with curvature, cap {cap:.1f} W at tick {kc} {t12}", tr.y[0])
            for f in r12.fails:
                rep.fail(f)
            n12 += 1
            curved += abs(free.a[kc - 1]) > 1e-6
            if min(tr.y[kc:]) < 300.0 - EPS:
                rep.fail(f"X3 with curvature, cap {cap:.1f} W at tick {kc} {t12}: fell to {min(tr.y[kc:]):.2f} W, below the 300 W line")
    if curved < n12 // 3:
        rep.fail(f"X3 with curvature: only {curved} of {n12} cuts had a non-zero curvature (the test misses its case)")
    # The check itself: a command 2 W under the capped line after X3 fails check_cap_floor.
    tr = run([steady(900, 5), steady(300, 40)], [Event(36, "cap", 541.7)], t=Tuning(50, 4.0, 4.0))
    k3 = tr.exc.index("X3")
    tr.y[k3 + 1] = min(tr.r[k3 + 1], tr.m[k3 + 1]) - 2.0
    r13 = Report()
    check(tr, Tuning(50, 4.0, 4.0), r13, "self-test", tr.y[0])
    if not any("below the capped line" in f for f in r13.fails):
        rep.fail("above: check_cap_floor() does not fail a command under the capped line after X3")
    # A353 (f625k, also at 06071c8): a recovery at rest 50 W below a line falling at 4.1 W/s at (50, x4, 1) climbed in
    # pulses under safe braking (6.25 s against D5's 5.29 s); safe braking now meets the line at rest from below.
    t16 = Tuning(50, 4.0, 1.0)
    bl16 = [Block(139, 246.25, 103.6302083333343), Block(46, 103.6302083333343, 56.4322916666661),
            Block(55, 56.4322916666661, 0.0), Block(40, 0.0, 0.0), Block(25, 0.0, 3.359356511097046e-14),
            Block(12, 3.359356511097046e-14, 6.103328415515826e-13), Block(203, 6.103328415515826e-13, 0.0),
            Block(5, 117.14451348958849, 114.46443377265335), Block(3, 114.46443377265335, 112.85638594249403),
            Block(32, 112.85638594249403, 95.70387575411249)]
    ev16 = [Event(86, "skip", 0.0), Event(457, "engage", 105.96781858201494), Event(353, "cap", 616.5980753014405),
            Event(373, "cap", 255.48177242744842), Event(0, "cap", 151.96156776433398), Event(194, "cap", 141.55408506668627),
            Event(484, "cap", 1e-09), Event(101, "cap", 1e-09), Event(102, "cap", 0.0), Event(103, "cap", 0.0),
            Event(104, "cap", 546.3312833820088), Event(105, "cap", -5.0), Event(106, "cap", 0.0), Event(107, "cap", 1e-09),
            Event(108, "cap", 540.7693665647844), Event(109, "cap", 1e-09), Event(110, "cap", 190.28109936183873)]
    tr = run(bl16, ev16 + nan_window(100, 39), t=t16, m0=2000.0)
    r16 = Report()
    check(tr, t16, r16, "A353: recovery under a falling line", tr.y[0])
    for f in r16.fails:
        rep.fail(f)
    land = next((k for k in range(139, len(tr.y)) if abs(tr.y[k] - tr.r[k]) <= EPS), None)
    if r16.recoveries == 0 or land is None or (land - 139) * DT > ramp_bound(t16, tr.r[139] - DT * tr.piece[139][1] - tr.y[138],
                                                                             0.0, 0.0, tr.piece[139][1]):
        rep.fail(f"A353: recovery checked {r16.recoveries}, on the line at tick {land} (D5's ramp_bound from tick 139)")
    # Round 17 (f625l). D5 below a falling line at (50, x1, 1): the meet from below now meets the line one tick of its
    # fall early, so safe braking accepts it on its own terms (7 of 400 recoveries missed D5, worst 1.31x). The
    # falsifier's repro, its second case (s = -3.75 W/s, 52 W below) and a sweep of 360 recoveries, 0 misses.
    t17 = Tuning(50, 1.0, 1.0)
    cases = [("f625l repro", [ramp(1000, 600, 80)], 950.0, 2)]
    misses = n17 = 0
    for s in (-0.5, -1.0, -2.0, -3.75, -5.0, -7.5, -10.0, -15.0, -20.0, -30.0, -40.0, -49.0):
        for d in (5, 10, 20, 30, 40, 52, 70, 100, 150, 200):
            for R in (2, 4, 5):
                cases.append((f"below a {s} W/s line, {d} W, X5 0..{R - 1}",
                              [ramp(1000, 1000 + s * min(80.0, 950 / abs(s)), min(80.0, 950 / abs(s))), steady(1000 + s * min(80.0, 950 / abs(s)), 20)],
                              1000.0 - d, R))
    for name, bl, y0, R in cases:
        tr = run(bl, nan_window(0, R), t=t17, y0=y0, m0=5000.0)
        r17 = Report()
        check(tr, t17, r17, f"D5 below a falling line {t17}: {name}", tr.y[0])
        for f in r17.fails:
            rep.fail(f)
        n17 += 1
        misses += any("recovery" in f for f in r17.fails)
        if name == "f625l repro" and r17.recoveries == 0:
            rep.fail("D5 below a falling line: the f625l repro checks no recovery")
    # K12's pass-through (f625l): a falling command passing within ε of the line, then on through it, is caught over
    # the whole window; the model itself stays on the 300 W line.
    t12b = Tuning(50, 4.0, 4.0)
    tr = run([steady(900, 5), steady(300, 40)], [Event(28, "cap", 358.33)], t=t12b)
    r18 = Report()
    check(tr, t12b, r18, "X3 at 358.33 W, tick 28 (K12's pass-through)", tr.y[0])
    for f in r18.fails:
        rep.fail(f)
    if min(tr.y[28:]) < 300.0 - EPS:
        rep.fail(f"X3 at 358.33 W, tick 28: the command falls to {min(tr.y[28:]):.2f} W under the 300 W line")
    # The check itself, on K12's own trace: an X3 that keeps the negative curvature passes within ε of the line
    # (299.7 W) still falling, then falls through it to 179.2 W; check() must report it (the self-test swaps the
    # cut for K12's for this one run).
    good = Easer._at_cap
    def k12(self, st, m, line):          # _at_cap with K12's change: the curvature kept after the slope is reduced
        floor = min(line, m)
        ok = lambda v, a: profile_extrema(m, v, a, slope_change(v, a, 0.0, self.t))[0] >= floor - 1e-9
        v, a = min(st.v, 0.0), min(st.a, 0.0)
        if ok(v, a):
            return State(m, v, a)
        lo, hi = v, 0.0
        for _ in range(60):
            mid = (lo + hi) / 2
            lo, hi = (lo, mid) if ok(mid, 0.0) else (mid, hi)
        return State(m, hi, a)
    Easer._at_cap = k12
    try:
        tr = run([steady(900, 5), steady(300, 40)], [Event(28, "cap", 358.33)], t=t12b)
    finally:
        Easer._at_cap = good
    passed = any(abs(tr.y[j] - tr.r[j]) <= EPS for j in range(28, len(tr.y)))
    r19 = Report()
    check(tr, t12b, r19, "self-test", tr.y[0])
    if not passed or min(tr.y[28:]) > 300.0 - 10 * EPS:
        rep.fail(f"above: K12's self-test trace does not pass the line and fall through it (min {min(tr.y[28:]):.1f} W)")
    if not any("below the capped line" in f for f in r19.fails):
        rep.fail("above: check_cap_floor() does not catch a fall through the line after a pass within ε of it")
    # An invalid initial cap (f625l L12): a NaN m0 is a fault from the first tick (no cap known), and the command holds.
    tr = run([steady(100, 5), steady(300, 20)], [], t=DEFAULT, m0=nan)
    if not all(tr.held) or max(tr.y) > 100.0 + 1e-9:
        rep.fail(f"a NaN initial cap: held on {sum(tr.held)} of {len(tr.y)} ticks, up to {max(tr.y):.2f} W (expected"
                 " held throughout, holding 100 W)")
    # 4. A cap below 0 W.
    for bl, ev, m0 in (([steady(200, 10), steady(300, 20)], [Event(20, "cap", -5.0)], 2000.0),
                       ([steady(200, 10)], [], -5.0), ([steady(0, 10)], [Event(4, "cap", -1e-6)], 100.0)):
        tr = run(bl, ev, t=DEFAULT, m0=m0)
        r4 = Report()
        check(tr, DEFAULT, r4, f"cap below 0 W: {bl} {ev} m0 {m0}", tr.y[0])
        for f in r4.fails:
            rep.fail(f)
        k0 = ev[0].tick if ev else 0
        if any(y < 0.0 or y > 0.0 for y in tr.y[k0:]):
            rep.fail(f"cap below 0 W: output {min(tr.y[k0:]):g}..{max(tr.y[k0:]):g} W (expected 0 W)")
    # The meet's end: sampled past its duration, the command holds where it met the line, at rest (in the line's frame
    # it falls back at -s), so the state it reports is the state it sent (check_state).
    for t, s in ((DEFAULT, 0.17), (DEFAULT, 8.0), (Tuning(50, 4.0, 1.0), 40.0)):
        ez = Easer(t)
        ez._plan(State(500.0, 0.0, 0.0), 120.0, s, t.r_up, -t.r_dn, None, True)
        ez.meet, ez.meet_fs = True, s
        dur = ez.traj.duration
        p_end, v_end, _ = ez.traj.at_time(dur)
        ez.elapsed = dur + 0.3 * DT
        got = ez._sample(False)
        want = (p_end - s * 0.3 * DT, -s, 0.0)
        if abs(v_end + s) > 1e-6 or any(abs(a - b) > 1e-9 for a, b in zip(got, want)):
            rep.fail(f"meet onto a {s} W/s line {t}: past its end {got}, expected {want} (at rest where it met the"
                     f" line; it ended with relative slope {v_end})")
    # Seed 1 plan 1270 (round 14): a return onto a -5 W/s ramp from 394 W above at the full descent rate after an offset
    # (Ruckig answered it 24 s slow, where ramp_bound allows 18.75 s): P7 on the ramp.
    t = Tuning(30, 1.25, 2.0)
    bl = [Block(8, 211.3162552851174, 155.94678244643234), Block(40, 567.75, 567.75), Block(2, 447.75, 447.75),
          Block(4, 0.0, 0.0), Block(80, 0.0, 389.09222933136766), Block(20, 389.09222933136766, 389.09222933136766),
          Block(1, 484.0704171388558, 485.77284654283574), Block(40, 485.77284654283574, 769.1445526888376),
          Block(20, 829.1445526888376, 829.1445526888376), Block(120, 142.87990757461876, 0.0)]
    ev = [Event(167, "skip", 0.0), Event(195, "offset", 1.05), Event(112, "engage", 65.87145234421628)]
    tr = run(bl, ev, t=t, m0=2000.0)
    r6 = Report()
    check(tr, t, r6, "a return onto a -5 W/s ramp: seed 1 plan 1270", tr.y[0])
    for f in r6.fails:
        rep.fail(f)
    # The check itself: a sent position 0.5 W off the state the easer reports fails check_state.
    tr = run([steady(100, 5), steady(300, 30)])
    tr.y[30] += 0.5
    r7 = Report()
    check(tr, DEFAULT, r7, "self-test", tr.y[0])
    if not any("state not the one sent" in f for f in r7.fails):
        rep.fail("above: check_state() does not fail a sent position off the reported state")
    # 5. The check itself: a held command above a rising line (rounds 1-13's step 5) fails check_above.
    tr = run([steady(300, 5), ramp(300, 350, 300)], [Event(100, "offset", 0.5)])
    for k in range(100, len(tr.y)):
        tr.y[k], tr.v[k], tr.a[k] = tr.y[99], 0.0, 0.0
    r5 = Report()
    check_above(tr, DEFAULT, r5, "self-test")
    if not any("P7 still above the line" in f for f in r5.fails):
        rep.fail("above: check_above() does not fail a command held above a rising line")
    if report:
        print(f"P7 on every line (round 14): the f625i repro, {n} plans over slopes to 0± (P7 checked on {checked}),"
              f" {len(cases)} not-a-number plans (X5, then normal planning), caps below 0 W (output 0 W)")


def main(argv):
    report = "--report" in argv
    n = int(argv[argv.index("--random") + 1]) if "--random" in argv else 2000
    seeds = ([int(x) for x in argv[argv.index("--seeds") + 1].split(",")] if "--seeds" in argv else [585])
    rep = Report()
    tables(rep, report)
    if "--golden" in argv:
        golden(rep, report, emit=True)
        return 0
    corners(rep, report)
    pinned(rep, report)
    boundary(rep, report)
    recovery(rep, report)
    persistent_random(rep, report)
    split_equivalence(rep, report)
    constants(rep, report)
    line_independence(rep, report)
    repros(rep, report)
    invalid_windows(rep, report)
    invalid_falls(rep, report)
    cap_events(rep, report)
    above(rep, report)
    golden(rep, report)
    ramp_pairs(rep, report)
    invalid_random(rep, report, int(argv[argv.index("--invalid") + 1]) if "--invalid" in argv else 2000)
    held(rep, report)
    for seed in seeds:
        randoms(rep, n, report, seed)
    if report:
        print("Named exceptions (ticks that break the curvature/jerk band): " +
              ", ".join(f"{k} {rep.cuts[k]} ticks, largest slope change {rep.worst.get(k, 0.0):.1f} W/s"
                        for k in ("X2", "X3", "X4")) + f"; X5 (an input not a number) {rep.cuts['X5']} ticks"
              f" ({rep.held_runs} runs checked against the stop, {rep.recoveries} landings after it checked);"
              f" P7 on every line: {rep.above_checked} downward steps checked")
        print(f"P8: {rep.plans} plans, {rep.ff_plans} with every input valid (never held, the status never raised);"
              f" erg_input_invalid raised on {rep.invalid_ticks} ticks, each with an input not a number")
    if rep.fails:
        print("FAIL", rep.nfail, "failures by kind:", rep.kinds)
        for f in rep.fails:
            print("  " + (f if len(f) <= 400 else f[:150] + " … " + f[-240:]))
        return 1
    print("erg_slew (simple easing, closed form): all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
