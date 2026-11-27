//! The easer (ADR-0002 §7.10.2 steps 1–9, X5a, X5; §7.10.3), on Ruckig through `rsruckig`.
//!
//! Ruckig is used as an online trajectory generator is meant to be used: a trajectory is
//! calculated when its input changes (a new frame or piece, a lead, chase or brake starting, a
//! cut, an event or an engagement) and is otherwise sampled tick by tick (step 7).
//!
//! On a refusal (Ruckig's error, or an answer onto a held line or a trackable ramp slower than
//! [`landing_bound`]/[`ramp_bound`]) the calculation is retried once on the same tick with every
//! limit tightened by [`TWIN`]`·max(1, |limit|)` (the twin, X5a). If the twin is refused too the
//! easer is **degraded** on that tick (X5): one tick of the closed-form time-optimal stop from its
//! state, and at rest a hold; Ruckig (and its twin) is tried first again on every tick. A plan,
//! cap or state that is not a number is degraded the same way.
#![expect(
    clippy::many_single_char_names,
    reason = "the ADR's notation: y, v, a (the state), k (the tick), m (the cap), c and s (a line)"
)]
use super::line::{Piece, line_at};
use super::profile::{landing_bound, profile_at, profile_extrema, ramp_bound, slope_change};
use super::{
    BRAKE_SLACK, CAP_SLACK, CUT_TOL_A, CUT_TOL_V, CUT_TOL_W, Cap, DT, EPS, KEY_TOL, Limits,
    ROUND_SNAP, State, TARGET_MARGIN, TWIN, clamp, tick_f,
};
use crate::tuning::SlewTuning;
use rsruckig::prelude::*;

/// A named exception of the step (ADR-0002 §7.10.2 step 8 and X5), named only when it changes the
/// state by more than floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// X2: a step that would end above the line and higher than it started stops instead.
    X2,
    /// X3: the cap; a command above `M` steps down to it.
    X3,
    /// X4: nothing is below 0 W.
    X4,
    /// X5: degraded (Ruckig and its twin refused, or the plan, cap or state is not a number).
    X5,
}

/// The easer's counters, for the ride's diagnostics and log (ADR-0002 §7.10.2: every refusal,
/// every twin recovery, every rejected answer and every degraded tick is counted).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    /// Ruckig refusals, the twin's included, and answers not accepted as slow.
    pub errors: u64,
    /// Refusals the twin recovered (X5a).
    pub twins: u64,
    /// Needed calculations refused by Ruckig and its twin: a degraded tick each.
    pub plan_failures: u64,
    /// Answers slower than P4/P7's bound for the move, not accepted (from either calculation).
    pub slow: u64,
    /// Trajectory calculations requested (each makes one or two Ruckig calls).
    pub recalcs: u64,
    /// Ticks whose plan, cap or state was not a number (degraded).
    pub invalid: u64,
    /// Degraded ticks (`erg_easer_degraded` raised), whatever the cause.
    pub degraded_ticks: u64,
}

/// What the easer is given for tick `k`.
#[derive(Clone, Copy, Debug)]
pub struct TickInput<'a> {
    /// The tick.
    pub k: i64,
    /// The line from tick `k` on ([`segments`](super::segments)); its first piece is in force on
    /// `k`. Empty only past the plan's end, where the easer holds degraded (no plan known).
    pub line: &'a [Piece],
    /// The cap in force ([`InputLayer`](super::InputLayer)).
    pub cap: Cap,
    /// An engagement or a discontinuity on this tick (an offset applied, a jump of the line, a
    /// raised cap that lets it jump): re-plan from the state, and no vertex carry-over (step 2).
    /// Other events re-plan through the frame or the cap they change.
    pub replan: bool,
    /// The inputs' or the plan's fault on this tick (an invalid value, a target that is not a
    /// number): degraded.
    pub fault: bool,
}

/// The trajectory in force: what it aims at, by which identity it is re-planned (step 7).
#[derive(Clone, Copy, Debug)]
enum Head {
    Brake,
    Meet(f64, f64),
    Lead(i64, f64, f64),
    Line(f64, f64),
}

#[derive(Clone, Copy, Debug)]
struct Key {
    head: Head,
    vmax: f64,
    vmin: f64,
    vel: Option<f64>,
    m: f64,
}

