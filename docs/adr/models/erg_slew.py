#!/usr/bin/env python3
"""Library-in-the-loop model of the eased ERG target, ADR-0002 §7.10 (#585, design round 4).

The motion is Ruckig's (https://github.com/pantor/ruckig, MIT, Community edition; Python bindings
`ruckig==0.19.4`). Every tick crank gives Ruckig three things:
- the commanded target's state: position y in W, velocity v in W/s, acceleration a in W/s²;
- a target state on the plan line, in a frame that moves with the line;
- the rider's limits.
It then takes the state Ruckig reaches one tick (0.25 s) later. This replaces round 3's
hand-written limiter. Only the choice of the target frame and limits (§7.10.2) and the cuts
X2-X4 are crank's own. The model checks the properties of §7.10.6 on every tick, with Ruckig in
the loop, and `--report` prints the tables the ADR quotes.

Setup: Ruckig is not in nixpkgs, so this model is not a flake check. #620 carries the same
properties as Rust property tests on `rsruckig`, and #634 tracks running it in CI.
    python3 -m venv .venv && .venv/bin/pip install ruckig==0.19.4
    .venv/bin/python docs/adr/models/erg_slew.py [--random N] [--report]
Exit 0 means every check passed.
"""
from __future__ import annotations

import math
import random
import sys
from dataclasses import dataclass, field

from ruckig import ControlInterface, InputParameter, Result, Ruckig, RuckigError, Trajectory

DT = 0.25          # one engine tick, s
EPS = 0.5          # W: "on the line" is |R - y| <= EPS
SETTLE_AFTER_TICKS = 20
TOL = 1e-6         # floating-point tolerance of the checks
N_ERRORS = 3       # X5: consecutive refusals answered from the trajectory in force, then the clamp chain
CHAIN_C = 0.0      # X5: the clamp chain's landing constant, in ease times (chain_bound); the grid needs −0.18
X5F_JERK = 2.0     # X5f: the clamp chain's sampled jerk is within this many j_max (measured 1.83 at f135b8c)
TARGET_SCALE = 1 - 2 ** -20   # a target velocity stays this far inside its limit (Ruckig 0.19.4 segfault, §7.10.2)


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
        return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))
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


def speed_limiter(v: float, v0: float, v1: float, dt: float, t: Tuning, order=("jerk", "acc", "vel")) -> tuple[float, bool]:
    """X5's clamp chain: ros2_control's SpeedLimiter::limit (ros-controls/ros2_controllers, humble,
    diff_drive_controller/src/speed_limiter.cpp, commit 3c61e4f, lines 78-138, Apache-2.0), one
    derivative up: it limits the commanded rate v (W/s) given the two previous sent rates v0, v1.
    One deliberate difference: the jerk step uses j·dt², not the reference's 2·j·dt², so the sampled
    jerk is bounded by j_max. Returns the rate and whether a later clamp moved it after the jerk
    clamp (the only case in which the sampled jerk can exceed j_max)."""
    after_jerk = None
    for step in order:
        if step == "jerk":
            dv, dv0 = v - v0, v0 - v1
            da = min(max(dv - dv0, -t.j_max * dt * dt), t.j_max * dt * dt)
            v = v0 + dv0 + da
        elif step == "acc":
            v = v0 + min(max(v - v0, -t.a_max * dt), t.a_max * dt)
        else:
            v = min(max(v, -t.r_dn), t.r_up)
        if step == "jerk":
            after_jerk = v
    return v, after_jerk is not None and abs(v - after_jerk) > 1e-12


def chain_rate(r: float, s: float, y: float, t: Tuning, a0: float = 0.0) -> float:
    """The rate the clamp chain asks for: the line's slope plus a closing rate, the one from which
    the command can still stop within the gap at a_max/2 with the jerk ramp-up
    (d = v²/(2·a_b) + v·a_b/j), and near the line the gap over 2τ (over τ it rings in a
    jerk-saturated limit cycle; track 2, PR #625 round 8). Aimed short by a·|a|/(2j), the rate the
    current curvature still adds while the jerk limit unwinds it."""
    e = abs(r - y)
    ab = 0.5 * t.a_max
    tj = ab / t.j_max
    v_stop = ab * (-tj + math.sqrt(tj * tj + 2 * e / ab))
    v_ref = min(max(s + math.copysign(min(v_stop, e / (2 * t.ease)), r - y), -t.r_dn), t.r_up)
    return v_ref - a0 * abs(a0) / (2 * t.j_max)


def handoff(v: float, a: float, t: Tuning) -> tuple[float, float]:
    """The chain's sampled state as Ruckig's continuous one for the tick Ruckig succeeds again: the
    slope at the sample (the backward difference plus a·Δt/2, given by the caller), and a curvature
    from which the rate limits can still be kept, both strictly inside the limits by
    2⁻²⁰·max(1, |limit|). On the limit itself (v = −r↓, or v ± a²/(2j) on a limit) Ruckig 0.19.4
    answers some inputs with a valid but slower profile (round 11: 23.7 s where 12.8 s is optimal)."""
    up = t.r_up - (1 - TARGET_SCALE) * max(1.0, t.r_up)
    dn = t.r_dn - (1 - TARGET_SCALE) * max(1.0, t.r_dn)
    v = min(max(v, -dn), up)
    a = min(max(a, -t.a_max), t.a_max)
    if a > 0 and v + a * a / (2 * t.j_max) > up:
        a = math.sqrt(2 * t.j_max * (up - v))
    elif a < 0 and v - a * a / (2 * t.j_max) < -dn:
        a = -math.sqrt(2 * t.j_max * (v + dn))
    return v, a


def slope_change(v: float, a: float, target: float, t: Tuning) -> list[tuple[float, float]]:
    """The time-optimal jerk-limited change of slope from (v, a) to (target, 0) under a_max and
    j_max, in closed form (no Ruckig): (duration, jerk) segments. The curvature ramps to a peak
    a_p ≤ a_max, holds, and ramps back to 0 (Biagiotti and Melchiorri 2008, §3.4, for the slope).
    Used for the planner's probes (safe braking, the floor, the lead's timing) and for braking under
    the clamp chain: neither can be refused."""
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
    """The state reached after tt seconds of a profile, then at constant slope (curvature 0)."""
    for d, jk in segs:
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


def hist_from(st: "State") -> list[float]:
    """Three sent values consistent with a state's slope and curvature, for the clamp chain's rates
    after a cut (a cut sets the state; X3 keeps a falling slope, so "at rest" would be wrong)."""
    v0, v1 = st.v, st.v - st.a * DT
    return [st.y - v0 * DT - v1 * DT, st.y - v0 * DT, st.y]


def chain_bound(t: Tuning, delta: float) -> float:
    """X5's landing time on a held line `delta` away from rest: Δ/r + τ·(CHAIN_C + 2·ln(max(1, Δ/ε))).
    Tight (round 11): over every valid grid tuning, steps of 0.6-600 W need CHAIN_C ≥ −0.18; the
    fallback_bound adds N_ERRORS + 1 ticks and the stop."""
    r = t.r_up if delta > 0 else t.r_dn
    return abs(delta) / r + t.ease * (CHAIN_C + 2 * math.log(max(1.0, abs(delta) / EPS)))


@dataclass
class _Line:
    """A frame line c + fs·k·Δt."""
    c: float
    fs: float

    def __call__(self, kk: int) -> float:
        return self.c + self.fs * kk * DT


def _cut(old: State, new: State, name: str, exc: str) -> tuple[State, str]:
    """Apply a cut; it is named (an exception) only if it changes the state by more than floating
    point: a landing that ends 1e-9 W past the line is clamped, not cut."""
    material = abs(old.y - new.y) > 1e-6 or abs(old.v - new.v) > 1e-5 or abs(old.a - new.a) > 1e-4
    return new, (name if material else exc)


