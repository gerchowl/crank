#!/usr/bin/env python3
"""Executable model of the eased ERG target, ADR-0002 §7.10 (#585).

Stdlib only. It implements the plan-line sampling of §7.10.2 and the limiter's steps 1-7
literally, iterates them over named scenarios and random plans, and checks the properties
P1-P6 of §7.10.6 and the numbers the ADR quotes (duration table, ramp table, short-interval
figures, engagement's first second, sampling table). The Rust implementation (#620) is the
authority for the product; this model is the design's own check, and #620's golden vectors
must agree with it.

Run: python3 docs/adr/models/erg_slew.py           (exit 0 = all checks pass)
     python3 docs/adr/models/erg_slew.py --report  (also prints every table the ADR quotes)
CI runs it as the `slew-model` flake check, `just slew-model` locally.
"""
from __future__ import annotations

import math
import random
import sys
from dataclasses import dataclass, field

DT = 0.25      # one engine tick, s
EPS = 0.5      # W: "on the line" is |R - y| <= EPS (half the 1 W resolution)
SETTLE_AFTER_TICKS = 20   # 5 s of settle after landing following a discontinuity or engagement
FLOAT = 1e-9   # floating-point tolerance of the checks, not a design tolerance


@dataclass(frozen=True)
class Tuning:
    """`Tuning.slew` (ADR-0001 §7.2): integer rise, factor and ease on their grids."""
    rise: int = 20          # W/s
    factor: float = 2.0     # fall = rise * factor
    ease: float = 2.0       # s

    @property
    def r_up(self) -> float:
        return float(self.rise)

    @property
    def r_dn(self) -> float:
        return self.rise * self.factor

    def j(self, up: bool) -> float:
        return (self.r_up if up else self.r_dn) / self.ease

    def valid(self) -> bool:
        return (5 <= self.rise <= 50 and 1 <= self.factor <= 4 and (self.factor * 4) % 1 == 0
                and 1 <= self.ease <= 4 and (self.ease * 4) % 1 == 0 and self.r_dn >= 20)


DEFAULT = Tuning()
CORNERS = [Tuning(5, 4.0, 4.0), Tuning(50, 1.0, 1.0), Tuning(50, 4.0, 1.0), Tuning(50, 1.0, 4.0),
           Tuning(5, 4.0, 1.0), Tuning(20, 1.0, 4.0), Tuning(50, 4.0, 4.0)]


# ---------------------------------------------------------------- the limiter (§7.10.2)

def n_of(v: float, t: Tuning) -> float:
    """Normalised slope: v / r_up rising, v / r_dn falling; P2 is |n| <= 1, P3 |dn| <= dt/ease."""
    return v / t.r_up if v >= 0 else v / t.r_dn


def v_of(n: float, t: Tuning) -> float:
    return n * t.r_up if n >= 0 else n * t.r_dn


def within_rates(s: float, t: Tuning) -> bool:
    return -t.r_dn - FLOAT <= s <= t.r_up + FLOAT