/// Two key values are one input up to floating point ([`KEY_TOL`] relative), so two
/// representations of one line never count as a new input.
#[expect(
    clippy::float_cmp,
    reason = "equal infinities (no cap) are the same key"
)]
fn same_f(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= KEY_TOL * 1.0_f64.max(a.abs()).max(b.abs())
}

fn same_head(a: Head, b: Head) -> bool {
    match (a, b) {
        (Head::Brake, Head::Brake) => true,
        (Head::Meet(c1, s1), Head::Meet(c2, s2)) | (Head::Line(c1, s1), Head::Line(c2, s2)) => {
            same_f(c1, c2) && same_f(s1, s2)
        }
        (Head::Lead(k1, r1, s1), Head::Lead(k2, r2, s2)) => {
            k1 == k2 && same_f(r1, r2) && same_f(s1, s2)
        }
        _ => false,
    }
}

fn same_key(a: &Key, b: Option<&Key>) -> bool {
    b.is_some_and(|b| {
        same_head(a.head, b.head)
            && same_f(a.vmax, b.vmax)
            && same_f(a.vmin, b.vmin)
            && match (a.vel, b.vel) {
                (None, None) => true,
                (Some(x), Some(y)) => same_f(x, y),
                _ => false,
            }
            && same_f(a.m, b.m)
    })
}

/// A cut from `old` to `new`, named only if it changes the state by more than floating point.
fn cut(old: State, new: State, name: Cut, exc: Option<Cut>) -> (State, Option<Cut>) {
    let material = (old.y - new.y).abs() > CUT_TOL_W
        || (old.v - new.v).abs() > CUT_TOL_V
        || (old.a - new.a).abs() > CUT_TOL_A;
    (new, if material { Some(name) } else { exc })
}

/// The trajectory's state at time `t`.
fn at(traj: &Trajectory<1>, t: f64) -> (f64, f64, f64) {
    let mut p = DataArrayOrVec::<f64, 1>::new(None, 0.0);
    let mut v = DataArrayOrVec::<f64, 1>::new(None, 0.0);
    let mut a = DataArrayOrVec::<f64, 1>::new(None, 0.0);
    traj.at_time(
        t,
        &mut Some(&mut p),
        &mut Some(&mut v),
        &mut Some(&mut a),
        &mut None,
        &mut None,
    );
    (p[0], v[0], a[0])
}

/// One Ruckig calculation's request, in the frame of a line `c + fs·t`.
#[derive(Clone, Copy, Debug)]
struct Request {
    st: State,
    rel: f64,
    fs: f64,
    vmax: f64,
    vmin: f64,
    vel: Option<f64>,
    meet: bool,
}

/// crank's policy around Ruckig (ADR-0002 §7.10.2): the target frame, the limits, re-planning,
/// safe braking, the cuts X2–X4, the twin X5a and degraded mode X5.
pub struct Easer {
    lim: Limits,
    otg: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    traj: Trajectory<1>,
    spare: Trajectory<1>,
    diag: Diagnostics,
    k: i64,
    degraded: bool,
    y_ok: f64,
    meet: bool,
    meet_fs: f64,
    frame: Option<(f64, f64, bool)>,
    prev_piece: Option<Piece>,
    key: Option<Key>,
    elapsed: f64,
    base: f64,
    braking: bool,
    /// Fault injection (ADR-0002 §7.10.7 item 6), tests only.
    #[cfg(test)]
    pub(crate) faults: super::tests::harness::Faults,
}

impl Easer {
    /// An easer for `tuning`'s limits (the mapping of ADR-0002 §7.10.2).
    #[must_use]
    pub fn new(tuning: &SlewTuning) -> Self {
        Self {
            lim: Limits::from(tuning),
            otg: Ruckig::new(None, DT),
            input: InputParameter::new(None),
            traj: Trajectory::new(None),
            spare: Trajectory::new(None),
            diag: Diagnostics::default(),
            k: 0,
            degraded: false,
            y_ok: 0.0,
            meet: false,
            meet_fs: 0.0,
            frame: None,
            prev_piece: None,
            key: None,
            elapsed: 0.0,
            base: 0.0,
            braking: false,
            #[cfg(test)]
            faults: super::tests::harness::Faults::default(),
        }
    }