class Easer:
    """crank's policy around Ruckig: the target frame, the limits, and the cuts X2-X4.

    Ruckig is used as an online trajectory generator is meant to be used: a trajectory is
    calculated when its input changes (a new frame, new limits, a cut, an engagement) and is
    otherwise sampled tick by tick. Re-planning every tick from the trajectory's own sampled
    state is avoided: Ruckig 0.19.4 can fail ("error in step 1") on such a state when it sits
    exactly on a velocity-limit boundary (found by this model, random plan 230)."""

    def __init__(self, t: Tuning, inject: float = 0.0, seed: int = 0, inject_from: int = 0):
        self.t = t
        self.inject_from = inject_from     # fault injection applies from this tick on
        self.k = 0
        # Fault injection (the suite's own test of X2-X5): this share of Ruckig calculations is
        # made to fail, deterministically, as if Ruckig had returned an error.
        self.inject = inject
        self.inject_rng = random.Random(seed)
        self.wasteful = 0          # calculations on an unchanged input (step 7 says: none)
        self.fail_brake = False    # fault injection: the braking plan of step 6 fails
        self.x5_tick = False       # X5 acted on the current tick
        self.had_traj = False      # a trajectory was in force at the start of the last tick
        self.otg = Ruckig(1)
        self.inp = InputParameter(1)
        self.traj = Trajectory(1)
        self.spare = Trajectory(1)
        self.probe = Trajectory(1)
        self.errors = 0            # Ruckig refusals (any calculation, probes included)
        self.failures = 0
        self.plan_failures = 0     # ... of a trajectory the command needed: X5 acts
        self.fail_run = 0          # consecutive ticks on which a needed trajectory was refused
        self.hist: list[float] = []   # the sent values since the last reset, for the clamp chain
        self.chain_ticks = 0
        self.inject_until = None
        self.refuse_ticks: set[int] = set()   # ticks on which every trajectory calculation is refused (tests)
        self.cont = None           # (elapsed, duration) of a continuation this tick
        self.slow = 0              # Ruckig answers slower than P4/P7's bound for the move, not accepted
        self.chain_now = self.chain_over = False   # the clamp chain moved the command this tick
        self.prev_piece = None
        self.x5_frame = None
        self.key = None
        self.elapsed = 0.0
        self.base = 0.0
        self.recalcs = 0
        self.chasing = False
        self.braking = False

    def _input(self, st: State, rel: float, fs: float, vmax: float, vmin: float, vel_target,
               scale: float = 1.0):
        t, i = self.t, self.inp
        vmax, vmin = vmax * scale, vmin * scale
        i.control_interface = ControlInterface.Velocity if vel_target is not None else ControlInterface.Position
        i.current_position, i.current_velocity, i.current_acceleration = [rel], [st.v - fs], [st.a]
        i.target_position = [0.0]
        tv = 0.0 if vel_target is None else vel_target - fs
        # Strictly inside the velocity limits: Ruckig 0.19.4 segfaults on a target velocity exactly
        # at max_velocity from some states (PR #625 round 8, seed 585 plan 19613).
        hi, lo = vmax - fs, vmin - fs
        tv = min(max(tv, lo + (1 - TARGET_SCALE) * max(1.0, abs(lo))), hi - (1 - TARGET_SCALE) * max(1.0, abs(hi)))
        i.target_velocity = [tv]
        i.target_acceleration = [0.0]
        i.max_velocity, i.min_velocity = [vmax - fs], [vmin - fs]
        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max * scale], [-t.a_max * scale], [t.j_max * scale]
        return i

    def _one(self, traj, st, rel, fs, vmax, vmin, vel_target, scale, injected) -> bool:
        try:
            if injected:
                raise RuckigError("injected")
            res = self.otg.calculate(self._input(st, rel, fs, vmax, vmin, vel_target, scale), traj)
        except (RuckigError, ValueError):   # ValueError: a result code the bindings do not name
            res = None
        return res in (Result.Working, Result.Finished)

    def _calc(self, traj, st, rel, fs, vmax, vmin, vel_target) -> bool:
        """Ruckig's calculate, once. False on a refusal (counted), and also on an answer the engine
        does not accept: a trajectory onto a held line (a flat frame, the rider's limits) slower than
        P4/P7's own landing_bound, Ruckig 0.19.4's second boundary symptom (a valid but slower
        profile: 61.3 s where 31.4 s is time-optimal, persistent-suite plan 128). Such an answer is
        handled like a refusal, by X5, so P4/P7 holds without track 1's twin calculation.
        Nothing is raised."""
        injected = (self.k in self.refuse_ticks) or (
            bool(self.inject) and traj is not self.probe and self.k >= self.inject_from
            and (self.inject_until is None or self.k < self.inject_until)
            and self.inject_rng.random() < self.inject)
        ok = self._one(traj, st, rel, fs, vmax, vmin, vel_target, 1.0, injected)
        if (ok and vel_target is None and fs == 0.0 and vmax == self.t.r_up and vmin == -self.t.r_dn
                and traj.duration > landing_bound(self.t, -rel, st.v, st.a)):
            ok = False
            self.slow += 1
        if not ok:
            self.errors += 1
            self.failures += 1
        return ok

    def _plan(self, st, rel, fs, vmax, vmin, vel_target) -> bool:
        """Calculate into the spare trajectory and swap it in only on success, so a failed
        calculation leaves the trajectory in force untouched (X5)."""
        if not self._calc(self.spare, st, rel, fs, vmax, vmin, vel_target):
            self.plan_failures += 1     # a trajectory the command needed was refused: X5 must act
            return False
        self.traj, self.spare = self.spare, self.traj
        return True

    def _sample(self, vel_mode: bool):
        dur, tt = self.traj.duration, self.elapsed
        if tt >= dur:
            p, v, a = self.traj.at_time(dur)
            extra = v[0] * (tt - dur) if vel_mode else 0.0
            return p[0] + extra, v[0], 0.0
        p, v, a = self.traj.at_time(tt)
        return p[0], v[0], a[0]

    def _brake_ok(self, st: State, k: int, segs: list[Seg], m: float) -> bool:
        """True if the braking profile from `st` (the state at tick k: the time-optimal stop, closed
        form, so it cannot be refused) stays at or below the plan line known ahead, tick by tick,
        until it is at rest."""
        t = self.t
        # The candidate itself first: on its own tick it may not end above the line either.
        if st.y > min(line_at(segs, k, m) + EPS / 2, m + 1e-6):
            return False
        prof = slope_change(st.v, st.a, 0.0, t)
        dur = sum(d for d, _ in prof)
        n = int(math.ceil(dur / DT))
        for i in range(1, n + 1):
            y = profile_at(st.y, st.v, st.a, prof, min(i * DT, dur))[0]
            # The line may be passed by at most EPS/2, strictly inside X2's EPS, so a braking path
            # that is accepted can never be cut; the cap not at all.
            if y > min(line_at(segs, k + i, m) + EPS / 2, m + 1e-6):
                return False
        return True

    def _continues(self, st: State, kk: int) -> bool:
        """True if the trajectory in force reproduces `st` at tick kk: it can then be continued for
        a tick (X5). Not after a cut or an engagement changed the state under it."""
        if self.key is None or self.x5_frame is None:
            return False
        # Only within the trajectory's own duration: past its end, a position trajectory would be
        # extrapolated along its frame line (a ramp's slope) and a velocity trajectory at its target
        # velocity, for as long as Ruckig refuses (the round-6 falsifier's case: down to 0 W).
        if self.elapsed + DT > self.traj.duration + 1e-9:
            return False
        vel_mode, fl, base = self.x5_frame
        p, v, a = self._sample(vel_mode)
        y = base + p if vel_mode else fl(kk) + p
        v = v if vel_mode else v + fl.fs
        return abs(y - st.y) < 1e-6 and abs(v - st.v) < 1e-5 and abs(a - st.a) < 1e-4

    def _floor_ok(self, st: State) -> bool:
        """True if the braking profile from `st` (closed form) stays at or above 0 W."""
        return profile_extrema(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, self.t))[0] >= -1e-9

    def step(self, st: State, k: int, segs: list[Seg], m: float, cut_before: bool) -> tuple[State, str]:
        """The command for tick k from the state at tick k - 1."""
        t = self.t
        self.k = k
        exc = ""
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
            st, exc = _cut(st, State(m, min(st.v, 0.0), min(st.a, 0.0)), "X3", exc)
        cur = segs[0]
        r_k = clamp(cur.at(k), m)
        s_cur = cur.s
        within = lambda s: -t.r_dn <= s <= t.r_up
        # Trackable: strictly inside the rates. A line at exactly the rate cannot be caught after an
        # eased start (that would need a slope above the rate), so it is chased like a steeper one.
        trackable = lambda s: -t.r_dn < s < t.r_up
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
            # Identified by the lines themselves, not by where a piece's list starts: a split of one
            # slope must not look like a new lead.
            lead_key = ("lead", nxt.k0, nxt.r0, nxt.s)     # the vertex it leads into
            # Lead at a vertex that ends or reverses a rise, or ends a fall without starting a rise;
            # never at one that starts or steepens a rise (that would raise the load before the plan).
            # Only from at or below the current line: above it, aiming at a higher next piece would raise it.
            # Where the slope falls, the next line extended back lies above the current one, and the
            # velocity is capped at the current slope: the lead starts only from on the current line
            # (tick k-1's state against tick k-1's line, within EPS). From above it would raise the
            # command above the plan; from below the cap would hold it back from the line. A fall's
            # end aims below the current line: no condition.
            leads = (nxt.joined and trackable(nxt.s)
                     and ((nxt.s < s_cur and abs(st.y - (r_k - s_cur * DT)) <= EPS)
                          or (s_cur < 0 and s_cur < nxt.s <= 0)))
            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))
            if self.key is not None and same_key(self.key[0], lead_key) and within(s_cur):
                fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                vmax, vmin = cap_v()
            elif leads:
                # From the tick the S-curve to the next slope needs (half of it, plus one tick): not
                # earlier, or a command still catching up the current line would be held back by the
                # velocity cap; not later, or it would overshoot the vertex. The S-curve's duration is
                # closed form, so the lead's timing never depends on a Ruckig answer.
                s_dur = sum(d for d, _ in slope_change(st.v, st.a, nxt.s, t))
                if (nxt.k0 - (k - 1)) * DT <= s_dur / 2 + DT + 1e-9:   # ties lead, whatever the floats
                    fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                    vmax, vmin = cap_v() if within(s_cur) else (t.r_up, -t.r_dn)
        lead = fs != s_cur or c != r_k - s_cur * k * DT
        line = lambda kk: c + fs * kk * DT
        brake = self.braking
        if brake:
            vel_target = 0.0
        elif trackable(fs) and fs > 0 and st.y - line(k) > EPS:
            # Above a rising line the command never rises: it eases to rest and lets the line come
            # up to it, then tracks it from below (no target that would raise it above the plan).
            vel_target = 0.0
        elif trackable(fs):
            vel_target = None
        else:
            # A line steeper than the rider's rate: chase it at the full rate, never away from it.
            # The gap is to where the line is now (tick k), not to the line extended back a tick.
            gap = line(k) - st.y
            # On the line (within EPS) counts as behind it: the command chases, it does not hold.
            vel_target = t.r_up if (gap > -EPS and fs > 0) else (-t.r_dn if (gap < EPS and fs < 0) else 0.0)
        key = ((("brake",) if brake else
                ("lead", segs[1].k0, segs[1].r0, segs[1].s) if lead
                else ("line", c, fs)),
               vmax, vmin, vel_target, m)
        x5 = False
        unchanged = same_key(key, self.key) and not exc and not cut_before
        self.had_traj = self._continues(st, k - 1)
        if not same_key(key, self.key) or exc or cut_before:
            self.recalcs += 1
            self.wasteful += unchanged
            if vel_target is None:
                ok = self._plan(st, st.y - line(k - 1), fs, vmax, vmin, None)
            else:
                ok = self._plan(st, 0.0, 0.0, t.r_up, -t.r_dn, vel_target)
            if ok:
                if vel_target is not None:
                    self.base = st.y
                self.key, self.elapsed = key, DT
                self.x5_frame = None
                self.fail_run = 0
            else:
                x5 = True
                self.fail_run += 1
        else:
            self.elapsed += DT
        self.chasing = vel_target is not None
        if x5:
            # X5 (round 10, the hybrid): Ruckig refused the trajectory the command needs.
            # 1. For up to N_ERRORS consecutive refusals, the trajectory in force is continued for
            #    the tick (within every limit, only within its own duration), and the re-plan is
            #    tried again on the next.
            # 2. Otherwise the clamp chain (ros2_control's SpeedLimiter, below), which cannot fail,
            #    moves the command towards the line; Ruckig is tried first again on every tick and
            #    takes over on its first success.
            self.x5_tick = True
            if self.fail_run <= N_ERRORS and self._continues(st, k - 1):
                self.elapsed += DT
                self.cont = (self.elapsed, self.traj.duration)
                vel_mode, fl, base = self.x5_frame
                p, v, a = self._sample(vel_mode)
                nst = State(base + p, v, a) if vel_mode else State(fl(k) + p, v + fl.fs, a)
                exc = exc or "X5"
            else:
                # The rates come from the sent values; after a cut before the step (X3: the cap dropped
                # below the command, a falling slope kept) they restart from the cut state.
                h = ([self.hist[0]] * 3 + self.hist)[-3:] if (self.hist and not exc) else hist_from(st)
                v0, v1 = (h[2] - h[1]) / DT, (h[1] - h[0]) / DT
                # The chain aims at the planner's reference, not the raw line: the frame in force (the
                # lead's next line extended back, where the planner leads into a vertex) with the
                # lead's rate caps, so it starts a planned fall when the planner would.
                lo_v, hi_v = max(vmin, -t.r_dn), min(vmax, t.r_up)
                s_t = min(max(fs, lo_v), hi_v)
                # The gap is taken at the state's own tick (k - 1): line(k) against y(k - 1) would build
                # one tick of the line's motion (s·Δt) into a permanent offset on a ramp (round 11).
                v_ref = min(max(chain_rate(clamp(line(k - 1), m), s_t, st.y, t, (v0 - v1) / DT), lo_v), hi_v)
                v_new, over = speed_limiter(v_ref, v0, v1, DT, t)
                a_new = (v_new - v0) / DT
                nst = State(st.y + v_new * DT, *handoff(v_new + a_new * DT / 2, a_new, t))
                # The planner's promises hold under the chain too: its step is taken only if the
                # closed-form stop from it stays at or below the line known ahead (and at or above
                # 0 W). Otherwise the command brakes by that stop, which cannot be refused either.
                if (rises(nst, t) and not self._brake_ok(nst, k, segs, m)) or (nst.v < -1e-9 and not self._floor_ok(nst)):
                    nst = State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, t), DT))
                    over = True       # a braking tick: the sampled jerk follows the stop, not the chain
                self.key, self.x5_frame, self.braking = None, None, False
                self.chain_ticks += 1
                self.chain_now, self.chain_over = True, over
                return self._after(st, nst, k, segs, m, r_k, exc or ("X5fo" if over else "X5f"), brake=True)
        if not x5:
            p, v, a = self._sample(vel_target is not None)
            if vel_target is None:
                nst = State(line(k) + p, v + fs, a)
                fl = _Line(c, fs)
                self.x5_frame = (False, fl, 0.0)
            else:
                nst = State(self.base + p, v, a)
                self.x5_frame = (True, None, self.base)
        return self._after(st, nst, k, segs, m, r_k, exc, brake)

    def _after(self, st: State, nst: State, k: int, segs: list[Seg], m: float, r_k: float, exc: str,
               brake: bool) -> tuple[State, str]:
        """Steps 6 and 8: safe braking, then the cuts."""
        t = self.t
        # Safe braking: a step after which the command rises or will rise (v > 0, or a braking fall
        # whose acceleration will carry it past rest) is taken only if Ruckig's braking trajectory from it (to rest,
        # within the same limits) stays at or below the plan line known ahead. Otherwise the command
        # eases out from the current state and keeps easing out until it is at rest.
        # The same for a fall towards 0 W: a falling step is taken only if its braking trajectory
        # stays at or above 0 W, nothing being below it.
        if (not brake and ((rises(nst, t) and not self._brake_ok(nst, k, segs, m))
                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):
            if self.fail_brake:
                self.errors += 1
                self.plan_failures += 1
            if not self.fail_brake and self._plan(st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0):
                self.braking = True
                self.base, self.elapsed = st.y, DT
                self.key = (("brake",), t.r_up, -t.r_dn, 0.0, m)
                self.chasing = True
                p, v, a = self._sample(True)
                nst = State(self.base + p, v, a)
                self.x5_frame = (True, None, self.base)
            else:
                # Ruckig refused the braking trajectory (X5): the same stop in closed form, which
                # cannot be refused, for this tick; the next tick re-plans.
                nst = State(*profile_at(st.y, st.v, st.a, slope_change(st.v, st.a, 0.0, t), DT))
                self.key, self.x5_frame = None, None
                exc = exc or "X5b"; self.x5_tick = True
        if self.braking and abs(nst.v) <= 1e-9:
            self.braking = False
            self.key = None if self.traj.duration > self.elapsed + 1e-9 else self.key
        # X2 again: a step that would rise and end above the line stops instead.
        if nst.y > r_k + EPS and (nst.v > 1e-9 or nst.y > st.y + 1e-7):
            nst, exc = _cut(nst, State(st.y, 0.0, 0.0), "X2", exc)
        if nst.y < 0:                                    # X4: nothing is below 0 W
            nst, exc = _cut(nst, State(0.0, 0.0, 0.0), "X4", exc)
        if nst.y > m:
            nst, exc = _cut(nst, State(m, 0.0, 0.0), "X3", exc)
        if exc and exc != "X5":
            self.key = None
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
    after_error: list = field(default_factory=list)  # a Ruckig error (X5) since the command was last on the line
    had_traj: list = field(default_factory=list)     # a trajectory was in force at the tick's start
    wasteful: int = 0
    failed: list = field(default_factory=list)       # Ruckig refused twice on this tick (X5 must act)
    x5: list = field(default_factory=list)           # X5 acted on this tick
    cont: list = field(default_factory=list)         # (elapsed, duration) where X5 continued the trajectory in force
    chain: list = field(default_factory=list)        # (the clamp chain moved the command, a later clamp overrode its jerk)
    flat: list = field(default_factory=list)
    piece: list = field(default_factory=list)        # (start tick, slope, last tick, starts with a jump) of the piece in force         # (value, last tick) of a flat piece starting here, else None
    events: list = field(default_factory=list)       # an event happened on this tick
    errors: int = 0
    slow: int = 0             # Ruckig answers not accepted as slower than P4/P7's bound
    y_init: float = 0.0      # the state before tick 0 (at rest unless v0 was given)
    v_init: float = 0.0


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, m0=2000.0,
        y0: float | None = None, v0: float = 0.0, inject: float = 0.0, inject_seed: int = 0,
        fail_brake: bool = False, inject_from: int = 0, refuse_until: int | None = None,
        refuse_ticks=()) -> Trace:
    blocks = [Block(b.ticks, b.frm, b.to) for b in blocks if b.ticks > 0]
    ev = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    ez = Easer(t, inject, inject_seed, inject_from)
    ez.inject_until = refuse_until
    ez.refuse_ticks = set(refuse_ticks)
    ez.fail_brake = fail_brake
    last_error_k = -10**9
    error_pending = False
    mult, m = 1.0, m0
    tr = Trace()
    st = None
    pending = origin is not None
    unforeseen = origin is not None      # an engagement is not known before it happens
    settle_left = 0
    prev_r, prev_s = None, 0.0
    k = 0
    while k < sum(b.ticks for b in blocks):
        disc = reset = False
        if ev.get(k):
            unforeseen = True     # skip, ±/coach, cap or engagement: not known before it happens
        # A cap change applies first, so an engagement on the same tick starts at or below the new cap.
        for e in sorted(ev.get(k, []), key=lambda e: e.kind != "cap"):
            if e.kind == "offset":
                mult, disc = e.value, True
            elif e.kind == "cap":
                m = e.value
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
                st, reset, pending = State(clamp(e.value, m)), True, True
        segs = segments(blocks, k, mult, m)
        r = clamp(segs[0].at(k), m)
        if segs[0].k0 == k and k > 0 and not segs[0].joined:
            disc = True
        if st is None:
            if origin is not None:
                st, reset = State(clamp(origin, m)), True
            else:
                st = State(r if y0 is None else y0, v0, 0.0)
                tr.y_init, tr.v_init = st.y, st.v
        if any(e.kind == "cap" for e in ev.get(k, [])) and prev_r is not None and r > prev_r + prev_s * DT + 1e-6:
            disc = True    # a raised cap lets the line jump up
        origin_k = st.y if reset else None
        errors_before = ez.errors
        failures_before = ez.plan_failures
        ez.x5_tick = False
        ez.chain_now = ez.chain_over = False
        ez.cont = None
        if reset or k == 0:
            ez.hist = [st.y] if (reset or v0 == 0.0) else [st.y - v0 * DT, st.y]
        st, exc = ez.step(st, k, segs, m, reset or disc)
        # The chain's rates come from the sent values since the last reset; a cut (X2's stop, the cap,
        # 0 W) is no rate, so the history restarts there, from the cut state's own slope.
        if exc in CUTS:
            ez.hist = hist_from(st)
        else:
            ez.hist.append(st.y)
        failed_now = ez.plan_failures > failures_before
        if failed_now:
            last_error_k = k
            error_pending = True     # open until the command is next on the line
        on = abs(r - st.y) <= EPS
        unforeseen_now = unforeseen
        error_now = error_pending
        landed = on and abs(st.v - segs[0].s) <= 1e-6 and abs(st.a) <= 1e-5   # on the line, moving with it
        if landed and not ev.get(k):
            unforeseen = False
            error_pending = failed_now
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
                         (tr.fell, prev_r is not None and r < prev_r - 1e-12), (tr.m, m),
                         (tr.unforeseen, unforeseen_now), (tr.origin, origin_k),
                         (tr.after_error, error_now), (tr.had_traj, ez.had_traj),
                         (tr.failed, failed_now), (tr.x5, ez.x5_tick),
                         (tr.chain, (ez.chain_now, ez.chain_over)),
                         (tr.cont, ez.cont),
                         (tr.flat, (r, segs[0].k0 + segs[0].n - 1) if segs[0].s == 0.0 else None),
                         (tr.piece, (segs[0].k0, segs[0].s, segs[0].k0 + segs[0].n - 1, segs[0].k0 == k and not segs[0].joined)),
                         (tr.events, bool(ev.get(k)))):
            lst.append(val)
        prev_r = r
        prev_s = segs[0].s     # the backward slope of tick k+1 is the slope of the block in force at k
        k += 1
    tr.errors = ez.errors
    tr.wasteful = ez.wasteful
    tr.slow = ez.slow
    return tr


