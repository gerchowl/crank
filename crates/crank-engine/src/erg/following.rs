//! The trainer-following machine (ADR-0002 §4.4.1, #54): one small machine per commanded
//! component, reply routing R1–R5, and an aggregate verdict that is a pure function of the
//! component verdicts (A1–A6).
//!
//! This is a literal port of the executable model `docs/adr/models/trainer_following.py`.
//! The reply rows are an ordered table ([`REPLY_ROWS`]); `check_table_order` in the model
//! keeps the ADR table and the model in lockstep, and the `refines_the_model` test ties
//! this table, the constants and the verdict traces to the fixture the model emits
//! (`--emit-fixture`), so the two cannot drift silently. Time is monotonic microseconds
//! (`MonoTime.0`); the model's seconds map 1:1 onto [`SEC`].
use crank_devices::{CommandStatus, ObservedControl, TrainerObservation};
use std::collections::BTreeMap;

/// One second in the machine's time unit (microseconds).
pub const SEC: u64 = 1_000_000;
/// `L`: how long after a value stopped being desired an echo of it still agrees.
pub const L: u64 = 2 * SEC;
/// `N_follow`: agreeing replies needed to verify a component.
pub const N_FOLLOW: u32 = 3;
/// `T_verify`: minimum span of those replies.
pub const T_VERIFY: u64 = 2 * SEC;
/// `T_mismatch`: minimum span of a disagreement episode before `not_following`.
pub const T_MISMATCH: u64 = 2 * SEC;
/// `T_unknown(c)` is `UNKNOWN_MULT·P_c + UNKNOWN_PAD` (HIL #44 Part A).
pub const UNKNOWN_MULT: u64 = 4;
/// See [`UNKNOWN_MULT`].
pub const UNKNOWN_PAD: u64 = SEC;

/// A commanded component: one control page the trainer echoes on page 71.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Component {
    /// Page 49.
    Erg,
    /// Page 48 (same reply period as ERG, ADR-0002 §4.4.1 components table).
    Resistance,
    /// Page 51: grade and Crr.
    Track,
    /// Page 50: wind.
    Wind,
}
impl Component {
    /// The component an echoed page belongs to; `None` for a non-control page (R1).
    #[must_use]
    pub fn of(c: ObservedControl) -> Option<Self> {
        Some(match c {
            ObservedControl::Erg { .. } => Self::Erg,
            ObservedControl::Resistance { .. } => Self::Resistance,
            ObservedControl::Track { .. } => Self::Track,
            ObservedControl::Wind { .. } => Self::Wind,
            ObservedControl::NotControl(_) => return None,
        })
    }
    /// Reply period `P_c`.
    #[must_use]
    pub const fn period(self) -> u64 {
        match self {
            Self::Erg | Self::Resistance => SEC,
            Self::Track | Self::Wind => 2 * SEC,
        }
    }
    /// `T_unknown(c)`: required time without evidence after which `c` is `silent`.
    #[must_use]
    pub const fn t_unknown(self) -> u64 {
        UNKNOWN_MULT * self.period() + UNKNOWN_PAD
    }
}

/// A component's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentState {
    /// Not enough evidence yet.
    Unverified,
    /// Verified: ≥ `N_follow` agreeing replies spanning ≥ `T_verify`.
    Following,
    /// A disagreement lasting ≥ `T_mismatch`, confirmed by a second reply.
    NotFollowing,
    /// No evidence for `T_unknown(c)` of required time.
    Silent,
}
/// The aggregate verdict reported as `RideStatus.trainer_following` (A1–A6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrainerFollowing {
    /// No claim: the machine is inactive, or every required component is silent.
    #[default]
    Unknown,
    /// Waiting for evidence.
    Checking,
    /// A disagreement is open but not yet confirmed.
    Suspect,
    /// Every required component is verified.
    Following,
    /// A required component is `not_following` or `silent`.
    NotFollowing,
}
impl TrainerFollowing {
    /// Compliance excludes the seconds of these verdicts (ADR-0002 §7.8).
    #[must_use]
    pub fn excludes_compliance(self) -> bool {
        matches!(self, Self::Suspect | Self::NotFollowing)
    }
}