    /// The limits Ruckig is handed.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.lim
    }
    /// `erg_easer_degraded` on the last tick (ADR-0002 §7.10.5 `ergEaserDegraded`).
    #[must_use]
    pub fn degraded(&self) -> bool {
        self.degraded
    }
    /// The counters so far.
    #[must_use]
    pub fn diagnostics(&self) -> Diagnostics {
        self.diag
    }

    /// An engagement's state (ADR-0002 §7.10.3): `origin` (item 5's 3 s power, 0 W without a
    /// value) clamped to `[0, M]`, at rest. Pass it to [`step`](Self::step) with
    /// [`TickInput::replan`]. An origin that is not a number is passed on as it is: the step
    /// treats it as a fault (the last command sent, at rest, degraded).
    #[must_use]
    pub fn engagement(origin: f64, cap: Cap) -> State {
        State::at_rest(if origin.is_finite() {
            clamp(origin, cap.get())
        } else {
            origin
        })
    }

    /// The engagement's completion rule (ADR-0002 §7.10.3): complete on the tick the command is on
    /// the line (`|line_w − y| ≤ EPS`); behind a line it cannot follow (`slope` at or beyond the
    /// rider's rate), on the first tick it moves towards the line at the full rate (the chase's
    /// target velocity, [`TARGET_MARGIN`] inside the rate).
    #[must_use]
    pub fn engagement_complete(&self, st: &State, line_w: f64, slope: f64) -> bool {
        let full = |r: f64| (st.v.abs() - r).abs() <= 2.0 * TARGET_MARGIN * r.max(1.0);
        (line_w - st.y).abs() <= EPS
            || (slope >= self.lim.r_up && st.y < line_w && st.v > 0.0 && full(self.lim.r_up))
            || (slope <= -self.lim.r_dn && st.y > line_w && st.v < 0.0 && full(self.lim.r_dn))
    }

    #[cfg(test)]
    fn injected(&mut self) -> bool {
        self.faults.calc(self.k)
    }
    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build injects faults here")]
    fn injected(&mut self) -> bool {
        false
    }
    #[cfg(test)]
    fn twin_answers(&self) -> bool {
        self.faults.twin_answers
    }
    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build injects faults here")]
    fn twin_answers(&self) -> bool {
        false
    }
    #[cfg(test)]
    fn fail_brake(&self) -> bool {
        self.faults.fail_brake
    }
    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "the test build injects faults here")]
    fn fail_brake(&self) -> bool {
        false
    }

    /// Ruckig's input for `q`, the twin's (every limit tightened) if `twin`.
    fn fill(&mut self, q: &Request, twin: bool) {
        let lim = self.lim;
        let tight = |x: f64| {
            if twin {
                x - (TWIN * 1.0_f64.max(x.abs())).copysign(x)
            } else {
                x
            }
        };
        let (vmax, vmin, amax, jmax) = (
            tight(q.vmax),
            tight(q.vmin),
            tight(lim.a_max),
            tight(lim.j_max),
        );
        let i = &mut self.input;
        i.control_interface = if q.vel.is_some() {
            ControlInterface::Velocity
        } else {
            ControlInterface::Position
        };
        i.current_position[0] = q.rel;
        i.current_velocity[0] = q.st.v - q.fs;
        i.current_acceleration[0] = q.st.a;
        i.target_position[0] = 0.0;
        // Onto the line with its slope, or (meet) at rest on it: −fs in the line's frame.
        let tv = match q.vel {
            None if q.meet => -q.fs,
            None => 0.0,
            Some(vt) => vt - q.fs,
        };
        // Strictly inside the velocity limits (Ruckig 0.19.4's segfault boundary).
        let (hi, lo) = (vmax - q.fs, vmin - q.fs);
        i.target_velocity[0] = tv
            .max(lo + TARGET_MARGIN * 1.0_f64.max(lo.abs()))
            .min(hi - TARGET_MARGIN * 1.0_f64.max(hi.abs()));
        i.target_acceleration[0] = 0.0;
        i.max_velocity[0] = hi;
        i.min_velocity = Some(DataArrayOrVec::new(None, lo));
        i.max_acceleration[0] = amax;
        i.min_acceleration = Some(DataArrayOrVec::new(None, -amax));
        i.max_jerk[0] = jmax;
    }

    /// One Ruckig calculation into the spare trajectory; true on an answer.
    fn one(&mut self, q: &Request, twin: bool, injected: bool) -> bool {
        if injected {
            return false;
        }
        self.fill(q, twin);
        matches!(
            self.otg.calculate(&self.input, &mut self.spare),
            Ok(RuckigResult::Working | RuckigResult::Finished)
        )
    }

    /// Ruckig, then on a refusal its twin (X5a), on the same tick, into the spare trajectory. An
    /// answer onto a held line or a trackable ramp, with the rider's limits, slower than the bound
    /// for the move is a refusal too, from either. False if both are refused (each counted).
    #[expect(
        clippy::float_cmp,
        reason = "the rider's own limits are recognised by identity, as the model does"
    )]
    fn calc(&mut self, q: &Request) -> bool {
        let injected = self.injected();
        let lim = self.lim;
        for twin in [false, true] {
            let mut ok = self.one(q, twin, injected && !(twin && self.twin_answers()));
            if ok
                && q.vel.is_none()
                && q.vmax == lim.r_up
                && q.vmin == -lim.r_dn
                && -lim.r_dn < q.fs
                && q.fs < lim.r_up
            {
                let bound = if q.fs == 0.0 {
                    landing_bound(&lim, -q.rel, q.st.v, q.st.a)
                } else {
                    ramp_bound(&lim, -q.rel, q.st.v, q.st.a, q.fs)
                };
                if self.spare.get_duration() > bound {
                    ok = false;
                    self.diag.slow += 1;
                }
            }
            if ok {
                self.diag.twins += u64::from(twin);
                return true;
            }
            self.diag.errors += 1;
        }
        false
    }

    /// Calculate into the spare and swap it in only on an answer, so a refused calculation leaves
    /// the trajectory in force untouched.
    fn plan(&mut self, q: &Request) -> bool {
        if !self.calc(q) {
            self.diag.plan_failures += 1;
            return false;
        }
        std::mem::swap(&mut self.traj, &mut self.spare);
        true
    }

    /// The trajectory in force at `elapsed`; past its end at constant slope (velocity mode), or
    /// (a meet) at rest where it met the line, else on the line.
    fn sample(&self, vel_mode: bool) -> (f64, f64, f64) {
        let (dur, tt) = (self.traj.get_duration(), self.elapsed);
        if tt >= dur {
            let (p, v, _) = at(&self.traj, dur);
            if self.meet && !vel_mode {
                return (p - self.meet_fs * (tt - dur), -self.meet_fs, 0.0);
            }
            let extra = if vel_mode { v * (tt - dur) } else { 0.0 };
            return (p + extra, v, 0.0);
        }
        at(&self.traj, tt)
    }

    /// Safe braking's predicate: the state rises or will rise (`v > 0`, or `a > 0` with
    /// `v + a²/(2·j_max) > 0`).
    fn rises(&self, st: &State) -> bool {
        st.v > 1e-9 || (st.a > 1e-9 && st.v + st.a * st.a / (2.0 * self.lim.j_max) > 1e-9)
    }

    /// Degraded mode's step: one tick of the closed-form time-optimal stop, at rest a hold.
    fn stop(&self, st: &State) -> State {
        let (y, v, a) = profile_at(
            st.y,
            st.v,
            st.a,
            &slope_change(st.v, st.a, 0.0, &self.lim),
            DT,
        );
        State { y, v, a }
    }

    /// The stop from `st` (the state at tick `k`), and `st` itself, stay at or below the line known
    /// ahead (by at most [`BRAKE_SLACK`]) and the cap (by at most [`CAP_SLACK`]), tick by tick,
    /// until at rest.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a stop lasts a few seconds: far below i64::MAX ticks"
    )]
    fn brake_ok(&self, st: &State, k: i64, segs: &[Piece], m: f64) -> bool {
        let ceiling = |kk: i64| (line_at(segs, kk, m) + BRAKE_SLACK).min(m + CAP_SLACK);
        if st.y > ceiling(k) {
            return false;
        }
        let prof = slope_change(st.v, st.a, 0.0, &self.lim);
        let dur = prof.duration();
        let n = (dur / DT).ceil() as i64;
        (1..=n).all(|i| {
            let y = profile_at(st.y, st.v, st.a, &prof, (tick_f(i) * DT).min(dur)).0;
            y <= ceiling(k + i)
        })
    }

    /// The stop from `st` stays at or above 0 W.
    fn floor_ok(&self, st: &State) -> bool {
        profile_extrema(st.y, st.v, st.a, &slope_change(st.v, st.a, 0.0, &self.lim)).0 >= -1e-9
    }

    /// X3 before the step (round 15–16): the command steps down to `M`, keeping its falling slope
    /// only as far as it can still stop at or above the line there, and no negative curvature once
    /// the slope is reduced.
    fn at_cap(&self, st: &State, m: f64, line: f64) -> State {
        let floor = line.min(m);
        let ok = |v: f64, a: f64| {
            profile_extrema(m, v, a, &slope_change(v, a, 0.0, &self.lim)).0 >= floor - 1e-9
        };
        let (v, a) = (st.v.min(0.0), st.a.min(0.0));
        if ok(v, a) {
            return State { y: m, v, a };
        }
        let (mut lo, mut hi) = (v, 0.0);
        for _ in 0..60 {
            let mid = f64::midpoint(lo, hi);
            if ok(mid, 0.0) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        State {
            y: m,
            v: hi,
            a: 0.0,
        }
    }

    /// Step 6 below a falling line (rounds 16–17): where safe braking rejects a rising step, meet
    /// the line at rest one tick of its fall early (`|s|·Δt` below it), checked by safe braking like
    /// any step. Not kept as a plan. `None`: not below a falling line, led, refused or rejected.
    fn meet_below(
        &mut self,
        st: &State,
        nst: &State,
        k: i64,
        segs: &[Piece],
        m: f64,
    ) -> Option<State> {
        let lim = self.lim;
        let (c, fs, lead) = self.frame?;
        if !self.rises(nst) {
            return None;
        }
        let rel = st.y - (c + fs * tick_f(k - 1) * DT);
        if lead || !(-lim.r_dn < fs && fs < 0.0) || rel >= 0.0 {
            return None;
        }
        let early = fs * DT;
        let q = Request {
            st: *st,
            rel: rel - early,
            fs,
            vmax: lim.r_up,
            vmin: -lim.r_dn,
            vel: None,
            meet: true,
        };
        if !self.calc(&q) {
            return None;
        }
        std::mem::swap(&mut self.traj, &mut self.spare);
        self.meet = true;
        self.meet_fs = fs;
        self.braking = false;
        self.key = Some(Key {
            head: Head::Meet(c, fs),
            vmax: lim.r_up,
            vmin: -lim.r_dn,
            vel: None,
            m,
        });
        self.elapsed = DT;
        let (p, v, a) = self.sample(false);
        let cand = State {
            y: c + fs * tick_f(k) * DT + p + early,
            v: v + fs,
            a,
        };
        if (self.rises(&cand) && !self.brake_ok(&cand, k, segs, m))
            || (cand.v < -1e-9 && !self.floor_ok(&cand))
        {
            self.key = None;
            self.meet = false;
            return None;
        }
        Some(cand)
    }

    /// The command for tick `tick.k` from the state `st` at tick `k − 1` (ADR-0002 §7.10.2's
    /// order), and the tick's named exception.
    pub fn step(&mut self, st: State, tick: &TickInput<'_>) -> (State, Option<Cut>) {
        let (st, exc) = self.step_inner(st, tick);
        self.diag.degraded_ticks += u64::from(self.degraded);
        (st, exc)
    }

    #[expect(
        clippy::float_cmp,
        clippy::too_many_lines,
        reason = "the per-tick rules of §7.10.2 in their order; a lead is recognised by the frame changing at all"
    )]
    fn step_inner(&mut self, st: State, tick: &TickInput<'_>) -> (State, Option<Cut>) {
        let lim = self.lim;
        let (k, m) = (tick.k, tick.cap.get());
        self.k = k;
        let mut exc = None;
        // A state that is not a number is replaced at rest: its value if a number, else the last
        // command sent (round 15).
        let state_ok = st.y.is_finite() && st.v.is_finite() && st.a.is_finite();
        let mut st = if state_ok {
            st
        } else {
            State::at_rest(clamp(if st.y.is_finite() { st.y } else { self.y_ok }, m))
        };
        // A plan, cap or state that is not a number is a fault: degraded (round 14). So is a tick
        // with no line in force (past the plan's end: no plan known).
        if tick.fault
            || !state_ok
            || tick.line.is_empty()
            || tick
                .line
                .iter()
                .any(|q| !(q.r0.is_finite() && q.s.is_finite()))
        {
            self.diag.invalid += 1;
            self.degraded = true;
            self.key = None;
            self.braking = false;
            self.prev_piece = None;
            let nst = self.stop(&st);
            return self.after(&st, nst, k, &[], m, f64::INFINITY, Some(Cut::X5), true);
        }
        // Step 2: the step from k−1 to k belongs to the piece in force over it: at a vertex on
        // tick k (a kink, not where the clamp starts or ends), still the previous piece.
        let in_force = tick.line[0];
        let mut segs: Vec<Piece> = Vec::with_capacity(tick.line.len() + 1);
        if let Some(prev) = self.prev_piece
            && in_force.k0 == k
            && in_force.joined
            && !tick.replan
            && (prev.at(k) - in_force.r0).abs() < 1e-9
        {
            segs.push(prev);
        }
        segs.extend_from_slice(tick.line);
        self.prev_piece = Some(in_force);
        // Step 1: X3, the cap is never eased.
        if st.y > m {
            let capped = self.at_cap(&st, m, clamp(segs[0].at(k), m));
            (st, exc) = cut(st, capped, Cut::X3, exc);
        }
        let cur = segs[0];
        let r_k = clamp(cur.at(k), m);
        let s_cur = cur.s;
        let within = |s: f64| -lim.r_dn <= s && s <= lim.r_up;
        let trackable = |s: f64| -lim.r_dn < s && s < lim.r_up;
        let mut fs = s_cur;
        let mut c = r_k - fs * tick_f(k) * DT;
        let (mut vmax, mut vmin) = (lim.r_up, -lim.r_dn);
        // Step 3: the lead into a vertex that ends or reverses a rise, or ends a fall.
        if let Some(&nxt) = segs.get(1) {
            let nv = clamp(nxt.r0, m);
            let lead_head = Head::Lead(nxt.k0, nxt.r0, nxt.s);
            let off = st.y - (r_k - s_cur * DT);
            let leads = nxt.joined
                && trackable(nxt.s)
                && ((nxt.s < s_cur && off.abs() <= EPS)
                    || (s_cur < 0.0 && s_cur < nxt.s && nxt.s <= 0.0));
            let cap_v = || {
                if nxt.s < s_cur {
                    (lim.r_up.min(s_cur), -lim.r_dn)
                } else {
                    (lim.r_up, (-lim.r_dn).max(s_cur))
                }
            };
            if self.key.is_some_and(|key| same_head(key.head, lead_head)) && within(s_cur) {
                fs = nxt.s;
                c = nv - nxt.s * tick_f(nxt.k0) * DT;
                (vmax, vmin) = cap_v();
            } else if leads {
                // From the tick the S-curve to the next slope needs (half of it, plus one tick);
                // closed form, so the timing never depends on a Ruckig answer; ties lead.
                let curve = slope_change(st.v, st.a, nxt.s, &lim).duration();
                if tick_f(nxt.k0 - (k - 1)) * DT <= curve / 2.0 + DT + KEY_TOL {
                    fs = nxt.s;
                    c = nv - nxt.s * tick_f(nxt.k0) * DT;
                    (vmax, vmin) = if within(s_cur) {
                        cap_v()
                    } else {
                        (lim.r_up, -lim.r_dn)
                    };
                }
            }
        }
        let lead = fs != s_cur || c != r_k - s_cur * tick_f(k) * DT;
        let line = |kk: i64| c + fs * tick_f(kk) * DT;
        let brake = self.braking;
        let mut meet = false;
        self.frame = Some((c, fs, lead));
        let vel_target = if brake {
            Some(0.0)
        } else if trackable(fs) && fs > 0.0 && st.y - line(k) > EPS {
            // Step 5: above a rising line, come down and meet it at rest (round 14).
            meet = true;
            None
        } else if trackable(fs) {
            None
        } else {
            // Step 4: a line steeper than the rate: chase it at the full rate, never away from
            // it; on the line counts as behind it.
            let gap = line(k) - st.y;
            Some(if gap > -EPS && fs > 0.0 {
                lim.r_up
            } else if gap < EPS && fs < 0.0 {
                -lim.r_dn
            } else {
                0.0
            })
        };
        let head = if brake {
            Head::Brake
        } else if meet {
            Head::Meet(c, fs)
        } else if lead {
            Head::Lead(segs[1].k0, segs[1].r0, segs[1].s)
        } else {
            Head::Line(c, fs)
        };
        let key = Key {
            head,
            vmax,
            vmin,
            vel: vel_target,
            m,
        };
        // Step 7: re-plan only on a change.
        if !same_key(&key, self.key.as_ref()) || exc.is_some() || tick.replan {
            self.diag.recalcs += 1;
            let q = match vel_target {
                None => Request {
                    st,
                    rel: st.y - line(k - 1),
                    fs,
                    vmax,
                    vmin,
                    vel: None,
                    meet,
                },
                Some(vt) => Request {
                    st,
                    rel: 0.0,
                    fs: 0.0,
                    vmax: lim.r_up,
                    vmin: -lim.r_dn,
                    vel: Some(vt),
                    meet: false,
                },
            };
            if !self.plan(&q) {
                // X5: degraded; no key kept, so Ruckig is tried first again next tick.
                self.degraded = true;
                self.key = None;
                self.braking = false;
                let nst = self.stop(&st);
                return self.after(&st, nst, k, &segs, m, r_k, exc.or(Some(Cut::X5)), true);
            }
            if vel_target.is_some() {
                self.base = st.y;
            }
            self.key = Some(key);
            self.elapsed = DT;
            self.meet = meet;
            self.meet_fs = fs;
        } else {
            self.elapsed += DT;
        }
        self.degraded = false;
        let (p, v, a) = self.sample(vel_target.is_some());
        let nst = if vel_target.is_none() {
            State {
                y: line(k) + p,
                v: v + fs,
                a,
            }
        } else {
            State {
                y: self.base + p,
                v,
                a,
            }
        };
        self.after(&st, nst, k, &segs, m, r_k, exc, brake)
    }

    /// Steps 6 and 8: safe braking, then the cuts.
    #[expect(
        clippy::too_many_arguments,
        reason = "the model's _after: the step's frame, line, cap and exception"
    )]
    fn after(
        &mut self,
        st: &State,
        mut nst: State,
        k: i64,
        segs: &[Piece],
        m: f64,
        r_k: f64,
        mut exc: Option<Cut>,
        brake: bool,
    ) -> (State, Option<Cut>) {
        let lim = self.lim;
        // Step 6: a step that rises or will rise is taken only if its stop stays at or below the
        // line ahead; a falling one only if its stop stays at or above 0 W.
        if !brake
            && ((self.rises(&nst) && !self.brake_ok(&nst, k, segs, m))
                || (nst.v < -1e-9 && !self.floor_ok(&nst)))
        {
            let fail_brake = self.fail_brake();
            if fail_brake {
                self.diag.errors += 2;
                self.diag.plan_failures += 1;
            }
            let met = if fail_brake {
                None
            } else {
                self.meet_below(st, &nst, k, segs, m)
            };
            let brake_q = Request {
                st: *st,
                rel: 0.0,
                fs: 0.0,
                vmax: lim.r_up,
                vmin: -lim.r_dn,
                vel: Some(0.0),
                meet: false,
            };
            if let Some(met) = met {
                nst = met;
            } else if !fail_brake && self.plan(&brake_q) {
                self.braking = true;
                self.meet = false;
                self.base = st.y;
                self.elapsed = DT;
                self.key = Some(Key {
                    head: Head::Brake,
                    vmax: lim.r_up,
                    vmin: -lim.r_dn,
                    vel: Some(0.0),
                    m,
                });
                let (p, v, a) = self.sample(true);
                nst = State {
                    y: self.base + p,
                    v,
                    a,
                };
            } else {
                // The braking trajectory refused by Ruckig and its twin: degraded, the same stop.
                nst = self.stop(st);
                self.key = None;
                exc = exc.or(Some(Cut::X5));
                self.degraded = true;
            }
        }
        if self.braking && nst.v.abs() <= 1e-9 {
            self.braking = false;
            if self.traj.get_duration() > self.elapsed + 1e-9 {
                self.key = None;
            }
        }
        // Step 8. X2: a step that would rise and end above the line stops instead.
        if nst.y > r_k + EPS && (nst.v > 1e-9 || nst.y > st.y + 1e-7) {
            (nst, exc) = cut(nst, State::at_rest(st.y), Cut::X2, exc);
        }
        // A landing within ROUND_SNAP outside [0, M] is rounding: snapped, not cut (round 12).
        if m < nst.y && nst.y <= m + ROUND_SNAP {
            nst.y = m;
        } else if -ROUND_SNAP <= nst.y && nst.y < 0.0 {
            nst.y = 0.0;
        }
        if nst.y < 0.0 {
            (nst, exc) = cut(nst, State::at_rest(0.0), Cut::X4, exc);
        }
        if nst.y > m {
            (nst, exc) = cut(nst, State::at_rest(m), Cut::X3, exc);
        }
        if exc.is_some_and(|e| e != Cut::X5) {
            self.key = None;
        }
        if nst.y.is_finite() {
            self.y_ok = nst.y;
        }
        (nst, exc)
    }
}

