#!/usr/bin/env python3
"""Library-in-the-loop model of the eased ERG target, ADR-0002 §7.10 (#585, design track 2).

Track 2 (owner decision 2026-11-15, option b): Ruckig with relaxed promises. Every tick Ruckig
(https://github.com/pantor/ruckig, MIT, Community edition; Python bindings `ruckig==0.19.4`)
plans from the commanded target's state (y W, v W/s, a W/s²) to a target state on the plan line:
- position: the line `LOOKAHEAD` ticks ahead;
- velocity: the line's slope there, clamped to the rider's rates;
- acceleration: 0;
- a minimum duration of `LOOKAHEAD` ticks.
The limits are v ∈ [−r↓, r↑], ±a_max and ±j_max, from the rider's tuning. The sent value is the
state one tick later, hard-clamped to [0, M]. That clamp is the only safety rule.

If Ruckig refuses, crank sends one more tick of the trajectory in force. After N_ERRORS
consecutive refusals it switches to the clamp chain of ros2_control's SpeedLimiter (jerk, then
acceleration, then velocity), which cannot fail, and returns to Ruckig on its first success.

Track 1 (rounds 4-7: lead and vertex frames, safe braking, the collinear merge, the X2-X5 cuts,
the continuation and closed form) is in the git history of PR #625 and in ADR-0002 §7.10.9.

The model checks the promises of §7.10.6 on every tick, with Ruckig in the loop, and `--report`
prints the figures the ADR quotes.

Setup: Ruckig is not in nixpkgs, so this model is not a flake check. #620 carries the same
promises as Rust property tests on `rsruckig`, and #634 tracks running it in CI.
    python3 -m venv .venv && .venv/bin/pip install ruckig==0.19.4
    .venv/bin/python docs/adr/models/erg_slew.py [--random N] [--seeds a,b] [--report]
Exit 0 means every check passed.
"""
from __future__ import annotations

import math
import random
import sys
from dataclasses import dataclass, field

from ruckig import InputParameter, Result, Ruckig, RuckigError, Trajectory

DT = 0.25          # one engine tick, s
EPS = 0.5          # W: "on the line" is |R - y| <= EPS
TOL = 1e-6         # floating-point tolerance of the checks
LOOKAHEAD = 2      # ticks: the target is the line this far ahead, reached no sooner (measured, §7.10.2)
N_ERRORS = 3       # consecutive Ruckig refusals answered from the trajectory in force
MIN_DURATION = True   # the look-ahead target is reached no sooner than LOOKAHEAD ticks
TARGET_SCALE = 1 - 2 ** -20   # the target velocity stays this far inside the rates (§7.10.2)


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
        """From rest to the rising rate in exactly `ease` seconds as one S-curve: a = 2·rise/τ."""
        return 2 * self.r_up / self.ease

    @property
    def j_max(self) -> float:
        """j = 4·rise/τ², both directions."""
        return 4 * self.r_up / self.ease ** 2

    def valid(self) -> bool:
        return (5 <= self.rise <= 50 and 1 <= self.factor <= 4 and (self.factor * 4) % 1 == 0
                and 1 <= self.ease <= 4 and (self.ease * 4) % 1 == 0 and self.r_dn >= 20)


DEFAULT = Tuning()
CORNERS = [Tuning(5, 4.0, 4.0), Tuning(50, 1.0, 1.0), Tuning(50, 4.0, 1.0), Tuning(50, 1.0, 4.0),
           Tuning(5, 4.0, 1.0), Tuning(20, 1.0, 4.0), Tuning(50, 4.0, 4.0), Tuning(20, 1.0, 1.0)]
GRID = [Tuning(r, f, e) for r in (5, 8, 10, 15, 20, 30, 40, 50) for f in (1.0, 1.25, 1.5, 2.0, 3.0, 4.0)
        for e in (1.0, 1.5, 2.0, 2.75, 3.0, 4.0) if Tuning(r, f, e).valid()]


# ---------------------------------------------------------------- the plan line

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
    kind: str     # "offset" (value = multiplier), "cap" (value = M), "skip", "engage" (value = trainer's W)
    value: float = 0.0


def steady(w, secs):
    return Block(int(round(secs / DT)), w, w)


def ramp(a, b, secs):
    return Block(int(round(secs / DT)), a, b)


def line(blocks: list[Block], k: int, mult: float) -> tuple[float, float]:
    """The plan line at tick k (multiplier applied, not clamped): value and slope. Past the end of
    the plan the last value is held."""
    k0 = 0
    for b in blocks:
        if k < k0 + b.ticks:
            return (b.frm + b.slope * (k - k0) * DT) * mult, b.slope * mult
        k0 += b.ticks
    return (blocks[-1].to * mult, 0.0) if blocks else (0.0, 0.0)


def clamp(v: float, m: float) -> float:
    return min(max(v, 0.0), m)


def target(blocks, k: int, mult: float, m: float, t: Tuning, look: int) -> tuple[float, float]:
    """Ruckig's target for the state sent at tick k, planned from the state sent at tick k - 1: the
    line at tick k - 1 + look, reached no sooner than `look` ticks (so at look = 1, R(k) one tick
    later), clamped to [0, M], with the line's slope there clamped to the rates (0 where the clamp
    holds the line). At look = 0 the target is R(k) with no minimum duration.
    The slope is clamped to the rates × TARGET_SCALE, not to the rates: Ruckig 0.19.4 segfaults on a
    target velocity exactly at max_velocity from some states (seed 585, plan 19613 of generator v7;
    the bare input is in PR #625's upstream draft), and one ulp inside it calculates."""
    r, s = line(blocks, k - 1 + max(look, 1), mult)
    if r <= 0.0 or r >= m:
        return clamp(r, m), 0.0
    return r, min(max(s, -t.r_dn * TARGET_SCALE), t.r_up * TARGET_SCALE)


# ---------------------------------------------------------------- the controller (§7.10.2)

@dataclass
class State:
    y: float
    v: float = 0.0
    a: float = 0.0


