//! Deterministic trainer and rider for the Part B procedure tests (no hardware, paused time).
//!
//! The trainer answers like the NEO in Part A: page 25 every 350 ms, page 16 every 1.35 s,
//! a page-71 reply 100 ms after a page-70 request echoing the last control page. Power
//! follows the applied control with a 0.8 s lag: ERG holds the target down to 35 rpm and
//! collapses below it, page 48 and SIM follow cadence (SIM through the road model). The
//! rider moves cadence towards the last cue at 30 rpm/s; `Descend` lowers it 5 rpm per 10 s.
use crate::{
    frames::{Record, page_of},
    rider::{Cue, Operator, Wire},
};
use crank_fec::{Page, decode};
use std::sync::Mutex;
use tokio::time::Instant;

const STEP_US: u64 = 50_000;

/// What the simulated rider does.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Answers the start confirmation.
    pub confirms: bool,
    /// Starts pedalling when cued.
    pub pedals: bool,
    /// Stops pedalling at this capture time, whatever the cue.
    pub stop_at_us: Option<u64>,
    /// The link drops at this capture time.
    pub drop_at_us: Option<u64>,
    /// Writes from this capture time reach the trainer but are never acknowledged.
    pub hang_from_us: Option<u64>,
    /// The trainer sends nothing from this capture time.
    pub silent_from_us: Option<u64>,
    /// The harness panics (once) when it reads telemetry at or after this capture time.
    pub panic_at_us: Option<u64>,
    /// Follows the check-9 cue to lower cadence (otherwise holds 90 rpm).
    pub descends: bool,
    /// ERG holds the target down to this cadence and collapses in proportion below it
    /// (0: never collapses).
    pub erg_floor_rpm: f64,
    /// Page 25 reports cadence as unavailable (0xFF) from this capture time.
    pub cadence_unavailable_from_us: Option<u64>,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            confirms: true,
            pedals: true,
            stop_at_us: None,
            drop_at_us: None,
            hang_from_us: None,
            silent_from_us: None,
            panic_at_us: None,
            descends: true,
            erg_floor_rpm: 35.0,
            cadence_unavailable_from_us: None,
        }
    }
}

#[derive(Debug)]
enum Applied {
    Resistance(f64),
    Erg(f64),
    Sim,
}

struct State {
    at: u64,
    records: Vec<Record>,
    rx: Vec<Record>,
    applied: Applied,
    grade: f64,
    mass: f64,
    power: f64,
    cadence: f64,
    goal: f64,
    descend_from: Option<u64>,
    accumulated: u16,
    events: u8,
    seq: u8,
    last_control: [u8; 13],
    replies: Vec<u64>,
    cues: Vec<(u64, Cue)>,
}

/// The simulator: a [`Wire`] and the rider's [`Operator`] in one.
pub struct Sim {
    plan: Plan,
    start: Instant,
    state: Mutex<State>,
    panicked: std::sync::atomic::AtomicBool,
}

fn emit(s: &mut State, at_us: u64, f: [u8; 13]) {
    let r = Record {
        at_us,
        rx: true,
        bytes: f.to_vec(),
    };
    s.records.push(r.clone());
    s.rx.push(r);
}

fn checked(mut f: [u8; 13]) -> [u8; 13] {
    f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
    f
}

