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
    """The clamped plan line from tick k on, as pieces: each block's line, split where the clamp
    to [0, M] starts or ends (a clamped piece is flat at 0 or M)."""
    out, k0, prev_end = [], 0, None
    for b in blocks:
        if b.ticks > 0 and k0 + b.ticks > k:
            raw = Seg(k0, b.frm * mult, b.slope * mult, b.ticks)
            joined = prev_end is not None and abs(b.frm * mult - prev_end) < 1e-9
            kk = k0
            while kk < k0 + b.ticks:
                v = raw.at(kk)
                state = 0 if v <= 0 else (2 if v >= m else 1)
                end = kk + 1
                while end < k0 + b.ticks:
                    w = raw.at(end)
                    if (0 if w <= 0 else (2 if w >= m else 1)) != state:
                        break
                    end += 1
                piece = (Seg(kk, raw.at(kk), raw.s, end - kk, joined) if state == 1
                         else Seg(kk, 0.0 if state == 0 else m, 0.0, end - kk, joined))
                if end <= k:
                    pass
                else:
                    out.append(piece)
                joined, kk = True, end
        if b.ticks > 0:
            prev_end = b.to * mult
        k0 += b.ticks
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


@dataclass(frozen=True)
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

    def __init__(self, t: Tuning, inject: float = 0.0, seed: int = 0):
        self.t = t
        # Fault injection (the suite's own test of X2-X5): this share of Ruckig calculations is
        # made to fail, deterministically, as if Ruckig had returned an error.
        self.inject = inject
        self.inject_rng = random.Random(seed)
        self.wasteful = 0          # calculations on an unchanged input (step 7 says: none)
        self.fail_brake = False    # fault injection: the braking plan of step 6 fails
        self.had_traj = False      # a trajectory was in force at the start of the last tick
        self.otg = Ruckig(1)
        self.inp = InputParameter(1)
        self.traj = Trajectory(1)
        self.spare = Trajectory(1)
        self.probe = Trajectory(1)
        self.errors = 0
        self.prev_piece = None
        self.x5_frame = None
        self.key = None
        self.elapsed = 0.0
        self.base = 0.0
        self.recalcs = 0
        self.chasing = False
        self.braking = False

    def _input(self, st: State, rel: float, fs: float, vmax: float, vmin: float, vel_target):
        t, i = self.t, self.inp
        i.control_interface = ControlInterface.Velocity if vel_target is not None else ControlInterface.Position
        i.current_position, i.current_velocity, i.current_acceleration = [rel], [st.v - fs], [st.a]
        i.target_position = [0.0]
        i.target_velocity = [0.0 if vel_target is None else vel_target - fs]
        i.target_acceleration = [0.0]
        i.max_velocity, i.min_velocity = [vmax - fs], [vmin - fs]
        i.max_acceleration, i.min_acceleration, i.max_jerk = [t.a_max], [-t.a_max], [t.j_max]
        return i

    def _calc(self, traj, st, rel, fs, vmax, vmin, vel_target) -> bool:
        """Ruckig's calculate; False on any error result or exception (counted, never raised)."""
        try:
            if self.inject and self.inject_rng.random() < self.inject:
                raise RuckigError("injected")
            res = self.otg.calculate(self._input(st, rel, fs, vmax, vmin, vel_target), traj)
        except RuckigError:
            res = None
        if res not in (Result.Working, Result.Finished):
            self.errors += 1
            return False
        return True

    def _plan(self, st, rel, fs, vmax, vmin, vel_target) -> bool:
        """Calculate into the spare trajectory and swap it in only on success, so a failed
        calculation leaves the trajectory in force untouched (X5)."""
        if not self._calc(self.spare, st, rel, fs, vmax, vmin, vel_target):
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
        """True if the braking trajectory from `st` (the state at tick k) stays at or below the
        plan line known ahead, tick by tick, until it is at rest."""
        t = self.t
        if not self._calc(self.probe, st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0):
            return False
        n = int(math.ceil(self.probe.duration / DT))
        for i in range(1, n + 1):
            tt = min(i * DT, self.probe.duration)
            y = st.y + self.probe.at_time(tt)[0][0]
            # The line may be passed by at most EPS/2, strictly inside X2's EPS, so a braking path
            # that is accepted can never be cut; the cap not at all.
            if y > min(line_at(segs, k + i, m) + EPS / 2, m):
                return False
        return True

    def _continues(self, st: State, kk: int) -> bool:
        """True if the trajectory in force reproduces `st` at tick kk: it can then be continued for
        a tick (X5). Not after a cut or an engagement changed the state under it."""
        if self.key is None or self.x5_frame is None:
            return False
        vel_mode, fl, base = self.x5_frame
        p, v, a = self._sample(vel_mode)
        y = base + p if vel_mode else fl(kk) + p
        v = v if vel_mode else v + fl.fs
        return abs(y - st.y) < 1e-6 and abs(v - st.v) < 1e-5 and abs(a - st.a) < 1e-4

    def _floor_ok(self, st: State) -> bool:
        if not self._calc(self.probe, st, 0.0, 0.0, self.t.r_up, -self.t.r_dn, 0.0):
            return False
        p, v, a = self.probe.at_time(self.probe.duration)
        return st.y + min(p[0], self.probe.position_extrema[0].min) >= -1e-9

    def step(self, st: State, k: int, segs: list[Seg], m: float, cut_before: bool) -> tuple[State, str]:
        """The command for tick k from the state at tick k - 1."""
        t = self.t
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
            lead_key = ("lead", cur.k0, s_cur, nxt.k0, nxt.r0, nxt.s)
            # Lead at a vertex that ends or reverses a rise, or ends a fall without starting a rise;
            # never at one that starts or steepens a rise (that would raise the load before the plan).
            # Only from at or below the current line: above it, aiming at a higher next piece would raise it.
            leads = (nxt.joined and trackable(nxt.s) and (nxt.s < s_cur or (s_cur < 0 and s_cur < nxt.s <= 0))
                     and st.y <= r_k - s_cur * DT + EPS)   # the state is at tick k-1: compare with the line there
            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))
            if self.key is not None and self.key[0] == lead_key and within(s_cur):
                fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                vmax, vmin = cap_v()
            elif leads and self._calc(self.probe, st, 0.0, 0.0, t.r_up, -t.r_dn, nxt.s):
                if (nxt.k0 - (k - 1)) * DT <= self.probe.duration / 2 + DT:
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
            vel_target = t.r_up if (gap > 0 and fs > 0) else (-t.r_dn if (gap < 0 and fs < 0) else 0.0)
        key = ((("brake",) if brake else
                ("lead", cur.k0, s_cur, segs[1].k0, segs[1].r0, segs[1].s) if lead
                else ("line", round(c, 9), round(fs, 12))),
               vmax, vmin, vel_target, m)
        x5 = False
        unchanged = key == self.key and not exc and not cut_before
        self.had_traj = self._continues(st, k - 1)
        if key != self.key or exc or cut_before:
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
            else:
                x5 = True
        else:
            self.elapsed += DT
        self.chasing = vel_target is not None
        if x5:
            # X5: Ruckig refused the new input. The trajectory in force (already within every limit)
            # is continued for this tick, and the re-plan is tried again on the next. Without one
            # (or after a cut changed the state under it) the command holds.
            if self._continues(st, k - 1):
                self.elapsed += DT
                vel_mode, fl, base = self.x5_frame
                p, v, a = self._sample(vel_mode)
                nst = State(base + p, v, a) if vel_mode else State(fl(k) + p, v + fl.fs, a)
                exc = exc or "X5"
            else:
                nst, exc = State(st.y, 0.0, 0.0), "X5h"
                self.key = None
        if not x5:
            p, v, a = self._sample(vel_target is not None)
            if vel_target is None:
                nst = State(line(k) + p, v + fs, a)
                fl = _Line(c, fs)
                self.x5_frame = (False, fl, 0.0)
            else:
                nst = State(self.base + p, v, a)
                self.x5_frame = (True, None, self.base)
        # Safe braking: a step after which the command rises or will rise (v > 0, or a braking fall
        # whose acceleration will carry it past rest) is taken only if Ruckig's braking trajectory from it (to rest,
        # within the same limits) stays at or below the plan line known ahead. Otherwise the command
        # eases out from the current state and keeps easing out until it is at rest.
        # The same for a fall towards 0 W: a falling step is taken only if its braking trajectory
        # stays at or above 0 W, nothing being below it.
        will_rise = nst.v > 1e-9 or (nst.a > 1e-9 and nst.v + nst.a * nst.a / (2 * t.j_max) > 1e-9)
        if (not brake and ((will_rise and not self._brake_ok(nst, k, segs, m))
                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):
            if self.fail_brake:
                self.errors += 1
            if not self.fail_brake and self._plan(st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0):
                self.braking = True
                self.base, self.elapsed = st.y, DT
                self.key = (("brake",), t.r_up, -t.r_dn, 0.0, m)
                self.chasing = True
                p, v, a = self._sample(True)
                nst = State(self.base + p, v, a)
                self.x5_frame = (True, None, self.base)
            else:
                exc = exc or "X5"   # the step stands; X2 below still bounds it
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
    errors: int = 0
    y_init: float = 0.0      # the state before tick 0 (at rest unless v0 was given)
    v_init: float = 0.0


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, m0=2000.0,
        y0: float | None = None, v0: float = 0.0, inject: float = 0.0, inject_seed: int = 0,
        fail_brake: bool = False) -> Trace:
    blocks = [Block(b.ticks, b.frm, b.to) for b in blocks if b.ticks > 0]
    ev = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    ez = Easer(t, inject, inject_seed)
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
        st, exc = ez.step(st, k, segs, m, reset or disc)
        if ez.errors > errors_before:
            last_error_k = k
            error_pending = True     # open until the command is next on the line
        on = abs(r - st.y) <= EPS
        unforeseen_now = unforeseen
        error_now = error_pending
        landed = on and abs(st.v - segs[0].s) <= 1e-6 and abs(st.a) <= 1e-5   # on the line, moving with it
        if landed and not ev.get(k):
            unforeseen = False
            error_pending = ez.errors > errors_before
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
                         (tr.after_error, error_now), (tr.had_traj, ez.had_traj)):
            lst.append(val)
        prev_r = r
        prev_s = segs[0].s     # the backward slope of tick k+1 is the slope of the block in force at k
        k += 1
    tr.errors = ez.errors
    tr.wasteful = ez.wasteful
    return tr