def speed_limiter(v: float, v0: float, v1: float, dt: float, t: Tuning, order=("jerk", "acc", "vel")) -> tuple[float, bool]:
    """ros2_control's SpeedLimiter::limit (ros-controls/ros2_controllers, humble,
    diff_drive_controller/src/speed_limiter.cpp, commit 3c61e4f, lines 78-138, Apache-2.0),
    one derivative up: it limits the commanded rate v (W/s) given the two previous rates v0, v1.
    One deliberate difference: the jerk step uses j·dt², not the reference's 2·j·dt², so the
    sampled jerk (the third difference of the sent values) is bounded by j_max, not 2·j_max.
    Returns the limited rate, and whether a later clamp moved it after the jerk clamp (the only case
    in which the sampled jerk can exceed j_max)."""
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
    """The rate the clamp chain asks for: the line's slope plus a closing rate. The closing rate is
    the one from which the command can still stop within the gap at half the acceleration limit,
    with the jerk limit's ramp-up (d = v²/(2·a_b) + v·a_b/j), and near the line the gap over 2τ: a
    linear zone the jerk-limited chain follows without ringing (over τ it rings in a jerk-saturated
    limit cycle, ±1.7 W at (5, ×4, 2); over 4τ it lands twice as slowly; §7.10.2)."""
    e = abs(r - y)
    ab = 0.5 * t.a_max
    tj = ab / t.j_max
    v_stop = ab * (-tj + math.sqrt(tj * tj + 2 * e / ab))
    v_ref = min(max(s + math.copysign(min(v_stop, e / (2 * t.ease)), r - y), -t.r_dn), t.r_up)
    # Aim short by the rate the current curvature still adds while the jerk limit unwinds it
    # (a·|a|/(2j)), so the chain meets its reference instead of overshooting it into the rate clamp.
    # Without it the state the chain hands back to Ruckig can be one from which the rate limit
    # cannot be kept (round 8: 22.9 W/s against 20 at (20, ×4, 3)).
    return v_ref - a0 * abs(a0) / (2 * t.j_max)


def handoff(v: float, a: float, t: Tuning) -> tuple[float, float]:
    """The chain's sampled state as Ruckig's continuous one, for the tick Ruckig succeeds again: the
    slope at the sample (the backward difference plus half a tick of curvature), and a curvature
    from which the rate limits can still be kept (v + a·|a|/(2j) within them). The chain's samples
    lag the continuous curvature, so without the clip Ruckig overshot the rate after the chain
    (round 8: −27.9 W/s against −25 at (20, ×1.25, 1.5)). The clip's jerk falls in the chain's
    window (§7.10.6, promise 1)."""
    v = min(max(v, -t.r_dn), t.r_up)
    a = min(max(a, -t.a_max), t.a_max)
    if a > 0 and v + a * a / (2 * t.j_max) > t.r_up:
        a = math.sqrt(2 * t.j_max * (t.r_up - v))
    elif a < 0 and v - a * a / (2 * t.j_max) < -t.r_dn:
        a = -math.sqrt(2 * t.j_max * (v + t.r_dn))
    return v, a


class Slew:
    def __init__(self, t: Tuning, look: int = LOOKAHEAD, n_errors: int = N_ERRORS, inject: float = 0.0,
                 inject_seed: int = 0, inject_from: int = 0):
        self.t, self.look, self.n_errors = t, look, n_errors
        self.otg = Ruckig(1)
        self.inp = InputParameter(1)
        self.traj = None            # the trajectory in force, and the time into it of the last sent state
        self.elapsed = 0.0
        self.errors = 0             # consecutive refusals
        self.refusals = 0           # all refusals (counted in the ride's diagnostics)
        self.rng = random.Random(inject_seed)
        self.inject, self.inject_from = inject, inject_from
        self.hist: list[float] = []   # the sent values, for the clamp chain's rates
        self.refuse: set[int] = set()   # ticks forced to fail (tests)

    def plan(self, st: State, r: float, s: float, k: int) -> Trajectory | None:
        t, i = self.t, self.inp
        i.current_position, i.current_velocity, i.current_acceleration = [st.y], [st.v], [st.a]
        i.target_position, i.target_velocity, i.target_acceleration = [r], [s], [0.0]
        i.max_velocity, i.min_velocity = [t.r_up], [-t.r_dn]
        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max]
        i.minimum_duration = self.look * DT if self.look > 0 and MIN_DURATION else None
        if k in self.refuse or (k >= self.inject_from and self.inject and self.rng.random() < self.inject):
            return None
        traj = Trajectory(1)
        try:
            res = self.otg.calculate(i, traj)
        except (RuckigError, ValueError) as err:   # ValueError: a result code the bindings do not name
            self.last_error = (str(err), st, r, s)
            return None
        return traj if res in (Result.Working, Result.Finished) else None

    def step(self, st: State, k: int, r: float, s: float, m: float) -> tuple[State, str]:
        """One tick: the state sent at tick k, and its tag ('' Ruckig, 'E' the trajectory in force
        after a refusal, 'C' the clamp chain, '+K' the hard clamp acted)."""
        traj = self.plan(st, r, s, k)
        if traj is not None:
            self.errors, self.traj, self.elapsed = 0, traj, DT
            p, v, a = traj.at_time(DT)
            nst, tag = State(p[0], v[0], a[0]), ""
        else:
            self.errors += 1
            self.refusals += 1
            if self.errors <= self.n_errors and self.traj is not None:
                self.elapsed += DT
                p, v, a = self.traj.at_time(self.elapsed)
                nst, tag = State(p[0], v[0], a[0]), "E"
            else:
                self.traj = None
                h = ([self.hist[0]] * 3 + self.hist)[-3:] if self.hist else [st.y] * 3
                v0, v1 = (h[2] - h[1]) / DT, (h[1] - h[0]) / DT
                v_new, over = speed_limiter(chain_rate(r, s, st.y, self.t, (v0 - v1) / DT), v0, v1, DT, self.t)
                a_new = (v_new - v0) / DT
                nst = State(st.y + v_new * DT, *handoff(v_new + a_new * DT / 2, a_new, self.t))
                tag = "Co" if over else "C"
        if nst.y > m or nst.y < 0.0:
            # The hard clamp: the sent value is the clamped one, at rest, and the history the chain
            # reads its rates from restarts there (a cap drop's jump is not a rate).
            nst, tag = State(clamp(nst.y, m)), tag + "K"
            self.traj = None
            self.hist = []
        self.hist.append(nst.y)
        return nst, tag

    def reset(self, y: float):
        """An engagement: the state is the trainer's, at rest; no trajectory is in force."""
        self.traj, self.errors = None, 0
        self.hist = [y]
        return State(y)