impl Sim {
    /// A trainer at 0 % with the rider standing by.
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            start: Instant::now(),
            state: Mutex::new(State {
                at: 0,
                records: Vec::new(),
                rx: Vec::new(),
                applied: Applied::Resistance(0.0),
                grade: 0.0,
                mass: 85.0,
                power: 0.0,
                cadence: 0.0,
                goal: 0.0,
                descend_from: None,
                accumulated: 0,
                events: 0,
                seq: 0,
                last_control: [0; 13],
                replies: Vec::new(),
                cues: Vec::new(),
            }),
            panicked: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("sim lock")
    }

    fn elapsed(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_micros()).expect("fits")
    }

    /// Cues given so far.
    pub fn cues(&self) -> Vec<Cue> {
        self.lock().cues.iter().map(|(_, c)| *c).collect()
    }

    /// Capture time of the first `cue`.
    pub fn cue_at(&self, cue: Cue) -> Option<u64> {
        self.lock()
            .cues
            .iter()
            .find(|(_, c)| *c == cue)
            .map(|(at, _)| *at)
    }

    /// Bring the trainer and rider to the present.
    fn advance(&self) {
        let now = self.elapsed();
        let mut s = self.lock();
        while s.at + STEP_US <= now {
            s.at += STEP_US;
            self.step(&mut s);
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn step(&self, st: &mut State) {
        let now = st.at;
        let dt = STEP_US as f64 / 1e6;
        let silent = self.plan.silent_from_us.is_some_and(|x| now >= x);
        if self.plan.stop_at_us.is_some_and(|x| now >= x) {
            st.goal = 0.0;
        } else if let Some(from) = st.descend_from {
            st.goal = (90.0 - 5.0 * ((now - from) / 10_000_000) as f64).max(0.0);
        }
        let delta = (st.goal - st.cadence).clamp(-30.0 * dt, 30.0 * dt);
        st.cadence += delta;
        let speed = st.cadence * 0.09;
        let follow = match st.applied {
            Applied::Erg(w) if self.plan.erg_floor_rpm > 0.0 => {
                w * (st.cadence / self.plan.erg_floor_rpm).min(1.0)
            }
            Applied::Erg(w) => w,
            Applied::Resistance(pct) => st.cadence * (0.5 + pct / 50.0),
            Applied::Sim => {
                speed * st.mass * 9.81 * (st.grade / 100.0 + 0.004) + 0.51 * speed * speed * speed
            }
        };
        st.power += (follow - st.power) * (dt / 0.8);
        if st.cadence < 1.0 {
            st.power = 0.0;
        }
        if silent {
            return;
        }
        if now.is_multiple_of(350_000) {
            st.events = st.events.wrapping_add(1);
            st.accumulated = st.accumulated.wrapping_add(st.power as u16);
            let watts = st.power.round() as u16;
            let acc = st.accumulated.to_le_bytes();
            let frame = checked([
                0xa4,
                9,
                0x4e,
                5,
                25,
                st.events,
                if self
                    .plan
                    .cadence_unavailable_from_us
                    .is_some_and(|x| now >= x)
                {
                    0xff
                } else {
                    st.cadence.round() as u8
                },
                acc[0],
                acc[1],
                (watts & 0xff) as u8,
                (watts >> 8) as u8,
                0x30,
                0,
            ]);
            emit(st, now, frame);
        }
        if now.is_multiple_of(1_350_000) {
            let mm = ((speed * 1000.0) as u16).to_le_bytes();
            let frame = checked([0xa4, 9, 0x4e, 5, 16, 25, 0, 0, mm[0], mm[1], 0xff, 0x30, 0]);
            emit(st, now, frame);
        }
        while st.replies.first().is_some_and(|due| *due <= now) {
            st.replies.remove(0);
            let last = st.last_control;
            let frame = checked([
                0xa4, 9, 0x4f, 5, 71, last[4], st.seq, 0, last[8], last[9], last[10], last[11], 0,
            ]);
            emit(st, now, frame);
        }
    }
}

impl Wire for Sim {
    fn now_us(&self) -> u64 {
        self.advance();
        self.lock().at
    }
    async fn send(&self, frame: [u8; 13]) -> Option<String> {
        if self.record_tx(frame) {
            std::future::pending::<()>().await;
        }
        self.advance();
        let s = self.lock();
        self.plan
            .drop_at_us
            .is_some_and(|x| s.at >= x)
            .then(|| "not connected".into())
    }
    fn mark(&self) -> usize {
        self.advance();
        self.lock().rx.len()
    }
    fn since(&self, mark: usize) -> Vec<Record> {
        self.advance();
        let now = self.lock().at;
        if self.plan.panic_at_us.is_some_and(|x| now >= x)
            && !self
                .panicked
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            panic!("simulated harness panic");
        }
        self.lock().rx[mark..].to_vec()
    }
    fn dropped(&self) -> bool {
        self.advance();
        self.plan.drop_at_us.is_some_and(|x| self.lock().at >= x)
    }
    fn records(&self) -> Vec<Record> {
        self.advance();
        self.lock().records.clone()
    }
}

impl Sim {
    /// Record and apply a write; `true` when it will never be acknowledged.
    fn record_tx(&self, frame: [u8; 13]) -> bool {
        self.advance();
        let mut s = self.lock();
        if self.plan.drop_at_us.is_some_and(|x| s.at >= x) {
            return false;
        }
        let at = s.at;
        s.records.push(Record {
            at_us: at,
            rx: false,
            bytes: frame.to_vec(),
        });
        match decode(&frame).map(|f| f.page) {
            Ok(Page::TargetPower { watts }) => s.applied = Applied::Erg(f64::from(watts)),
            Ok(Page::BasicResistance { pct }) => {
                s.applied = Applied::Resistance(pct.map_or(0.0, f64::from));
            }
            Ok(Page::Track { grade_pct, .. }) => {
                s.applied = Applied::Sim;
                s.grade = grade_pct.map_or(0.0, f64::from);
            }
            Ok(Page::Wind { .. }) => s.applied = Applied::Sim,
            Ok(Page::UserConfig {
                rider_kg, bike_kg, ..
            }) => {
                s.mass = f64::from(rider_kg.unwrap_or(75.0) + bike_kg.unwrap_or(10.0));
            }
            Ok(Page::Request { page: 71, .. }) => {
                s.seq = s.seq.wrapping_add(1);
                s.replies.push(at + 100_000);
            }
            _ => {}
        }
        if matches!(page_of(&frame), Some(48..=51)) {
            s.seq = s.seq.wrapping_add(1);
            s.last_control = frame;
        }
        self.plan.hang_from_us.is_some_and(|x| at >= x)
    }
}

impl Operator for Sim {
    fn cue(&self, at_us: u64, cue: Cue) {
        let mut s = self.lock();
        s.cues.push((at_us, cue));
        if !self.plan.pedals {
            return;
        }
        match cue {
            Cue::Start | Cue::Steady => s.goal = s.goal.max(90.0),
            Cue::Cadence(rpm) => (s.goal, s.descend_from) = (f64::from(rpm), None),
            Cue::Descend if self.plan.descends => s.descend_from = Some(at_us),
            Cue::Descend => {}
            Cue::Stop => (s.goal, s.descend_from) = (0.0, None),
        }
    }
    async fn confirm(&self, _: &str) -> bool {
        self.plan.confirms
    }
}
