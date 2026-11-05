#!/usr/bin/env python3
"""Executable model of the route speed model (a), ADR-0005 §2.4 (#523).

Stdlib only. It implements the equations of §2.4 literally and checks them:

* check_constants(): the ADR's parameter table, surface Crr table and integration constants are the
  model's (the model does not read the ADR; this fails when either side is edited alone).
* check_steady_state(): the integrator settles on the analytic steady speed (bisection on the
  power balance) on every grade of the worked examples.
* check_step(): the decided step H = 25 ms is within 0.01 m/s and 0.5 m (per 120 s) of a 1 ms reference.
* check_v_min(): the start-up regulariser V_MIN moves the distance of a 30 s standing start by less
  than 0.5 m from its V_MIN → 0 limit.
* check_wire(): what crank sends (page 51 grade and Crr, page 50 wind coefficient) is on the FE-C grid,
  and the model integrates the decoded wire values, so model (a) and the trainer use the same numbers.
* check_invariants(): speed is never negative, position never decreases, a stop stays a stop on a
  climb (no rolling back) and a paused ride resumes from rest.
* check_relief(): the low-cadence relief of §2.8.1 arms only at or above the arming grade (≥ 2 %), never while
  coasting on a descent or the flat, and ends when the climb ends even at cadence 0.
* worked examples (§2.4.6): flat at 200 W, an 8 % climb at 200 W, a −6 % descent at 0 W, a stop and start, gravel,
  and the relief's arming grade per FTP (§2.8.1).

Run: python3 docs/adr/models/route_speed.py [path/to/0005-routes.md]   (exit 0 = all checks pass;
the ADR path defaults to the sibling file).
"""
from __future__ import annotations

import math
import os
import re
import sys
from dataclasses import dataclass, replace

# --- constants (ADR-0005 §2.4.2) ---
G = 9.80665          # standard gravity, m/s² (3rd CGPM 1901)
RHO = 1.225          # air density, kg/m³ (ISA sea level, 15 °C; ISO 2533:1975)
M_ROT = 1.4          # rotational equivalent mass of both wheels, kg (thin hoop: I/r² = rim + tyre + tube)
ETA_HUB = 1.0        # power measured at the trainer's hub is already past the drivetrain
CDA = 0.40           # drag area, m² (the FE-C default 0.51 kg/m factored as A·Cd = 0.4)
DRAFTING = 1.0       # FE-C drafting factor (1.0 = no drafting)
WIND_MPS = 0.0       # head wind, m/s (route rides: still air)
CRR_DEFAULT = 0.004  # asphalt; the FE-C default and ADR-0002 §7.9
RIDER_KG, BIKE_KG = 75.0, 9.0   # worked examples only; the ride takes them from the profile and equipment

# --- integration (ADR-0005 §2.4.3) ---
TICK = 0.25          # engine tick, s (ADR-0001 §5)
H = 0.025            # integration step, s: 10 sub-steps per tick
V_MIN = 0.5          # m/s: propulsive force is P / max(v, V_MIN)
V_STOP = 0.05        # m/s: below this with no net forward force, the bike is at rest

# --- low-cadence relief on climbs (ADR-0005 §2.8.1) ---
RELIEF_RPM, RELIEF_HOLD_S = 30, 3       # arms: cadence below 30 rpm for 3 s on a grade at or above the arming grade
RETURN_RPM, RETURN_HOLD_S = 50, 2       # ends: cadence at least 50 rpm for 2 s (or the grade falls below arming)
LOWEST_GEAR = 34 / 28                   # equipment default: compact 34 chainring, 28 cog (ratio 1.2143)
WHEEL_CIRC_M = math.pi * (0.622 + 2 * 0.025)   # 700×25c: ISO 25-622, bead seat 622 mm + 2 × 25 mm tyre = 2.111 m
GRADE_ARM_MIN = 2.0                     # %: the arming grade is never below this, so never on a flat or a descent

