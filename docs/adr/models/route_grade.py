#!/usr/bin/env python3
"""Executable model of elevation → grade, ADR-0005 §2.3 (#523).

Stdlib only. On real, noisy GPS elevation from public sample tracks
(`fixtures/gpx-elevation-samples.csv`, from gpxpy's test files, Apache-2.0) it:

* resamples each track segment by distance (haversine, IUGG mean Earth radius) every STEP_M metres;
* smooths elevation with a Gaussian kernel over distance for each candidate sigma and prints, per
  segment: total ascent, 99th-percentile and maximum |grade|, the RMS change of elevation and grade
  sign reversals per km;
* checks the decision of §2.3.2: SIGMA_M is the smallest candidate that holds the spurious grade of
  the flat lakeshore segment under FLAT_MAX_PCT, and it is the knee on the climbs: the next larger
  candidate removes a larger share of their ascent than the step from the previous one did;
* runs the send pipeline of §2.3.3–2.3.5 (cap, difficulty scaling, slew limit, FE-C grid, at most
  one page-51 change per second) on a grade step and checks its bounds;
* checks that the ADR's numbers are the model's (check_constants).

Run: python3 docs/adr/models/route_grade.py [path/to/0005-routes.md]   (exit 0 = all checks pass).
"""
from __future__ import annotations

import csv
import io
import math
import os
import re
import sys

EARTH_R_M = 6371008.8      # IUGG mean Earth radius (Moritz 2000)
STEP_M = 10.0              # resampling step, m (§2.3.1)
SIGMA_M = 100.0            # Gaussian smoothing sigma, m (§2.3.2)
CANDIDATES = (0.0, 10.0, 25.0, 50.0, 100.0, 200.0)
FLAT_SEGMENT = "cerknica-1"           # lakeshore: the true grade is close to 0
CLIMB_SEGMENTS = ("korita-1", "korita-2")
FLAT_MAX_PCT = 3.0         # spurious grade allowed on a flat road (§2.3.2)
GRADE_MIN_PCT, GRADE_MAX_PCT = -15.0, 20.0   # cap (§2.3.3)
SLEW_PCT_PER_S = 1.0       # largest change of the sent grade per second (§2.3.5)
SEND_PERIOD_S = 1.0        # at most one page-51 change per second (ADR-0002 §7.9)
GRADE_GRID_PCT = 0.01      # crank's grade resolution (§2.3.5): the FE-C wire grid
DESCENT_SCALE = 1.0        # default difficulty scaling on descents (§2.3.4)
TICK_S = 0.25


def load(path):
    rows = [ln for ln in open(path, encoding="utf-8") if not ln.startswith("#")]
    segs = {}
    for r in csv.DictReader(io.StringIO("".join(rows))):
        segs.setdefault(r["segment"], []).append((float(r["lat"]), float(r["lon"]), float(r["ele"])))
    return segs


def haversine(a, b):
    la1, lo1, la2, lo2 = map(math.radians, (a[0], a[1], b[0], b[1]))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 2 * EARTH_R_M * math.asin(math.sqrt(h))


def resample(pts, step=STEP_M):
    """Elevation every `step` m of along-track distance, linear between points (§2.3.1)."""
    s = [0.0]
    for a, b in zip(pts, pts[1:]):
        s.append(s[-1] + haversine(a, b))
    out, x, j = [], 0.0, 0
    while x <= s[-1]:
        while s[j + 1] < x:
            j += 1
        span = s[j + 1] - s[j]
        f = (x - s[j]) / span if span > 0 else 0.0
        out.append(pts[j][2] + f * (pts[j + 1][2] - pts[j][2]))
        x += step
    return out


def smooth(ele, sigma, step=STEP_M):
    """Gaussian kernel over distance, truncated at 3 sigma, renormalised at the ends (§2.3.2)."""
    if sigma == 0:
        return list(ele)
    k = int(3 * sigma / step)
    w = [math.exp(-0.5 * (i * step / sigma) ** 2) for i in range(-k, k + 1)]
    out = []
    for i in range(len(ele)):
        num = den = 0.0
        for j, wj in enumerate(w):
            t = i + j - k
            if 0 <= t < len(ele):
                num += wj * ele[t]
                den += wj
        out.append(num / den)
    return out


def grades(ele, step=STEP_M):
    return [(b - a) / step * 100.0 for a, b in zip(ele, ele[1:])]


def stats(raw, sigma):
    s = smooth(raw, sigma)
    g = grades(s)
    ag = sorted(abs(x) for x in g)
    km = len(raw) * STEP_M / 1000
    return {
        "ascent": sum(max(0.0, b - a) for a, b in zip(s, s[1:])),
        "p99": ag[int(0.99 * len(ag))],
        "max": ag[-1],
        "rms": math.sqrt(sum((a - b) ** 2 for a, b in zip(raw, s)) / len(raw)),
        "flips": sum(1 for a, b in zip(g, g[1:]) if a * b < 0) / km,
    }


def check_sigma(table):
    """SIGMA_M is the smallest candidate with a flat spurious max grade under FLAT_MAX_PCT, and it is
    the knee on the climbs: doubling it removes more relief than the step that led to it."""
    ok = [sg for sg in CANDIDATES if table[FLAT_SEGMENT][sg]["max"] < FLAT_MAX_PCT]
    assert ok and ok[0] == SIGMA_M, ("smallest sigma under the flat bound", ok)
    prev = CANDIDATES[CANDIDATES.index(SIGMA_M) - 1]
    nxt = CANDIDATES[CANDIDATES.index(SIGMA_M) + 1]
    for seg in CLIMB_SEGMENTS:
        a_prev, a, a_next = (table[seg][x]["ascent"] for x in (prev, SIGMA_M, nxt))
        lost_to = (a_prev - a) / a_prev
        lost_after = (a - a_next) / a
        assert lost_after > lost_to, ("not the knee", seg, lost_to, lost_after)


