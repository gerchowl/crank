#!/usr/bin/env python3
"""Executable model of the trainer-following machine, ADR-0002 §4.4.1 (#54).

Stdlib only. It implements the component machine table (C1–C9), reply routing (R2–R5),
the activity rule and the aggregate function (A1–A6) of §4.4.1 literally, and checks them:

* check_aggregate(), check_routing(): exhaustive over every component-state combination and
  every mode pair / history age / reply.
* explore_component(): breadth-first to the fixpoint of one component machine, with requirement
  toggles, under an independent monitor; fair variants check the decision deadline.
* explore(): whole-machine walks (link events, every desired change, echo and failure replies,
  silence), to the fixpoint or over every input sequence up to a depth. States are deduplicated
  after normalising timestamps.
* scenarios(): named regressions, one per pre-mortem counterexample so far.

Run: python3 docs/adr/models/trainer_following.py [path/to/0002-ble-trainer.md]   (exit 0 = all
checks pass; the ADR path defaults to the sibling file); CI runs it as the
`following-model` flake check, `just model` locally.
"""
from __future__ import annotations

import sys
from collections import deque
from dataclasses import dataclass, replace

# --- constants (§4.4.1), in seconds; the model uses integer time in `unit` per second ---
# L = 5 s: HIL #44 Part A measured an end-to-end p99 of 2.19 s (ERG) and 4.21 s (SIM) (ADR-0002 §4.4.1).
L_S, N_FOLLOW, T_VERIFY_S, T_MISMATCH_S, T_UNKNOWN_S = 5, 3, 2, 2, 5
P_S = {"erg": 1, "track": 2, "wind": 2}  # reply period per component (§4.4.1 echo schedule)