# ---------------------------------------------------------------- checks (§7.10.6)

class Report:
    def __init__(self):
        self.fails: list[str] = []
        self.worst = {"X2": 0.0, "X3": 0.0, "X4": 0.0}
        self.cuts = {"X2": 0, "X3": 0, "X4": 0, "X5": 0, "X5f": 0}
        self.where = {}
        self.errors = 0
        self.plans = 0
        self.plans_with_errors = 0

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
    return {"X2": t.r_up + step, "X3": t.r_up + step, "X4": t.r_dn + step}


CUTS = ("X2", "X3", "X4")


def chain_window(tr: Trace, k: int) -> bool:
    """True if the jerk window ending at tick k (this tick and the three before) contains a clamp
    chain tick whose acceleration or rate clamp overrode its jerk clamp, or the first Ruckig tick
    after the chain (the hand-off's curvature clip): the only places X5 may break the jerk band."""
    for j in range(max(0, k - 3), k + 1):
        if tr.chain[j][1] or (j > 0 and tr.chain[j - 1][0] and not tr.chain[j][0]):
            return True
    return False


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


def fallback_bound(t: Tuning, delta: float, v0: float, a0: float = 0.0) -> float:
    """P4/P7's bound where X5 acts (a Ruckig refusal): the clamp chain's landing time over the gap
    plus the distance of stopping the state first, plus that stop's time and N_ERRORS + 1 ticks (the
    trajectory in force before the chain takes over)."""
    v_eff = abs(v0) + a0 * a0 / (2 * t.j_max)
    t_stop = abs(a0) / t.j_max + v_eff / t.a_max + t.a_max / t.j_max if (v0 or a0) else 0.0
    d = abs(delta) + v_eff * t_stop
    return chain_bound(t, math.copysign(d, delta)) + t_stop + (N_ERRORS + 1) * DT


