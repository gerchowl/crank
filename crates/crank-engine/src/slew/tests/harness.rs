//! The model's `run()` and `check()` (`docs/adr/models/erg_slew.py`), ported: a plan through the
//! input layer and the easer, tick by tick, and every property of ADR-0002 §7.10.6 judged on the
//! sent positions.
use crate::slew::profile::{landing_bound, profile_at, ramp_bound, slope_change};
use crate::slew::{
    Block, Cut, DT, EPS, Easer, InputLayer, Limits, SETTLE_AFTER_TICKS, State, TickInput, clamp,
    plan_finite, segments, tick_f,
};
use crate::tuning::SlewTuning;
use std::collections::{BTreeMap, BTreeSet};

/// The checks' floating-point tolerance (the model's `TOL`).
pub(crate) const TOL: f64 = 1e-6;

/// Fault injection (ADR-0002 §7.10.7 item 6): which Ruckig calculations are refused as if Ruckig
/// had returned an error. An injected refusal fails the twin as well unless `twin_answers`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Faults {
    /// Refuse this share of calculations from tick `from` (until `until`), drawn from `seed`.
    pub share: f64,
    pub seed: u64,
    pub from: i64,
    pub until: Option<i64>,
    /// Every calculation on these ticks is refused.
    pub ticks: BTreeSet<i64>,
    /// An injected refusal fails the first calculation only.
    pub twin_answers: bool,
    /// Every braking plan of step 6 is refused, its twin too.
    pub fail_brake: bool,
    /// Each calculation's draw, in order (to replay a refusal pattern in the model).
    pub log: Vec<(i64, bool)>,
}

impl Faults {
    /// Whether the calculation now (one draw per calculation, as the model's) is refused.
    pub fn calc(&mut self, k: i64) -> bool {
        let refused = self.ticks.contains(&k)
            || (self.share > 0.0
                && k >= self.from
                && self.until.is_none_or(|u| k < u)
                && self.uniform() < self.share);
        self.log.push((k, refused));
        refused
    }
    /// From tick `from` on, every calculation refused (the twin too).
    pub fn persistent(from: i64) -> Self {
        Self {
            share: 1.0,
            from,
            ..Self::default()
        }
    }
    /// Every calculation on `ticks` refused.
    pub fn on(ticks: impl IntoIterator<Item = i64>) -> Self {
        Self {
            ticks: ticks.into_iter().collect(),
            ..Self::default()
        }
    }
    fn uniform(&mut self) -> f64 {
        let mut r = Rng(self.seed);
        let x = r.next_f64();
        self.seed = r.0;
        x
    }
}

/// splitmix64: a small deterministic generator for fault injection and sampled tests.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(pub u64);

impl Rng {
    /// Uniform in [0, 1).
    #[expect(clippy::cast_precision_loss, reason = "53 bits to a uniform f64")]
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1_u64 << 53) as f64
    }
    /// Uniform in [lo, hi).
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
    /// An index in 0..n.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "small n"
    )]
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_f64() * n as f64) as usize).min(n - 1)
    }
    /// One of `xs`.
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

/// An event's kind (the model's `Event.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `value` is the multiplier.
    Offset,
    /// `value` is the cap `M`.
    Cap,
    /// Skip to the next block.
    Skip,
    /// An engagement from origin `value`.
    Engage,
}

/// An event on a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ev {
    pub tick: i64,
    pub kind: Kind,
    pub value: f64,
}

pub(crate) fn ev(tick: i64, kind: Kind, value: f64) -> Ev {
    Ev { tick, kind, value }
}

/// A plan to run (the model's `run` arguments).
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub tuning: SlewTuning,
    pub blocks: Vec<Block>,
    pub events: Vec<Ev>,
    pub origin: Option<f64>,
    pub m0: f64,
    pub y0: Option<f64>,
    pub v0: f64,
    pub faults: Faults,
}