# ---------------------------------------------------------------- checks (§7.10.6)

class Report:
    def __init__(self):
        self.fails: list[str] = []
        self.worst = {"X2": 0.0, "X3": 0.0, "X4": 0.0, "X5h": 0.0}
        self.cuts = {"X2": 0, "X3": 0, "X4": 0, "X5": 0, "X5h": 0}
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
    return {"X2": t.r_up + step, "X3": t.r_up + step, "X4": t.r_dn + step,
            "X5h": max(t.r_up, t.r_dn) + step}


CUTS = ("X2", "X3", "X4", "X5h")


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
        cap_drop = (exc in ("X3", "X5h") and k > 0 and tr.m[k] < tr.m[k - 1] - 1e-12 and not tr.reset[k]
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
            # The cut that explains a broken window: this tick's, or one of the two before it.
            culprit = next((tr.exc[k - d] for d in (0, 1, 2) if k - d >= 0 and n - d >= 1 and cut[n - d]), None)
            if cap_drop:
                # A position step down to the new cap; the slope is kept if it was falling (X3 never
                # stops a fall), so the state's own slope change stays within X3's bound.
                dvs = abs(tr.v[k] - tr.v[k - 1])
                if dvs > bnd[exc] + tol_v:   # X3 keeps a falling slope; an X5h hold on the same tick stops it
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
                # Domains: X2, X3 and X4 only while an unforeseeable event or a Ruckig error (X5) is
                # pending; X5h only where no trajectory was in force.
                if exc in ("X2", "X3", "X4") and not (tr.unforeseen[k] or tr.after_error[k]):
                    rep.fail(f"{name} k={k}: {exc} outside its domain (no unforeseeable event or error pending)")
                if exc == "X5h" and tr.had_traj[k]:
                    rep.fail(f"{name} k={k}: X5h hold although a trajectory was in force")
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


def pinned(rep: Report, report: bool):
    """Regressions from the safety falsifications of PR #625, exact floats."""
    cases = [
        # Round-4 falsifier, blocker 1: Ruckig "error in step 1" at the defaults (seed 7, plan 226).
        ("ruckig error, seed 7 plan 226", DEFAULT,
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
        if name in no_cut and any(e in CUTS for e in tr.exc):
            rep.fail(f"pinned: {name}: a cut {sorted(set(e for e in tr.exc if e in CUTS))} where the design eases")
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
        if report:
            cuts = sorted(set(e for e in tr.exc if e))
            print(f"  pinned {name}: Ruckig errors {tr.errors}, tagged {cuts}, max |dv| "
                  f"{max(abs(tr.v[k] - tr.v[k - 1]) for k in range(1, len(tr.v))):.2f} W/s")


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
        blocks, events = random_plan(rng, t)
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
    corners(rep, report)
    pinned(rep, report)
    ramp_pairs(rep, report)
    injected(rep, report)
    held(rep, report)
    for seed in seeds:
        randoms(rep, n, report, seed)
    if report:
        print("Named exceptions (ticks that break the curvature/jerk band): " +
              ", ".join(f"{k} {rep.cuts[k]} ticks, largest slope change {rep.worst.get(k, 0.0):.1f} W/s"
                        for k in ("X2", "X3", "X4", "X5h")) + f"; X5 continuations {rep.cuts['X5']} ticks")
    if rep.fails:
        print("FAIL", rep.nfail, "failures by kind:", rep.kinds)
        for f in rep.fails:
            print("  " + f[:400])
        return 1
    print("erg_slew (ruckig in the loop): all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