def check_landing(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """P4 and P7 on every plan: wherever a flat piece starts (a step, a ramp's end, an event, an
    engagement), the command reaches it within landing_bound, if the piece and the plan last that
    long without a further event. A command stuck off a held line (a Ruckig error that never
    recovers) fails here."""
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
        if abs(delta) <= EPS and abs(v0) < 1e-9:
            continue
        bound = landing_bound(t, delta, v0, a0)
        n = int(math.ceil(bound / DT))
        # Where X5 acts within the window, the fallback's own bound applies (it is the larger).
        nf = int(math.ceil(fallback_bound(t, delta, v0, a0) / DT))
        if any(tr.x5[k:k + nf + 1]):
            bound = max(bound, fallback_bound(t, delta, v0, a0))
            n = int(math.ceil(bound / DT))
        if k + n > min(last, len(tr.y) - 1) or any(tr.events[k + 1:k + n + 1]) or any(tr.reset[k + 1:k + n + 1]):
            continue
        ok = (lambda y: y <= val + EPS) if delta < 0 else (lambda y: abs(y - val) <= EPS)
        if not any(ok(tr.y[j]) for j in range(k, k + n + 1)):
            rep.fail(f"{name} k={k}: P4/P7 not at the held line {val:.2f} W within {bound:.2f} s"
                     f" (from {y0:.2f} W, slope {v0:.2f} W/s)")


def ramp_bound(t: Tuning, delta: float, v0: float, a0: float, s: float, fallback: bool) -> float:
    """P4/P7 onto a line of slope s strictly within the rates (round 11): landing_bound (or
    fallback_bound where X5 acts) in the line's frame, with the relative rates r↑ − s and r↓ + s,
    the slope taken relative to the line, and the ease to the rider's full falling rate."""
    up, dn = t.r_up - s, t.r_dn + s
    r = up if delta > 0 else dn
    if delta < 0 and s > 0:
        # Above a rising line the command holds (step 5) and the line comes up to it: the gap closes
        # at the line's own slope, after the command has stopped.
        r = s
    elif delta > 0 and s < 0:
        # Below a falling line safe braking lets the command rise only as far as it can stop below
        # the line ahead, so the line comes down to it: at worst at the line's own slope.
        r = min(r, -s)
    v_rel = v0 - s
    v_eff = abs(v_rel) + a0 * a0 / (2 * t.j_max)
    stop_t = abs(a0) / t.j_max + v_eff / t.a_max + t.a_max / t.j_max if (v_rel or a0) else 0.0
    d = abs(delta) + v_eff * stop_t
    if fallback:
        return d / r + t.ease * (CHAIN_C + 2 * math.log(max(1.0, d / EPS))) + stop_t + (N_ERRORS + 1) * DT
    return d / r + t.ease * (1 + t.factor) / 2 + stop_t + 0.75


def check_ramp_landing(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """P4/P7 at X5's hand-back onto a ramp (round 11): where the clamp chain hands the command back
    to Ruckig on a ramp piece strictly within the rates, the command is on it (from below) or at or
    below it (from above) within ramp_bound, if the piece and the plan last that long with no
    further event. A slow answer accepted at the hand-back fails here (the round-11 falsifier's
    2100 W, then −20 W/s ramp: 28.75 s above the line instead of 18.75 s)."""
    n = len(tr.y)
    for k in range(1, n):
        k0, s, last, jumped = tr.piece[k]
        if s == 0.0 or not (-t.r_dn < s < t.r_up):
            continue
        # Only where X5 hands the command back to Ruckig: elsewhere on ramps track 1's P6 governs (the
        # excess may grow while the line falls), and P4/P7 are promised on held lines.
        handback = tr.chain[k - 1][0] and not tr.chain[k][0]
        if not handback:
            continue
        y0, v0, a0 = (tr.y[k - 1], tr.v[k - 1], tr.a[k - 1]) if not tr.reset[k] else (tr.y[k], 0.0, 0.0)
        r_prev = tr.r[k] - s * DT
        delta = r_prev - y0
        if abs(delta) <= EPS and abs(v0 - s) < 1e-9:
            continue
        bound = ramp_bound(t, delta, v0, a0, s, False)
        nf = int(math.ceil(ramp_bound(t, delta, v0, a0, s, True) / DT))
        if any(tr.x5[k:k + nf + 1]):
            bound = max(bound, ramp_bound(t, delta, v0, a0, s, True))
        m_ = int(math.ceil(bound / DT))
        end = k + m_
        if end > min(last, n - 1) or any(tr.events[k + 1:end + 1]) or any(tr.reset[k + 1:end + 1]):
            continue
        ok = (lambda j: tr.y[j] <= tr.r[j] + EPS) if delta < 0 else (lambda j: abs(tr.y[j] - tr.r[j]) <= EPS)
        if not any(ok(j) for j in range(k, end + 1)):
            rep.fail(f"{name} k={k}: P4/P7 not on the ramp line (slope {s:.2f} W/s) within {bound:.2f} s"
                     f" (from {y0:.2f} W, {delta:+.2f} W off, slope {v0:.2f} W/s)")


def check(tr: Trace, t: Tuning, rep: Report, name: str, y_start: float) -> None:
    """Every property on the SENT positions y_k: slope, curvature and jerk are their finite
    differences, so a cut cannot hide behind the easer's internal state. A window of finite
    differences that contains a cut is judged by that cut's bound and domain instead."""
    rep.plans += 1
    rep.errors += tr.errors
    if tr.wasteful:
        rep.fail(f"{name}: {tr.wasteful} Ruckig calculations on an unchanged input (step 8)")
    rep.plans_with_errors += tr.errors > 0
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
        if exc == "X5":
            rep.cuts["X5"] += 1
        if tr.chain[k][0]:
            rep.cuts["X5f"] += 1
        if tr.reset[k]:            # an engagement restarts from rest at its origin
            o = tr.origin[k]
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
            if (not within and abs(curv) <= a_lim + tol_a and chain_window(tr, k)
                    and (jerk is None or abs(jerk) <= X5F_JERK * j_lim + tol_j)):
                within = True      # X5's clamp chain: its jerk only, where a later clamp overrode it, within X5F_JERK·j_max
                rep.cuts["X5f jerk"] = rep.cuts.get("X5f jerk", 0) + 1
                rep.worst["X5f jerk"] = max(rep.worst.get("X5f jerk", 0.0), abs(jerk or 0.0) / j_lim)
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
                # Domains: X2, X3 and X4 only while an unforeseeable event is pending (round 11: no
                # longer while only a Ruckig error is; X5's chain keeps the planner's promises).
                if exc in ("X2", "X3", "X4") and not tr.unforeseen[k]:
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
        if tr.failed[k] and not tr.x5[k]:
            rep.fail(f"{name} k={k}: Ruckig failed and X5 did not act")
        if tr.x5[k] and not exc:
            rep.fail(f"{name} k={k}: X5 acted without its tag")
        if tr.cont[k] is not None and tr.cont[k][0] > tr.cont[k][1] + 1e-9:
            rep.fail(f"{name} k={k}: X5 continued the trajectory in force past its end ({tr.cont[k][0]:.2f} > {tr.cont[k][1]:.2f} s)")
        if exc == "X4" and (tr.v[k] < -TOL or tr.a[k] < -TOL):
            rep.fail(f"{name} k={k}: X4 left a falling slope at 0 W")
    check_landing(tr, t, rep, name)
    check_ramp_landing(tr, t, rep, name)


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


BARE_269 = dict(  # Ruckig 0.19.4 refuses this rest-to-rest move: Δ = rise·ease exactly (round-5 falsifier)
    current_position=41.25, current_velocity=0.0, current_acceleration=0.0,
    max_velocity=15.0, min_velocity=-22.5, max_acceleration=2 * 15 / 2.75, max_jerk=4 * 15 / 2.75 ** 2)


def bare_269(scale: float = 1.0) -> bool:
    """Ruckig on the bare #269-class input, every limit times `scale`; True if it calculates."""
    o, i, tr = Ruckig(1), InputParameter(1), Trajectory(1)
    b = BARE_269
    i.current_position, i.current_velocity, i.current_acceleration = (
        [b["current_position"]], [b["current_velocity"]], [b["current_acceleration"]])
    i.target_position, i.target_velocity, i.target_acceleration = [0.0], [0.0], [0.0]
    i.max_velocity, i.min_velocity = [b["max_velocity"] * scale], [b["min_velocity"] * scale]
    i.max_acceleration, i.min_acceleration = [b["max_acceleration"] * scale], [-b["max_acceleration"] * scale]
    i.max_jerk = [b["max_jerk"] * scale]
    try:
        return o.calculate(i, tr) in (Result.Working, Result.Finished)
    except RuckigError:
        return False


GOLDEN_PLANS = {
    # The clamp chain alone (every calculation refused from tick 0), round 11: its reference pinned.
    "the clamp chain alone, 100 -> 400 W, defaults": (DEFAULT, [steady(100, 5), steady(400, 40)], [], None, dict(inject=1.0)),
    "the clamp chain alone, 400 -> 100 W, (50, x4, 4)": (Tuning(50, 4.0, 4.0), [steady(400, 5), steady(100, 40)], [], None, dict(inject=1.0)),
    # A fall large enough (gap > 9·r↑·τ at ×4) for the chain's stopping curve (a_max/2) to bind.
    "the clamp chain alone, 600 -> 100 W, (5, x4, 4)": (Tuning(5, 4.0, 4.0), [steady(600, 5), steady(100, 60)], [], None, dict(inject=1.0)),
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
    # X5 under a persistent error from a tick on (every calculation failing): steps 2-4 in order.
    "persistent error mid-fall, then a jump up": (DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 40)],
                                                   [], None, dict(inject=1.0, inject_from=60)),
    "persistent error mid-rise on a ramp": (DEFAULT, [steady(100, 5), ramp(100, 400, 60), steady(400, 20)],
                                            [], None, dict(inject=1.0, inject_from=100)),
    "persistent error while chasing a steep fall": (DEFAULT, [steady(500, 5), ramp(500, 100, 4), steady(100, 30)],
                                                    [], None, dict(inject=1.0, inject_from=26)),
}
# From `--golden`; a change to any rule that moves one of these traces fails.
GOLDEN = {
    'the clamp chain alone, 100 -> 400 W, defaults': (55682.130015, 124, 144),
    'the clamp chain alone, 400 -> 100 W, (50, x4, 4)': (34257.022009, 160, 160),
    'the clamp chain alone, 600 -> 100 W, (5, x4, 4)': (72244.123767, 240, 240),
    'ramp 100->400 W over 60 s': (87839.925566, 6, 6),
    'fall 300->100 W over 60 s': (66100.0, 0, 0),
    'ramp at the rate 100->300 W over 10 s': (35099.996567, 45, 45),
    'steep 100->400 W over 10 s': (55649.993349, 65, 65),
    'steep 400->100 W over 5 s': (28450.000924, 39, 39),
    'steep rise behind a fall': (47653.361105, 59, 79),
    'jump 100->400 W': (39950.0, 65, 85),
    'jump 400->100 W': (26150.0, 39, 59),
    '5 s at 400 W then 150 W': (24675.396732, 27, 47),
    'ramp 37 -> 49.5 W/s (50, x2, 1)': (103768.044493, 29, 29),
    'rise into a falling ramp (15, x2, 2)': (16319.294248, 20, 20),
    'engagement 0->250 W': (22875.0, 55, 75),
    'engagement above a rising ramp (50, x1.5, 1)': (9242.790611, 14, 21),
    'cap drop mid-fall': (32440.0, 30, 50),
    'skip mid-rise': (14775.0, 15, 35),
    'fall -171.8 then -145.7 W/s (50, x4, 1.5)': (14198.054637, 33, 53),
    'engaged on a short fall (40, x1.25, 2.75)': (36451.869461, 43, 49),
    'engaged just above a steep rise': (49669.334359, 65, 66),
    'engaged just below a steep fall (50, x4, 1)': (3387.42123, 17, 37),
    'engaged 0.98 W above a gentle rise (8, x3, 1)': (28405.528387, 3, 4),
    'ramp 5 -> 5.004 W/s': (140803.204816, 6, 6),
    'ramp 5 -> 5.004 W/s at 1000 W': (375115.704816, 6, 6),
    'jump onto a ramp that bends 5 -> 5.004 W/s mid-landing': (148302.450899, 22, 42),
    'cap 250: 300 W, ramp to 150 W in 0.25 s': (28541.515139, 55, 81),
    'persistent error mid-fall, then a jump up': (96395.907218, 108, 128),
    'persistent error mid-rise on a ramp': (93798.380845, 33, 33),
    'persistent error while chasing a steep fall': (35267.342079, 117, 117),
}