@dataclass
class Limiter:
    t: Tuning
    y: float = 0.0
    v: float = 0.0
    tracking: bool = False   # the previous tick ended on the line, slope-compatible (step 3)
    exc: str = ""            # the named exception the last tick used, if any (P3)

    def engage(self, origin: float, m: float) -> None:
        """§7.10.3: start from (origin, 0)."""
        self.y, self.v, self.tracking, self.exc = min(max(origin, 0.0), m), 0.0, False, ""

    def step(self, r: float, s: float, disc: bool, m: float) -> None:
        """One tick: plan line `r`, its slope `s` (0 on a discontinuity tick), cap `m`."""
        t = self.t
        self.exc = ""
        # 1. Ceiling: the cap is never eased (exception X3).
        if self.y > m:
            self.y, self.tracking, self.exc = m, False, "ceiling"
            if self.v > 0:
                self.v = 0.0
        # 2. Slope band: the normalised slope moves by at most dt/ease (P3) inside [-1, 1] (P2).
        n = n_of(self.v, t)
        lo, hi = v_of(max(-1.0, n - DT / t.ease), t), v_of(min(1.0, n + DT / t.ease), t)
        # 3. On the line: follow it, its own kinks included (exception X1).
        if self.tracking and not disc and within_rates(s, t):
            v = min(max(s, -t.r_dn), t.r_up)
            if not lo - FLOAT <= v <= hi + FLOAT:
                self.exc = "kink"
        else:
            # 4. Gap and direction. g = 0 means: move with the line (sigma = 0, b = 0).
            g = r - self.y - s * DT
            sigma = 0 if g == 0 else (1 if g > 0 else -1)
            # 5. Braking slope towards the line.
            b = 0.0
            if sigma:
                j = t.j(sigma > 0)
                b = j * (math.sqrt(DT * DT + 2 * abs(g) / j) - DT)
            # 6. Desired slope, never away from the line: above it, never rising; below it, falling
            # only with a line it can follow. Then the slope band (P2, P3), unconditionally.
            want = s + sigma * b
            if sigma < 0:
                want = min(want, 0.0)
            elif sigma > 0:
                want = max(want, min(0.0, s) if s >= -t.r_dn else 0.0)
            v = min(max(want, lo), hi)
            # 7. Never rise to above the line (exception X2); never fall below 0 W (exception X4).
            if v > 0 and self.y + v * DT > r:
                cut = max(0.0, (r - self.y) / DT)
                if cut < lo - FLOAT:
                    self.exc = "rise-cut"
                v = min(v, cut)
        # 8. In both modes: never above M, never below 0 W (exceptions X3 and X4).
        if self.y + v * DT > m:
            v = (m - self.y) / DT
            if v < lo - FLOAT:
                self.exc = "ceiling"
        if self.y + v * DT < 0:
            v = -self.y / DT
            if v > hi + FLOAT:
                self.exc = "floor"
        self.v = v
        self.y += v * DT
        # Landing: on the line with a slope the next tick can follow within the band.
        on = abs(r - self.y) <= EPS
        self.tracking = (on and within_rates(s, t)
                         and abs(n_of(self.v, t) - n_of(min(max(s, -t.r_dn), t.r_up), t)) <= DT / t.ease + FLOAT)


# ---------------------------------------------------------------- the plan line (§7.10.2 sampling)

@dataclass
class Block:
    ticks: int
    frm: float
    to: float

    @property
    def slope(self) -> float:
        return (self.to - self.frm) / (self.ticks * DT) if self.ticks else 0.0


@dataclass
class Event:
    tick: int
    kind: str          # "offset" (value = multiplier), "cap" (value = M), "skip", "engage" (value = origin)
    value: float = 0.0


def line(blocks: list[Block], events: list[Event], m0: float = 2000.0, sampling: str = "start"):
    """Per tick (R, s, disc, M, block_start).

    sampling "start" (§7.10.2): R_k = P(t_k) at the tick's start, s_k = (R_k - R_{k-1})/dt when the
    plan is continuous at t_k, 0 on a discontinuity tick. "round2": dev's frac = elapsed/dur with
    s = 0 on a block's first tick. "end": the last tick of a ramp carries its end value.
    """
    ev = {}
    for e in events:
        ev.setdefault(e.tick, []).append(e)
    mult, m = 1.0, m0
    bi, k_in, k = 0, 0, 0
    prev_r = None
    prev_end = None
    while bi < len(blocks):
        b = blocks[bi]
        if b.ticks == 0:
            bi += 1
            continue
        disc = False
        start = k_in == 0
        for e in ev.get(k, []):
            if e.kind == "offset":
                mult, disc = e.value, True
            elif e.kind == "cap":
                m = e.value
            elif e.kind == "skip" and not start:
                bi, k_in = bi + 1, 0
                while bi < len(blocks) and blocks[bi].ticks == 0:
                    bi += 1
                if bi >= len(blocks):
                    return
                b, start = blocks[bi], True
        if sampling == "end" and b.ticks > 1:
            p = b.frm + (b.to - b.frm) * k_in / (b.ticks - 1)
        else:
            p = b.frm + (b.to - b.frm) * k_in / b.ticks
        r = min(max(p * mult, 0.0), m)
        if start and prev_end is not None and abs(b.frm - prev_end) > 1e-12:
            disc = True
        if prev_r is None or disc:
            s = 0.0
        elif sampling == "round2" and start:
            s = 0.0
        elif sampling == "round2":
            s = b.slope * mult if 0 < r < m else 0.0
        else:
            s = (r - prev_r) / DT
        yield k, r, s, disc, m, start
        prev_r, prev_end = r, b.to
        k += 1
        k_in += 1
        if k_in >= b.ticks:
            bi, k_in = bi + 1, 0