impl Plan {
    pub fn new(tuning: SlewTuning, blocks: Vec<Block>) -> Self {
        Self {
            tuning,
            blocks,
            events: Vec::new(),
            origin: None,
            m0: 2000.0,
            y0: None,
            v0: 0.0,
            faults: Faults::default(),
        }
    }
    pub fn events(mut self, events: Vec<Ev>) -> Self {
        self.events = events;
        self
    }
    pub fn origin(mut self, origin: f64) -> Self {
        self.origin = Some(origin);
        self
    }
    pub fn cap(mut self, m0: f64) -> Self {
        self.m0 = m0;
        self
    }
    pub fn y0(mut self, y0: f64) -> Self {
        self.y0 = Some(y0);
        self
    }
    pub fn faults(mut self, faults: Faults) -> Self {
        self.faults = faults;
        self
    }
}

/// A tuning `(rise, factor, ease)`.
pub(crate) fn tu(rise: u16, fall_factor: f64, ease_s: f64) -> SlewTuning {
    SlewTuning {
        rise_w_per_s: rise,
        fall_factor,
        ease_s,
    }
}
pub(crate) fn defaults() -> SlewTuning {
    SlewTuning::default()
}
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "plan durations are small and non-negative"
)]
fn ticks_of(secs: f64) -> u32 {
    (secs / DT).round() as u32
}
pub(crate) fn steady(w: f64, secs: f64) -> Block {
    Block {
        ticks: ticks_of(secs),
        from_w: w,
        to_w: w,
    }
}
pub(crate) fn ramp(a: f64, b: f64, secs: f64) -> Block {
    Block {
        ticks: ticks_of(secs),
        from_w: a,
        to_w: b,
    }
}

/// One run's per-tick record (the model's `Trace`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Trace {
    pub r: Vec<f64>,
    pub y: Vec<f64>,
    pub v: Vec<f64>,
    pub a: Vec<f64>,
    pub on: Vec<bool>,
    pub settle: Vec<bool>,
    pub exc: Vec<Option<Cut>>,
    pub reset: Vec<bool>,
    pub fell: Vec<bool>,
    pub m: Vec<f64>,
    pub unforeseen: Vec<bool>,
    pub origin: Vec<f64>,
    pub failed: Vec<bool>,
    pub degraded: Vec<bool>,
    pub flat: Vec<Option<(f64, i64)>>,
    pub piece: Vec<(i64, f64, i64, bool)>,
    pub events: Vec<bool>,
    pub errors: u64,
    pub slow: u64,
    pub twins: u64,
    pub recalcs: u64,
    pub y_init: f64,
    pub v_init: f64,
    /// Each calculation's fault-injection draw (tick, refused), in order.
    pub calc_log: Vec<(i64, bool)>,
}

fn total(blocks: &[Block]) -> i64 {
    blocks.iter().map(|b| i64::from(b.ticks)).sum()
}