def persistent(rep: Report, report: bool):
    """X5 under a persistent Ruckig error: every calculation fails. The clamp chain must land every
    step within the fallback's bound (check_landing), at the defaults and the corners, and never
    overshoot the held line by more than ε."""
    n = 0
    for t in [DEFAULT] + CORNERS:
        for a, b in ((100.0, 400.0), (400.0, 100.0), (233.0, 200.0), (150.0, 150.0 + t.rise * t.ease)):
            tr = run([steady(a, 5), steady(b, fallback_bound(t, b - a, 0.0) + 10)], t=t, inject=1.0)
            check(tr, t, rep, f"persistent Ruckig error {a:g}->{b:g} {t}", a)
            if not any(c for c, _ in tr.chain):
                rep.fail(f"persistent Ruckig error {a:g}->{b:g} {t}: the clamp chain did not act")
            over = max((y - b) if b > a else (b - y) for y in tr.y[20:])
            if over > EPS:
                rep.fail(f"persistent Ruckig error {a:g}->{b:g} {t}: the chain overshot the line by {over:.2f} W")
            n += 1
    if report:
        print(f"Persistent Ruckig error: {n} steps land by the clamp chain (X5) within its bound")


def constants(rep: Report, report: bool):
    """The tolerances #620 must match, at their boundaries: SLOPE_TOL (collinear lines merge at a
    relative slope difference of 0.5e-9, not at 2e-9) and VALUE_TOL (pieces 1e-7 W apart merge,
    1e-5 W apart do not)."""
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
    # X5's clamp chain, in the reference order (jerk, then acceleration, then rate), on any input;
    # within |v0 - v1| <= a_max·Δt two orders agree, so one pin lies outside it.
    for args, want in (((100.0, 19.0, 14.0), 20.0), ((100.0, 10.0, 2.0), 15.0)):
        got = speed_limiter(*args, DT, DEFAULT)[0]
        if got != want:
            rep.fail(f"constants: speed_limiter{args} = {got} != {want} (jerk, acceleration, rate)")
    rng = random.Random(147)
    for _ in range(20000):
        tt = rng.choice(GRID)
        v0 = rng.uniform(-tt.r_dn, tt.r_up); v1 = v0 - rng.uniform(-tt.a_max, tt.a_max) * DT
        out, _ = speed_limiter(rng.uniform(-3 * tt.r_dn, 3 * tt.r_up), v0, v1, DT, tt)
        if not (-tt.r_dn - 1e-9 <= out <= tt.r_up + 1e-9) or abs(out - v0) > tt.a_max * DT + 1e-9:
            rep.fail(f"constants: speed_limiter left the rate or curvature band {tt}")
            break
    if N_ERRORS != 3:
        rep.fail(f"constants: N_ERRORS = {N_ERRORS}, stated 3")
    if report:
        print("Constants SLOPE_TOL and VALUE_TOL pinned at their boundaries; the snap, the raw join, the merge"
              " after the clamp; the clamp chain's order")


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


