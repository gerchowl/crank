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

from ruckig import ControlInterface, InputParameter, Result, Ruckig, Trajectory

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

    def limits(self, up: bool) -> tuple[float, float]:
        return self.a_max, self.j_max

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


class Easer:
    """crank's policy around Ruckig: the target frame, the limits, and the cuts X2-X4.

    Ruckig is used as an online trajectory generator is meant to be used: a trajectory is
    calculated when its input changes (a new frame, new limits, a cut, an engagement) and is
    otherwise sampled tick by tick. Re-planning every tick from the trajectory's own sampled
    state is avoided: Ruckig 0.19.4 can fail ("error in step 1") on such a state when it sits
    exactly on a velocity-limit boundary (found by this model, random plan 230)."""

    def __init__(self, t: Tuning):
        self.t = t
        self.otg = Ruckig(1)
        self.inp = InputParameter(1)
        self.traj = Trajectory(1)
        self.probe = Trajectory(1)
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

    def _calc(self, traj, st, rel, fs, vmax, vmin, vel_target):
        res = self.otg.calculate(self._input(st, rel, fs, vmax, vmin, vel_target), traj)
        if res not in (Result.Working, Result.Finished):
            raise RuntimeError(f"ruckig {res}: {st} rel {rel} frame {fs} v [{vmin},{vmax}] {self.t}")

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
        self._calc(self.probe, st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0)
        n = int(math.ceil(self.probe.duration / DT))
        for i in range(1, n + 1):
            tt = min(i * DT, self.probe.duration)
            y = st.y + self.probe.at_time(tt)[0][0]
            if y > line_at(segs, k + i, m) + EPS:
                return False
        return True

    def _floor_ok(self, st: State) -> bool:
        self._calc(self.probe, st, 0.0, 0.0, self.t.r_up, -self.t.r_dn, 0.0)
        p, v, a = self.probe.at_time(self.probe.duration)
        return st.y + min(p[0], self.probe.position_extrema[0].min) >= -1e-9

    def step(self, st: State, k: int, segs: list[Seg], m: float, cut_before: bool) -> tuple[State, str]:
        """The command for tick k from the state at tick k - 1."""
        t = self.t
        exc = ""
        if st.y > m:                                     # X3: the cap is never eased
            st, exc = State(m, min(st.v, 0.0), min(st.a, 0.0)), "X3"
        cur = segs[0]
        r_k = clamp(cur.at(k), m)
        s_cur = cur.s
        within = lambda s: -t.r_dn <= s <= t.r_up
        # Trackable: strictly inside the rates. A line at exactly the rate cannot be caught after an
        # eased start (that would need a slope above the rate), so it is chased like a steeper one.
        trackable = lambda s: -t.r_dn < s < t.r_up
        # X2: above the line, the command never rises: a rising slope is cut to 0, and with it any
        # acceleration that would raise it again. Braking a fall (v < 0, a > 0) is not cut.
        if st.y > r_k + EPS and (st.v > 1e-9 or st.a > 1e-9 and st.v >= 0):
            st, exc = State(st.y, 0.0, min(st.a, 0.0)), exc or "X2"
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
            leads = nxt.joined and trackable(nxt.s) and (nxt.s < s_cur or (s_cur < 0 and s_cur < nxt.s <= 0))
            cap_v = lambda: ((min(t.r_up, s_cur), -t.r_dn) if nxt.s < s_cur else (t.r_up, max(-t.r_dn, s_cur)))
            if self.key is not None and self.key[0] == lead_key and within(s_cur):
                fs, c = nxt.s, nv - nxt.s * nxt.k0 * DT
                vmax, vmin = cap_v()
            elif leads:
                self._calc(self.probe, st, 0.0, 0.0, t.r_up, -t.r_dn, nxt.s)
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
            rel0 = st.y - line(k - 1)
            vel_target = t.r_up if (rel0 < 0 and fs > 0) else (-t.r_dn if (rel0 > 0 and fs < 0) else 0.0)
        key = ((("brake",) if brake else
                ("lead", cur.k0, s_cur, segs[1].k0, segs[1].r0, segs[1].s) if lead
                else ("line", round(c, 9), round(fs, 12))),
               vmax, vmin, vel_target, m)
        if key != self.key or exc or cut_before:
            self.recalcs += 1
            if vel_target is None:
                self._calc(self.traj, st, st.y - line(k - 1), fs, vmax, vmin, None)
            else:
                self._calc(self.traj, st, 0.0, 0.0, t.r_up, -t.r_dn, vel_target)
                self.base = st.y
            self.key, self.elapsed = key, DT
        else:
            self.elapsed += DT
        self.chasing = vel_target is not None
        p, v, a = self._sample(vel_target is not None)
        if vel_target is None:
            nst = State(line(k) + p, v + fs, a)
        else:
            nst = State(self.base + p, v, a)
        # Safe braking: a rising step is taken only if Ruckig's braking trajectory from it (to rest,
        # within the same limits) stays at or below the plan line known ahead. Otherwise the command
        # eases out from the current state and keeps easing out until it is at rest.
        # The same for a fall towards 0 W: a falling step is taken only if its braking trajectory
        # stays at or above 0 W, nothing being below it.
        if (not brake and ((nst.v > 1e-9 and not self._brake_ok(nst, k, segs, m))
                           or (nst.v < -1e-9 and not self._floor_ok(nst)))):
            self.braking = True
            self._calc(self.traj, st, 0.0, 0.0, t.r_up, -t.r_dn, 0.0)
            self.base, self.elapsed = st.y, DT
            self.key = (("brake",), t.r_up, -t.r_dn, 0.0, m)
            self.chasing = True
            p, v, a = self._sample(True)
            nst = State(self.base + p, v, a)
        if self.braking and abs(nst.v) <= 1e-9:
            self.braking = False
            self.key = None if self.traj.duration > self.elapsed + 1e-9 else self.key
        # X2 again: a step that would rise and end above the line stops instead.
        if nst.y > r_k + EPS and (nst.v > 0 or nst.y > st.y):
            nst, exc = State(st.y, 0.0, 0.0), "X2"
        if nst.y < 0:                                    # X4: nothing is below 0 W
            nst, exc = State(0.0, 0.0, 0.0), "X4"
        if nst.y > m:
            nst, exc = State(m, 0.0, 0.0), "X3"
        if exc:
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


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, m0=2000.0,
        y0: float | None = None, v0: float = 0.0) -> Trace:
    blocks = [Block(b.ticks, b.frm, b.to) for b in blocks if b.ticks > 0]
    ev = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    ez = Easer(t)
    mult, m = 1.0, m0
    tr = Trace()
    st = None
    pending = origin is not None
    settle_left = 0
    prev_r, prev_s = None, 0.0
    k = 0
    while k < sum(b.ticks for b in blocks):
        disc = reset = False
        for e in ev.get(k, []):
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
        if any(e.kind == "cap" for e in ev.get(k, [])) and prev_r is not None and r > prev_r + prev_s * DT + 1e-6:
            disc = True    # a raised cap lets the line jump up
        st, exc = ez.step(st, k, segs, m, reset or disc)
        on = abs(r - st.y) <= EPS
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
                         (tr.fell, prev_r is not None and r < prev_r - 1e-12)):
            lst.append(val)
        prev_r = r
        prev_s = segs[0].s     # the backward slope of tick k+1 is the slope of the block in force at k
        k += 1
    return tr