/// Run a plan (the model's `run`): the input layer, the line, the easer, settle.
#[expect(clippy::too_many_lines, reason = "the model's run(), in its order")]
pub(crate) fn run(p: &Plan) -> Trace {
    let mut blocks: Vec<Block> = p.blocks.iter().filter(|b| b.ticks > 0).copied().collect();
    let mut evs: BTreeMap<i64, Vec<Ev>> = BTreeMap::new();
    for e in &p.events {
        evs.entry(e.tick).or_default().push(*e);
    }
    let mut ez = Easer::new(&p.tuning);
    ez.faults = p.faults.clone();
    let mut inputs = InputLayer::new(p.m0);
    let mut tr = Trace::default();
    let mut st: Option<State> = None;
    let mut pending = p.origin.is_some();
    let mut unforeseen = p.origin.is_some();
    let mut settle_left = 0_u32;
    let (mut prev_r, mut prev_s) = (None::<f64>, 0.0);
    let mut k = 0_i64;
    while k < total(&blocks) {
        let (mut disc, mut reset) = (false, false);
        inputs.begin_tick();
        let here = evs.get(&k).cloned().unwrap_or_default();
        if !here.is_empty() {
            unforeseen = true;
        }
        let mut ordered = here.clone();
        ordered.sort_by_key(|e| e.kind != Kind::Cap); // caps first, otherwise in order
        for e in &ordered {
            match e.kind {
                Kind::Offset => disc |= inputs.offset(e.value),
                Kind::Cap => inputs.cap(e.value),
                Kind::Skip => {
                    // the block in force: cut it at k, if it has started and is not the last
                    let mut k0 = 0_i64;
                    let found = blocks.iter().position(|b| {
                        let inside = k0 <= k && k < k0 + i64::from(b.ticks);
                        if !inside {
                            k0 += i64::from(b.ticks);
                        }
                        inside
                    });
                    if let Some(idx) = found {
                        let b = blocks[idx];
                        let done = k - k0;
                        if done > 0 && idx + 1 < blocks.len() {
                            blocks[idx] = Block {
                                ticks: u32::try_from(done).expect("small"),
                                from_w: b.from_w,
                                to_w: b.from_w + b.slope() * tick_f(done) * DT,
                            };
                        }
                    }
                }
                Kind::Engage => {
                    if st.is_some() {
                        if inputs.engage(e.value) {
                            st = Some(Easer::engagement(e.value, inputs.cap_in_force()));
                            reset = true;
                            pending = true;
                        } else if !reset {
                            st = Some(State::at_rest(e.value));
                            reset = true;
                            pending = true;
                        }
                    }
                }
            }
        }
        let me = inputs.cap_in_force();
        let mult = inputs.multiplier();
        let fault = !plan_finite(&blocks, k, mult) || inputs.fault();
        let segs = segments(&blocks, k, mult, me.get());
        let r = clamp(segs[0].at(k), me.get());
        if segs[0].k0 == k && k > 0 && !segs[0].joined {
            disc = true;
        }
        if st.is_none() {
            if let Some(o) = p.origin {
                st = Some(Easer::engagement(o, me));
                reset = true;
            } else {
                let s0 = State {
                    y: p.y0.unwrap_or(r),
                    v: p.v0,
                    a: 0.0,
                };
                tr.y_init = s0.y;
                tr.v_init = s0.v;
                st = Some(s0);
            }
        }
        if here.iter().any(|e| e.kind == Kind::Cap)
            && prev_r.is_some_and(|pr| r > pr + prev_s * DT + 1e-6)
        {
            disc = true; // a raised cap lets the line jump up
        }
        let cur = st.expect("set above");
        let origin_k = if reset { cur.y } else { f64::NAN };
        let failures_before = ez.diagnostics().plan_failures;
        let (nst, exc) = ez.step(
            cur,
            &TickInput {
                k,
                line: &segs,
                cap: me,
                replan: reset || disc,
                fault,
            },
        );
        st = Some(nst);
        let failed_now = ez.diagnostics().plan_failures > failures_before;
        let on = (r - nst.y).abs() <= EPS;
        let unforeseen_now = unforeseen;
        let landed = on && (nst.v - segs[0].s).abs() <= 1e-6 && nst.a.abs() <= 1e-5;
        if landed && here.is_empty() {
            unforeseen = false;
        }
        if disc {
            pending = true;
        }
        if !on {
            settle_left = 0;
        } else if pending {
            pending = false;
            settle_left = SETTLE_AFTER_TICKS;
        }
        let settle = !on || settle_left > 0 || pending;
        if on && settle_left > 0 {
            settle_left -= 1;
        }
        tr.r.push(r);
        tr.y.push(nst.y);
        tr.v.push(nst.v);
        tr.a.push(nst.a);
        tr.on.push(on);
        tr.settle.push(settle);
        tr.exc.push(exc);
        tr.reset.push(reset);
        tr.fell.push(prev_r.is_some_and(|pr| r < pr - 1e-12));
        tr.m.push(me.get());
        tr.unforeseen.push(unforeseen_now);
        tr.origin.push(origin_k);
        tr.failed.push(failed_now);
        tr.degraded.push(ez.degraded());
        tr.flat
            .push((segs[0].s == 0.0).then_some((r, segs[0].k0 + segs[0].n - 1)));
        tr.piece.push((
            segs[0].k0,
            segs[0].s,
            segs[0].k0 + segs[0].n - 1,
            segs[0].k0 == k && !segs[0].joined,
        ));
        tr.events.push(!here.is_empty());
        prev_r = Some(r);
        prev_s = segs[0].s;
        k += 1;
    }
    let d = ez.diagnostics();
    tr.errors = d.errors;
    tr.slow = d.slow;
    tr.twins = d.twins;
    tr.recalcs = d.recalcs;
    tr.calc_log = std::mem::take(&mut ez.faults.log);
    tr
}