@dataclass
class Trace:
    r: list = field(default_factory=list)       # the plan line at the tick (clamped to [0, M])
    s: list = field(default_factory=list)       # its slope
    y: list = field(default_factory=list)       # the sent value
    v: list = field(default_factory=list)
    a: list = field(default_factory=list)
    tag: list = field(default_factory=list)
    m: list = field(default_factory=list)
    reset: list = field(default_factory=list)   # an engagement reset the state on this tick
    jump: list = field(default_factory=list)    # the line is discontinuous at this tick (a step, an event)
    events: list = field(default_factory=list)
    refusals: int = 0
    y_init: float = 0.0


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, m0=2000.0,
        y0: float | None = None, v0: float = 0.0, inject: float = 0.0, inject_seed: int = 0,
        inject_from: int = 0, look: int = LOOKAHEAD, n_errors: int = N_ERRORS, refuse=()) -> Trace:
    blocks = [Block(b.ticks, b.frm, b.to) for b in blocks if b.ticks > 0]
    ev: dict[int, list[Event]] = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    sl = Slew(t, look, n_errors, inject, inject_seed, inject_from)
    sl.refuse = set(refuse)
    mult, m = 1.0, m0
    tr = Trace()
    total = sum(b.ticks for b in blocks)
    r0, _ = line(blocks, 0, mult)
    if origin is not None:
        st = sl.reset(clamp(origin, m))
    else:
        st = State(clamp(r0, m) if y0 is None else y0, v0, 0.0)
    tr.y_init = st.y
    sl.hist = [st.y] if v0 == 0.0 else [st.y - v0 * DT, st.y]
    prev = None
    for k in range(total):
        reset = False
        for e in sorted(ev.get(k, []), key=lambda e: e.kind != "cap"):   # a cap first
            if e.kind == "offset":
                mult = e.value
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
            elif e.kind == "engage":
                st, reset = sl.reset(clamp(e.value, m)), True
        rl, sl_k = line(blocks, k, mult)
        r = clamp(rl, m)
        s = 0.0 if r != rl else sl_k
        jump = prev is not None and abs(r - (prev[0] + prev[1] * DT)) > 1e-6
        rt, st_ = target(blocks, k, mult, m, t, look)
        st, tag = sl.step(st, k, rt, st_, m)
        for lst, val in ((tr.r, r), (tr.s, s), (tr.y, st.y), (tr.v, st.v), (tr.a, st.a), (tr.tag, tag),
                         (tr.m, m), (tr.reset, reset), (tr.jump, jump), (tr.events, bool(ev.get(k)))):
            lst.append(val)
        prev = (r, s)
    tr.refusals = sl.refusals
    return tr


# ---------------------------------------------------------------- checks (§7.10.6)

class Report:
    def __init__(self):
        self.fails: list[str] = []
        self.nfail = 0
        self.kinds: dict[str, int] = {}
        self.refusals = 0
        self.plans_with_refusals = 0
        self.chain_ticks = 0
        self.clamp_ticks = 0
        self.exempt = {"clamp": 0, "engage": 0, "chain": 0}
        self.excess = {}        # (family, tuning) -> (worst excess above the line, plan)

    def fail(self, msg: str):
        self.nfail += 1
        kind = " ".join(msg.split()[:2])
        self.kinds[kind] = self.kinds.get(kind, 0) + 1
        if len(self.fails) < 40:
            self.fails.append(msg)

    def worst(self, key, val, where):
        if key not in self.excess or val > self.excess[key][0]:
            self.excess[key] = (val, where)


def sampled(tr: Trace, k: int) -> tuple[float, float, float]:
    """Backward differences of the sent values at tick k: rate, curvature and jerk."""
    ys = [tr.y_init] * 3 + tr.y
    n = k + 3
    v = [(ys[n - i] - ys[n - i - 1]) / DT for i in range(3)]
    a = [(v[i] - v[i + 1]) / DT for i in range(2)]
    return v[0], a[0], (a[0] - a[1]) / DT


def exempt(tr: Trace, k: int, what: str) -> str | None:
    """The ticks where a sampled limit may break, exhaustively (§7.10.6, promise 1). `what` is the
    broken limit ('rate', 'curvature', 'jerk'). The window of the differences (this tick and the
    three before) contains:
    - 'clamp': the hard clamp at 0 W or M acting (including a lowered cap below the command);
    - 'engage': an engagement resetting the state to the trainer's reading;
    - 'chain': the clamp chain's jerk, on a tick where its acceleration or rate clamp moved the rate
      after the jerk clamp (tag 'Co'; the reference algorithm's order), or on the first Ruckig tick
      after the chain (the hand-off's curvature clip). Rate and curvature are never exempt there."""
    for j in range(max(0, k - 3), k + 1):
        if "K" in tr.tag[j]:
            return "clamp"
        if tr.reset[j]:
            return "engage"
    if what == "jerk" and any("Co" in tr.tag[j] or (j > 0 and "C" in tr.tag[j - 1] and "C" not in tr.tag[j])
                              for j in range(max(0, k - 3), k + 1)):
        return "chain"
    return None


