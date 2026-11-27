#!/usr/bin/env python3
"""Golden vectors and model plans for #620's Rust port of the easer (`crank_engine::slew`).

The Rust tests read `crates/crank-engine/tests/fixtures/slew_golden.json`:
- `golden`: cases (tuning, blocks, events, origin, cap, faults) with per-tick vectors (R, y, v, a, on
  the line, cut, degraded). The Rust test compares its own run with them within 1e-6 and rewrites
  them on request (`CRANK_SLEW_GOLDEN=write`); `--check` here runs every case through this model
  (upstream Ruckig) and compares the committed vectors within 1e-6, so the two implementations are
  held together (ADR-0002 §7.10.2 "Determinism").
- `plans`: the model's plans that come from Python's random generators (pinned seeds, the falsifiers'
  schemes), without vectors; the Rust property tests run them under every property.

    just slew-vectors --write   # cases + upstream vectors
    just slew-vectors --check   # committed vectors vs upstream (the default)

Fault injection here is deterministic only: a share of 1.0 from a tick (until a tick), refused ticks,
every braking plan refused, the twin answering.
"""
from __future__ import annotations

import json
import math
import random
import sys

import erg_slew as M
from erg_slew import (DEFAULT, Block, Event, Tuning, GOLDEN_PLANS, golden_random, steady, ramp, run,
                      random_plan, random_plan_v6, random_plan_v7, cap_event_plan, fall_plan, GRID)

PATH = "../../../crates/crank-engine/tests/fixtures/slew_golden.json"  # from docs/adr/models (`just slew-vectors`)
TOL = 1e-6


def tuning(t: Tuning) -> dict:
    return {"riseWPerS": t.rise, "fallFactor": t.factor, "easeS": t.ease}


def case(name, t, blocks, events=(), origin=None, **kw):
    faults = {}
    if kw.get("inject"):
        assert kw["inject"] == 1.0, name
        faults["from"] = kw.get("inject_from", 0)
        if kw.get("refuse_until") is not None:
            faults["until"] = kw["refuse_until"]
    if kw.get("refuse_ticks"):
        faults["ticks"] = sorted(kw["refuse_ticks"])
    for f in ("fail_brake", "twin_answers"):
        if kw.get(f):
            faults[f] = True
    return {"name": name, "tuning": tuning(t),
            "blocks": [[b.ticks, b.frm, b.to] for b in blocks if b.ticks > 0],
            "events": [[e.tick, e.kind, e.value] for e in events],
            "origin": origin, "m0": kw.get("m0", 2000.0), "y0": kw.get("y0"), "v0": kw.get("v0", 0.0),
            "faults": faults}


def run_case(c) -> M.Trace:
    tz = c["tuning"]
    t = Tuning(tz["riseWPerS"], tz["fallFactor"], tz["easeS"])
    f = c["faults"]
    kw = dict(m0=c["m0"], y0=c["y0"], v0=c["v0"], fail_brake=f.get("fail_brake", False),
              twin_answers=f.get("twin_answers", False), refuse_ticks=set(f.get("ticks", [])))
    if "from" in f:
        kw.update(inject=1.0, inject_from=f["from"], refuse_until=f.get("until"))
    return run([Block(*b) for b in c["blocks"]], [Event(*e) for e in c["events"]], t=t, origin=c["origin"], **kw)


def vectors(tr: M.Trace) -> dict:
    r9 = lambda xs: [round(x, 9) for x in xs]
    return {"r": r9(tr.r), "y": r9(tr.y), "v": r9(tr.v), "a": r9(tr.a), "on": tr.on,
            "cut": [e or None for e in tr.exc], "degraded": tr.degraded}