/// The checks' findings (the model's `Report`).
#[derive(Debug, Default)]
pub(crate) struct Report {
    pub fails: Vec<String>,
    pub recoveries: u64,
    pub degraded_runs: u64,
    pub above_checked: u64,
    pub plans: u64,
    pub twins: u64,
    pub errors: u64,
}

impl Report {
    pub fn fail(&mut self, msg: String) {
        if self.fails.len() < 25 {
            self.fails.push(msg);
        }
    }
    pub fn assert_clean(&self) {
        assert!(
            self.fails.is_empty(),
            "{} failures:\n{}",
            self.fails.len(),
            self.fails.join("\n")
        );
    }
}

/// The slope change each cut may make (ADR-0002 §7.10.2's table).
pub(crate) fn bound_of(lim: &Limits, c: Cut) -> f64 {
    let step = lim.a_max * DT;
    match c {
        Cut::X2 => lim.r_up + step,
        Cut::X3 => lim.r_up.max(lim.r_dn) + step,
        Cut::X4 | Cut::X5 => lim.r_dn + step,
    }
}

fn is_cut(c: Option<Cut>) -> bool {
    matches!(c, Some(Cut::X2 | Cut::X3 | Cut::X4))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "bounds are finite and small"
)]
fn ticks_ceil(secs: f64) -> i64 {
    (secs / DT).ceil() as i64
}

fn idx(k: i64) -> usize {
    usize::try_from(k).expect("a tick index")
}

/// Any of `xs[lo..=hi]` (clipped to the vector).
fn any(xs: &[bool], lo: i64, hi: i64) -> bool {
    let n = i64::try_from(xs.len()).expect("small");
    (lo.max(0)..=hi.min(n - 1)).any(|j| xs[idx(j)])
}

fn cap_window(tr: &Trace, k: i64) -> bool {
    (1..=2).any(|d| {
        let j = k - d;
        j >= 1 && tr.m[idx(j)] < tr.m[idx(j - 1)] - 1e-12 && tr.y[idx(j - 1)] > tr.m[idx(j)] + 1e-9
    })
}