def fallback_landing(rep: Report, report: bool):
    """X5's landing bound, tight: every grid tuning, small and mid steps both ways, every
    calculation refused from tick 0 (the clamp chain alone). Each lands within fallback_bound and
    never passes the held line (round 11)."""
    worst = (-1e9, None)
    for t in GRID:
        for d in (0.6, 1.0, 2.0, 5.0, 20.0, t.rise * t.ease):
            for a, b in ((300.0, 300.0 + d), (300.0 + d, 300.0)):
                fb = fallback_bound(t, b - a, 0.0)
                tr = run([steady(a, 5), steady(b, fb + 5)], t=t, inject=1.0)
                ok = (lambda y: abs(y - b) <= EPS) if b > a else (lambda y: y <= b + EPS)
                land = next(((k - 20 + 1) * DT for k in range(20, len(tr.y)) if ok(tr.y[k])), None)
                over = max((y - b) if b > a else (b - y) for y in tr.y[20:])
                if land is None or land > fb + 1e-9:
                    rep.fail(f"fallback landing {a:g}->{b:g} {t}: {land} s > fallback_bound {fb:.2f} s")
                elif land - fb > worst[0]:
                    worst = (land - fb, (t, a, b))
                if over > 1e-6:
                    rep.fail(f"fallback landing {a:g}->{b:g} {t}: the chain passed the held line by {over:.3f} W")
    if report:
        print(f"Fallback landing: {len(GRID)} tunings × 12 steps, the clamp chain alone; latest {worst[0]:+.2f} s against fallback_bound {worst[1]}")


def persistent_random(rep: Report, report: bool, n: int = 300):
    """X5 under a persistent Ruckig error from ANY tick and ANY state: random plans (round 7's
    generator, caps, events), every calculation failing from a random onset tick. Every property,
    P4/P7 (check_landing, with the fallback's bound) included, must hold. Two schemes: this suite's
    own (seed 7585) and the round-7 falsifier's q4v7 (seeds 31000 + i)."""
    chain = 0
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
            tr = run(blocks, events, t=t, m0=m0, inject=1.0, inject_from=onset)
            check(tr, t, rep, f"persistent {scheme}#{i} from tick {onset} {t} {blocks} {events} m0={m0}", tr.y[0])
            chain += sum(c for c, _ in tr.chain)
    if report:
        print(f"Persistent Ruckig error from a random tick: 2 × {n} random plans (incl. the falsifier's q4v7), {chain} chain ticks")


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
    test and the boundary of `trackable`. #620's golden vectors carry the same plans tick by tick."""
    got = {}
    for name, spec in GOLDEN_PLANS.items():
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
    tuning of the full valid set: the input class Ruckig 0.19.4 refuses (§7.10.2, X5). Each must
    land within P4/P7's bound (check_landing)."""
    ts = [Tuning(r, f / 4, e / 4) for r in range(5, 51) for f in range(4, 17) for e in range(4, 17)
          if Tuning(r, f / 4, e / 4).valid()][::stride]
    errs = n = 0
    for t in ts:
        for mul in (1, 2, 3):
            d = t.rise * t.ease * mul
            for a, b in ((300.0, 300.0 - d), (300.0 - d, 300.0)):
                if min(a, b) < 0:
                    continue
                tr = run([steady(a, 5), steady(b, fallback_bound(t, b - a, 0.0) + 5)], t=t)
                check(tr, t, rep, f"boundary Δ = {mul}·rise·ease {a:g}->{b:g} {t}", a)
                errs += tr.errors
                n += 1
    if report:
        print(f"Δ = rise·ease steps: {n} plans over {len(ts)} tunings, Ruckig refusals {errs} (all landed through X5)")