/// A trajectory's (position, velocity, acceleration).
#[cfg(test)]
type Sampled = (f64, f64, f64);

/// Test access to the rules one by one (the model's suites call its private methods).
#[cfg(test)]
impl Easer {
    fn position_request(st: State, rel: f64, fs: f64, vmax: f64, vmin: f64, meet: bool) -> Request {
        Request {
            st,
            rel,
            fs,
            vmax,
            vmin,
            vel: None,
            meet,
        }
    }
    /// One Ruckig calculation (the first, or the twin), nothing injected.
    pub(crate) fn test_one(
        &mut self,
        st: State,
        rel: f64,
        fs: f64,
        vmax: f64,
        vmin: f64,
        twin: bool,
    ) -> bool {
        self.one(
            &Self::position_request(st, rel, fs, vmax, vmin, false),
            twin,
            false,
        )
    }
    /// The limits Ruckig is handed for the twin: (`max_velocity`, `min_velocity`,
    /// `max_acceleration`, `max_jerk`).
    pub(crate) fn test_twin_limits(&mut self, vmax: f64, vmin: f64) -> (f64, f64, f64, f64) {
        self.fill(
            &Self::position_request(State::at_rest(41.25), 41.25, 0.0, vmax, vmin, false),
            true,
        );
        let i = &self.input;
        (
            i.max_velocity[0],
            i.min_velocity.as_ref().map_or(f64::NAN, |v| v[0]),
            i.max_acceleration[0],
            i.max_jerk[0],
        )
    }
    /// Ruckig then its twin with the slow-answer rule, at tick `k` (for fault injection).
    pub(crate) fn test_calc(&mut self, k: i64, st: State, rel: f64, fs: f64) -> bool {
        self.k = k;
        let (vmax, vmin) = (self.lim.r_up, -self.lim.r_dn);
        self.calc(&Self::position_request(st, rel, fs, vmax, vmin, false))
    }
    /// The meet onto a line of slope `s` from `st`, `rel` W off it, sampled at `elapsed` past the
    /// trajectory's end: (the trajectory's end state, the sample).
    pub(crate) fn test_meet_end(
        &mut self,
        st: State,
        rel: f64,
        s: f64,
        past: f64,
    ) -> Option<(Sampled, Sampled)> {
        let (vmax, vmin) = (self.lim.r_up, -self.lim.r_dn);
        if !self.plan(&Self::position_request(st, rel, s, vmax, vmin, true)) {
            return None;
        }
        self.meet = true;
        self.meet_fs = s;
        let dur = self.traj.get_duration();
        let end = at(&self.traj, dur);
        self.elapsed = dur + past;
        Some((end, self.sample(false)))
    }
    /// Steps 6 and 8 on their own (the hard clamp).
    pub(crate) fn test_after(
        &mut self,
        st: State,
        nst: State,
        k: i64,
        segs: &[Piece],
        m: f64,
        r_k: f64,
    ) -> (State, Option<Cut>) {
        self.after(&st, nst, k, segs, m, r_k, None, true)
    }
    /// Safe braking's check on its own.
    pub(crate) fn test_brake_ok(&self, st: State, k: i64, segs: &[Piece], m: f64) -> bool {
        self.brake_ok(&st, k, segs, m)
    }
}