/// Every property on the sent positions `y_k` (the model's `check`): slope, curvature and jerk
/// are their finite differences, so a cut cannot hide behind the easer's state.
#[expect(clippy::too_many_lines, reason = "the model's check(), in its order")]
pub(crate) fn check(tr: &Trace, t: &SlewTuning, rep: &mut Report, name: &str) {
    rep.plans += 1;
    rep.twins += tr.twins;
    rep.errors += tr.errors;
    let lim = Limits::from(t);
    // The limits as ADR-0002 §7.10.2 states them, computed here and not taken from the easer.
    let rise = f64::from(t.rise_w_per_s);
    let (a_lim, j_lim) = (2.0 * rise / t.ease_s, 4.0 * rise / (t.ease_s * t.ease_s));
    let (tol_v, tol_a, tol_j) = (1e-6, 1e-5, 1e-4);
    let (mut ys, mut cut): (Vec<f64>, Vec<bool>) = (Vec::new(), Vec::new());
    for (ku, &y) in tr.y.iter().enumerate() {
        let k = i64::try_from(ku).expect("small");
        let (r, exc, m) = (tr.r[ku], tr.exc[ku], tr.m[ku]);
        if !(y.is_finite() && tr.v[ku].is_finite() && tr.a[ku].is_finite()) {
            rep.fail(format!("{name} k={k}: NaN"));
            return;
        }
        if tr.reset[ku] {
            let o = tr.origin[ku];
            (ys, cut) = (vec![o; 3], vec![false; 3]);
        } else if k == 0 {
            let y0 = tr.y_init;
            if tr.v_init == 0.0 {
                (ys, cut) = (vec![y0; 3], vec![false; 3]);
            } else {
                (ys, cut) = (vec![y0], vec![false]);
            }
        }
        let prev = *ys.last().expect("seeded");
        ys.push(y);
        let this_cut = is_cut(exc);
        cut.push(this_cut);
        let n = ys.len() - 1;
        let vs = (ys[n] - ys[n - 1]) / DT;
        let cap_drop = exc == Some(Cut::X3)
            && k > 0
            && tr.m[ku] < tr.m[ku - 1] - 1e-12
            && !tr.reset[ku]
            && prev > tr.m[ku] + 1e-9;
        if !cap_drop && !(-lim.r_dn - tol_v..=lim.r_up + tol_v).contains(&vs) {
            rep.fail(format!("{name} k={k}: P2 sent slope {vs:.4}"));
        }
        if n >= 2 {
            let vs0 = (ys[n - 1] - ys[n - 2]) / DT;
            let curv = (vs - vs0) / DT;
            let jerk = (n >= 3).then(|| {
                let vs00 = (ys[n - 2] - ys[n - 3]) / DT;
                ((vs - vs0) - (vs0 - vs00)) / DT / DT
            });
            let mut within =
                curv.abs() <= a_lim + tol_a && jerk.is_none_or(|j| j.abs() <= j_lim + tol_j);
            let culprit = if cut[n] { exc } else { None };
            if culprit.is_none() && !within {
                for d in 1..=2_usize {
                    if n >= d + 2 && cut[n - d] {
                        let kc = ku.wrapping_sub(d);
                        let dv_sent =
                            ((ys[n - d] - ys[n - d - 1]) - (ys[n - d - 1] - ys[n - d - 2])).abs()
                                / DT;
                        let dv_state = if k - i64::try_from(d).expect("1 or 2") >= 1 {
                            (tr.v[kc] - tr.v[kc - 1]).abs()
                        } else {
                            0.0
                        };
                        let dv_c = dv_sent.max(dv_state);
                        let curv_ok =
                            curv.abs() <= a_lim + if d == 1 { dv_c / DT } else { 0.0 } + tol_a;
                        let jerk_ok =
                            jerk.is_none_or(|j| j.abs() <= j_lim + 2.0 * dv_c / (DT * DT) + tol_j);
                        if curv_ok && jerk_ok {
                            within = true;
                        }
                        break;
                    }
                }
            }
            if cap_window(tr, k) {
                within = true;
            }
            if cap_drop {
                let dvs = (tr.v[ku] - tr.v[ku - 1]).abs();
                if dvs > bound_of(&lim, Cut::X3) + tol_v {
                    rep.fail(format!(
                        "{name} k={k}: X3 at a cap drop changed the slope by {dvs:.3}"
                    ));
                }
            } else if !within && culprit.is_none() {
                rep.fail(format!(
                    "{name} k={k}: P3 sent curvature {curv:.3} (<= {a_lim:.3}) / jerk {jerk:?} (<= {j_lim:.3})"
                ));
            } else if !within && this_cut {
                let prev_cap_drop =
                    k > 1 && tr.m[ku - 1] < tr.m[ku - 2] - 1e-12 && ys[n - 2] > tr.m[ku - 1] + 1e-9;
                let dv = if prev_cap_drop {
                    (tr.v[ku] - tr.v[ku - 1]).abs()
                } else {
                    (vs - vs0).abs()
                };
                let c = exc.expect("a cut");
                if dv > bound_of(&lim, c) + tol_v {
                    rep.fail(format!(
                        "{name} k={k}: {c:?} slope change {dv:.3} > bound {:.3}",
                        bound_of(&lim, c)
                    ));
                }
            }
        }
        // Domains: X2–X4 only while an unforeseeable event is pending (every cut tick).
        if this_cut && !tr.unforeseen[ku] {
            rep.fail(format!(
                "{name} k={k}: {exc:?} outside its domain (no unforeseeable event pending)"
            ));
        }
        // P5
        if y > r + EPS + TOL && y > prev + TOL {
            rep.fail(format!(
                "{name} k={k}: P5 rose to above the line y={y:.3} R={r:.3}"
            ));
        }
        // P6
        if k > 0 && !tr.reset[ku] {
            let e0 = (prev - tr.r[ku - 1] - EPS).max(0.0);
            let e1 = (y - r - EPS).max(0.0);
            if e1 > e0 + TOL && !tr.fell[ku] {
                rep.fail(format!(
                    "{name} k={k}: P6 excess grew without a falling line ({e0:.3}->{e1:.3})"
                ));
            }
        }
        if y < -TOL || y > m + TOL {
            rep.fail(format!("{name} k={k}: y={y:.3} outside [0, M={m}]"));
        }
        if tr.failed[ku] && !tr.degraded[ku] {
            rep.fail(format!(
                "{name} k={k}: Ruckig and its twin refused and the easer is not degraded"
            ));
        }
        if tr.degraded[ku] && exc.is_none() {
            rep.fail(format!("{name} k={k}: degraded without its tag"));
        }
        if exc == Some(Cut::X4) && (tr.v[ku] < -TOL || tr.a[ku] < -TOL) {
            rep.fail(format!("{name} k={k}: X4 left a falling slope at 0 W"));
        }
    }
    check_landing(tr, &lim, rep, name);
    check_above(tr, &lim, rep, name);
    check_state(tr, &lim, rep, name);
    check_cap_floor(tr, &lim, rep, name);
    check_recovery(tr, &lim, rep, name);
    check_degraded(tr, &lim, rep, name);
}