/// Reply class for a required component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// The echo agrees with a recently desired value (R2).
    Agree,
    /// Anything else about a required component (R3, R5).
    Disagree,
}

/// One component's evidence, expiry and verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comp {
    /// Verdict.
    pub state: ComponentState,
    streak_n: u32,
    streak_first: Option<u64>,
    dis_since: Option<u64>,
    last_evidence: u64,
    not_required_since: Option<u64>,
}
impl Comp {
    /// C1: the component starts being required.
    #[must_use]
    pub fn fresh(now: u64) -> Self {
        Self {
            state: ComponentState::Unverified,
            streak_n: 0,
            streak_first: None,
            dis_since: None,
            last_evidence: now,
            not_required_since: None,
        }
    }
    /// Whether a disagreement episode is open.
    #[must_use]
    pub fn disagreeing(&self) -> bool {
        self.dis_since.is_some()
    }
}

/// One row of the reply table, in the ADR's row order (§4.4.1).
pub struct Row {
    /// Row id, `C2`–`C6`.
    pub id: &'static str,
    /// Input classes the row handles: `(agree, disagree)`.
    pub on: (bool, bool),
    /// A pre-step applies and evaluation continues; otherwise the first match wins.
    pub pre: bool,
    guard: fn(&Comp, Component, u64) -> bool,
    act: fn(Comp, Component, u64) -> Comp,
}
/// The row ids of the ADR table in order (C1 and C7–C9 are not reply rows).
pub const TABLE_ORDER: [&str; 9] = ["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9"];

fn stale(c: &Comp, comp: Component, now: u64) -> bool {
    c.state == ComponentState::Silent || now.saturating_sub(c.last_evidence) >= comp.t_unknown()
}
fn agree_add(mut c: Comp, now: u64) -> Comp {
    if c.streak_n == 0 {
        c.streak_first = Some(now);
    }
    c.streak_n += 1;
    c.last_evidence = now;
    c
}
fn verifies(c: &Comp, _: Component, now: u64) -> bool {
    let a = agree_add(*c, now);
    a.streak_n >= N_FOLLOW
        && a.streak_first
            .is_some_and(|f| now.saturating_sub(f) >= T_VERIFY)
}
fn disagree_add(mut c: Comp, now: u64) -> Comp {
    c.streak_n = 0;
    c.streak_first = None;
    c.dis_since = Some(c.dis_since.unwrap_or(now));
    c.last_evidence = now;
    c
}
fn mismatch(c: &Comp, _: Component, now: u64) -> bool {
    now.saturating_sub(c.dis_since.unwrap_or(now)) >= T_MISMATCH
}
/// The reply rows C2–C6, walked in order (mirrors the model's `REPLY_ROWS`).
pub const REPLY_ROWS: [Row; 5] = [
    Row {
        id: "C2",
        on: (true, true),
        pre: true,
        guard: stale,
        act: |mut c, _, _| {
            c.state = ComponentState::Unverified;
            c.streak_n = 0;
            c.streak_first = None;
            c.dis_since = None;
            c
        },
    },
    Row {
        id: "C3",
        on: (true, false),
        pre: false,
        guard: verifies,
        act: |c, _, now| {
            let mut c = agree_add(c, now);
            c.state = ComponentState::Following;
            c.dis_since = None;
            c
        },
    },
    Row {
        id: "C4",
        on: (true, false),
        pre: false,
        guard: |_, _, _| true,
        act: |c, _, now| agree_add(c, now),
    },
    Row {
        id: "C5",
        on: (false, true),
        pre: false,
        guard: mismatch,
        act: |c, _, now| {
            let mut c = disagree_add(c, now);
            c.state = ComponentState::NotFollowing;
            c
        },
    },
    Row {
        id: "C6",
        on: (false, true),
        pre: false,
        guard: |_, _, _| true,
        act: |c, _, now| {
            let was_following = c.state == ComponentState::Following;
            let mut c = disagree_add(c, now);
            if was_following {
                c.state = ComponentState::Unverified;
            }
            c
        },
    },
];