def check(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    rep.refusals += tr.refusals
    rep.plans_with_refusals += tr.refusals > 0
    for k in range(len(tr.y)):
        y, r, m = tr.y[k], tr.r[k], tr.m[k]
        rep.chain_ticks += "C" in tr.tag[k]
        rep.clamp_ticks += "K" in tr.tag[k]
        # Promise 2: never above M, never below 0.
        if y > m + 1e-9 or y < -1e-9:
            rep.fail(f"range: y={y:.6f} outside [0, {m:.2f}] at k={k} in {name}")
        # Promise 1: rate, curvature and jerk of the sent values, except where listed.
        v, a, j = sampled(tr, k)
        for what, val, lim in (("rate", v, t.r_up if v > 0 else t.r_dn), ("curvature", a, t.a_max), ("jerk", j, t.j_max)):
            if abs(val) > lim * (1 + TOL) + TOL:
                why = exempt(tr, k, what)
                if why is None:
                    rep.fail(f"limit broken outside the listed cases: {what} {val:.4f} > {lim:g}, tag {tr.tag[k]!r} at k={k} in {name}")
                else:
                    rep.exempt[why] += 1
    check_n_errors(tr, rep, name)
    check_landing(tr, t, rep, name)


def check_n_errors(tr: Trace, rep: Report, name: str) -> None:
    """Promise 4's mechanism, from the tags: a refusal is answered from the trajectory in force for
    at most N_ERRORS consecutive ticks, then by the clamp chain."""
    run_e = 0
    for k, tag in enumerate(tr.tag):
        run_e = run_e + 1 if "E" in tag else 0
        if run_e > N_ERRORS:
            rep.fail(f"N_ERRORS exceeded: the trajectory in force for {run_e} ticks at k={k} in {name}")


def double_s_time(delta: float, t: Tuning, up: bool) -> float:
    """The time-optimal rest-to-rest duration under (rate, a_max, j_max) (Biagiotti and Melchiorri
    2008, §3.4): the landing time Ruckig achieves on a held line from rest."""
    d, v, a, j = abs(delta), (t.r_up if up else t.r_dn), t.a_max, t.j_max
    if v * j >= a * a:
        ta = v / a + a / j       # time to reach v
    else:
        ta = 2 * math.sqrt(v / j)
    if d >= v * ta:              # reaches the rate
        return d / v + ta
    # does not reach the rate: peak velocity vp with d = vp·ta(vp)
    lo, hi = 0.0, v
    for _ in range(100):
        vp = (lo + hi) / 2
        tap = vp / a + a / j if vp * j >= a * a else 2 * math.sqrt(vp / j)
        if vp * tap < d:
            lo = vp
        else:
            hi = vp
    vp = (lo + hi) / 2
    return 2 * (vp / a + a / j if vp * j >= a * a else 2 * math.sqrt(vp / j))


def chain_bound(t: Tuning, delta: float) -> float:
    """The landing time of the clamp chain on a held line `delta` away, from rest (promise 4 under a
    persistent Ruckig error): Δ/r + τ·(1 + 2·ln(max(1, Δ/ε))). The closing rate brakes at a_max/2,
    and in the linear zone the gap shrinks by e every 2τ. Measured margin on the grid: ≥ τ − 0.02 s."""
    r = t.r_up if delta > 0 else t.r_dn
    return abs(delta) / r + t.ease * (1 + 2 * math.log(max(1.0, abs(delta) / EPS)))


def landing_bound(t: Tuning, delta: float, v0: float, a0: float) -> float:
    """Promise 4 from any state: the chain's bound over the gap plus the distance of stopping the
    state first (unwinding the curvature, then the slope), plus that stop's time and N_ERRORS + 1
    ticks (a refusal answered from the trajectory in force before the chain takes over)."""
    v_eff = abs(v0) + a0 * a0 / (2 * t.j_max)
    t_stop = abs(a0) / t.j_max + v_eff / t.a_max + t.a_max / t.j_max if (v0 or a0) else 0.0
    d = abs(delta) + v_eff * t_stop
    return chain_bound(t, math.copysign(d, delta)) + t_stop + (N_ERRORS + 1) * DT


def check_landing(tr: Trace, t: Tuning, rep: Report, name: str) -> None:
    """Promise 4 on every plan: wherever a held piece starts (a step, a ramp's end, an event, an
    engagement), the command is within ε of it within landing_bound, if the piece and the plan last
    that long with no further event. A command stuck or ringing off a held line fails here."""
    n = len(tr.y)
    for k in range(1, n):
        if tr.s[k] != 0.0 or (tr.s[k - 1] == 0.0 and not tr.jump[k] and not tr.events[k] and not tr.reset[k]):
            continue
        val = tr.r[k]
        y0, v0, a0 = (tr.y[k - 1], tr.v[k - 1], tr.a[k - 1]) if not tr.reset[k] else (tr.y[k], 0.0, 0.0)
        if abs(val - y0) <= EPS and abs(v0) < 1e-9:
            continue
        bound = landing_bound(t, val - y0, v0, a0)
        m = int(math.ceil(bound / DT))
        end = k + m
        if end >= n or any(tr.s[j] != 0.0 or abs(tr.r[j] - val) > 1e-9 or tr.events[j] for j in range(k + 1, end + 1)):
            continue
        if not any(abs(tr.y[j] - val) <= EPS for j in range(k, end + 1)):
            rep.fail(f"landing late: not on the held line {val:.2f} W within {bound:.2f} s"
                     f" (from {y0:.2f} W, slope {v0:.2f} W/s, curvature {a0:.2f}) at k={k} in {name}")


def ramp_error(tr: Trace, t: Tuning) -> tuple[float, float]:
    """Promise 5 (measured): on every ramp piece within the rates, once the command is established
    (within ε of the line with the line's slope within 1e-6 W/s), the worst error until the piece
    ends (LOOKAHEAD ticks before its end, where the target sees the next piece); and the longest
    time to establish, in s."""
    worst_err, worst_est = 0.0, 0.0
    k, n = 0, len(tr.y)
    while k < n:
        s = tr.s[k]
        j = k
        while j + 1 < n and tr.s[j + 1] == s and not tr.jump[j + 1] and not tr.events[j + 1] and not tr.reset[j + 1]:
            j += 1
        if s != 0.0 and -t.r_dn < s < t.r_up and "K" not in "".join(tr.tag[k:j + 1]):
            last = j - LOOKAHEAD
            est = next((i for i in range(k, last + 1) if abs(tr.y[i] - tr.r[i]) <= EPS and abs(tr.v[i] - s) <= 1e-6), None)
            if est is not None:
                worst_est = max(worst_est, (est - k) * DT)
                worst_err = max([worst_err] + [abs(tr.y[i] - tr.r[i]) for i in range(est, last + 1)])
        k = j + 1
    return worst_err, worst_est


def land_time(tr: Trace, after: int = 0) -> float | None:
    """The first tick (in s from `after`) from which the command stays within EPS of the line."""
    last_off = None
    for k in range(after, len(tr.y)):
        if abs(tr.y[k] - tr.r[k]) > EPS:
            last_off = k
    if last_off is None:
        return 0.0
    if last_off == len(tr.y) - 1:
        return None
    return (last_off + 1 - after) * DT


# ---------------------------------------------------------------- sweeps (§7.10.6)

def step_plans(t: Tuning):
    """Steps onto held lines, both directions: small, Δ = rise·ease (Ruckig's #269 boundary) and large."""
    for d in (1.0, 5.0, 20.0, t.rise * t.ease, 2 * t.rise * t.ease, 100.0, 300.0):
        for a, b in ((300.0, 300.0 + d), (300.0 + d, 300.0)):
            yield f"step {a:g}->{b:g}", [steady(a, 5), steady(b, double_s_time(d, t, b > a) + 2 * N_ERRORS * DT + 15)], {}


def slope_pair_plans(t: Tuning):
    """Every slope pair at a vertex: steady, ramp s1 for 10 s, ramp s2 for 5 s, steady; slopes 0,
    fractions of the rates, the rates, and beyond them."""
    slopes = sorted(set([0.0] + [f * t.r_up for f in (0.1, 0.5, 0.74, 0.99, 1.0, 1.5)] +
                        [-f * t.r_dn for f in (0.1, 0.5, 0.74, 0.99, 1.0, 1.5)]))
    for s1 in slopes:
        for s2 in slopes:
            w0 = 1500.0
            w1, w2 = w0 + s1 * 10, w0 + s1 * 10 + s2 * 5
            if min(w1, w2) < 0:
                continue
            yield f"pair {s1:g}->{s2:g}", [steady(w0, 5), ramp(w0, w1, 10), ramp(w1, w2, 5), steady(w2, 15)], {"m0": 4000.0}


def cap_plans(t: Tuning):
    """The hard clamp: ramps into the cap and into 0 W, a cap lowered below the command and raised,
    a block above the cap, and clamp-then-drop."""
    M = 250.0
    for s in (0.5 * t.r_up, t.r_up, 3 * t.r_up):
        yield f"ramp into the cap {s:g}", [steady(100, 5), ramp(100, 100 + s * 20, 20), steady(100 + s * 20, 15)], {"m0": M}
    for s in (0.5 * t.r_dn, t.r_dn):
        yield f"ramp into 0 W {-s:g}", [steady(150, 5), ramp(150, 150 - s * 20, 20), steady(0, 15)], {}
    yield "cap lowered below the command", [steady(400, 20), steady(400, 20)], {"events": [Event(30, "cap", 250.0)]}
    yield "cap raised", [steady(400, 30)], {"m0": 250.0, "events": [Event(20, "cap", 600.0)]}
    for hi in (300.0, 400.0):
        for low in (0.0, 150.0):
            yield f"block {hi:g} above the cap, drop to {low:g}", [steady(100, 10), steady(hi, 4), ramp(hi, low, 0.25), steady(low, 30)], {"m0": M}


def jump_plans(t: Tuning):
    """Skips, offsets and engagements mid-ramp and mid-step, and plain jumps between ramps."""
    for kind, val in (("skip", 0.0), ("offset", 1.1), ("offset", 0.9), ("engage", 50.0), ("engage", 500.0)):
        for at in (25, 45, 70):
            yield f"{kind} {val:g} at {at}", [steady(200, 5), ramp(200, 200 + 0.7 * t.r_up * 10, 10), steady(400, 10), ramp(400, 300, 10), steady(300, 20)], \
                {"events": [Event(at, kind, val)]}
    yield "jump onto a rising ramp", [steady(100, 5), ramp(300, 300 + 0.5 * t.r_up * 20, 20), steady(300 + 0.5 * t.r_up * 20, 15)], {}
    yield "jump onto a falling ramp", [steady(500, 5), ramp(300, 300 - 0.5 * t.r_dn * 5, 5), steady(300 - 0.5 * t.r_dn * 5, 15)], {}


def above_plan(tr: Trace, t: Tuning) -> tuple[float, int]:
    """The worst rise of the command above every value the plan line had since the command was last
    at or below it, where nothing forces it (as in excess()). Lag behind a falling line never
    exceeds that; only motion away from the plan does (promise 3, the second figure)."""
    forced, worst, at, hi = True, 0.0, -1, -math.inf
    for k in range(len(tr.y)):
        if tr.reset[k] or (k > 0 and (tr.r[k] < tr.r[k - 1] - t.r_dn * DT - 1e-6
                                      or (tr.jump[k] and tr.r[k] < tr.r[k - 1] + tr.s[k - 1] * DT - 1e-6))):
            forced = True
        elif tr.y[k] <= tr.r[k] + 1e-6:
            forced, hi = False, tr.r[k]
        hi = max(hi, tr.r[k])
        if not forced and tr.y[k] - hi > worst:
            worst, at = tr.y[k] - hi, k
    return worst, at


FAMILIES = {"steps": step_plans, "slope pairs": slope_pair_plans, "cap/clamp": cap_plans, "skip/jump": jump_plans}


def excess(tr: Trace, t: Tuning) -> tuple[float, int]:
    """The worst excess of the command above the line where nothing forces it (promise 3): not
    after the line jumped down (a step down, a lowered cap, an offset down) or fell faster than r↓
    in one tick (a ramp steeper than r↓), or an engagement, until the command first reaches the line."""
    forced, worst, at = True, 0.0, -1
    for k in range(len(tr.y)):
        if tr.reset[k] or (k > 0 and (tr.r[k] < tr.r[k - 1] - t.r_dn * DT - 1e-6
                                      or (tr.jump[k] and tr.r[k] < tr.r[k - 1] + tr.s[k - 1] * DT - 1e-6))):
            forced = True
        elif tr.y[k] <= tr.r[k] + 1e-6:
            forced = False
        if not forced and tr.y[k] - tr.r[k] > worst:
            worst, at = tr.y[k] - tr.r[k], k
    return worst, at


def run_plan(t: Tuning, bl, kw, **extra) -> Trace:
    kw = dict(kw)
    ev = kw.pop("events", [])
    kw.update(extra)
    return run(bl, ev, t=t, **kw)


def families(rep: Report, report: bool, tunings) -> None:
    """The measured sweeps of promise 3 (and every promise checked on every tick of them)."""
    for fam, gen in FAMILIES.items():
        for t in tunings:
            for name, bl, kw in gen(t):
                tr = run_plan(t, bl, kw)
                check(tr, t, rep, f"{fam}: {name} {t}")
                o, k = excess(tr, t)
                rep.worst((fam, t), o, f"{name} (tick {k})")
                o, k = above_plan(tr, t)
                rep.worst((fam + " above", t), o, f"{name} (tick {k})")
    if report:
        print("Promise 3, the worst excess above the line where nothing forces it (W), defaults | worst tuning;"
              " then '<family> above': the worst rise above every line value since the command was last at or below the line:")
        for fam in list(FAMILIES) + [f + " above" for f in FAMILIES]:
            d = rep.excess.get((fam, DEFAULT), (0.0, "-"))
            w = max(((rep.excess[(fam, t)][0], t, rep.excess[(fam, t)][1]) for t in tunings), key=lambda x: x[0])
            print(f"  {fam}: {d[0]:.1f} ({d[1]}) | {w[0]:.1f} at ({w[1].rise}, ×{w[1].factor:g}, {w[1].ease:g}) ({w[2]})")


def landing(rep: Report, report: bool, tunings) -> None:
    """Promise 4 from rest: Ruckig lands on a held line in the time-optimal double-S time (plus a
    tick's rounding); the clamp chain within chain_bound, under a persistent error from tick 0."""
    worst_r, worst_c = (-1e9, None), (-1e9, None)
    for t in tunings:
        for d in (0.6, 1.0, 5.0, 20.0, t.rise * t.ease, 2 * t.rise * t.ease, 100.0, 300.0, 600.0):
            for up in (True, False):
                a, b = (300.0, 300.0 + d) if up else (300.0 + d, 300.0)
                T = double_s_time(d, t, up)
                plan = [steady(a, 5), steady(b, T * 6 + 100)]
                tr = run(plan, t=t)
                check(tr, t, rep, f"landing {a:g}->{b:g} {t}")
                lt = land_time(tr, 20)
                refused = tr.refusals > 0
                slack = (N_ERRORS + 2) * DT if refused else DT
                if lt is None or lt > T + slack:
                    rep.fail(f"landing {a:g}->{b:g} {t}: Ruckig lands in {lt} s > double-S {T:.2f} + {slack:g} s")
                elif lt - T > worst_r[0]:
                    worst_r = (lt - T, (t, a, b, refused))
                tr = run(plan, t=t, inject=1.0)
                check(tr, t, rep, f"landing under a persistent error {a:g}->{b:g} {t}")
                lt, cb = land_time(tr, 20), chain_bound(t, b - a)
                if lt is None or lt > cb:
                    rep.fail(f"landing under a persistent error {a:g}->{b:g} {t}: {lt} s > chain bound {cb:.2f} s")
                elif lt - cb > worst_c[0]:
                    worst_c = (lt - cb, (t, a, b))
    if report:
        print(f"Promise 4 from rest: Ruckig lands within the double-S time {worst_r[0]:+.2f} s at worst {worst_r[1]};"
              f" the clamp chain within its bound {worst_c[0]:+.2f} s at worst {worst_c[1]}")


def ramps(rep: Report, report: bool, tunings) -> None:
    """Promise 5: ramps within the rates, entered from a held line, a jump and a ramp, both ways."""
    worst_e, worst_t = (0.0, None), (0.0, None)
    never = total = 0
    by_f: dict[str, tuple[int, int]] = {}
    for t in tunings:
        for f in (0.1, 0.5, 0.9, 0.99):
            for sgn in (1, -1):
                s = sgn * f * (t.r_up if sgn > 0 else t.r_dn)
                w = 500.0 + abs(s) * 80     # high enough that no ramp touches 0 W, and M above all of it
                big = w + abs(s) * 80 + 1000.0
                for name, plan in (("from held", [steady(w, 5), ramp(w, w + s * 60, 60), steady(w + s * 60, 10)]),
                                   ("from a jump", [steady(w - 300, 5), ramp(w, w + s * 60, 60), steady(w + s * 60, 10)]),
                                   ("from a ramp", [steady(w, 5), ramp(w, w - s * 10, 10), ramp(w - s * 10, w - s * 10 + s * 60, 60), steady(w + s * 50, 10)])):
                    tr = run(plan, t=t, m0=big)
                    check(tr, t, rep, f"ramp {s:g} {name} {t}")
                    e, est = ramp_error(tr, t)
                    total += 1
                    nv = e == 0.0 and est == 0.0 and abs(tr.y[-41] - tr.r[-41]) > EPS
                    never += nv
                    key = f"{'+' if sgn > 0 else '-'}{f:g}"
                    by_f[key] = (by_f.get(key, (0, 0))[0] + nv, by_f.get(key, (0, 0))[1] + 1)
                    if e > RAMP_TOL:
                        rep.fail(f"ramp {s:g} {name} {t}: tracking error {e:.3g} W > {RAMP_TOL:g} once established")
                    if e > worst_e[0]:
                        worst_e = (e, (t, s, name))
                    if est > worst_t[0]:
                        worst_t = (est, (t, s, name))
    if report:
        print(f"Promise 5: once established, the ramp tracking error is at most {worst_e[0]:.3g} W {worst_e[1]};"
              f" establishing takes at most {worst_t[0]:.2f} s {worst_t[1]}; {never} of {total} 60 s ramps are never"
              f" established before they end, by slope as a share of the rate: "
              + ", ".join(f"{k} {v[0]}/{v[1]}" for k, v in sorted(by_f.items())))


RAMP_TOL = 0.25    # W: promise 5's bound once a ramp is established (measured, §7.10.6)

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


def pinned(rep: Report, report: bool) -> None:
    """The falsifiers' repros (f625b, f625c, f625d), held to the new promises, and the mechanism
    pins: N_ERRORS ticks from the trajectory in force, then the chain; no refusal on a ramp steeper
    than the rate (the target slope is clamped)."""
    for name, t, bl, kw in REPROS:
        tr = run_plan(t, bl, kw)
        check(tr, t, rep, f"pinned: {name}")
    # The trajectory in force for N_ERRORS ticks, then the chain, then Ruckig on its first success.
    tr = run([steady(100, 5), steady(300, 30)], t=DEFAULT, inject=1.0, inject_from=25)
    want = [""] * 25 + ["E"] * N_ERRORS + ["C"] * 5
    if [g.rstrip("o") for g in tr.tag[:len(want)]] != want:
        rep.fail(f"pinned: refusals from tick 25: tags {tr.tag[20:35]} != {want[20:35]}")
    for t in [DEFAULT] + CORNERS:
        s = 3 * t.r_up
        tr = run([steady(100, 5), ramp(100, 100 + s * 10, 10), steady(100 + s * 10, 30)], t=t, m0=4000.0)
        check(tr, t, rep, f"pinned: a ramp at 3× the rate {t}")
        if tr.refusals:
            rep.fail(f"pinned: a ramp at 3× the rate {t}: {tr.refusals} Ruckig refusals (target slope not clamped?)")
    if report:
        print(f"Pinned: {len(REPROS)} falsifier repros (f625b, f625c, f625d) hold the promises; the refusal path and the"
              f" clamped target slope are pinned")


def single_refusal(rep: Report) -> None:
    """One refusal is one tick of the trajectory in force, and Ruckig is used again on the next tick."""
    ref = run([steady(100, 5), steady(300, 30)], t=DEFAULT)
    tr = run([steady(100, 5), steady(300, 30)], t=DEFAULT, refuse={25})
    if tr.tag[25] != "E" or tr.tag[26] != "" or abs(tr.y[25] - ref.y[25]) > 1e-9:
        rep.fail(f"pinned: a single refusal at tick 25: tags {tr.tag[24:28]}, y {tr.y[25]} vs {ref.y[25]}")


def constants(rep: Report, report: bool) -> None:
    """The stated constants, the limits handed to Ruckig (a rest-to-rest plan takes exactly the
    double-S time), and the clamp chain's order (rate and curvature always in band)."""
    for name, val, want in (("N_ERRORS", N_ERRORS, 3), ("LOOKAHEAD", LOOKAHEAD, 2), ("EPS", EPS, 0.5), ("DT", DT, 0.25)):
        if val != want:
            rep.fail(f"constants: {name} = {val}, stated {want}")
    for t in [DEFAULT] + CORNERS:
        for d in (37.0, 300.0):
            for up in (True, False):
                sl = Slew(t)
                traj = sl.plan(State(300.0), 300.0 + (d if up else -d), 0.0, 0)
                if traj is None or abs(traj.duration - double_s_time(d, t, up)) > 1e-6:
                    rep.fail(f"constants: rest-to-rest {d:g} W {'up' if up else 'down'} {t}: Ruckig"
                             f" {traj and traj.duration} s != double-S {double_s_time(d, t, up):.6f} s (the limits)")
    rng = random.Random(147)
    for _ in range(20000):
        t = rng.choice(GRID)
        v0 = rng.uniform(-t.r_dn, t.r_up)
        v1 = v0 - rng.uniform(-t.a_max, t.a_max) * DT
        v = rng.uniform(-3 * t.r_dn, 3 * t.r_up)
        out, _ = speed_limiter(v, v0, v1, DT, t)
        if not (-t.r_dn - 1e-9 <= out <= t.r_up + 1e-9) or abs(out - v0) > t.a_max * DT + 1e-9:
            rep.fail(f"constants: speed_limiter({v}, {v0}, {v1}) = {out} {t}: rate or curvature out of band (order)")
            break
    # The reference order on any input (the port must match the function, not only its reachable
    # domain): within |v0 − v1| ≤ a_max·dt, jerk-then-acceleration equals acceleration-then-jerk,
    # so these pins use histories outside it.
    for args, want in (((100.0, 19.0, 14.0), 20.0), ((100.0, 10.0, 2.0), 15.0)):
        got = speed_limiter(*args, DT, DEFAULT)[0]
        if got != want:
            rep.fail(f"constants: speed_limiter{args} at the defaults = {got} != {want} (jerk, then acceleration, then rate)")


def persistent_random(rep: Report, report: bool, n: int = 300) -> None:
    """Every Ruckig calculation failing from a random tick, on random plans (the falsifier's q4v7
    scheme: seeds 31000 + i, generator v7)."""
    chain = 0
    for i in range(n):
        rng = random.Random(31000 + i)
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        total = sum(b.ticks for b in blocks)
        k0 = rng.randrange(max(1, total))
        tr = run(blocks, events, t=t, m0=m0, inject=1.0, inject_from=k0)
        check(tr, t, rep, f"persistent#{i} from {k0} {t} {blocks} {events} m0={m0}")
        chain += sum("C" in g for g in tr.tag)
    if report:
        print(f"Persistent Ruckig error from a random tick: {n} random plans (generator v7), {chain} chain ticks; every promise holds")


def injected(rep: Report, report: bool, n: int = 300, share: float = 0.2) -> None:
    rng = random.Random(626)
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        tr = run(blocks, events, t=t, inject=share, inject_seed=i)
        check(tr, t, rep, f"injected#{i} {t} {blocks} {events}")
    if report:
        print(f"Fault injection: {n} random plans with {share:.0%} of Ruckig calculations failing; every promise holds")


GOLDEN_PLANS = {
    "defaults step 100->300": (DEFAULT, [steady(100, 5), steady(300, 20)], {}),
    "defaults step 300->100": (DEFAULT, [steady(300, 5), steady(100, 20)], {}),
    "defaults ramp 100->400 over 60 s": (DEFAULT, [steady(100, 5), ramp(100, 400, 60), steady(400, 10)], {}),
    "defaults fall vertex -4 -> -40": (DEFAULT, [steady(1500, 5), ramp(1500, 1460, 10), ramp(1460, 1260, 5), steady(1260, 15)], {"m0": 4000.0}),
    "(50, x4, 4) fall -148 W/s": (Tuning(50, 4.0, 4.0), [steady(1500, 5), ramp(1500, 20, 10), steady(20, 20)], {"m0": 4000.0}),
    "defaults ramp into the cap 250": (DEFAULT, [steady(100, 5), ramp(100, 500, 20), steady(500, 10)], {"m0": 250.0}),
    "defaults cap lowered to 250 under 400": (DEFAULT, [steady(400, 40)], {"events": [Event(30, "cap", 250.0)]}),
    "defaults engagement at 50 W on a rise": (DEFAULT, [steady(200, 5), ramp(200, 340, 10), steady(340, 15)], {"events": [Event(25, "engage", 50.0)]}),
    "defaults persistent error from a step": (DEFAULT, [steady(100, 5), steady(300, 40)], {"inject": 1.0, "inject_from": 25}),
    "(5, x4, 2) persistent error, 10 W fall": (Tuning(5, 4.0, 2.0), [steady(310, 5), steady(300, 60)], {"inject": 1.0}),
    "bare #269 input 41.25->0": (Tuning(15, 1.5, 2.75), [steady(41.25, 5), steady(0, 30)], {}),
    "defaults 30/30 100<->400": (DEFAULT, [steady(100, 30), steady(400, 30)] * 2, {}),
}

GOLDEN = {
    'defaults step 100->300': (21500.0, 0, 0),
    'defaults step 300->100': (16900.015255, 0, 0),
    'defaults ramp 100->400 over 60 s': (77811.857864, 0, 0),
    'defaults fall vertex -4 -> -40': (196047.700484, 0, 0),
    '(50, x4, 4) fall -148 W/s': (129515.981979, 0, 0),
    'defaults ramp into the cap 250': (28040.84704, 0, 0),
    'defaults cap lowered to 250 under 400': (44500.0, 0, 1),
    'defaults engagement at 50 W on a rise': (27862.447917, 0, 0),
    'defaults persistent error from a step': (44932.227882, 152, 0),
    '(5, x4, 2) persistent error, 10 W fall': (78350.149536, 260, 0),
    'bare #269 input 41.25->0': (1329.216053, 1, 1),
    'defaults 30/30 100<->400': (106350.025923, 0, 0),
}


def golden(rep: Report, report: bool, emit: bool = False) -> None:
    """Golden traces for #620's port: Σ y, chain ticks and clamp ticks per plan, pinned."""
    out = {}
    for name, (t, bl, kw) in GOLDEN_PLANS.items():
        tr = run_plan(t, bl, kw)
        check(tr, t, rep, f"golden: {name}")
        out[name] = (round(sum(tr.y), 6), sum("C" in g for g in tr.tag), sum("K" in g for g in tr.tag))
    if emit:
        print("GOLDEN = {")
        for name, val in out.items():
            print(f"    {name!r}: {val},")
        print("}")
        return
    for name, val in out.items():
        want = GOLDEN.get(name)
        if want is None or abs(want[0] - val[0]) > 1e-6 * max(1.0, abs(want[0])) or want[1:] != val[1:]:
            rep.fail(f"golden: {name}: {val} != pinned {want}")
    if report:
        print(f"Golden traces: {len(out)} plans pinned (Σ y, chain ticks, clamp ticks)")


def random_plan_v6(rng, t):
    """Round 6's generator: grid-exact steps (Δ = rise·ease and its multiples, levels on the 0.25 W
    grid), ramps split into blocks of one slope with float noise, ± offsets, caps, skips, engagements."""
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
    """Round 7's generator: v6's mix, plus clamp-then-drop (a block far above any cap, then a fall
    within one tick) and ramps that start at exactly 0 W."""
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


def randoms(rep: Report, n: int, report: bool, seed: int = 585) -> None:
    rng = random.Random(seed)
    r0, p0, c0 = rep.refusals, rep.plans_with_refusals, rep.chain_ticks
    ticks = 0
    worst = (0.0, None)
    worst_def = (0.0, None)
    above, above_def = (0.0, None), (0.0, None)
    for i in range(n):
        t = rng.choice(GRID)
        blocks, events = random_plan_v7(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        tr = run(blocks, events, t=t, origin=origin, m0=rng.choice([2000.0, rng.uniform(200, 800)]))
        if tr.y:
            check(tr, t, rep, f"random#{i} {t} {blocks} {events} origin {origin}")
            ticks += len(tr.y)
            o, k = excess(tr, t)
            if o > worst[0]:
                worst = (o, (i, t, k))
            if t == DEFAULT and o > worst_def[0]:
                worst_def = (o, (i, k))
            o, k = above_plan(tr, t)
            if o > above[0]:
                above = (o, (i, t, k))
            if t == DEFAULT and o > above_def[0]:
                above_def = (o, (i, k))
    if report:
        print(f"Random plans, seed {seed}: {n} plans (generator v7), {ticks} ticks, every promise checked on every tick;"
              f" Ruckig refusals {rep.refusals - r0} in {rep.plans_with_refusals - p0} plans, chain ticks"
              f" {rep.chain_ticks - c0}; worst excess {worst[0]:.1f} W {worst[1]}, at the defaults {worst_def[0]:.1f} W {worst_def[1]};"
              f" worst above the plan {above[0]:.1f} W {above[1]}, at the defaults {above_def[0]:.1f} W {above_def[1]}")


def lookahead_table(tunings) -> None:
    """The look-ahead choice (§7.10.2): promise 3's figures and ramp establishment per look-ahead."""
    for look in (0, 1, 2, 3, 4, 6):
        row = []
        for fam, gen in FAMILIES.items():
            d = max(excess(run_plan(DEFAULT, bl, kw, look=look), DEFAULT)[0] for _, bl, kw in gen(DEFAULT))
            w = max(max(excess(run_plan(t, bl, kw, look=look), t)[0] for _, bl, kw in gen(t)) for t in tunings)
            row.append(f"{fam} {d:.1f} | {w:.1f}")
        est = []
        for s in (0.5 * DEFAULT.r_up, -0.5 * DEFAULT.r_dn):
            tr = run([steady(1500, 5), ramp(1500, 1500 + s * 60, 60), steady(1500 + s * 60, 10)], t=DEFAULT, m0=4000.0, look=look)
            e, et = ramp_error(tr, DEFAULT)
            est.append(f"{et:.2f} s" if (e or et or abs(tr.y[100] - tr.r[100]) <= EPS) else "never")
        print(f"  look-ahead {look}: " + "; ".join(row) + f"; a 60 s ramp at ±half the rate established after {', '.join(est)} at the defaults")


def main(argv):
    report = "--report" in argv
    n = int(argv[argv.index("--random") + 1]) if "--random" in argv else 2000
    seeds = ([int(x) for x in argv[argv.index("--seeds") + 1].split(",")] if "--seeds" in argv else [585])
    tunings = GRID if "--grid" in argv else [DEFAULT] + CORNERS
    rep = Report()
    if "--golden" in argv:
        golden(rep, report, emit=True)
        return 0
    if "--lookahead" in argv:
        lookahead_table(tunings)
        return 0
    constants(rep, report)
    single_refusal(rep)
    pinned(rep, report)
    families(rep, report, tunings)
    landing(rep, report, tunings)
    ramps(rep, report, tunings)
    persistent_random(rep, report)
    injected(rep, report)
    golden(rep, report)
    for seed in seeds:
        randoms(rep, n, report, seed)
    if report:
        print(f"Promise 1's listed exceptions (ticks whose sampled rate, curvature or jerk broke a limit): "
              + ", ".join(f"{k} {v}" for k, v in rep.exempt.items()) + f"; clamp ticks {rep.clamp_ticks},"
              f" chain ticks {rep.chain_ticks}")
    if rep.fails:
        print("FAIL", rep.nfail, "failures by kind:", rep.kinds)
        for f in rep.fails:
            print("  " + f[:400])
        return 1
    print("erg_slew (ruckig in the loop, track 2): all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