def pinned(rep: Report, report: bool):
    """Regressions from the safety falsifications of PR #625, exact floats."""
    # The bare #269-class input: upstream refuses it; X5 (the trajectory in force, then the clamp
    # chain) lands the step (the "stuck step" cases below).
    if bare_269():
        rep.fail("pinned: the bare #269 input now calculates: upstream fixed it; update §7.10.2 and this pin")
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
        ("round-4 #269 plan, seed 7 plan 226 (no longer refused)", DEFAULT,
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
            print(f"  pinned {name}: Ruckig refusals {tr.errors}, tagged {sorted(set(e for e in tr.exc if e))}")
        if name in no_cut and any(e in CUTS for e in tr.exc):
            rep.fail(f"pinned: {name}: a cut {sorted(set(e for e in tr.exc if e in CUTS))} where the design eases")
    # X5 only continues a trajectory that reproduces the state: an engagement 2 W off a trajectory at
    # rest, then every calculation failing, must not continue it (a 2 W jump back), but move by the
    # clamp chain.
    tr = run([steady(100, 5), steady(400, 60)], [Event(160, "engage", 402.0)], t=DEFAULT, inject=1.0,
             inject_from=160)
    check(tr, DEFAULT, rep, "pinned: engagement 2 W off a landed trajectory, then every calculation failing",
          tr.y[0])
    if not any(c for c, _ in tr.chain[160:]):
        rep.fail("pinned: engagement 2 W off a landed trajectory: no clamp-chain move")
    # The same while the trajectory is still in flight (round 7: a landed one is never continued): at a
    # slow tuning, an engagement 2 W off a rise in progress (slopes and curvatures within 10 of it),
    # then every calculation failing, must move by the clamp chain, not jump back.
    t5 = Tuning(5, 4.0, 4.0)
    ref = run([steady(100, 5), steady(400, 80)], t=t5)
    tr = run([steady(100, 5), steady(400, 80)], [Event(100, "engage", ref.y[99] + 2.0)], t=t5, inject=1.0,
             inject_from=100)
    check(tr, t5, rep, "pinned: engagement 2 W off a rise in flight, then every calculation failing", tr.y[0])
    if not any(c for c, _ in tr.chain[100:]):
        rep.fail("pinned: engagement 2 W off a rise in flight: no clamp-chain move")
    # X3 after the step: a ramp into the cap whose braking plan fails (a Ruckig error at that moment).
    tr = run([steady(580.7, 5), ramp(580.7, 1131.0, 60)], t=Tuning(40, 2.0, 2.0), m0=790.99, fail_brake=True)
    check(tr, Tuning(40, 2.0, 2.0), rep, "pinned: ramp into the cap, braking plan fails", tr.y[0])
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
    # (1, 2866, v7) and (7, 1525, v7), round 10: a cap drop below a falling command with Ruckig refusing; the
    # chain must keep the falling slope X3 kept (its rates rebuilt from the cut state, not "at rest").
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
            print(f"  pinned seed {seed} plan {idx}: Ruckig errors {tr.errors}, tagged {cuts}, max |dv| "
                  f"{max(abs(tr.v[k] - tr.v[k - 1]) for k in range(1, len(tr.v))):.2f} W/s")


# The safety falsifiers' repros of PR #625 (f625b: round 5, f625c: round 6, f625d: round 7) and
# track 2's segfault plan, run as plain plans and held to every property.
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
    ("q1 fall ramp, persistent from 58", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(300, 30)], {"inject": 1.0, "inject_from": 58}),
    ("q1 steep fall, persistent from 40", DEFAULT, [steady(400, 5), ramp(400, 100, 5), steady(100, 30)], {"inject": 1.0, "inject_from": 40}),
    ("q2 fall, persistent from 58, (5, x4, 4)", Tuning(5, 4.0, 4.0), [steady(400, 5), ramp(400, 300, 10), steady(300, 120)], {"inject": 1.0, "inject_from": 58}),
    ("q3 jump up to 500, persistent from 60", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 120)], {"inject": 1.0, "inject_from": 60}),
    ("q3 jump up to 500, persistent from 59", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(500, 120)], {"inject": 1.0, "inject_from": 59}),
    ("q3 300 held, persistent from 58", DEFAULT, [steady(400, 5), ramp(400, 300, 10), steady(300, 120)], {"inject": 1.0, "inject_from": 58}),
    ("q6 800->809.7->848.5", DEFAULT, [steady(800, 5), ramp(800, 809.7, 0.5), ramp(809.7, 848.5, 2), steady(848.5, 15)], {}),
    ("q7 cap 250: 300, ramp to 150 in 0.25 s", DEFAULT, [steady(300, 10), ramp(300, 150, 0.25), steady(150, 60)], {"m0": 250.0}),
    ("q7 cap 250: 300, ramp to 240 in 1 s", DEFAULT, [steady(300, 10), ramp(300, 240, 1), steady(240, 60)], {"m0": 250.0}),
    ("q7 0 W, ramp to 400 in 0.25 s", DEFAULT, [steady(0, 10), ramp(0, 400, 0.25), steady(400, 60)], {}),
    ("q8 cap 250: 400 4 s, ramp to 150", DEFAULT, [steady(100, 10), steady(400, 4), ramp(400, 150, 0.25), steady(150, 30)], {"m0": 250.0}),
    ("q8 cap 250: 300 4 s, ramp to 150", DEFAULT, [steady(100, 10), steady(300, 4), ramp(300, 150, 0.25), steady(150, 30)], {"m0": 250.0}),
    ("q9 cap 250: 400 2 s, ramp to 0", DEFAULT, [steady(100, 10), steady(400, 2), ramp(400, 0, 0.25), steady(0, 30)], {"m0": 250.0}),
    # Ruckig 0.19.4 segfaults on a target velocity exactly at max_velocity from this plan's state at tick 241
    ("segfault: a ramp beyond the rate, target velocity at the rate (seed 585, plan 19613)", Tuning(30, 4.0, 4.0),
     [steady(391.25, 2), ramp(391.25, 686.9500551790104, 2), steady(494.0, 15), steady(0.0, 1), ramp(0.0, 106.09132761117559, 5),
      steady(106.09132761117559, 5), steady(554.0, 30), steady(614.0, 0.5), ramp(614.0, 776.9755103030757, 0.5)], {}),
    # f625d (round 7's falsifier)
    ("p3 cap 250, 400 then 300", DEFAULT, [steady(100, 10), steady(400, 6), steady(300, 30)], {"m0": 250.0}),
    ("p3 cap 250, 400 then 300, persistent from 40", DEFAULT, [steady(100, 10), steady(400, 6), steady(300, 30)], {"m0": 250.0, "inject": 1.0, "inject_from": 40}),
    ("p4 cap 250 staircase", DEFAULT, [steady(100, 10)] + [steady(400 - 10 * i, 2) for i in range(10)] + [steady(300, 30)], {"m0": 250.0}),
    ("p4 cap 250 staircase, persistent from 40", DEFAULT, [steady(100, 10)] + [steady(400 - 10 * i, 2) for i in range(10)] + [steady(300, 30)], {"m0": 250.0, "inject": 1.0, "inject_from": 40}),
    ("p5 300, 0, 200", DEFAULT, [steady(300, 10), steady(0, 4), steady(200, 40)], {}),
    ("p5 300, 0, 200, persistent from 41", DEFAULT, [steady(300, 10), steady(0, 4), steady(200, 40)], {"inject": 1.0, "inject_from": 41}),
    ("p5 300, 0, 250, persistent from 41", DEFAULT, [steady(300, 10), steady(0, 4), steady(250, 40)], {"inject": 1.0, "inject_from": 41}),
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
    # X5's mechanism: N_ERRORS ticks of the trajectory in force, then the chain; a single refusal is
    # one tick of the trajectory in force and Ruckig is used again on the next.
    # (An offset at tick 25 forces a re-plan while the step's trajectory is in force.)
    plan, ev = [steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1)]
    tr = run(plan, ev, t=DEFAULT, inject=1.0, inject_from=25)
    tags = ["C" if c else ("E" if e == "X5" else e) for e, (c, _) in zip(tr.exc, tr.chain)]
    want = [""] * 25 + ["E"] * 3 + ["C"] * 5      # 3: the stated N_ERRORS, not the constant
    if tags[:len(want)] != want:
        rep.fail(f"repro: X5 from tick 25: {tags[20:35]} != {want[20:35]} (N_ERRORS ticks in force, then the chain)")
    once = run(plan, ev, t=DEFAULT, inject=1.0, inject_from=25, refuse_until=26)
    if not (once.exc[25] == "X5" and once.exc[26] != "X5" and not any(c for c, _ in once.chain)):
        rep.fail(f"repro: a single refusal at tick 25: {once.exc[24:28]} (one tick in force, then Ruckig)")
    back = run(plan, ev, t=DEFAULT, inject=1.0, inject_from=25, refuse_until=35)
    if not (any(c for c, _ in back.chain[28:35]) and not any(c for c, _ in back.chain[35:])):
        rep.fail("repro: after the refusals end, the command does not return to Ruckig")
    # Round 11 (falsifier at f135b8c): X5's mechanism, pinned behaviourally.
    tags = lambda tr: ["C" if c else ("E" if e == "X5" else e) for e, (c, _) in zip(tr.exc, tr.chain)]
    # fail_run restarts after a success: a second refusal episode starts with the trajectory in force.
    two = run([steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1), Event(60, "offset", 1.2)], t=DEFAULT,
              refuse_ticks=set(range(25, 31)) | {60})
    if tags(two)[60] != "E":
        rep.fail(f"repro: a second refusal episode at tick 60 starts with {tags(two)[60]!r}, not the trajectory in force")
    # Continuation on a ramp frame: the trajectory in force reproduces the state at tick k - 1.
    rf = run([steady(100, 5), ramp(100, 300, 20), steady(300, 10)], [Event(30, "offset", 1.1)], t=DEFAULT, refuse_ticks={30, 31})
    if tags(rf)[30:32] != ["E", "E"]:
        rep.fail(f"repro: refusals on a ramp frame at ticks 30-31: {tags(rf)[30:32]} (the trajectory in force expected)")
    # The chain aims at the clamped line: under a cap, no cut at M.
    cap = run([steady(100, 5), steady(400, 40)], t=DEFAULT, m0=250.0, inject=1.0, inject_from=20)
    if "X3" in cap.exc or abs(cap.y[-1] - 250.0) > EPS:
        rep.fail(f"repro: persistent refusal under a cap of 250 W: X3 cuts {cap.exc.count('X3')}, ends at {cap.y[-1]:.2f} W")
    # Blocker 1: refusals on a planned 100 -> 400 -> 100 W at the defaults add no load above the plan and no
    # cut (the chain's closed-form braking check); its curvature and jerk stay in band.
    for k0, n in ((40, 12), (40, 4), (44, 8)):
        base = run([steady(100, 10), steady(400, 3), steady(100, 30)], t=DEFAULT)
        tr = run([steady(100, 10), steady(400, 3), steady(100, 30)], t=DEFAULT, refuse_ticks=set(range(k0, k0 + n)))
        check(tr, DEFAULT, rep, f"repro: 100/400/100 W, refusals {k0}..{k0 + n - 1}", 100.0)
        up = max([0.0] + [tr.y[k] - tr.r[k] for k in range(1, len(tr.y)) if tr.y[k] > tr.y[k - 1] + 1e-7])
        if any(e in CUTS for e in tr.exc) or up > EPS:
            rep.fail(f"repro: 100/400/100 W, refusals {k0}..{k0 + n - 1}: cuts {[e for e in tr.exc if e in CUTS]}, rose {up:.2f} W above the plan")
    for hi in (300, 400):
        tr = run([steady(100, 10), steady(hi, 4), ramp(hi, 150, 0.25), steady(150, 30)], t=DEFAULT, m0=250.0, inject=1.0,
                 inject_from=40)
        check(tr, DEFAULT, rep, f"repro: round-6 plan at {hi} W, cap 250, refusals from tick 40", 100.0)
    # Blocker 2: the hand-back onto a falling ramp, strictly inside the rates (the 2100 W plan).
    ho = run([steady(2100, 10), ramp(1800, 0, 90), steady(0, 30)], t=DEFAULT, m0=4000.0, refuse_ticks=set(range(40, 62)))
    check(ho, DEFAULT, rep, "repro: hand-back onto a -20 W/s ramp (refusals 40..61)", 2100.0)
    above = sum(1 for y, r in zip(ho.y, ho.r) if y > r + EPS) * DT
    if above > 18.75 + 1e-9:
        rep.fail(f"repro: hand-back onto a -20 W/s ramp: {above} s above the line, more than without refusals (18.75 s)")
    # A braking plan refused (every one): the closed-form stop (X5b) instead; the fall to 0 W keeps P7.
    for t in [DEFAULT] + CORNERS:
        for hi, lo in ((400.0, 0.0), (400.0, 100.0), (600.0, 200.0)):
            tr = run([steady(hi, 10), steady(lo, 60)], t=t, fail_brake=True)
            check(tr, t, rep, f"repro: every braking plan refused, {hi:g} -> {lo:g} W {t}", hi)
    # The hard clamp is absolute whatever the planner or X5 returns (round 11: since the closed-form
    # floor and braking checks, X3 after the step and X4 never act in the suites, so pin them directly).
    for t in (DEFAULT, Tuning(50, 4.0, 1.0)):
        ez = Easer(t)
        # Below 0 W (X4: at rest at 0 W), and just above M within ε of a line held at M (X3, not X2).
        for line_w, st0, nst, want, cut in ((200.0, State(1.0), State(-3.0, -12.0, -5.0), State(0.0, 0.0, 0.0), "X4"),
                                             (400.0, State(249.9), State(250.3, 1.0, 0.0), State(250.0, 0.0, 0.0), "X3")):
            segs = segments([steady(line_w, 20)], 0, 1.0, 250.0)
            out, exc = ez._after(st0, nst, 4, segs, 250.0, min(line_w, 250.0), "", True)
            if (out.y, out.v, out.a) != (want.y, want.v, want.a) or exc != cut:
                rep.fail(f"repro: the hard clamp on {nst} gives {out} {exc!r}, not {want} {cut}")
    # The chain tracks a ramp onto the line (its gap at the state's own tick): persistent refusals on
    # ramps within the rates, both ways, at the defaults and the corners.
    for t in [DEFAULT] + CORNERS:
        for f in (0.5, -0.5):
            s_ = f * (t.r_up if f > 0 else t.r_dn)
            w = 300.0 + abs(s_) * 70
            # Refused from the ramp's start (track 1 re-plans only on a change), up to 20 s before its end (the
            # planner's lead into the end vertex is not the chain's).
            tr = run([steady(w, 5), ramp(w, w + s_ * 60, 60), steady(w + s_ * 60, 10)], t=t, m0=1e6, inject=1.0, inject_from=20)
            tail = [abs(tr.y[k] - tr.r[k]) for k in range(20 + int(fallback_bound(t, s_ * 4, 0.0) / DT) + 8, 20 + 160)]
            if tail and max(tail) > EPS:
                rep.fail(f"repro: the chain on a {s_:g} W/s ramp {t} stays {max(tail):.2f} W off the line")
    # A cap drop below a command the chain moves down at the full rate: X3 before the step keeps the
    # falling slope (its bound), on chain ticks too.
    for t in (DEFAULT, Tuning(20, 4.0, 2.0)):
        plan = [steady(600, 10), steady(100, 40)]
        free = run(plan, t=t, inject=1.0, inject_from=40)
        kc = next(k for k in range(40, len(free.y)) if free.v[k] <= -0.9 * t.r_dn)   # the chain near the full falling rate
        tr = run(plan, [Event(kc + 2, "cap", free.y[kc + 1] - 40.0)], t=t, inject=1.0, inject_from=40)
        check(tr, t, rep, f"repro: a cap drop below the chain at the full falling rate {t}", 100.0)
    if report:
        print(f"Repros: {len(REPROS)} falsifier plans (f625b/c/d) and the segfault plan; X5's order pinned; round 11's"
              f" blockers and mechanism pins")