# ---------------------------------------------------------------- checks (§7.10.6)

class Report:
    def __init__(self):
        self.fails: list[str] = []
        self.worst = {"X2": 0.0, "X3": 0.0, "X4": 0.0}
        self.cuts = {"X2": 0, "X3": 0, "X4": 0}
        self.where = {}
        self.retreat = 0.0

    def fail(self, msg):
        if len(self.fails) < 25:
            self.fails.append(msg)


def check(tr: Trace, t: Tuning, rep: Report, name: str, y_start: float) -> None:
    py, pv, pa, pr = y_start, None, None, None
    for k in range(len(tr.y)):
        y, v, a, r, exc = tr.y[k], tr.v[k], tr.a[k], tr.r[k], tr.exc[k]
        if tr.reset[k]:
            pv = pa = pr = None
        if any(math.isnan(x) or math.isinf(x) for x in (y, v, a)):
            rep.fail(f"{name} k={k}: NaN")
            return
        if not -t.r_dn - TOL <= v <= t.r_up + TOL:                       # P2
            rep.fail(f"{name} k={k}: P2 v={v:.4f}")
        if abs(a) > t.a_max + TOL:                                         # P3 acceleration
            rep.fail(f"{name} k={k}: |a|={a:.4f} > {t.a_max}")
        if pv is not None:
            jerk, dv = abs(a - pa) / DT, abs(v - pv)
            broken = jerk > t.j_max + TOL or dv > t.a_max * DT + TOL       # P3 jerk, no corner
            if broken and exc:
                rep.worst[exc] = max(rep.worst[exc], dv)
                rep.cuts[exc] += 1
                rep.where.setdefault(exc, name)
            elif broken:
                rep.fail(f"{name} k={k}: P3 jerk {jerk:.3f} > {t.j_max:.3f} or dv {dv:.3f} > {t.a_max * DT:.3f}"
                         f" (v {pv:.2f}->{v:.2f}, a {pa:.2f}->{a:.2f})")
        if y > r + EPS + TOL and (v > TOL or (k and not tr.reset[k] and y > py + TOL)):   # P5
            rep.fail(f"{name} k={k}: P5 rising above the line y={y:.3f} R={r:.3f} v={v:.3f}")
        if pr is not None:                                                  # P6
            e0, e1 = max(0.0, py - pr - EPS), max(0.0, y - r - EPS)
            if e1 > e0 + TOL and not tr.fell[k]:
                rep.fail(f"{name} k={k}: P6 excess grew without a falling line ({e0:.3f}->{e1:.3f})")
        if y < -TOL:
            rep.fail(f"{name} k={k}: y={y:.3f} < 0")
        py, pv, pa, pr = y, v, a, r


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


def randoms(rep: Report, n: int, report: bool):
    rng = random.Random(585)
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
        print(f"Random plans: {n} plans, {ticks} ticks, every property checked on every tick")


def main(argv):
    report = "--report" in argv
    n = int(argv[argv.index("--random") + 1]) if "--random" in argv else 2000
    rep = Report()
    tables(rep, report)
    corners(rep, report)
    held(rep, report)
    randoms(rep, n, report)
    if report:
        print("Named cuts: " + ", ".join(f"{k} {rep.cuts[k]} ticks, largest slope change {v:.1f} W/s"
                                         for k, v in rep.worst.items()))
    if rep.fails:
        print("FAIL")
        for f in rep.fails:
            print("  " + f[:400])
        return 1
    print("erg_slew (ruckig in the loop): all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