# --- surface class → Crr (ADR-0005 §2.6.2), never above the FE-C maximum ---
CRR_MAX = 0.0127     # FE-C page 51 maximum (unit 0xFE, ADR-0002 §4.3)
SURFACE_CRR = {
    "smooth_paved": 0.004,   # FE-C / Garmin "asphalt"
    "rough_paved": 0.008,    # FE-C / Garmin "rough road"
    "gravel": 0.012,         # Steyn & Warnich 2014: gravel = 3 × bituminous, × 0.004
    "grass": 0.0127,         # 3.5 × bituminous = 0.014, clamped to CRR_MAX
    "sand": 0.0127,          # 15 × bituminous = 0.060, clamped to CRR_MAX
    "cobbles": 0.0127,       # no measurement found: the FE-C ceiling (§5, owner question)
}

# --- FE-C wire grids (ADR-0002 §4.2) ---
GRADE_UNIT = 0.01    # % per unit, page 51
CRR_UNIT = 5e-5      # page 51
CW_UNIT = 0.01       # kg/m, page 50


@dataclass(frozen=True)
class Params:
    mass_kg: float = RIDER_KG + BIKE_KG
    cda: float = CDA
    crr: float = CRR_DEFAULT
    eta: float = ETA_HUB
    rho: float = RHO
    wind_mps: float = WIND_MPS
    drafting: float = DRAFTING

    @property
    def m_eff(self):
        # translational mass plus the wheels' rotational equivalent mass
        return self.mass_kg + M_ROT


def resist(v, grade_pct, p: Params):
    """Resistive force, N, at speed v ≥ 0 on a grade (rise over run, %). Gravity is negative downhill."""
    th = math.atan(grade_pct / 100.0)
    f_grav = p.mass_kg * G * math.sin(th)
    f_roll = p.crr * p.mass_kg * G * math.cos(th)
    va = v + p.wind_mps
    f_aero = 0.5 * p.rho * p.cda * p.drafting * va * abs(va)
    return f_grav + f_roll + f_aero


def accel(v, power_w, grade_pct, p: Params, v_min=V_MIN):
    """dv/dt of a moving bike, m/s² (v ≥ 0)."""
    f_prop = p.eta * power_w / max(v, v_min)
    return (f_prop - resist(v, grade_pct, p)) / p.m_eff


def held_at_rest(power_w, grade_pct, p: Params, v_min=V_MIN):
    """A bike at rest stays at rest when rolling resistance can hold the other forces: it never
    rolls backwards (a trainer cannot) and gravity alone sets it moving only downhill."""
    th = math.atan(grade_pct / 100.0)
    f_aero0 = 0.5 * p.rho * p.cda * p.drafting * p.wind_mps * abs(p.wind_mps)   # head wind at rest
    drive = p.eta * power_w / v_min - p.mass_kg * G * math.sin(th) - f_aero0
    return drive <= p.crr * p.mass_kg * G * math.cos(th)


def step(v, power_w, grade_pct, p: Params, h=H, v_min=V_MIN):
    """One explicit-midpoint (RK2) step. Returns (v_next, distance_m)."""
    if v == 0.0 and held_at_rest(power_w, grade_pct, p, v_min):
        return 0.0, 0.0
    vm = max(0.0, v + 0.5 * h * accel(v, power_w, grade_pct, p, v_min))
    v2 = max(0.0, v + h * accel(vm, power_w, grade_pct, p, v_min))
    if v2 <= V_STOP and held_at_rest(power_w, grade_pct, p, v_min):
        v2 = 0.0
    return v2, 0.5 * (v + v2) * h


def tick(v, power_w, grade_pct, p: Params, h=H, v_min=V_MIN):
    """One engine tick (TICK s) of sub-steps. Power and grade are constant over the tick."""
    n = round(TICK / h)
    d = 0.0
    for _ in range(n):
        v, dd = step(v, power_w, grade_pct, p, h, v_min)
        d += dd
    return v, d