fn len_i(tr: &Trace) -> i64 {
    i64::try_from(tr.y.len()).expect("small")
}

/// P4/P7 on every plan: wherever a flat piece starts, the command reaches it within
/// `landing_bound`, if the piece and the plan last that long without a further event.
#[expect(
    clippy::float_cmp,
    reason = "the same flat piece, exactly (the model's ==)"
)]
pub(crate) fn check_landing(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let n = len_i(tr);
    for k in 1..n {
        let ku = idx(k);
        let Some((val, last)) = tr.flat[ku] else {
            continue;
        };
        if tr.flat[ku - 1].is_some_and(|(v0, _)| v0 == val) && !tr.events[ku] && !tr.reset[ku] {
            continue;
        }
        let (y0, v0, a0) = if tr.reset[ku] {
            (tr.origin[ku], 0.0, 0.0)
        } else {
            (tr.y[ku - 1], tr.v[ku - 1], tr.a[ku - 1])
        };
        let delta = val - y0;
        if delta.abs() <= EPS && v0.abs() < 1e-9 {
            continue;
        }
        let bound = landing_bound(lim, delta, v0, a0);
        let nb = ticks_ceil(bound);
        if any(&tr.degraded, k, k + nb) {
            continue;
        }
        if k + nb > last.min(n - 1)
            || any(&tr.events, k + 1, k + nb)
            || any(&tr.reset, k + 1, k + nb)
        {
            continue;
        }
        let ok = |y: f64| {
            if delta < 0.0 {
                y <= val + EPS
            } else {
                (y - val).abs() <= EPS
            }
        };
        if !(k..=k + nb).any(|j| ok(tr.y[idx(j)])) {
            rep.fail(format!(
                "{name} k={k}: P4/P7 not at the held line {val:.2} W within {bound:.2} s (from {y0:.2} W, slope {v0:.2} W/s)"
            ));
        }
    }
}