@dataclass
class Trace:
    r: list = field(default_factory=list)
    y: list = field(default_factory=list)
    v: list = field(default_factory=list)
    s: list = field(default_factory=list)
    m: list = field(default_factory=list)
    on: list = field(default_factory=list)
    settle: list = field(default_factory=list)
    exc: list = field(default_factory=list)
    disc: list = field(default_factory=list)
    eng: list = field(default_factory=list)   # the origin on an engagement tick, else None


def run(blocks, events=(), t: Tuning = DEFAULT, origin: float | None = None, y0=None, v0=0.0,
        sampling="start", m0=2000.0) -> Trace:
    """Iterate the limiter over a plan. Starts engaged at `origin`, or on the line (y0 None)."""
    lim = Limiter(t)
    tr = Trace()
    settle_left = 0
    pending = False
    events = list(events)
    eng = {e.tick: e.value for e in events if e.kind == "engage"}
    first = True
    for k, r, s, disc, m, _start in line(blocks, events, m0, sampling):
        if first:
            if origin is not None:
                lim.engage(origin, m)
                pending = True
            else:
                lim.y, lim.v, lim.tracking = (r if y0 is None else y0), v0, y0 is None
            first = False
        engaged = None
        if k in eng:
            lim.engage(eng[k], m)
            pending = True
            engaged = lim.y
        lim.step(r, s, disc, m)
        on = abs(r - lim.y) <= EPS
        if disc:
            pending = True
        if not on:
            settle_left = 0
        elif pending:
            pending, settle_left = False, SETTLE_AFTER_TICKS
        st = (not on) or settle_left > 0 or pending
        if on and settle_left > 0:
            settle_left -= 1
        for lst, val in ((tr.r, r), (tr.y, lim.y), (tr.v, lim.v), (tr.s, s), (tr.m, m), (tr.on, on),
                         (tr.settle, st), (tr.exc, lim.exc), (tr.disc, disc), (tr.eng, engaged)):
            lst.append(val)
    return tr


# ---------------------------------------------------------------- property checks (§7.10.6)

class Violations:
    def __init__(self):
        self.items: list[str] = []
        self.worst = {"kink": 0.0, "rise-cut": 0.0, "floor": 0.0, "dip": 0.0}

    def add(self, msg):
        if len(self.items) < 20:
            self.items.append(msg)