def ride(segments, p: Params, v0=0.0, h=H, v_min=V_MIN):
    """segments: [(seconds, power_w, grade_pct, event)], event None or "pause" (resume from rest).
    Returns the per-tick trace [(t, v, x)]."""
    t, v, x, out = 0.0, v0, 0.0, []
    for secs, pw, gr, event in segments:
        if event == "pause":
            v = 0.0     # §2.4.4: every pause resumes from rest; position is frozen
            continue
        for _ in range(round(secs / TICK)):
            v, d = tick(v, pw, gr, p, h, v_min)
            t, x = t + TICK, x + d
            out.append((t, v, x))
    return out


def steady_speed(power_w, grade_pct, p: Params):
    """Analytic steady speed: the v ≥ 0 where η·P = F(v)·v (bisection); 0 if the bike stops."""
    def net(v):
        return p.eta * power_w - resist(v, grade_pct, p) * v
    if power_w <= 0:
        # coasting: the terminal speed where the resistive force is zero, if gravity drives it
        if resist(0.0, grade_pct, p) >= 0:
            return 0.0
        lo, hi = 0.0, 200.0
        for _ in range(200):
            mid = 0.5 * (lo + hi)
            lo, hi = (mid, hi) if resist(mid, grade_pct, p) < 0 else (lo, mid)
        return 0.5 * (lo + hi)
    lo, hi = 0.0, 200.0
    for _ in range(200):
        mid = 0.5 * (lo + hi)
        lo, hi = (mid, hi) if net(mid) > 0 else (lo, mid)
    return 0.5 * (lo + hi)


def relief_speed(cadence_rpm=RELIEF_RPM, gear=LOWEST_GEAR, circ=WHEEL_CIRC_M):
    """Road speed, m/s, at a cadence in a gear (crank rev/s × ratio × wheel circumference)."""
    return cadence_rpm / 60.0 * gear * circ


def power_needed(v, grade_pct, p: Params):
    """Steady power, W, to hold speed v on a grade (η·P = F·v)."""
    return resist(v, grade_pct, p) * v / p.eta


def arming_grade(ftp_w, p: Params, cadence_rpm=RELIEF_RPM, gear=LOWEST_GEAR, circ=WHEEL_CIRC_M):
    """§2.8.1: the grade at which holding `cadence_rpm` in the lowest gear needs more than the rider's FTP, so
    a rider below that cadence there cannot pedal back up and is stalling. Floored at GRADE_ARM_MIN."""
    v = relief_speed(cadence_rpm, gear, circ)
    lo, hi = 0.0, 100.0
    if power_needed(v, hi, p) <= ftp_w:
        return hi
    for _ in range(200):
        mid = 0.5 * (lo + hi)
        lo, hi = (mid, hi) if power_needed(v, mid, p) < ftp_w else (lo, mid)
    return max(GRADE_ARM_MIN, 0.5 * (lo + hi))


def relief_trace(samples, arm_grade):
    """§2.8.1 relief state over 1 s samples [(cadence_rpm, target_grade_pct)] → [released: bool].
    Arms after RELIEF_HOLD_S below RELIEF_RPM with the target at or above the arming grade; ends after
    RETURN_HOLD_S at or above RETURN_RPM, or at once when the target falls below the arming grade."""
    out, released, low, high = [], False, 0, 0
    for cad, grade in samples:
        armed = grade >= arm_grade
        if not released:
            low = low + 1 if (cad < RELIEF_RPM and armed) else 0
            if low >= RELIEF_HOLD_S:
                released, high = True, 0
        else:
            high = high + 1 if cad >= RETURN_RPM else 0
            if high >= RETURN_HOLD_S or not armed:
                released, low = False, 0
        out.append(released)
    return out