/// D5: from the recovery tick, on (from below) or at or below (from above) the line within
/// `landing_bound` / `ramp_bound`.
pub(crate) fn check_recovery(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let n = len_i(tr);
    for k in 1..n {
        let ku = idx(k);
        if !tr.degraded[ku - 1] || tr.degraded[ku] || tr.reset[ku] || tr.events[ku] {
            continue;
        }
        let (_, s, last, _) = tr.piece[ku];
        let (y0, v0, a0) = (tr.y[ku - 1], tr.v[ku - 1], tr.a[ku - 1]);
        let (delta, bound) = if s == 0.0 {
            let delta = tr.r[ku] - y0;
            if delta.abs() <= EPS && v0.abs() < 1e-9 {
                continue;
            }
            (delta, landing_bound(lim, delta, v0, a0))
        } else if -lim.r_dn < s && s < lim.r_up {
            let delta = (tr.r[ku] - s * DT) - y0;
            if delta.abs() <= EPS && (v0 - s).abs() < 1e-9 {
                continue;
            }
            (delta, ramp_bound(lim, delta, v0, a0, s))
        } else {
            continue;
        };
        let end = k + ticks_ceil(bound);
        if end > last.min(n - 1)
            || any(&tr.events, k + 1, end)
            || any(&tr.reset, k + 1, end)
            || any(&tr.degraded, k, end)
        {
            continue;
        }
        rep.recoveries += 1;
        let ok = |j: i64| {
            let (y, r) = (tr.y[idx(j)], tr.r[idx(j)]);
            if delta < 0.0 {
                y <= r + EPS
            } else {
                (y - r).abs() <= EPS
            }
        };
        if !(k..=end).any(ok) {
            rep.fail(format!(
                "{name} k={k}: X5 recovery not on the line (slope {s:.2} W/s) within {bound:.2} s (from {y0:.2} W, {delta:+.2} W off, slope {v0:.2} W/s)"
            ));
        }
    }
}

/// P7 on every line (round 14): after a downward step a command above the line is at or below
/// it within `landing_bound` (held) or `ramp_bound` (a trackable ramp).
pub(crate) fn check_above(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let n = len_i(tr);
    for k in 1..n {
        let ku = idx(k);
        let (_, s, last, jumped) = tr.piece[ku];
        if tr.reset[ku] || !(tr.events[ku] || jumped) || !(-lim.r_dn < s && s < lim.r_up) {
            continue;
        }
        let (y0, v0, a0) = (tr.y[ku - 1], tr.v[ku - 1], tr.a[ku - 1]);
        let delta = (tr.r[ku] - s * DT) - y0;
        if delta >= -EPS {
            continue;
        }
        let bound = if s == 0.0 {
            landing_bound(lim, delta, v0, a0)
        } else {
            ramp_bound(lim, delta, v0, a0, s)
        };
        let end = k + ticks_ceil(bound);
        if end > last.min(n - 1)
            || any(&tr.events, k + 1, end)
            || any(&tr.reset, k + 1, end)
            || any(&tr.degraded, k, end)
        {
            continue;
        }
        rep.above_checked += 1;
        if !(k..=end).any(|j| tr.y[idx(j)] <= tr.r[idx(j)] + EPS) {
            rep.fail(format!(
                "{name} k={k}: P7 still above the line (slope {s:.2} W/s) after {bound:.2} s (from {y0:.2} W, {delta:+.2} W off, slope {v0:.2} W/s)"
            ));
        }
    }
}

/// X3 never undershoots (rounds 16–17): after a cap cut, the command does not fall below the line
/// it capped, `min(line, M)`, by more than ε until it has landed on the line (moving with it), the
/// piece changes, an event, an engagement or degraded mode.
pub(crate) fn check_cap_floor(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let n = len_i(tr);
    let ahead = ticks_ceil(lim.r_up.max(lim.r_dn) / lim.a_max + lim.a_max / lim.j_max);
    for k in 0..n {
        let ku = idx(k);
        if tr.exc[ku] != Some(Cut::X3) {
            continue;
        }
        for j in k..n {
            let ju = idx(j);
            if j > k
                && (tr.events[ju]
                    || tr.reset[ju]
                    || tr.degraded[ju]
                    || is_cut(tr.exc[ju])
                    || tr.piece[ju].0 != tr.piece[ku].0)
            {
                break;
            }
            if j + ahead >= n {
                break;
            }
            if j > k
                && (tr.y[ju - 1] - tr.r[ju - 1]).abs() <= EPS
                && (tr.v[ju - 1] - tr.piece[ju - 1].1).abs() <= 1e-6
                && tr.a[ju - 1].abs() <= 1e-5
            {
                break;
            }
            let ahead_min = (j..=j + ahead)
                .map(|i| tr.r[idx(i)])
                .fold(f64::INFINITY, f64::min);
            let floor = tr.r[ku].min(tr.m[ku]).min(ahead_min);
            if tr.y[ju] < floor - EPS && (j == k || tr.y[ju] < tr.y[ju - 1] - 1e-9) {
                rep.fail(format!(
                    "{name} k={j}: below the capped line after X3 at k={k}: {:.3} W < {floor:.3} W - ε",
                    tr.y[ju]
                ));
                return;
            }
        }
    }
}