# --- send pipeline (§2.3.3–2.3.5) ---
def target_grade(raw_pct, scale_all=1.0, scale_descent=DESCENT_SCALE):
    """Route grade → the grade crank wants the trainer at: scaling, then the cap (§2.3.3, §2.3.4)."""
    g = raw_pct * scale_all
    if g < 0:
        g *= scale_descent
    return min(GRADE_MAX_PCT, max(GRADE_MIN_PCT, g))


def send_trace(targets_per_tick):
    """Slew-limit the target per tick, quantise to GRADE_GRID_PCT and emit at most one change per
    SEND_PERIOD_S. Returns [(t, sent_grade or None)] per tick (None = nothing written that tick)."""
    out, cur, last_sent, last_t = [], None, None, -1e9
    max_step = SLEW_PCT_PER_S * TICK_S
    for i, tgt in enumerate(targets_per_tick):
        t = i * TICK_S
        cur = tgt if cur is None else cur + max(-max_step, min(max_step, tgt - cur))
        q = round(round(cur / GRADE_GRID_PCT) * GRADE_GRID_PCT, 2)
        if q != last_sent and t - last_t >= SEND_PERIOD_S - 1e-9:
            out.append((t, q))
            last_sent, last_t = q, t
        else:
            out.append((t, None))
    return out


def check_pipeline():
    # a 0 → 25 % step in the data (an unsmoothed grade profile, or a corrupt file) at t = 10 s
    tgts = [target_grade(0.0)] * 40 + [target_grade(25.0)] * 160
    tr = send_trace(tgts)
    sent = [(t, g) for t, g in tr if g is not None]
    assert max(g for _, g in sent) <= GRADE_MAX_PCT, "cap"
    for (t1, g1), (t2, g2) in zip(sent, sent[1:]):
        assert t2 - t1 >= SEND_PERIOD_S - 1e-9, ("send period", t1, t2)
        assert abs(g2 - g1) <= SLEW_PCT_PER_S * (t2 - t1) + GRADE_GRID_PCT + 1e-9, ("slew", t1, g1, t2, g2)
    reach = next(t for t, g in sent if g == GRADE_MAX_PCT) - 10.0
    # descents are scaled by default, then capped
    assert target_grade(-10.0) == -10.0 and target_grade(-40.0) == GRADE_MIN_PCT
    assert target_grade(-10.0, scale_descent=0.5) == -5.0 and target_grade(10.0, scale_all=0.5) == 5.0
    assert target_grade(30.0, scale_all=0.5) == 15.0, "scaling comes before the cap"
    return reach, len(sent)


def check_constants(adr_path):
    text = open(adr_path, encoding="utf-8").read()

    def row(sym):
        m = re.search(r"^\| `" + re.escape(sym) + r"`[^|]*\| ([-0-9.]+)", text, flags=re.M)
        assert m, ("ADR parameter row not found", sym)
        return float(m.group(1))

    model = {"STEP_M": STEP_M, "SIGMA_M": SIGMA_M, "FLAT_MAX_PCT": FLAT_MAX_PCT,
             "GRADE_MIN_PCT": GRADE_MIN_PCT, "GRADE_MAX_PCT": GRADE_MAX_PCT,
             "SLEW_PCT_PER_S": SLEW_PCT_PER_S, "SEND_PERIOD_S": SEND_PERIOD_S,
             "GRADE_GRID_PCT": GRADE_GRID_PCT, "DESCENT_SCALE": DESCENT_SCALE}
    for sym, val in model.items():
        assert abs(row(sym) - val) < 1e-12, ("ADR != model", sym, row(sym), val)


def main(argv):
    here = os.path.dirname(__file__)
    adr = argv[1] if len(argv) > 1 else os.path.join(here, "..", "0005-routes.md")
    check_constants(adr)
    print("constants: ADR-0005 §2.3 == model (step, sigma, flat bound, cap, slew, send period, grid, scaling), ok")
    segs = load(os.path.join(here, "fixtures", "gpx-elevation-samples.csv"))
    table = {}
    for name, pts in segs.items():
        raw = resample(pts)
        table[name] = {sg: stats(raw, sg) for sg in CANDIDATES}
        print(f"{name}: {len(pts)} points, {len(raw) * STEP_M / 1000:.2f} km, resampled every {STEP_M:.0f} m")
        for sg in CANDIDATES:
            s = table[name][sg]
            mark = "  <- decided" if sg == SIGMA_M else ""
            print(f"  sigma {sg:5.0f} m: ascent {s['ascent']:5.0f} m, p99 |grade| {s['p99']:5.1f} %, "
                  f"max |grade| {s['max']:5.1f} %, rms change {s['rms']:4.1f} m, "
                  f"sign reversals {s['flips']:4.1f}/km{mark}")
    check_sigma(table)
    print(f"sigma: {SIGMA_M:.0f} m is the smallest candidate with the flat segment under {FLAT_MAX_PCT} % "
          f"and the knee of the climbs' ascent, ok")
    reach, n = check_pipeline()
    print(f"pipeline: a 0 -> 25 % data step is capped at {GRADE_MAX_PCT} % and reaches it {reach:.1f} s later "
          f"in {n} page-51 changes (<= {SLEW_PCT_PER_S} %/s, >= {SEND_PERIOD_S:.0f} s apart), ok")
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