def check_relief():
    """Relief never arms off a climb, arms on a stall, and ends when the climb ends even at cadence 0."""
    p = Params()
    for ftp in (100, 150, 200, 280, 400):
        g = arming_grade(ftp, p)
        assert g >= GRADE_ARM_MIN > 0, ("arming grade", ftp, g)
        # at the arming grade, holding the relief cadence needs the FTP (or the floor applies)
        assert g == GRADE_ARM_MIN or abs(power_needed(relief_speed(), g, p) - ftp) < 0.01, ("derivation", ftp, g)
    g = arming_grade(200, p)
    # coasting at cadence 0 on a descent and on the flat, for a minute: never released
    for grade in (-15.0, -6.0, -0.5, 0.0, 1.9):
        assert not any(relief_trace([(0, grade)] * 60, g)), ("released off a climb", grade)
    # a stall on a climb: released after RELIEF_HOLD_S
    tr = relief_trace([(80, g + 2)] * 5 + [(20, g + 2)] * 5, g)
    assert tr.index(True) == 5 + RELIEF_HOLD_S - 1, tr
    # released at the top of a climb, then the road tips into a descent at cadence 0: released ends at once
    tr = relief_trace([(10, g + 2)] * 5 + [(0, -6.0)] * 5, g)
    assert tr[4] and not any(tr[5:]), tr
    # spinning back up ends it after RETURN_HOLD_S
    tr = relief_trace([(10, g + 2)] * 4 + [(60, g + 2)] * 3, g)
    assert tr[3] and tr[4] and not tr[5], tr


# --- what crank sends (ADR-0005 §2.4.5): on the wire grid, and the model uses the decoded values ---
def to_wire(grade_pct, crr, cda, rho=RHO):
    return (round(grade_pct / GRADE_UNIT), round(min(crr, CRR_MAX) / CRR_UNIT), round(cda * rho / CW_UNIT))


def from_wire(grade_u, crr_u, cw_u, rho=RHO):
    return grade_u * GRADE_UNIT, crr_u * CRR_UNIT, cw_u * CW_UNIT / rho


# --- checks ---
def check_steady_state():
    p = Params()
    for pw, gr in ((200, 0.0), (200, 8.0), (0, -6.0), (300, 3.0), (150, -2.0)):
        trace = ride([(600, pw, gr, None)], p)
        v_end, v_ss = trace[-1][1], steady_speed(pw, gr, p)
        assert abs(v_end - v_ss) < 0.01, ("steady state", pw, gr, v_end, v_ss)


def check_step():
    p = Params()
    segs = [(60, 200, 0.0, None), (30, 0, 0.0, None), (0, 0, 0, "pause"), (30, 400, 8.0, None)]
    a, b = ride(segs, p, h=H), ride(segs, p, h=0.001)
    dv = max(abs(x[1] - y[1]) for x, y in zip(a, b))
    dx = abs(a[-1][2] - b[-1][2])
    assert dv < 0.01 and dx < 0.5, ("step", dv, dx)
    return dv, dx


def check_v_min():
    """V_MIN only regularises P/v at rest: a 30 s standing start covers within 0.5 m of the V_MIN → 0
    limit (V_MIN = 0.05 m/s, 1 ms step)."""
    p = Params()
    d = ride([(30, 200, 0.0, None)], p)[-1][2]
    d0 = ride([(30, 200, 0.0, None)], p, h=0.001, v_min=0.05)[-1][2]
    assert abs(d - d0) < 0.5, ("V_MIN sensitivity", d, d0)
    return d - d0


def check_wire():
    for gr, surface in ((8.0, "smooth_paved"), (-6.0, "gravel"), (3.33, "cobbles"), (0.0, "rough_paved")):
        crr = SURFACE_CRR[surface]
        gu, cu, wu = to_wire(gr, crr, CDA)
        g2, c2, cda2 = from_wire(gu, cu, wu)
        assert 0 <= cu <= 0xFE and abs(g2 - gr) <= GRADE_UNIT / 2 + 1e-12, ("wire", gr, surface)
        assert abs(cda2 - CDA) <= CW_UNIT / RHO / 2 + 1e-12, ("cw grid", cda2)
        assert to_wire(g2, c2, cda2) == (gu, cu, wu), "decoded values re-encode identically"
    assert all(c <= CRR_MAX for c in SURFACE_CRR.values()), "a table Crr above the FE-C maximum"