/// The state the easer plans from is the state it sent: over a tick at piecewise-constant jerk,
/// `Δy = Δt·(v0 + v1)/2 − (a1 − a0)·Δt²/12` within `j_max·Δt³/6`.
pub(crate) fn check_state(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let tol = lim.j_max * DT * DT * DT / 6.0 + 1e-6;
    for ku in 1..tr.y.len() {
        if tr.reset[ku] || is_cut(tr.exc[ku]) {
            continue;
        }
        let want =
            DT * (tr.v[ku - 1] + tr.v[ku]) / 2.0 - (tr.a[ku] - tr.a[ku - 1]) * DT * DT / 12.0;
        if (tr.y[ku] - tr.y[ku - 1] - want).abs() > tol {
            rep.fail(format!(
                "{name} k={ku}: state not the one sent: Δy {:.6} W, from the slope and curvature {want:.6} W",
                tr.y[ku] - tr.y[ku - 1]
            ));
            return;
        }
    }
}

/// D4: from its first tick, degraded mode follows the closed-form stop from the state it had,
/// to 1e-6 W, then holds; until the recovery, an event, an engagement or a cut.
pub(crate) fn check_degraded(tr: &Trace, lim: &Limits, rep: &mut Report, name: &str) {
    let n = tr.y.len();
    let mut k = 0;
    while k < n {
        if !tr.degraded[k] || is_cut(tr.exc[k]) {
            k += 1;
            continue;
        }
        let (y0, v0, a0) = if tr.reset[k] {
            (tr.origin[k], 0.0, 0.0)
        } else if k == 0 {
            (tr.y_init, tr.v_init, 0.0)
        } else {
            (tr.y[k - 1], tr.v[k - 1], tr.a[k - 1])
        };
        let prof = slope_change(v0, a0, 0.0, lim);
        rep.degraded_runs += 1;
        let mut j = k;
        while j < n
            && tr.degraded[j]
            && !is_cut(tr.exc[j])
            && (j == k || (!tr.events[j] && !tr.reset[j]))
        {
            let steps = i64::try_from(j - k + 1).expect("small");
            let want = profile_at(y0, v0, a0, &prof, tick_f(steps) * DT).0;
            if (tr.y[j] - want).abs() > 1e-6 {
                rep.fail(format!(
                    "{name} k={j}: X5 degraded off the closed-form stop from tick {k} (y={:.6}, the stop {want:.6})",
                    tr.y[j]
                ));
                break;
            }
            j += 1;
        }
        k = j.max(k + 1);
    }
}

/// Run and check a plan; the trace for further assertions.
pub(crate) fn run_checked(p: &Plan, rep: &mut Report, name: &str) -> Trace {
    let tr = run(p);
    check(&tr, &p.tuning, rep, name);
    tr
}

/// The tuning grid of the model (`GRID`): every valid tuning over a coarse lattice.
pub(crate) fn grid() -> Vec<SlewTuning> {
    let mut out = Vec::new();
    for r in [5_u16, 8, 10, 15, 20, 30, 40, 50] {
        for f in [1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
            for e in [1.0, 1.5, 2.0, 2.75, 3.0, 4.0] {
                if f64::from(r) * f >= 20.0 {
                    out.push(tu(r, f, e));
                }
            }
        }
    }
    out
}

/// The corners of the bounds (the model's `CORNERS`).
pub(crate) fn corners() -> Vec<SlewTuning> {
    vec![
        tu(5, 4.0, 4.0),
        tu(50, 1.0, 1.0),
        tu(50, 4.0, 1.0),
        tu(50, 1.0, 4.0),
        tu(5, 4.0, 1.0),
        tu(20, 1.0, 4.0),
        tu(50, 4.0, 4.0),
        tu(20, 1.0, 1.0),
    ]
}