def golden_cases() -> list:
    out = []
    for name, spec in {**GOLDEN_PLANS, **golden_random()}.items():
        t, blocks, events, origin = spec[:4]
        kw = spec[4] if len(spec) > 4 else {}
        out.append(case(name, t, blocks, events, origin, **kw))
    # The falsifier's cases (ADR-0002 §7.10.6) and the figures' plans of tables(), at the defaults.
    d = DEFAULT
    for name, blocks, events, origin in (
            ("skip 100->112 W", [steady(100, 10), steady(112, 10), steady(112, 20)], [Event(44, "skip")], None),
            ("skip 300->250 W", [steady(300, 10), steady(250, 10), steady(250, 20)], [Event(44, "skip")], None),
            ("engage at 260 W on a -20 W/s ramp", [steady(300, 5), ramp(300, 100, 10), steady(100, 30)],
             [Event(28, "engage", 260.0)], None),
            ("3 s at 400 W, then 100 W", [steady(100, 10), steady(400, 3), steady(100, 30)], [], None),
            ("manual -10 % mid-rise", [steady(100, 5), steady(400, 30)], [Event(30, "offset", 0.9)], None),
            ("cap drops to 250 W mid-rise", [steady(100, 5), steady(400, 30)], [Event(34, "cap", 250.0)], None),
            ("ramp 50->150 W over 30 s", [steady(50, 10), ramp(50, 150, 30), steady(150, 15)], [], None),
            ("ramp 100->400 W over 120 s", [steady(100, 10), ramp(100, 400, 120), steady(400, 15)], [], None),
            ("owner's case: ramp 100->180 W over 60 s into 220 W", [steady(100, 10), ramp(100, 180, 60), steady(220, 60)],
             [], None),
            ("engagement 0->137 W", [steady(137, 60)], [], 0.0),
            ("jump 137->220 W", [steady(137, 5), steady(220, 60)], [], None),
            ("jump 220->137 W", [steady(220, 5), steady(137, 60)], [], None),
            ("15/15 x6 100/400 W", [b for _ in range(6) for b in (steady(100, 15), steady(400, 15))] + [steady(100, 30)],
             [], None)):
        out.append(case(name, d, blocks, events, origin))
    out.append(case("engagement 0->400 W (50, x1, 1)", Tuning(50, 1.0, 1.0), [steady(400, 30)], [], 0.0))
    out.append(case("refused from tick 25", d, [steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1)],
                    inject=1.0, inject_from=25))
    out.append(case("a single refusal at tick 25", d, [steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1)],
                    inject=1.0, inject_from=25, refuse_until=26))
    out.append(case("every first calculation refused from tick 25, the twin answering", d,
                    [steady(100, 5), steady(300, 30)], [Event(25, "offset", 1.1)], inject=1.0, inject_from=25,
                    twin_answers=True))
    out.append(case("ramp into the cap, braking plan fails (40, x2, 2)", Tuning(40, 2.0, 2.0),
                    [steady(580.7, 5), ramp(580.7, 1131.0, 60)], m0=790.99, fail_brake=True))
    return out


def seeded(seed, idx, gen):
    rng = random.Random(seed)
    for _ in range(idx + 1):
        t = rng.choice(GRID)
        blocks, events = gen(rng, t)
        origin = rng.uniform(0, 300) if rng.random() < 0.3 else None
        m0 = rng.choice([2000.0, rng.uniform(200, 800)])
    return t, blocks, events, origin, m0