def check_invariants():
    p = Params()
    # every power/grade combination over a grid: v ≥ 0, x never decreases
    for gr in (-15, -6, -2, 0, 2, 8, 20):
        for pw in (0, 50, 200, 600):
            for v0 in (0.0, 1.0, 12.0):
                tr = ride([(20, pw, gr, None)], p, v0=v0)
                assert all(v >= 0 for _, v, _ in tr)
                assert all(b[2] >= a[2] for a, b in zip(tr, tr[1:]))
    # a stop on a climb stays a stop: 0 W on 8 % from 5 m/s comes to rest and stays there
    tr = ride([(60, 0, 8.0, None)], p, v0=5.0)
    rest = [i for i, (_, v, _) in enumerate(tr) if v == 0.0]
    assert rest and all(tr[i][1] == 0.0 for i in range(rest[0], len(tr))), "rolled back or crept"
    # resuming from a pause starts from rest
    tr = ride([(30, 200, 0.0, None), (0, 0, 0, "pause"), (0.25, 200, 0.0, None)], p)
    assert tr[-1][1] < 1.5, ("resume from rest", tr[-1][1])


def check_constants(adr_path):
    """ADR-0005 §2.4.2 parameters, §2.4.3 integration, §2.6.2 surface table and §2.8.1 relief constants == model."""
    text = open(adr_path, encoding="utf-8").read()

    def row(sym):
        m = re.search(r"^\| `" + re.escape(sym) + r"`[^|]*\| ([-0-9.]+)", text, flags=re.M)
        assert m, ("ADR parameter row not found", sym)
        return float(m.group(1))

    model = {"g": G, "rho": RHO, "M_rot": M_ROT, "eta_hub": ETA_HUB, "CdA": CDA, "Crr_default": CRR_DEFAULT, "drafting": DRAFTING,
             "H": H, "V_MIN": V_MIN, "V_STOP": V_STOP, "Crr_max": CRR_MAX,
             "RELIEF_RPM": RELIEF_RPM, "RELIEF_HOLD_S": RELIEF_HOLD_S, "RETURN_RPM": RETURN_RPM,
             "RETURN_HOLD_S": RETURN_HOLD_S, "LOWEST_GEAR": round(LOWEST_GEAR, 4),
             "WHEEL_CIRC_M": round(WHEEL_CIRC_M, 3), "GRADE_ARM_MIN": GRADE_ARM_MIN}
    for sym, val in model.items():
        assert abs(row(sym) - val) < 1e-9, ("ADR != model", sym, row(sym), val)
    table = {k: float(v) for k, v in re.findall(r"^\| `(\w+)` \| [^|]*\| ([0-9.]+) \|", text, flags=re.M)
             if k in SURFACE_CRR}
    assert table == SURFACE_CRR, ("surface Crr table: ADR != model", table, SURFACE_CRR)


def kmh(v):
    return v * 3.6