def check_trace(tr: Trace, t: Tuning, vio: Violations, name: str, y_start: float) -> None:
    prev_y, prev_v, prev_r, prev_s = y_start, None, None, None
    retreat = 0.0
    for k in range(len(tr.r)):
        y, v, r, m, exc, s = tr.y[k], tr.v[k], tr.r[k], tr.m[k], tr.exc[k], tr.s[k]
        if tr.eng[k] is not None:   # an engagement restarts the limiter at its origin (§7.10.3)
            prev_y, prev_v, retreat = tr.eng[k], 0.0, 0.0
        if any(map(math.isnan, (y, v))) or any(map(math.isinf, (y, v))):
            vio.add(f"{name} k={k}: NaN/inf")
        # P2 rate, on the real motion of y (the ceiling tick is a position cut, X3).
        motion = (y - prev_y) / DT
        if exc != "ceiling" and not -t.r_dn - FLOAT <= motion <= t.r_up + FLOAT:
            vio.add(f"{name} k={k}: P2 slope {motion:.3f}")
        if abs(motion - v) > 1e-6 and exc != "ceiling":
            vio.add(f"{name} k={k}: v is not the motion")
        # P3 ease, except the named exceptions.
        if prev_v is not None:
            dn = abs(n_of(v, t) - n_of(prev_v, t))
            if dn > DT / t.ease + FLOAT:
                if exc in ("kink", "rise-cut", "floor"):
                    vio.worst[exc] = max(vio.worst[exc], abs(v - prev_v))
                    if exc == "kink" and abs(v - prev_v) > abs(s - prev_s) + t.j(False) * DT + FLOAT:
                        vio.add(f"{name} k={k}: X1 larger than the line's kink")
                    if exc == "rise-cut" and not (prev_v > 0 and 0 <= v < prev_v):
                        vio.add(f"{name} k={k}: X2 is not a cut of a rise")
                elif exc != "ceiling":
                    vio.add(f"{name} k={k}: P3 |dn| {dn:.4f} > {DT / t.ease:.4f} (v {prev_v:.3f} -> {v:.3f})")
        # P1/P5: a rising command never ends more than EPS above the line (on the line it may sit
        # up to EPS above it, the landing residual); off the line above it, it never rises.
        if v > FLOAT and y > r + EPS + FLOAT:
            vio.add(f"{name} k={k}: P5 rises to above the line (y {y:.3f} R {r:.3f} v {v:.3f})")
        # P6 excess grows only when the line falls; y <= M; y >= 0.
        if prev_r is not None:
            e0, e1 = max(0.0, prev_y - prev_r), max(0.0, y - r)
            if e1 > e0 + FLOAT and r >= prev_r - FLOAT:
                vio.add(f"{name} k={k}: P6 excess grew without a falling line")
        if y > m + FLOAT or y < -FLOAT:
            vio.add(f"{name} k={k}: y {y:.3f} outside [0, M={m}]")
        # Retreat (allowed, below the plan): below the line, falling faster than the line it can
        # follow, only while the slope eases out of a fall it was already in.
        if y < r - EPS and v < min(0.0, max(s, -t.r_dn)) - FLOAT:
            retreat += (min(0.0, max(s, -t.r_dn)) - v) * DT
            vio.worst["dip"] = max(vio.worst["dip"], retreat)
            bound = t.r_dn * t.ease / 2 + t.r_dn * DT
            if retreat > bound + FLOAT:
                vio.add(f"{name} k={k}: retreat below the line {retreat:.2f} W > {bound:.2f}")
        else:
            retreat = 0.0
        prev_y, prev_v, prev_r, prev_s = y, v, r, s


def d_cont(delta: float, r: float, tau: float) -> float:
    delta = abs(delta)
    return delta / r + tau if delta >= r * tau else 2 * math.sqrt(delta * tau / r)


def land_time(tr: Trace, after: int = 0) -> float | None:
    """Seconds from tick `after` until the first tick from which the command stays on the line."""
    for k in range(len(tr.on) - 1, -1, -1):
        if not tr.on[k]:
            return (k + 1 - after) * DT if k + 1 < len(tr.on) else None
    return 0.0


def steady(w: float, secs: float) -> Block:
    return Block(int(round(secs / DT)), w, w)


def ramp(a: float, b: float, secs: float) -> Block:
    return Block(int(round(secs / DT)), a, b)