def model_plans() -> list:
    """The model's plans from Python's generators, as data."""
    out = []
    # pinned(): random plans the falsifications found, by (seed, plan), and round 6/7's generator plans.
    for seed, idx in ((7, 226), (585, 442), (585, 7929), (2, 13093), (2, 19521), (3, 15889),
                      (585, 2694), (585, 4693), (7, 2536), (585, 3020), (7, 1773), (7, 5618)):
        t, b, e, o, m0 = seeded(seed, idx, random_plan)
        out.append(case(f"pinned seed {seed} plan {idx}", t, b, e, o, m0=m0))
    for seed, idx, gen in ((585, 4670, random_plan_v6), (585, 4191, random_plan_v7), (1, 2866, random_plan_v7),
                           (7, 1525, random_plan_v7), (1, 3167, random_plan_v7), (2, 6492, random_plan_v7),
                           (7, 3875, random_plan_v7), (7, 5964, random_plan_v7)):
        t, b, e, o, m0 = seeded(seed, idx, gen)
        out.append(case(f"pinned {gen.__name__} seed {seed} plan {idx}", t, b, e, o, m0=m0))
    # repros(): 123000 + 287 with refusals 38..49, and 56000 + 792, 1227, 656.
    rng = random.Random(123000 + 287)
    t = rng.choice(GRID)
    b, e = random_plan_v7(rng, t)
    m0 = rng.choice([2000.0, rng.uniform(200, 800), rng.uniform(100, 300)])
    out.append(case("random 123000+287, refusals 38..49", t, b, e, m0=m0, refuse_ticks=set(range(38, 50))))
    for i, win in ((792, None), (1227, None), (656, (1, 80))):
        rng = random.Random(56000 + i)
        t = rng.choice(GRID)
        b, e = random_plan_v7(rng, t)
        m0 = rng.choice([2000.0, rng.uniform(200, 800), rng.uniform(100, 300)])
        k0 = rng.randrange(max(1, sum(x.ticks for x in b)))
        L = rng.choice([1, 2, 3, 4, 5, 6, 8, 12, 20, 40, 80])
        if win:
            k0, L, m0 = win[0], win[1], 2000.0
        out.append(case(f"random 56000+{i}, refusals {k0}..{k0 + L - 1}", t, b, e, m0=m0,
                        refuse_ticks=set(range(k0, k0 + L))))
    # refusal_windows(): the falsifiers' schemes, seeds 56000 + i and 70000 + i.
    for base_seed in (56000, 70000):
        for i in range(150):
            rng = random.Random(base_seed + i)
            t = rng.choice(GRID)
            b, e = random_plan_v7(rng, t)
            m0 = rng.choice([2000.0, rng.uniform(200, 800)])
            k0 = rng.randrange(max(1, sum(x.ticks for x in b)))
            L = rng.choice([1, 2, 3, 4, 6, 8, 12] if base_seed == 56000 else [4, 5, 6, 8, 12, 20, 40])
            out.append(case(f"refusal window {base_seed}+{i} ticks {k0}..{k0 + L - 1}", t, b, e, m0=m0,
                            refuse_ticks=set(range(k0, k0 + L))))
    # persistent_random(): the round-7 falsifier's q4v7 scheme (seeds 31000 + i).
    for i in range(150):
        rng = random.Random(31000 + i)
        t = rng.choice(GRID)
        b, e = random_plan_v7(rng, t)
        m0 = rng.choice([2000.0, rng.uniform(200, 800)])
        onset = rng.randrange(max(1, sum(x.ticks for x in b)))
        out.append(case(f"persistent q4v7#{i} from tick {onset}", t, b, e, m0=m0, inject=1.0, inject_from=onset))
    # cap_events() and repros(): the round-13 falsifier's cap events during refusals (777000 + i).
    for i in (2717,) + tuple(range(0, 200)):
        t, bl, ev, rt, lo = cap_event_plan(i)
        out.append(case(f"cap event during refusals 777000+{i}", t, bl, ev, refuse_ticks=rt))
    # refusal_falls(): planned falls with refusal windows (121000 + i).
    for i in range(200):
        t, b, m0, k0, L = fall_plan(i)
        out.append(case(f"refusal on a planned fall #{i}", t, b, m0=m0, refuse_ticks=set(range(k0, k0 + L))))
    return out


def write(path):
    cases = golden_cases()
    for c in cases:
        c["vectors"] = vectors(run_case(c))
    data = {"about": "ADR-0002 §7.10 golden vectors (#620): written by crank-engine's slew golden test"
                     " (CRANK_SLEW_GOLDEN=write), checked against docs/adr/models/erg_slew.py (upstream Ruckig"
                     " 0.19.4) by docs/adr/models/erg_slew_vectors.py --check, within 1e-6.",
            "golden": cases, "plans": model_plans()}
    with open(path, "w") as f:
        json.dump(data, f, separators=(",", ":"), allow_nan=True)
        f.write("\n")
    print(f"wrote {len(cases)} golden cases, {len(data['plans'])} plans to {path}")


def check(path) -> int:
    with open(path) as f:
        data = json.load(f)
    bad = 0
    worst = 0.0
    for c in data["golden"]:
        got = vectors(run_case(c))
        want = c["vectors"]
        for key in ("r", "y", "v", "a"):
            if len(got[key]) != len(want[key]):
                print(f"{c['name']}: {key} has {len(want[key])} ticks, the model {len(got[key])}")
                bad += 1
                break
            dev = max((abs(x - y) for x, y in zip(got[key], want[key])), default=0.0)
            worst = max(worst, dev)
            if dev > TOL:
                k = next(i for i, (x, y) in enumerate(zip(got[key], want[key])) if abs(x - y) > TOL)
                print(f"{c['name']}: {key}[{k}] = {want[key][k]!r}, the model {got[key][k]!r}")
                bad += 1
                break
        for key in ("on", "cut", "degraded"):
            if got[key] != want[key]:
                k = next(i for i, (x, y) in enumerate(zip(got[key], want[key])) if x != y)
                print(f"{c['name']}: {key}[{k}] = {want[key][k]!r}, the model {got[key][k]!r}")
                bad += 1
    print(f"{len(data['golden'])} golden cases against the model: {bad} disagree; largest difference {worst:.3g}")
    return 1 if bad else 0


if __name__ == "__main__":
    args = sys.argv[1:]
    p = next((a for a in args if not a.startswith("--")), PATH)
    if "--write" in args:
        write(p)
        sys.exit(0)
    sys.exit(check(p))