/// Apply an Agree or Disagree to a component by walking [`REPLY_ROWS`] in order.
#[must_use]
pub fn reply_component(mut c: Comp, comp: Component, class: Class, now: u64) -> Comp {
    for row in &REPLY_ROWS {
        let handles = match class {
            Class::Agree => row.on.0,
            Class::Disagree => row.on.1,
        };
        if !handles || !(row.guard)(&c, comp, now) {
            continue;
        }
        c = (row.act)(c, comp, now);
        if !row.pre {
            return c;
        }
    }
    c
}
/// C7: a required, non-silent component with no evidence for `T_unknown(c)` goes silent.
#[must_use]
pub fn tick_component(mut c: Comp, comp: Component, now: u64) -> Comp {
    if c.state != ComponentState::Silent && now.saturating_sub(c.last_evidence) >= comp.t_unknown()
    {
        c.state = ComponentState::Silent;
        c.streak_n = 0;
        c.streak_first = None;
        c.dis_since = None;
    }
    c
}
/// C8: the component stops being required. `dis_since` and `not_following` are kept.
#[must_use]
pub fn stop_required(mut c: Comp, now: u64) -> Comp {
    if c.state == ComponentState::Following {
        c.state = ComponentState::Unverified;
    }
    c.streak_n = 0;
    c.streak_first = None;
    c.not_required_since = Some(now);
    c
}
/// C9 (or C1 when it was not required for longer than `T_unknown(c)`): required again.
#[must_use]
pub fn resume_required(prev: Option<Comp>, comp: Component, now: u64) -> Comp {
    match prev {
        Some(mut c)
            if c.not_required_since
                .is_some_and(|n| now.saturating_sub(n) <= comp.t_unknown()) =>
        {
            let gap = now.saturating_sub(c.not_required_since.unwrap_or(now));
            c.last_evidence += gap;
            c.not_required_since = None;
            c
        }
        _ => Comp::fresh(now),
    }
}

/// Whether the machine is active, and what the engine desires, per component.
type Required = BTreeMap<Component, ObservedControl>;
fn required(mode: Option<&[ObservedControl]>) -> Required {
    mode.into_iter()
        .flatten()
        .filter_map(|c| Component::of(*c).map(|k| (k, *c)))
        .collect()
}