def excluded_seconds(tr: Trace, lo: int, hi: int) -> int:
    """Seconds (4 ticks) in [lo, hi) with a settle tick (compliance.rs: one excludes its second)."""
    return sum(1 for sec in range(lo // 4, hi // 4) if any(tr.settle[sec * 4:(sec + 1) * 4]))


# ---------------------------------------------------------------- checks and tables

def tunings_grid():
    for rise in (5, 8, 10, 15, 20, 30, 40, 50):
        for factor in (1.0, 1.25, 1.5, 2.0, 3.0, 4.0):
            for ease in (1.0, 1.5, 2.0, 2.75, 3.0, 4.0):
                t = Tuning(rise, factor, ease)
                if t.valid():
                    yield t


def check_steep_and_g0(vio, report):
    """Round-3 blocker 1: g = 0 on a steep ramp entered from the line must not pass the rate."""
    rows = []
    for name, blocks, t in (
        ("100->400 W over 10 s, rise 20", [steady(100, 5), ramp(100, 400, 10), steady(400, 20)], DEFAULT),
        ("400->100 W over 5 s, fall 40", [steady(400, 5), ramp(400, 100, 5), steady(100, 20)], DEFAULT),
    ):
        tr = run(blocks, t=t)
        check_trace(tr, t, vio, name, tr.r[0])
        rows.append((name, max(tr.v), min(tr.v), land_time(tr, 20)))
    if report:
        print("\nSteep ramps entered from the line (defaults):")
        for name, vmax, vmin, lt in rows:
            print(f"  {name}: slope {vmin:.2f} .. {vmax:.2f} W/s, on the line {lt:.2f} s after the ramp's start")


def check_first_second(vio, report):
    """Round-3 blocker 2: the first second of an engagement or a steep-ramp entry adds <= 0.625 r/tau."""
    worst = {}
    for t in [DEFAULT, Tuning(50, 1.0, 1.0)] + CORNERS:
        bound = 10 * t.j(True) * DT * DT
        cases = {
            "engage 0 -> 400 W": run([steady(400, 30)], t=t, origin=0.0),
            "engage behind a steep ramp": run([ramp(100, 500, 4), steady(500, 30)], t=t, origin=100.0),
            "steep ramp from the line": run([steady(100, 2), ramp(100, 500, 4), steady(500, 30)], t=t),
        }
        for name, tr in cases.items():
            k0 = 8 if name.startswith("steep") else 0
            y_before = tr.y[k0 - 1] if k0 else (100.0 if "behind" in name else 0.0)
            gain = tr.y[k0 + 3] - y_before
            if gain > bound + FLOAT:
                vio.add(f"first second {name} {t}: {gain:.3f} > {bound:.3f}")
            worst[(t, name)] = gain
            check_trace(tr, t, vio, f"{name} {t}", y_before)
    if report:
        print("\nFirst second of an engagement / steep-ramp entry (bound 10*j*dt^2 = 0.625 r/tau):")
        for t in (DEFAULT, Tuning(50, 1.0, 1.0)):
            g = max(v for (tt, _), v in worst.items() if tt == t)
            print(f"  rise {t.rise} ease {t.ease}: {g:.2f} W (bound {0.625 * t.r_up / t.ease:.2f} W)")


def check_durations(vio, report):
    rows = []
    for (a, b) in ((150, 170), (137, 220), (100, 250), (100, 400)):
        up = run([steady(a, 5), steady(b, 60)])
        dn = run([steady(b, 5), steady(a, 60)])
        rows.append((f"{b - a} W ({a} <-> {b})", land_time(up, 20), land_time(dn, 20)))
    e137 = land_time(run([steady(137, 60)], origin=0.0))
    e250 = land_time(run([steady(250, 60)], origin=0.0))
    for name, lu, ld in rows:
        delta = float(name.split(" ")[0])
        for lt, r in ((lu, DEFAULT.r_up), (ld, DEFAULT.r_dn)):
            if lt is None or lt > d_cont(delta, r, DEFAULT.ease) + 0.75 + FLOAT:
                vio.add(f"duration {name}: {lt} > d + 0.75")
    owner = run([steady(100, 10), ramp(100, 180, 60), steady(220, 60)])
    owner_land = land_time(owner, 280)
    if report:
        print("\nDuration table (defaults), on the line after:")
        for name, lu, ld in rows:
            print(f"  {name}: rising {lu:.2f} s, falling {ld:.2f} s")
        print(f"  engagement 0 -> 137 W: {e137:.2f} s, 0 -> 250 W: {e250:.2f} s")
        print(f"  owner's case, ramp 100 -> 180 W over 60 s into 220 W: lands {owner_land:.2f} s after the boundary")
    return rows


def check_ramps(vio, report):
    """Ramps within the rates, entered from an equal target: zero off-line and zero excluded seconds."""
    rows = []
    for (a, b, secs) in ((100, 400, 60), (50, 150, 30), (300, 100, 60), (100, 400, 120), (100, 400, 15),
                         (400, 100, 7.5), (100, 300, 10)):
        for t in [DEFAULT] + CORNERS:
            if not within_rates((b - a) / secs, t):
                continue
            blocks = [steady(a, 10), ramp(a, b, secs), steady(b, 10)]
            tr = run(blocks, t=t)
            check_trace(tr, t, vio, f"ramp {a}->{b}/{secs} {t}", a)
            off = sum(1 for o in tr.on if not o)
            if off:
                vio.add(f"ramp {a}->{b}/{secs}s {t}: {off} off-line ticks")
            if t == DEFAULT:
                rows.append((f"{a} -> {b} W over {secs:g} s", off, excluded_seconds(tr, 40, 40 + int(secs * 4))))
    # Round 3: the three samplings on a ramp at exactly the rise rate.
    samp = {}
    for mode in ("round2", "end", "start"):
        blocks = [steady(100, 10), ramp(100, 500, 20), steady(500, 10)]
        tr = run(blocks, sampling=mode)
        samp[mode] = (excluded_seconds(tr, 0, len(tr.r)), sum(1 for o in tr.on if not o),
                      tr.s[41] if len(tr.s) > 41 else None, min(tr.v[118:124]))
    if samp["start"][0] or samp["start"][1]:
        vio.add(f"sampling 'start' excludes {samp['start'][0]} s on an exact-rate ramp")
    if report:
        print("\nRamps within the rates (defaults), entered from an equal target:")
        for name, off, ex in rows:
            print(f"  {name}: {off} off-line ticks, {ex} excluded seconds")
        print("\nSampling of a ramp at exactly 20 W/s (100 -> 500 W over 20 s, defaults):")
        for mode, (ex, off, s1, vmin) in samp.items():
            print(f"  {mode}: {ex} excluded s, {off} off-line ticks, line slope {s1:.4f} W/s,"
                  f" min slope near the end {vmin:.2f} W/s")


def check_short(vio, report):
    out = {}
    for t, tag in ((DEFAULT, "defaults"), (Tuning(50, 4.0, 1.0), "fastest")):
        for name, on_s in (("30/30", 30), ("15/15", 15)):
            reps = [b for _ in range(6) for b in (steady(100, on_s), steady(400, on_s))]
            tr = run(reps + [steady(100, 30)], t=t)
            check_trace(tr, t, vio, f"{name} {tag}", 100)
            n = int(on_s / DT)
            lo, hi = 9 * n, 10 * n   # the 5th rep's hard block
            seg = tr.y[lo:hi]
            out[(tag, name)] = (max(seg), sum(seg) / len(seg))
        tr = run([steady(100, 10), steady(600, 5), steady(100, 20)], t=t)
        check_trace(tr, t, vio, f"5 s at 600 {tag}", 100)
        out[(tag, "5 s at 600 W")] = (max(tr.y), None)
    if report:
        print("\nShort intervals (100 <-> 400 W, the 5th rep's hard block):")
        for (tag, name), (pk, mean) in out.items():
            print(f"  {tag} {name}: peak {pk:.0f} W" + (f", mean {mean:.0f} W" if mean else ""))


def first_at_or_below(tr: Trace) -> float | None:
    for k in range(len(tr.y)):
        if tr.y[k] <= tr.r[k] + EPS:
            return (k + 1) * DT
    return None


def brakeable(gap: float, v0: float, j: float) -> bool:
    """A slope towards the line that braking at j still stops within the gap."""
    return abs(v0) * DT + v0 * v0 / (2 * j) <= gap


def check_held(vio, report):
    """P4: onto a held line from rest, or moving towards it with a slope it can brake within the
    gap: on the line within d + 0.75 s. Claim 2: after a downward jump, from any slope, the command
    is at or below the line within d(Δ; r_dn, τ) + 0.75 s <= Δ/20 + 4.75 s."""
    rng = random.Random(585)
    worst, worst_above = 0.0, 0.0
    for _ in range(30000):
        t = rng.choice(GRID)
        up = rng.random() < 0.5
        delta = rng.uniform(1, 600)
        a = rng.uniform(0, 600) if up else rng.uniform(delta, 1200)
        b = a + delta if up else a - delta
        r = t.r_up if up else t.r_dn
        v0 = rng.uniform(-t.r_dn, t.r_up) if rng.random() < 0.6 else 0.0
        tr = run([steady(b, d_cont(delta, r, t.ease) + 60)], t=t, y0=a, v0=v0)
        check_trace(tr, t, vio, f"held {a:.1f}->{b:.1f} v0 {v0:.1f} {t}", a)
        lt = land_time(tr)
        if lt is None:
            vio.add(f"held {a}->{b} {t}: never lands")
            continue
        towards = v0 == 0 or ((v0 > 0) == up and brakeable(delta, v0, t.j(up)))
        if towards:
            exc = lt - d_cont(delta, r, t.ease)
            worst = max(worst, exc)
            if exc > 0.75 + FLOAT:
                vio.add(f"P4 held {a:.1f}->{b:.1f} v0 {v0:.1f} {t}: {lt:.2f} s > d + 0.75")
        if not up:
            above = first_at_or_below(tr)
            bound = d_cont(delta, t.r_dn, t.ease) + 0.75
            worst_above = max(worst_above, above - d_cont(delta, t.r_dn, t.ease))
            if above > bound + FLOAT or above > delta / 20 + 4.75 + FLOAT:
                vio.add(f"claim 2 {delta:.1f} W v0 {v0:.1f} {t}: above the line {above:.2f} s")
    if report:
        print(f"\nP4 over 30 000 held-line cases: largest excess over d(Δ; r, τ) {worst:.2f} s (bound 0.75 s);"
              f" time above the line after a downward jump, any initial slope: largest excess"
              f" {worst_above:.2f} s over d(Δ; r_dn, τ)")


def random_plan(rng: random.Random, t: Tuning):
    blocks = []
    w = rng.uniform(0, 500)
    for _ in range(rng.randint(2, 8)):
        secs = rng.choice([0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120])
        kind = rng.random()
        if kind < 0.4:
            nw = rng.uniform(0, 600)
            blocks.append(steady(nw, secs))
            w = nw
        else:
            start = w if rng.random() < 0.6 else rng.uniform(0, 600)
            if kind < 0.7:   # within the rates
                slope = rng.uniform(-t.r_dn, t.r_up)
            else:            # steeper than the rates
                slope = rng.choice([-1, 1]) * rng.uniform(t.r_up, 4 * t.r_dn)
            end = max(0.0, start + slope * secs)
            blocks.append(ramp(start, end, secs))
            w = end
    total = sum(b.ticks for b in blocks)
    events = []
    for _ in range(rng.randint(0, 4)):
        k = rng.randrange(max(1, total))
        kind = rng.choice(["offset", "cap", "skip", "engage"])
        val = {"offset": rng.choice([0.9, 0.95, 1.0, 1.05, 1.1, 1.3]), "cap": rng.uniform(100, 700),
               "skip": 0.0, "engage": rng.uniform(0, 400)}[kind]
        events.append(Event(k, kind, val))
    return blocks, events


def check_random(vio, report):
    rng = random.Random(625)
    n = 0
    for _ in range(4000):
        t = rng.choice(GRID)
        blocks, events = random_plan(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        tr = run(blocks, events, t=t, origin=origin, m0=rng.choice([2000.0, rng.uniform(200, 800)]))
        if not tr.r:
            continue
        y_start = tr.y[0] - tr.v[0] * DT if tr.exc[0] != "ceiling" else tr.y[0]
        check_trace(tr, t, vio, f"random#{n} {t}", y_start)
        # Ramps within the rates that start on the line stay on it (zero off-line ticks).
        n += 1
    if report:
        print(f"\nRandom plans: {n} plans of 2-8 blocks with steps, ramps within and beyond the rates,"
              f" offsets, caps, skips and engagements over the tuning grid")
        print("  largest slope change on a named exception: " +
              ", ".join(f"{k} {v:.1f} W/s" for k, v in vio.worst.items() if k != "dip"))
        print(f"  largest retreat below the line: {vio.worst['dip']:.1f} W"
              f" (bound r_dn·τ/2 + r_dn·Δt)")


def check_quoted(vio):
    """The numbers ADR-0002 §7.10 quotes, recomputed (a change to the limiter must amend the ADR)."""
    got = {}
    for a, b in ((150, 170), (137, 220), (100, 250), (100, 400)):
        got[f"rise {a}->{b}"] = land_time(run([steady(a, 5), steady(b, 60)]), 20)
        got[f"fall {b}->{a}"] = land_time(run([steady(b, 5), steady(a, 60)]), 20)
    got["engage 137"] = land_time(run([steady(137, 60)], origin=0.0))
    got["engage 250"] = land_time(run([steady(250, 60)], origin=0.0))
    got["owner"] = land_time(run([steady(100, 10), ramp(100, 180, 60), steady(220, 60)]), 280)
    got["steep up"] = land_time(run([steady(100, 5), ramp(100, 400, 10), steady(400, 20)]), 20)
    got["steep down"] = land_time(run([steady(400, 5), ramp(400, 100, 5), steady(100, 20)]), 20)
    got["jump-entered ramp excl"] = excluded_seconds(run([steady(100, 10), ramp(150, 400, 60), steady(400, 10)]), 40, 280)
    got["300 rise excl"] = excluded_seconds(run([steady(100, 10), steady(400, 60)]), 40, 280)
    tr = run([ramp(100, 500, 10), steady(500, 30)], origin=100.0)
    got["engage steep full rate"] = (next(i for i, v in enumerate(tr.v) if v >= 20 - FLOAT) + 1) * DT
    got["engage steep lands"] = land_time(tr)
    for t, tag in ((DEFAULT, "d"), (Tuning(50, 4.0, 1.0), "f")):
        for name, on_s in (("30/30", 30), ("15/15", 15)):
            reps = [b for _ in range(6) for b in (steady(100, on_s), steady(400, on_s))]
            seg = run(reps + [steady(100, 30)], t=t).y[9 * int(on_s / DT):10 * int(on_s / DT)]
            got[f"{tag} {name} peak"], got[f"{tag} {name} mean"] = round(max(seg)), round(sum(seg) / len(seg))
        got[f"{tag} 5s peak"] = round(max(run([steady(100, 10), steady(600, 5), steady(100, 20)], t=t).y))
    want = {"rise 150->170": 2.5, "fall 170->150": 1.75, "rise 137->220": 5.75, "fall 220->137": 3.75,
            "rise 100->250": 9.25, "fall 250->100": 5.5, "rise 100->400": 16.75, "fall 400->100": 9.25,
            "engage 137": 8.5, "engage 250": 14.25, "owner": 3.5, "steep up": 17.0, "steep down": 9.5,
            "jump-entered ramp excl": 10, "300 rise excl": 22, "engage steep full rate": 2.25,
            "engage steep lands": 22.0, "d 30/30 mean": 318, "d 15/15 peak": 383, "d 15/15 mean": 236,
            "d 5s peak": 182, "f 30/30 mean": 367, "f 15/15 mean": 335, "f 5s peak": 331}
    for k, w in want.items():
        if got[k] is None or abs(got[k] - w) > 1e-9:
            vio.add(f"quoted number {k}: model {got[k]}, ADR {w}")


GRID = list(tunings_grid())


def main(argv):
    report = "--report" in argv
    vio = Violations()
    check_steep_and_g0(vio, report)
    check_first_second(vio, report)
    check_durations(vio, report)
    check_ramps(vio, report)
    check_short(vio, report)
    check_held(vio, report)
    check_random(vio, report)
    check_quoted(vio)
    if vio.items:
        print("FAIL")
        for i in vio.items:
            print("  " + i)
        return 1
    print("erg_slew: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