@dataclass(frozen=True)
class K:
    unit: int = 1  # time steps per second

    @property
    def L(self): return L_S * self.unit
    @property
    def T_verify(self): return T_VERIFY_S * self.unit
    @property
    def T_mismatch(self): return T_MISMATCH_S * self.unit
    @property
    def T_unknown(self): return T_UNKNOWN_S * self.unit
    def P(self, c): return P_S[c] * self.unit
    def T_decide(self, c):
        # §4.4.1 decision deadline: T_verify + T_mismatch + 3·P_c
        return self.T_decide_p(self.P(c))
    def T_decide_p(self, p):
        return self.T_verify + self.T_mismatch + 3 * p
    def V(self, c):
        # §4.4.1 guarantee 4: time from a component's first agreeing reply (replies every P_c)
        # to its verification: P_c · max(N_follow − 1, ⌈T_verify / P_c⌉)
        p = self.P(c)
        return p * max(N_FOLLOW - 1, -(-self.T_verify // p))


def components(mode):
    """Desired mode → {component: grid value}. None = no desired mode."""
    if mode is None:
        return {}
    if mode[0] == "erg":
        return {"erg": mode[1]}
    return {"track": mode[1], "wind": mode[2]}


# --- component machine (§4.4.1 table C1–C9) ---
@dataclass(frozen=True)
class Comp:
    state: str = "unverified"          # unverified | following | not_following | silent
    streak_n: int = 0
    streak_first: int | None = None
    dis_since: int | None = None
    last_evidence: int = 0             # last A/D reply, or (re)start; paused while not required
    not_required_since: int | None = None  # None while required


def fresh(now):
    return Comp(last_evidence=now)


# The component table (§4.4.1) in the ADR's row order. TABLE_ORDER is checked against the ADR
# text (check_table_order); REPLY_ROWS drives comp_reply, so the order evaluated here is the order
# the ADR states. C2 is a pre-step: when its guard holds it applies and evaluation continues.
# Every other reply row is first-match-wins among the rows for its input class.
TABLE_ORDER = ("C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9")


@dataclass(frozen=True)
class Row:
    id: str
    on: str            # "A", "D" or "AD"
    pre: bool          # pre-step (applies and continues) vs. first-match-wins
    guard: object      # (comp, now, k) -> bool
    act: object        # (comp, now, k) -> comp


def _stale(c, now, k):
    return c.state == "silent" or now - c.last_evidence >= k.T_unknown


def _reset(c, now, k):
    return replace(c, state="unverified", streak_n=0, streak_first=None, dis_since=None)


def _agree_add(c, now):
    first = now if c.streak_n == 0 else c.streak_first
    return replace(c, streak_n=c.streak_n + 1, streak_first=first, last_evidence=now)


def _verifies(c, now, k):
    a = _agree_add(c, now)
    return a.streak_n >= N_FOLLOW and now - a.streak_first >= k.T_verify


def _disagree_add(c, now):
    dis = c.dis_since if c.dis_since is not None else now
    return replace(c, streak_n=0, streak_first=None, dis_since=dis, last_evidence=now)


def _mismatch(c, now, k):
    return now - (c.dis_since if c.dis_since is not None else now) >= k.T_mismatch


REPLY_ROWS = (
    Row("C2", "AD", True, _stale, _reset),
    Row("C3", "A", False, _verifies,
        lambda c, now, k: replace(_agree_add(c, now), state="following", dis_since=None)),
    Row("C4", "A", False, lambda c, now, k: True, lambda c, now, k: _agree_add(c, now)),
    Row("C5", "D", False, _mismatch,
        lambda c, now, k: replace(_disagree_add(c, now), state="not_following")),
    Row("C6", "D", False, lambda c, now, k: True,
        lambda c, now, k: replace(_disagree_add(c, now),
                                  state="unverified" if c.state == "following" else c.state)),
)


def comp_reply(c: Comp, cls: str, now, k: K) -> Comp:
    """Apply an Agree ("A") or Disagree ("D") to a component by walking REPLY_ROWS in order."""
    for row in REPLY_ROWS:
        if cls not in row.on or not row.guard(c, now, k):
            continue
        c = row.act(c, now, k)
        if not row.pre:
            return c
    raise AssertionError("no row matched")


def comp_agree(c: Comp, now, k: K) -> Comp:
    return comp_reply(c, "A", now, k)


def comp_disagree(c: Comp, now, k: K) -> Comp:
    return comp_reply(c, "D", now, k)


def comp_tick(c: Comp, now, k: K) -> Comp:
    if c.state != "silent" and now - c.last_evidence >= k.T_unknown:   # C7
        return replace(c, state="silent", streak_n=0, streak_first=None, dis_since=None)
    return c


def comp_stop_required(c: Comp, now) -> Comp:                   # C8
    st = "unverified" if c.state == "following" else c.state
    return replace(c, state=st, streak_n=0, streak_first=None, not_required_since=now)


def comp_resume_required(c: Comp | None, now, k: K) -> Comp:    # C9
    if c is None or c.not_required_since is None or now - c.not_required_since > k.T_unknown:
        return fresh(now)
    gap = now - c.not_required_since
    return replace(c, last_evidence=c.last_evidence + gap, not_required_since=None)


# --- whole machine: activity, desired history, reply routing ---
@dataclass(frozen=True)
class Machine:
    link_ready: bool = False
    desired: tuple | None = None
    active: bool = False
    comps: tuple = ()        # sorted ((name, Comp), ...)
    hist: tuple = ()         # ((component, value, until), ...) values desired until `until`

    def comp_map(self):
        return dict(self.comps)


def _set(m: Machine, comps: dict, hist=None) -> Machine:
    return replace(m, comps=tuple(sorted(comps.items())),
                   hist=m.hist if hist is None else tuple(sorted(hist)))


def recently_desired(m: Machine, c, v, now, k: K):
    req = components(m.desired)
    if req.get(c) == v:
        return True
    return any(hc == c and hv == v and now - until <= k.L for hc, hv, until in m.hist)


def recently_required(m: Machine, c, now, k: K):
    return any(hc == c and now - until <= k.L for hc, _, until in m.hist)


def set_link(m: Machine, ready: bool, now, k: K) -> Machine:
    return _activity(replace(m, link_ready=ready), m, now, k)


def set_desired(m: Machine, mode, now, k: K) -> Machine:
    if mode == m.desired:
        return m
    new = replace(m, desired=mode)
    if m.active and new.link_ready and mode is not None:
        old_req, new_req = components(m.desired), components(mode)
        hist = [h for h in m.hist if now - h[2] <= k.L]
        comps = m.comp_map()
        for c, v in old_req.items():
            if new_req.get(c) != v:
                hist.append((c, v, now))
            if c not in new_req:
                comps[c] = comp_stop_required(comps[c], now)
        for c in new_req:
            if c not in old_req:
                comps[c] = comp_resume_required(comps.get(c), now, k)
        return _set(new, comps, hist)
    return _activity(new, m, now, k)


def _activity(new: Machine, old: Machine, now, k: K) -> Machine:
    want = new.link_ready and new.desired is not None
    if want and not old.active:                                 # Activate
        comps = {c: fresh(now) for c in components(new.desired)}
        return _set(replace(new, active=True), comps, [])
    if not want and old.active:                                 # Deactivate
        return replace(new, active=False, comps=(), hist=())
    return new


def reply(m: Machine, r, now, k: K) -> Machine:
    """Reply routing R2–R5. r = ('echo', component, value) | ('fail', component).
    R1 (neutral observations: page-25 flags, NotControl, Pending/Other) has no effect and is
    therefore not part of the alphabet."""
    if not m.active:
        return m
    req = components(m.desired)
    comps = m.comp_map()
    kind, c = r[0], r[1]
    if c in req:
        if kind == "echo" and recently_desired(m, c, r[2], now, k):
            comps[c] = comp_agree(comps[c], now, k)             # R2 Agree
        else:
            comps[c] = comp_disagree(comps[c], now, k)          # R3 Disagree
    elif not recently_required(m, c, now, k):                   # R5 foreign page
        for rc in req:
            comps[rc] = comp_disagree(comps[rc], now, k)
    # else R4: a component required within L that no longer is → Neutral
    return _set(m, comps)


def tick(m: Machine, now, k: K) -> Machine:
    if not m.active:
        return m
    req = components(m.desired)
    comps = m.comp_map()
    for c in req:
        comps[c] = comp_tick(comps[c], now, k)
    hist = [h for h in m.hist if now - h[2] <= k.L]
    return _set(m, comps, hist)


def aggregate(m: Machine) -> str:
    """§4.4.1 aggregate: a pure function of the required components' verdicts."""
    if not m.active:
        return "unknown"                                        # A1
    cm = m.comp_map()
    states = [cm[c] for c in components(m.desired)]
    if all(s.state == "silent" for s in states):
        return "unknown"                                        # A2
    if any(s.state in ("not_following", "silent") for s in states):
        return "not_following"                                  # A3
    if all(s.state == "following" for s in states):
        return "following"                                      # A4
    if any(s.dis_since is not None for s in states):
        return "suspect"                                        # A5
    return "checking"                                           # A6


def step(m: Machine, now, k: K, link=None, desired="keep", rep=None) -> Machine:
    """One instant: link event, desired change, reply, then the tick (fixed order)."""
    if link == "lost":
        m = set_link(m, False, now, k)
    elif link == "ready":
        m = set_link(m, True, now, k)
    if desired != "keep":
        m = set_desired(m, desired, now, k)
    if rep is not None:
        m = reply(m, rep, now, k)
    return tick(m, now, k)


# --- normalisation for exhaustive exploration ---
BASE = 1000


def normalise(m: Machine, now, k: K) -> Machine:
    """Shift times so now = BASE and clamp ages beyond every threshold they are compared with."""
    def clamp(t, cap):
        if t is None:
            return None
        return BASE - min(now - t, cap)
    comps = {}
    for name, c in m.comps:
        if c.not_required_since is None:
            nrs, le = None, clamp(c.last_evidence, k.T_unknown + 1)
        else:
            # not required: the silence clock is paused (C9), so keep its required-time age
            nrs = clamp(c.not_required_since, k.T_unknown + 1)
            le = nrs - min(c.not_required_since - c.last_evidence, k.T_unknown + 1)
        comps[name] = Comp(
            state=c.state, streak_n=min(c.streak_n, N_FOLLOW),
            streak_first=clamp(c.streak_first, k.T_verify),
            dis_since=clamp(c.dis_since, k.T_mismatch),
            last_evidence=le, not_required_since=nrs)
    hist = [(c, v, BASE - (now - u)) for c, v, u in m.hist if now - u <= k.L]
    return replace(m, comps=tuple(sorted(comps.items())), hist=tuple(sorted(hist)))


# --- monitors: properties computed from the raw input stream, not from machine internals ---
@dataclass(frozen=True)
class Mon:
    # per component: (consecutive A replies since last D/silence/re-require, capped N_FOLLOW,
    #                 age of the first of them capped, count of D replies since last verification
    #                 capped 2, age of first such D capped, age of last decisive reply capped)
    pc: tuple = ()
    und: tuple = ()  # per component: age of the current undecided period (fair walk only)
    hit: tuple = ()  # components whose reply this instant was a Disagree completing (a)
    run: tuple = ()  # fair walk: per component, age of its first reply in the current all-Agree run


def mon_get(mon: Mon, c):
    for name, v in mon.pc:
        if name == c:
            return v
    return (0, None, 0, None, False, 0)


def mon_set(mon: Mon, c, v):
    d = dict(mon.pc)
    d[c] = v
    return replace(mon, pc=tuple(sorted(d.items())))


# Discretised environments. Grid values are abstract: 0/1 stand for "this value" and "another
# value"; a reply value never desired within L is a Disagree, which the second value provides.
CONFIGS = {
    # single-component mode: ERG target changes, echoes of both values, failure status,
    # and a foreign page (page 51 echo while ERG is desired)
    "erg": (["keep", None, ("erg", 0), ("erg", 1)],
            [None, ("echo", "erg", 0), ("echo", "erg", 1), ("fail", "erg"), ("echo", "track", 0)]),
    # two-component mode: grade changes, ERG ↔ SIM toggles, per-component echoes and failure
    # (link loss and `desired = None` both deactivate, which clears every variable; that path is
    # covered by "erg";
    # a failure status is classified like a mismatching echo, which value 1 provides)
    "sim": (["keep", ("sim", 0, 0), ("sim", 1, 0), ("erg", 0)],
            [None, ("echo", "track", 0), ("echo", "track", 1), ("echo", "wind", 0),
             ("echo", "wind", 1), ("echo", "erg", 0)]),
}
LINK_EVENTS = {"erg": True, "sim": False}


def classify(m_before: Machine, r, now, k: K):
    """Reply class per required component, recomputed from the desired history (the spec's
    definition), independently of the component machines."""
    req = components(m_before.desired)
    if not m_before.active or r is None:
        return {}
    c = r[1]
    if c in req:
        ok = r[0] == "echo" and recently_desired(m_before, c, r[2], now, k)
        return {c: "A" if ok else "D"}
    if recently_required(m_before, c, now, k):
        return {}
    return {rc: "D" for rc in req}


def check_invariants(m: Machine, mon: Mon, k: K, ctx):
    agg = aggregate(m)
    # (e) activity: active exactly while link Ready and a desired mode exists
    assert m.active == (m.link_ready and m.desired is not None), ("activity", ctx)
    req = components(m.desired) if m.active else {}
    cm = m.comp_map()
    for c in req:
        a_n, a_age, d_n, d_age, a_ok, _ = mon_get(mon, c)
        # (b) following needs N_FOLLOW agreeing replies about every required component
        #     spanning T_verify since its last disagreement, silence or re-requirement:
        #     one stale agreeing reply (or a burst) never yields following.
        if agg == "following":
            assert a_n >= N_FOLLOW and a_age is not None and a_age >= k.T_verify, ("b", c, ctx)
        # (c) recovery: once the last N_FOLLOW replies about every required component agree
        #     and span T_verify, the aggregate is following unless a sibling is silent/D.
        # (a) detection: two D replies about c, T_mismatch apart, with no verification
        #     between them → not_following.
        if c in mon.hit:
            assert agg == "not_following", ("a", c, ctx)
        # single disagreeing reply never yields not_following through c
        if cm[c].state == "not_following":
            assert d_n >= 2, ("single-D", c, ctx)
        # (d) a required component without evidence for T_unknown (of required time) is never
        #     masked: aggregate is not following, and not_following if a sibling still talks.
        if cm[c].state == "silent":
            assert agg in ("unknown", "not_following"), ("d", c, ctx)
        # silence is exactly T_unknown of required time without a reply about c
        assert (cm[c].state == "silent") == (mon_get(mon, c)[5] >= k.T_unknown), ("d-sil", c, ctx)
    if m.active and agg != "unknown":
        good = all(mon_get(mon, c)[4] and mon_get(mon, c)[5] < k.T_unknown for c in req)
        if good:
            assert agg == "following", ("c", ctx)


def mon_update(mon: Mon, m_prev: Machine, m_pre: Machine, m_after: Machine, r, now, k: K) -> Mon:
    """Advance the monitors by one instant: ages +1, then the reply's classes.

    Per component: (a_n, a_age, d_n, d_age, a_ok) where a_n counts consecutive Agree replies
    since the last Disagree/silence/re-requirement (capped), a_age is the age of the first of
    them, d_n/d_age count Disagree replies since the last verification, and a_ok records that an
    Agree arrived while a_n ≥ N_FOLLOW and a_age ≥ T_verify (the recovery precondition)."""
    if not m_after.active:
        return Mon()
    req_prev = components(m_prev.desired) if m_prev.active else {}
    req_after = components(m_after.desired)
    classes = classify(m_pre, r, now, k)
    cm = m_after.comp_map()
    cm_pre = m_pre.comp_map()

    def age(x, cap):
        return None if x is None else min(x + 1, cap)
    out = Mon()
    for c in cm:
        a_n, a_age, d_n, d_age, a_ok, sil = mon_get(mon, c) if m_prev.active else (0, None, 0, None, False, 0)
        a_age, d_age = age(a_age, k.T_verify), age(d_age, k.T_mismatch)
        if c in req_prev:
            sil = min(sil + 1, k.T_unknown)          # required time up to this instant
        if c in req_after and c not in req_prev:
            a_n, a_age, a_ok = 0, None, False      # verification never carries over (C8/C9)
            p = cm_pre[c]
            if not m_prev.active or (p.last_evidence == now and p.dis_since is None
                                     and p.state == "unverified"):
                d_n, d_age, sil = 0, None, 0       # fresh component (C1)
        cls = classes.get(c)
        if cls is not None and sil >= k.T_unknown:
            a_n, a_age, a_ok, d_n, d_age = 0, None, False, 0, None   # evidence too old (C2)
        if cls == "A":
            a_n, a_age = (1, 0) if a_n == 0 else (min(a_n + 1, N_FOLLOW), a_age)
            a_ok = a_n >= N_FOLLOW and a_age >= k.T_verify
            sil = 0
        elif cls == "D":
            a_n, a_age, a_ok = 0, None, False
            d_n, d_age = (1, 0) if d_n == 0 else (min(d_n + 1, 2), d_age)
            sil = 0
            if d_n >= 2 and d_age >= k.T_mismatch:
                out = replace(out, hit=tuple(sorted(set(out.hit) | {c})))
        if c in req_after and sil >= k.T_unknown:
            a_n, a_age, d_n, d_age, a_ok = 0, None, 0, None, False   # silent (C7)
        if cm[c].state == "following":
            d_n, d_age = 0, None
        if c not in req_after:
            # not required: agreement is dropped (C8). A pending Disagree keeps its count; any
            # later Disagree comes after at least one gap step, so its age is ≥ T_mismatch.
            a_n, a_age, a_ok = 0, None, False
            d_age = k.T_mismatch if d_n else None
        out = mon_set(out, c, (a_n, a_age, d_n, d_age, a_ok, sil))
    return out


def explore(k: K, config: str, fair=False, depth=None, limit=3_000_000):
    """Exhaustive BFS over the normalised state space: to the fixpoint, or over every input
    sequence of length ≤ depth when a depth bound is given."""
    start = (Machine(), Mon(), 0)  # third: step parity for the fair SIM schedule
    seen = {start}
    q = deque([(start, 0)])
    max_undecided = {}
    while q:
        (m, mon, parity), level = q.popleft()
        if depth is not None and level >= depth:
            continue
        now = BASE + 1
        mm = replace(m)  # state was normalised at BASE; advance one step
        if fair or not LINK_EVENTS[config]:
            links = [None] if m.link_ready else ["ready"]
        else:
            links = [None, "lost" if m.link_ready else "ready"]
        for link in links:
            for d in CONFIGS[config][0]:
                if fair and d is None:
                    continue
                for r in CONFIGS[config][1]:
                    if fair and not _fair_reply(mm, link, d, r, parity):
                        continue
                    m2 = step(mm, now, k, link=link, desired=d, rep=r)
                    m_pre = mm
                    if link == "lost":
                        m_pre = set_link(mm, False, now, k)
                    elif link == "ready":
                        m_pre = set_link(mm, True, now, k)
                    if d != "keep":
                        m_pre = set_desired(m_pre, d, now, k)
                    mon2 = mon_update(mon, mm, m_pre, m2, r, now, k)
                    if fair:
                        mon2 = _fair_undecided(mon, mon2, m_pre, m2, k, max_undecided)
                        mon2 = _fair_recovery(mon, mon2, mm, m_pre, m2, r, now, k)
                    ctx = (m, mon, link, d, r)
                    check_invariants(m2, mon2, k, ctx)
                    s = (normalise(m2, now, k), replace(mon2, hit=()), (parity + 1) % 2 if fair else 0)
                    if s not in seen:
                        seen.add(s)
                        if len(seen) > limit:
                            raise RuntimeError("state space exceeds limit")
                        q.append((s, level + 1))
    return len(seen), max_undecided


def _fair_reply(m: Machine, link, d, r, parity):
    """Fair environment: link stays Ready once up; each instant carries exactly one reply,
    about a required component at its period (ERG every step, SIM alternating wind/track)."""
    if r is None:
        return False
    mode = m.desired if d == "keep" else d
    if mode is None:
        return False
    req = components(mode)
    if r[1] not in req:
        return False
    if "erg" in req:
        return True
    return r[1] == ("wind" if parity == 0 else "track")


def _fair_undecided(mon_old, mon_new, m_pre, m2, k, max_undecided):
    """Decision deadline: a required component that keeps getting replies at its period is
    decided (following or not_following) within T_decide of becoming undecided."""
    if not m2.active:
        return replace(mon_new, und=())
    old = dict(mon_old.und)
    und = {}
    req_pre = components(m_pre.desired) if m_pre.active else {}
    for c, comp in m2.comps:
        if c not in components(m2.desired):
            continue
        if comp.state in ("following", "not_following"):
            continue
        a = old.get(c, -1) + 1 if c in req_pre else 0
        und[c] = a
        max_undecided[c] = max(max_undecided.get(c, 0), a)
        assert a <= k.T_decide(c), ("deadline", c, a, m2)
    return replace(mon_new, und=tuple(sorted(und.items())))


def _fair_recovery(mon_old, mon_new, m_prev, m_pre, m2, r, now, k):
    """Aggregate deadline (§4.4.1 guarantee 4): during an unbroken run of agreeing replies with an
    unchanged set of required components, the aggregate is following once every required
    component c has had its first reply of the run at least V_c ago. In SIM the two components'
    first replies of a run are at most T_poll apart (the echo schedule), so the aggregate bound is
    T_poll + V = 5 s after the first reply of either."""
    if not m2.active:
        return replace(mon_new, run=())
    req = components(m2.desired)
    same = m_prev.active and components(m_prev.desired).keys() == req.keys()
    run = dict(mon_old.run) if same else {}
    run = {c: a + 1 for c, a in run.items()}
    cls = classify(m_pre, r, now, k)
    if any(v == "D" for v in cls.values()):
        run = {}
    else:
        for c, v in cls.items():
            if v == "A" and c not in run:
                # the schedule: a component joins the run at most T_poll after the first one
                assert all(a <= k.unit for a in run.values()), ("phase offset", run, c, m2)
                run[c] = 0
    # cap ages at the bound they are compared with (keeps the state space finite)
    run = {c: min(a, k.V(c)) for c, a in run.items()}
    if run.keys() == req.keys() and all(a >= k.V(c) for c, a in run.items()):
        assert aggregate(m2) == "following", ("aggregate deadline", run, m2)
    return replace(mon_new, run=tuple(sorted(run.items())))


# --- component walk: one component machine in isolation (C1–C9), exhaustively ---
@dataclass(frozen=True)
class CMon:
    """Independent monitor for one component, driven by the raw inputs only."""
    a_n: int = 0              # consecutive Agree since last Disagree/silence/stop/start (capped)
    a_age: int | None = None  # age of the first of them (capped T_verify)
    a_ok: bool = False        # an Agree arrived with a_n ≥ N_FOLLOW and a_age ≥ T_verify
    d_n: int = 0              # Disagree replies since last verification/silence/start (capped 2)
    d_age: int | None = None  # age of the first of them (capped T_mismatch)
    req_sil: int = 0          # required time since the last reply or start (capped T_unknown)
    gap: int | None = None    # time since c stopped being required (capped T_unknown + 1)


def explore_component(k: K, period=None):
    """Free walk (period None): every step may start, stop or resume the requirement and carry
    no reply, an Agree or a Disagree. Fair walk (period P): always required, a reply of either
    class exactly every P steps; checks the decision deadline. Returns (states, max undecided)."""
    cap_sil, cap_gap = k.T_unknown, k.T_unknown + 1
    start = (fresh(BASE), True, CMon(), 0, 0)   # comp, required, monitor, phase, undecided age
    seen, q, worst = {start}, deque([start]), 0
    while q:
        c, req, mon, phase, und = q.popleft()
        now = BASE + 1
        events = [None] if period else [None, "start", "stop" if req else "resume"]
        replies = [None, "A", "D"]
        if period:
            replies = ["A", "D"] if phase == 0 else [None]
        for ev in events:
            for r in replies:
                c2, req2, mo = c, req, mon
                ages = lambda x, cap: None if x is None else min(x + 1, cap)
                mo = replace(mo, a_age=ages(mo.a_age, k.T_verify), d_age=ages(mo.d_age, k.T_mismatch),
                             gap=ages(mo.gap, cap_gap) if not req else None)
                if ev == "start":
                    c2, req2, mo = fresh(now), True, CMon()
                elif ev == "stop":
                    c2, req2 = comp_stop_required(c2, now), False
                    mo = replace(mo, a_n=0, a_age=None, a_ok=False, gap=0)
                elif ev == "resume":
                    fresh_again = mo.gap is not None and mo.gap + 0 > k.T_unknown
                    c2, req2 = comp_resume_required(c2, now, k), True
                    mo = CMon() if fresh_again else replace(mo, gap=None)
                fresh_again = ev == "resume" and mo == CMon()
                # the interval up to this instant counts as required time iff c was required
                # at its start (a stop counts it, a resume does not); a reply then resets it
                if req and ev != "start":
                    mo = replace(mo, req_sil=min(mo.req_sil + 1, cap_sil))
                hit = stale = False
                pre = c2
                if r is not None and req2:
                    if mo.req_sil >= k.T_unknown:      # monitor's view of stale evidence (C2)
                        stale = True
                        mo = replace(mo, a_n=0, a_age=None, a_ok=False, d_n=0, d_age=None)
                    if r == "A":
                        c2 = comp_agree(c2, now, k)
                        a_n, a_age = (1, 0) if mo.a_n == 0 else (min(mo.a_n + 1, N_FOLLOW), mo.a_age)
                        mo = replace(mo, a_n=a_n, a_age=a_age,
                                     a_ok=a_n >= N_FOLLOW and a_age >= k.T_verify, req_sil=0)
                    else:
                        c2 = comp_disagree(c2, now, k)
                        d_n, d_age = (1, 0) if mo.d_n == 0 else (min(mo.d_n + 1, 2), mo.d_age)
                        hit = d_n >= 2 and d_age >= k.T_mismatch
                        mo = replace(mo, a_n=0, a_age=None, a_ok=False, d_n=d_n, d_age=d_age, req_sil=0)
                if req2:
                    c2 = comp_tick(c2, now, k)
                if req2 and mo.req_sil >= k.T_unknown:
                    mo = replace(mo, a_n=0, a_age=None, a_ok=False, d_n=0, d_age=None)
                if c2.state == "following":
                    mo = replace(mo, d_n=0, d_age=None)
                ctx = (c, req, mon, ev, r)
                # (a) two Disagree T_mismatch apart without verification → not_following
                if hit:
                    assert c2.state == "not_following", ("comp-a", ctx)
                # (b) following only after N_FOLLOW agreeing replies spanning T_verify
                if c2.state == "following":
                    assert mo.a_n >= N_FOLLOW and mo.a_age >= k.T_verify, ("comp-b", ctx)
                # (c) recovery: that evidence while required and not silent → following
                if mo.a_ok and req2 and c2.state != "silent":
                    assert c2.state == "following", ("comp-c", ctx)
                # single Disagree never → not_following
                if c2.state == "not_following":
                    assert mo.d_n >= 2 or (pre.state == "not_following" and r != "D") \
                        or c.state == "not_following", ("comp-single-D", ctx)
                # (d) silence is exactly T_unknown of required time without a reply
                if req2:
                    assert (c2.state == "silent") == (mo.req_sil >= k.T_unknown), ("comp-d", ctx)
                # not_following ends only through verification, silence or a fresh start
                if c.state == "not_following" and c2.state != "not_following":
                    # otherwise only a fresh start (C1: start, or resume after > T_unknown)
                    if c2.state not in ("following", "silent"):
                        assert ev == "start" or (ev == "resume" and fresh_again) or stale, \
                            ("comp-exit", ctx)
                und2 = 0
                if period:
                    und2 = 0 if c2.state in ("following", "not_following") else und + 1
                    worst = max(worst, und2)
                    assert und2 <= k.T_decide_p(period), ("deadline", und2, ctx)
                n = normalise(Machine(active=True, comps=(("c", c2),)), now, k).comps[0][1]
                st = (n, req2, mo, (phase + 1) % period if period else 0, und2)
                if st not in seen:
                    seen.add(st)
                    q.append(st)
    return len(seen), worst


def check_aggregate():
    """A1–A6 over every combination of component states (1 and 2 required components)."""
    states = [("unverified", None), ("unverified", 1), ("following", None),
              ("not_following", None), ("not_following", 1), ("silent", None)]
    for n, mode in ((1, ("erg", 0)), (2, ("sim", 0, 0))):
        names = list(components(mode))
        for combo in __import__("itertools").product(states, repeat=n):
            comps = {nm: Comp(state=st, dis_since=d) for nm, (st, d) in zip(names, combo)}
            m = _set(Machine(link_ready=True, desired=mode, active=True), comps, [])
            agg = aggregate(m)
            sts = [st for st, _ in combo]
            assert (agg == "following") == all(x == "following" for x in sts), combo
            if "silent" in sts and not all(x == "silent" for x in sts):
                assert agg == "not_following", combo          # missing component never masked
            if all(x == "silent" for x in sts):
                assert agg == "unknown", combo
            if agg == "not_following":
                assert any(x in ("not_following", "silent") for x in sts), combo
            if agg == "suspect":
                assert any(d is not None for _, d in combo), combo
    assert aggregate(Machine()) == "unknown"


def check_routing(k: K):
    """R2–R5: every reply touches only its own component, or every required one (R5)."""
    now = BASE
    modes = [("erg", 0), ("erg", 1), ("sim", 0, 0), ("sim", 1, 0), ("sim", 0, 1)]
    replies = [("echo", c, v) for c in ("erg", "track", "wind") for v in (0, 1)] \
        + [("fail", c) for c in ("erg", "track", "wind")]
    for prev in [None] + modes:
        for cur in modes:
            for age in range(0, k.L + 2):
                m = set_link(Machine(), True, now - age - 1, k)
                m = set_desired(m, prev or cur, now - age - 1, k)
                m = set_desired(m, cur, now - age, k)
                for r in replies:
                    m2 = reply(m, r, now, k)
                    before, after = m.comp_map(), m2.comp_map()
                    req = components(cur)
                    changed = {c for c in after if after[c] != before.get(c)}
                    target = r[1]
                    if target in req:
                        assert changed == {target}, (prev, cur, age, r, changed)
                        agree = after[target].streak_n == 1
                        want = r[0] == "echo" and (req[target] == r[2] or any(
                            hc == target and hv == r[2] and now - u <= k.L for hc, hv, u in m.hist))
                        assert agree == want, (prev, cur, age, r)
                    elif recently_required(m, target, now, k):
                        assert not changed, (prev, cur, age, r, changed)             # R4
                    else:
                        assert changed == set(req), (prev, cur, age, r, changed)     # R5


# --- named regression scenarios (every pre-mortem counterexample so far) ---
def run(events, k: K, until):
    """events: {time: dict(link=..., desired=..., rep=...)}; returns aggregate per instant."""
    m, out = Machine(), {}
    for t in range(0, until + 1):
        e = events.get(t, {})
        m = step(m, t, k, link=e.get("link"), desired=e.get("desired", "keep"), rep=e.get("rep"))
        out[t] = aggregate(m)
    return out


def ramp(start, until, stuck=None, lag=0, every=1, k=K()):
    """ERG ramp 100, 101, ... one value per second; replies every second echo the value
    desired `lag` seconds earlier, or `stuck` if given."""
    ev = {0: {"link": "ready", "desired": ("erg", 100)}}
    for s in range(1, until + 1):
        t = s * k.unit
        ev.setdefault(t, {})
        if s % every == 0:
            ev[t]["desired"] = ("erg", 100 + s)
        v = stuck if stuck is not None else 100 + max(0, s - lag)
        ev[t]["rep"] = ("echo", "erg", v)
    return ev


def first(out, state, after=0):
    return next((t for t in sorted(out) if t >= after and out[t] == state), None)


def scenarios():
    k = K()
    res = {}

    # #46 r2: 200→250→200; one delayed reply echoing 200 must not verify (stale reply).
    ev = {0: {"link": "ready", "desired": ("erg", 200)}, 1: {"desired": ("erg", 250)},
          2: {"desired": ("erg", 200)}, 3: {"rep": ("echo", "erg", 200)}}
    out = run(ev, k, 6)
    res["r46_stale_200_250_200"] = all(v != "following" for v in out.values())

    # #54 r1-1: grace restarted on each ramp tick → a stuck trainer was never detected.
    out = run(ramp(0, 30, stuck=100), k, 30)
    t = first(out, "not_following")
    res["r54_1_ramp_stuck_detected"] = t is not None and t <= L_S + 2 * 1 + T_MISMATCH_S + 1

    # #54 r1-2: one stale matching reply flipped the state to following.
    ev = {0: {"link": "ready", "desired": ("erg", 200)}, 3: {"rep": ("echo", "erg", 200)}}
    out = run(ev, k, 4)
    res["r54_1_single_stale_reply"] = all(v != "following" for v in out.values())

    # burst: three agreeing replies within < T_verify never verify (sub-second time).
    k4 = K(unit=4)
    ev = {0: {"link": "ready", "desired": ("erg", 200)}, 1: {"rep": ("echo", "erg", 200)},
          2: {"rep": ("echo", "erg", 200)}, 3: {"rep": ("echo", "erg", 200)},
          4: {"rep": ("echo", "erg", 200)}}
    out = run(ev, k4, 7)
    res["r54_1_burst_never_verifies"] = all(v != "following" for v in out.values())

    # #54 r2-1: a Transition every second with disagreeing replies renewed the grace forever.
    ev = {0: {"link": "ready", "desired": ("erg", 0)}}
    for s in range(1, 30):
        ev[s] = {"desired": ("erg", s % 7 + 10), "rep": ("echo", "erg", 99)}
    out = run(ev, k, 30)
    t = first(out, "not_following")
    res["r54_2_transition_storm_detected"] = t is not None and t <= 1 + T_MISMATCH_S + 1

    # #54 r2-2: second ride on an open connection must activate without a new Link(Ready).
    ev = {0: {"link": "ready", "desired": ("erg", 150)}}
    for s in range(1, 6):
        ev[s] = {"rep": ("echo", "erg", 150)}
    ev[6] = {"desired": None}                      # ride 1 ends, link stays Ready
    ev[8] = {"desired": ("erg", 120)}              # ride 2 starts
    for s in range(9, 14):
        ev[s] = {"rep": ("echo", "erg", 120)}
    out = run(ev, k, 14)
    res["r54_2_second_ride_activates"] = out[7] == "unknown" and out[13] == "following"

    # #54 r2-3: wind (page 50) echoes alone must not verify SIM while track never applies.
    ev = {0: {"link": "ready", "desired": ("erg", 200)}}
    for s in range(1, 4):
        ev[s] = {"rep": ("echo", "erg", 200)}
    ev[4] = {"desired": ("sim", 5, 0)}
    for s in range(5, 25):
        ev[s] = {"rep": ("echo", "wind", 0)}
    out = run(ev, k, 25)
    t = first(out, "not_following", after=4)
    res["r54_2_sim_wind_only"] = (all(out[s] != "following" for s in range(4, 26))
                                  and t is not None and t <= 4 + T_UNKNOWN_S)

    # #54 r3-1: both SIM components expire while telemetry continues, then correct
    # alternating replies resume: recovery must complete (no cross-component deadlock).
    ev = {0: {"link": "ready", "desired": ("sim", 3, 0)}}
    for s in range(1, 7):
        ev[s] = {"rep": ("echo", "wind" if s % 2 else "track", 0 if s % 2 else 3)}
    # silence 7..15 (page 71 silent), then alternating correct replies
    for s in range(16, 30):
        ev[s] = {"rep": ("echo", "wind" if s % 2 else "track", 0 if s % 2 else 3)}
    out = run(ev, k, 30)
    t = first(out, "following", after=16)
    # exact: track first at 16, wind first at 17 (T_poll later), each verified V = 4 s later
    res["r54_3_stale_components_recover"] = (out[14] == "unknown"
                                             and t == 16 + 1 + k.V("track"))

    # #54 r4 (astra): after silence, wind at 10/12/14 s and track at 11/13/15 s →
    # following exactly at 15 s = first reply + T_poll + V (5 s), not 4 s.
    ev = {0: {"link": "ready", "desired": ("sim", 3, 0)}}
    for s in range(10, 30):
        ev[s] = {"rep": ("echo", "wind", 0) if s % 2 == 0 else ("echo", "track", 3)}
    out = run(ev, k, 30)
    res["r54_4_sim_recovery_phase_offset"] = (
        out[9] == "unknown" and first(out, "following") == 15
        and 15 - 10 == 1 + k.V("wind") == 1 + k.V("track"))

    # ERG recovery bound is exact too: after silence, replies every second from 10 s →
    # following exactly V = 2 s after the first.
    ev = {0: {"link": "ready", "desired": ("erg", 150)}}
    for s in range(10, 20):
        ev[s] = {"rep": ("echo", "erg", 150)}
    out = run(ev, k, 20)
    res["r54_4_erg_recovery_exact"] = out[9] == "unknown" and first(out, "following") == 10 + k.V("erg")

    # #54 r3-2: grade changes every second, only lagging track echoes, no wind ever:
    # the missing wind component must surface as not_following.
    ev = {0: {"link": "ready", "desired": ("sim", 0, 0)}}
    for s in range(1, 30):
        ev[s] = {"desired": ("sim", s, 0), "rep": ("echo", "track", s - 1)}
    out = run(ev, k, 30)
    t = first(out, "not_following")
    res["r54_3_lagging_cannot_mask_missing_wind"] = (
        t is not None and t <= T_UNKNOWN_S and all(v != "following" for v in out.values()))

    # #54 r3-3: repeated transitions must not defeat the verification deadline: a trainer
    # lagging 1 s behind a target that changes every second still verifies.
    out = run(ramp(0, 30, lag=1), k, 30)
    t = first(out, "following")
    res["r54_3_transitions_do_not_block_verification"] = (
        t is not None and t <= 1 + 2 + T_VERIFY_S and all(
            out[s] == "following" for s in range(t, 31)))

    # ERG ↔ SIM toggling faster than T_unknown cannot restart the missing-track clock.
    ev = {0: {"link": "ready", "desired": ("sim", 1, 0)}}
    for s in range(1, 60):
        mode = ("sim", 1, 0) if (s // 2) % 2 == 0 else ("erg", 200)
        rep = ("echo", "wind", 0) if mode[0] == "sim" else ("echo", "erg", 200)
        ev[s] = {"desired": mode, "rep": rep}
    out = run(ev, k, 60)
    res["r54_3_kind_toggle_cannot_mask_track"] = first(out, "not_following") is not None and all(
        out[s] != "following" for s in range(60) if (s // 2) % 2 == 0)

    # silent trainer: checking for T_unknown after activation, then unknown.
    out = run({0: {"link": "ready", "desired": ("erg", 100)}}, k, 12)
    res["silent_trainer_checking_then_unknown"] = (
        all(out[s] == "checking" for s in range(0, T_UNKNOWN_S))
        and all(out[s] == "unknown" for s in range(T_UNKNOWN_S, 13)))

    # reconnect mid-ramp: unknown → checking → following.
    ev = ramp(0, 30, lag=0)
    ev[10] = {"link": "lost"}
    ev[13] = {"link": "ready", "desired": ("erg", 113)}
    for s in range(11, 13):
        ev[s].pop("rep", None)
    out = run(ev, k, 30)
    res["reconnect_mid_ramp"] = (out[11] == "unknown" and out[13] == "checking"
                                 and first(out, "following", after=13) is not None)
    return res


def check_table_order(adr_path):
    """The ADR's component table (§4.4.1) and this model evaluate the same rows in the same order:
    row ids equal TABLE_ORDER, the reply rows appear in REPLY_ROWS order with the same input class,
    and exactly C2 is the pre-step."""
    import re
    text = open(adr_path, encoding="utf-8").read()
    section = text[text.index("#### 4.4.1"):text.index("## 5. Decision")]
    rows = re.findall(r"^\| (C\d) \| ([^|]*)\|", section, flags=re.M)
    ids = tuple(r[0] for r in rows)
    assert ids == TABLE_ORDER, ("ADR table order", ids, TABLE_ORDER)
    when = dict(rows)
    reply_ids = [r.id for r in REPLY_ROWS]
    assert list(ids[ids.index(reply_ids[0]):ids.index(reply_ids[-1]) + 1]) == reply_ids, \
        ("reply rows", ids, reply_ids)
    for row in REPLY_ROWS:
        cell = when[row.id]
        assert ("pre-step" in cell) == row.pre, ("pre-step marker", row.id, cell)
        if not row.pre:
            klass = "D" if cell.strip().startswith("Disagree") else "A" if cell.strip().startswith("Agree") else "?"
            assert klass == row.on, ("input class", row.id, cell, row.on)


SIM_FREE_DEPTH = 6  # every input sequence of length ≤ 6 (137 k states, ~30 s)


def main(argv):
    ok = True
    import os
    adr = argv[1] if len(argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "0002-ble-trainer.md")
    check_table_order(adr)
    print("table order: ADR §4.4.1 rows == model evaluation order (C1–C9, C2 pre-step), ok")
    for name, passed in scenarios().items():
        print(f"scenario {name}: {'ok' if passed else 'FAIL'}")
        ok &= passed
    k = K()
    check_aggregate()
    print("aggregate A1–A6: ok over every component-state combination")
    check_routing(k)
    print("routing R2–R5: ok over every mode pair, history age and reply")
    n, _ = explore_component(k)
    print(f"component walk (free, C1–C9 + requirement toggles): {n} states to fixpoint, ok")
    for p in (1, 2):
        n, worst = explore_component(k, period=p)
        print(f"component walk (fair, P={p}s): {n} states to fixpoint, "
              f"max undecided {worst}s <= T_decide {k.T_decide_p(p * k.unit)}s, ok")
    for config, fair, depth in (("erg", False, None), ("erg", True, None),
                                ("sim", True, None), ("sim", False, SIM_FREE_DEPTH)):
        n, und = explore(k, config, fair=fair, depth=depth)
        bound = "fixpoint" if depth is None else f"all sequences of length <= {depth}"
        print(f"machine walk {config} fair={fair}: {n} states ({bound}), ok; max undecided {und}")
    print("all checks passed" if ok else "SCENARIO FAILURES")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