def worked_examples():
    p = Params()
    lines = []
    # 1. flat, 200 W
    tr = ride([(300, 200, 0.0, None)], p)
    v_ss = steady_speed(200, 0.0, p)
    t95 = next(t for t, v, _ in tr if v >= 0.95 * v_ss)
    lines.append(f"1. flat 0 %, 200 W from rest: steady {kmh(v_ss):.1f} km/h, 95 % of it after {t95:.1f} s, "
                 f"{tr[-1][2] / 1000:.2f} km in 5 min")
    # 2. 8 % climb, 200 W
    v8 = steady_speed(200, 8.0, p)
    tr = ride([(300, 200, 8.0, None)], p, v0=v_ss)
    lines.append(f"2. climb 8 %, 200 W, entered at the flat speed: steady {kmh(v8):.1f} km/h "
                 f"(after 60 s {kmh(tr[239][1]):.1f} km/h), {tr[-1][2] / 1000:.2f} km in 5 min, "
                 f"{tr[-1][2] * 0.08 / math.sqrt(1.0064):.0f} m climbed")
    # 3. −6 % descent, 0 W
    v6 = steady_speed(0, -6.0, p)
    tr = ride([(120, 0, -6.0, None)], p, v0=v_ss)
    lines.append(f"3. descent −6 %, 0 W, entered at the flat speed: terminal {kmh(v6):.1f} km/h "
                 f"(after 30 s {kmh(tr[119][1]):.1f} km/h, after 120 s {kmh(tr[-1][1]):.1f} km/h)")
    # 4. stop and start on the flat
    segs = [(120, 200, 0.0, None), (30, 0, 0.0, None), (0, 0, 0, "pause"), (60, 200, 0.0, None)]
    tr = ride(segs, p)
    v_before, v_coast = tr[479][1], tr[599][1]
    after = tr[600:]
    t90 = next(t for t, v, _ in after if v >= 0.9 * v_ss) - tr[599][0]
    lines.append(f"4. stop/start, flat: 200 W for 120 s ({kmh(v_before):.1f} km/h), coast 30 s at 0 W "
                 f"({kmh(v_coast):.1f} km/h), pause (resume from rest), 200 W again: 90 % of "
                 f"{kmh(v_ss):.1f} km/h after {t90:.1f} s; {tr[-1][2] / 1000:.2f} km in all")
    # 5. surface: the same 200 W on gravel (OSM surface=gravel)
    vg = steady_speed(200, 0.0, replace(p, crr=SURFACE_CRR["gravel"]))
    lines.append(f"5. flat 0 %, 200 W on gravel (Crr {SURFACE_CRR['gravel']}): steady {kmh(vg):.1f} km/h "
                 f"(tarmac {kmh(v_ss):.1f} km/h)")
    # 6. low-cadence relief: the arming grade per FTP (§2.8.1)
    v = relief_speed()
    arms = ", ".join(f"FTP {f} W: {arming_grade(f, p):.1f} %" for f in (150, 200, 280))
    lines.append(f"6. relief arming grade, {RELIEF_RPM} rpm in 34x28 on 700x25c = {kmh(v):.1f} km/h: {arms} "
                 f"(at 10 % that speed needs {power_needed(v, 10.0, p):.0f} W, at 20 % {power_needed(v, 20.0, p):.0f} W)")
    return lines


def main(argv):
    adr = argv[1] if len(argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "0005-routes.md")
    check_constants(adr)
    print("constants: ADR-0005 §2.4.2, §2.4.3, §2.6.2, §2.8.1 == model (parameters, integration, surface Crr, relief), ok")
    check_steady_state()
    print("steady state: integrator == analytic power balance within 0.01 m/s on 5 grades, ok")
    dv, dx = check_step()
    print(f"step: H = {H * 1000:.0f} ms vs 1 ms reference: max speed error {dv:.4f} m/s, "
          f"distance error {dx:.3f} m over 120 s, ok")
    dd = check_v_min()
    print(f"V_MIN: a 30 s standing start at 200 W covers {dd:+.2f} m against the V_MIN -> 0 limit, ok")
    check_wire()
    print("wire: grade, Crr and wind coefficient round-trip the FE-C grid; table Crr <= 0.0127, ok")
    check_invariants()
    print("invariants: v >= 0, position never decreases, no rolling back, resume from rest, ok")
    check_relief()
    print(f"relief: arming grade >= {GRADE_ARM_MIN} % for every FTP; never released coasting on a descent or "
          "the flat; arms on a stall; ends when the climb ends, also at cadence 0, ok")
    print(f"worked examples (rider + bike {RIDER_KG + BIKE_KG:.0f} kg, CdA {CDA} m², Crr {CRR_DEFAULT}, "
          f"rho {RHO} kg/m³, hub power, still air):")
    for line in worked_examples():
        print("  " + line)
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