/// The whole machine: activity, desired history, reply routing and the aggregate.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    link_ready: bool,
    desired: Option<Vec<ObservedControl>>,
    active: bool,
    comps: BTreeMap<Component, Comp>,
    /// `(component, value, until)`: values desired until `until`, kept for `L`.
    hist: Vec<(Component, ObservedControl, u64)>,
}
impl Machine {
    /// An inactive machine.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Whether the machine is active (link `Ready` ∧ a desired mode).
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }
    fn recently_desired(&self, c: Component, v: ObservedControl, now: u64) -> bool {
        required(self.desired.as_deref()).get(&c) == Some(&v)
            || self
                .hist
                .iter()
                .any(|&(hc, hv, until)| hc == c && hv == v && now.saturating_sub(until) <= L)
    }
    fn recently_required(&self, c: Component, now: u64) -> bool {
        self.hist
            .iter()
            .any(|&(hc, _, until)| hc == c && now.saturating_sub(until) <= L)
    }
    fn sync_activity(&mut self, was_active: bool, now: u64) {
        let want = self.link_ready && self.desired.is_some();
        if want && !was_active {
            self.active = true;
            self.comps = required(self.desired.as_deref())
                .into_keys()
                .map(|c| (c, Comp::fresh(now)))
                .collect();
            self.hist.clear();
        } else if !want && was_active {
            self.active = false;
            self.comps.clear();
            self.hist.clear();
        }
    }
    /// The trainer link became `Ready` or stopped being so.
    pub fn set_link(&mut self, ready: bool, now: u64) {
        let was = self.active;
        self.link_ready = ready;
        self.sync_activity(was, now);
    }
    /// The engine's desired mode as grid components; `None` = no control.
    pub fn set_desired(&mut self, mode: Option<Vec<ObservedControl>>, now: u64) {
        if mode == self.desired {
            return;
        }
        let was = self.active;
        let old = required(self.desired.as_deref());
        self.desired = mode;
        if !(was && self.link_ready && self.desired.is_some()) {
            self.sync_activity(was, now);
            return;
        }
        let new = required(self.desired.as_deref());
        self.hist.retain(|h| now.saturating_sub(h.2) <= L);
        for (c, v) in &old {
            if new.get(c) != Some(v) {
                self.hist.push((*c, *v, now));
            }
            if !new.contains_key(c)
                && let Some(comp) = self.comps.get(c).copied()
            {
                self.comps.insert(*c, stop_required(comp, now));
            }
        }
        for c in new.keys().filter(|c| !old.contains_key(c)) {
            let next = resume_required(self.comps.get(c).copied(), *c, now);
            self.comps.insert(*c, next);
        }
    }
    /// Route one page-71 / page-25 observation (R1–R5).
    pub fn observe(&mut self, obs: &TrainerObservation, now: u64) {
        if !self.active {
            return;
        }
        // R1: neutral whatever the status (an unsupported request gives `NotControl` with
        // `NotSupported`, #226), and `Pending`/`Other` status. No status on a control page
        // is no claim either.
        let (Some(control), Some(status)) = (obs.control, obs.status) else {
            return;
        };
        let Some(c) = Component::of(control) else {
            return;
        };
        if matches!(status, CommandStatus::Pending | CommandStatus::Other(_)) {
            return;
        }
        let req = required(self.desired.as_deref());
        if req.contains_key(&c) {
            let agrees = status == CommandStatus::Pass && self.recently_desired(c, control, now);
            self.apply(
                c,
                if agrees {
                    Class::Agree
                } else {
                    Class::Disagree
                },
                now,
            );
        } else if !self.recently_required(c, now) {
            for rc in req.into_keys() {
                self.apply(rc, Class::Disagree, now);
            }
        }
        // else R4: required within `L`, no longer is: neutral.
    }
    fn apply(&mut self, c: Component, class: Class, now: u64) {
        if let Some(comp) = self.comps.get(&c).copied() {
            self.comps.insert(c, reply_component(comp, c, class, now));
        }
    }
    /// C7 for every required component, and expiry of the desired history.
    pub fn tick(&mut self, now: u64) {
        if !self.active {
            return;
        }
        for c in required(self.desired.as_deref()).into_keys() {
            if let Some(comp) = self.comps.get(&c).copied() {
                self.comps.insert(c, tick_component(comp, c, now));
            }
        }
        self.hist.retain(|h| now.saturating_sub(h.2) <= L);
    }
    /// The required components' verdicts, in component order.
    #[must_use]
    pub fn components(&self) -> Vec<(Component, ComponentState)> {
        required(self.desired.as_deref())
            .into_keys()
            .filter_map(|c| self.comps.get(&c).map(|k| (c, k.state)))
            .collect()
    }
    /// A1–A6: a pure function of the required components' verdicts.
    #[must_use]
    pub fn verdict(&self) -> TrainerFollowing {
        use ComponentState::{Following, NotFollowing, Silent};
        if !self.active {
            return TrainerFollowing::Unknown;
        }
        let states: Vec<&Comp> = required(self.desired.as_deref())
            .into_keys()
            .filter_map(|c| self.comps.get(&c))
            .collect();
        if states.iter().all(|s| s.state == Silent) {
            TrainerFollowing::Unknown
        } else if states
            .iter()
            .any(|s| matches!(s.state, NotFollowing | Silent))
        {
            TrainerFollowing::NotFollowing
        } else if states.iter().all(|s| s.state == Following) {
            TrainerFollowing::Following
        } else if states.iter().any(|s| s.disagreeing()) {
            TrainerFollowing::Suspect
        } else {
            TrainerFollowing::Checking
        }
    }
    /// The first required component that makes the verdict `not_following`, for the
    /// status line ("grade not confirmed").
    #[must_use]
    pub fn blocking(&self) -> Option<Component> {
        // Only a verdict of `not_following` names a component (§4.4.1 A3): when every
        // component is silent the verdict is `unknown` (A2) and nothing is blamed.
        if self.verdict() != TrainerFollowing::NotFollowing {
            return None;
        }
        self.components()
            .into_iter()
            .find(|(_, s)| matches!(s, ComponentState::NotFollowing | ComponentState::Silent))
            .map(|(c, _)| c)
    }
}