def refusal_windows(rep: Report, report: bool, n: int = 300):
    """Round 11: a window of 1-40 refused ticks at a random place in random plans (round 7's
    generator, the falsifier's schemes: seeds 56000 + i and 70000 + i), against the same plan without
    refusals. Every property holds, no cut appears that the plan without refusals does not have, and
    the command never rises further above the plan (X5 keeps the planner's promises)."""
    worst_up = worst_lag = 0.0
    for base_seed in (56000, 70000):
        for i in range(n):
            rng = random.Random(base_seed + i)
            t = rng.choice(GRID)
            blocks, events = random_plan_v7(rng, t)
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
            total = sum(b.ticks for b in blocks)
            k0 = rng.randrange(max(1, total))
            L = rng.choice([1, 2, 3, 4, 6, 8, 12] if base_seed == 56000 else [4, 5, 6, 8, 12, 20, 40])
            a = run(blocks, events, t=t, m0=m0)
            b = run(blocks, events, t=t, m0=m0, refuse_ticks=set(range(k0, k0 + L)))
            name = f"refusal window {base_seed}+{i} ticks {k0}..{k0 + L - 1} {t}"
            check(b, t, rep, name, b.y[0])
            if sum(e in CUTS for e in b.exc) > sum(e in CUTS for e in a.exc):
                rep.fail(f"{name}: X5 added a cut {[e for e in b.exc if e in CUTS]}")
            ups = [max([0.0] + [tr.y[k] - tr.r[k] for k in range(1, len(tr.y)) if tr.y[k] > tr.y[k - 1] + 1e-7]) for tr in (a, b)]
            lags = [max([0.0] + [tr.y[k] - tr.r[k] for k in range(1, len(tr.y)) if tr.y[k] <= tr.y[k - 1] + 1e-7]) for tr in (a, b)]
            worst_up, worst_lag = max(worst_up, ups[1] - ups[0]), max(worst_lag, lags[1] - lags[0])
            if ups[1] > max(ups[0], EPS) + 1e-6:
                rep.fail(f"{name}: X5 rose {ups[1]:.2f} W above the plan (without refusals {ups[0]:.2f} W)")
    if report:
        print(f"Refusal windows: 2 × {n} random plans; extra rise above the plan {worst_up:.2f} W at most, extra lag"
              f" on falls {worst_lag:.2f} W at most")


def injected(rep: Report, report: bool, n: int = 300, share: float = 0.2):
    """Random plans with a share of Ruckig calculations forced to fail: X5 continues the
    trajectory in force, and the cuts X2-X4 keep every property where it cannot."""
    rng = random.Random(1585)
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan(rng, t)
        tr = run(blocks, events, t=t, m0=rng.choice([2000.0, rng.uniform(200, 800)]), inject=share, inject_seed=i)
        check(tr, t, rep, f"injected#{i} {t} {blocks} {events}", tr.y[0])
    if report:
        print(f"Fault injection: {n} random plans with {share:.0%} of Ruckig calculations failing")


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
            above = next((k + 1) * DT for k in range(len(tr.y)) if tr.y[k] <= tr.r[k] + EPS)
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


def randoms(rep: Report, n: int, report: bool, seed: int = 585):
    rng = random.Random(seed)
    errors0, plans0, with0 = rep.errors, rep.plans, rep.plans_with_errors
    ticks = 0
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        tr = run(blocks, events, t=t, origin=origin, m0=rng.choice([2000.0, rng.uniform(200, 800)]))
        if tr.y:
            check(tr, t, rep, f"random#{i} {t} {blocks} {events} origin {origin}", tr.y[0])
            ticks += len(tr.y)
    if report:
        print(f"Random plans, seed {seed}: {n} plans, {ticks} ticks, every property checked on every tick;"
              f" Ruckig errors {rep.errors - errors0} in {rep.plans_with_errors - with0} plans"
              f" ({(rep.plans_with_errors - with0) / max(1, n):.3%} of plans)")


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
    persistent(rep, report)
    fallback_landing(rep, report)
    persistent_random(rep, report)
    split_equivalence(rep, report)
    constants(rep, report)
    line_independence(rep, report)
    repros(rep, report)
    refusal_windows(rep, report)
    golden(rep, report)
    ramp_pairs(rep, report)
    injected(rep, report)
    held(rep, report)
    for seed in seeds:
        randoms(rep, n, report, seed)
    if report:
        print("Named exceptions (ticks that break the curvature/jerk band): " +
              ", ".join(f"{k} {rep.cuts[k]} ticks, largest slope change {rep.worst.get(k, 0.0):.1f} W/s"
                        for k in ("X2", "X3", "X4")) + f"; X5: trajectory in force {rep.cuts['X5']} ticks, clamp chain"
              f" {rep.cuts['X5f']} ticks, of which {rep.cuts.get('X5f jerk', 0)} jerk-window ticks")
    if rep.fails:
        print("FAIL", rep.nfail, "failures by kind:", rep.kinds)
        for f in rep.fails:
            print("  " + (f if len(f) <= 400 else f[:150] + " … " + f[-240:]))
        return 1
    print("erg_slew (ruckig in the loop): all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
