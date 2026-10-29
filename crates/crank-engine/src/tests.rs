//! Rule-level tests: each drives the actor synchronously on a manual clock, so every
//! assertion is about exact ticks. Scenario tests with the simulator are in
//! `tests/scenarios.rs`.
use crate::control::{PolicyContext, TargetPolicy};
use crate::engine::{CommandError, Engine, EngineCmd, PauseReason, RideSetup, RideState};
use crate::erg::{ErgPolicy, TrainerFollowing};
use crate::record::{RecordError, RideRecorder, SecondRow};
use crate::ticks::{Hold, SessionTicks, TICK_US, TickWindow};
use crank_devices::{
    Clock, ControlMode, DeviceInfo, HeartRateLink, HrEvent, HrSample, LinkState, MonoTime,
    TargetLimit, TrainerCaps, TrainerCommand, TrainerEvent, TrainerLink, TrainerSample,
    TrainerStatus,
};
use crank_domain::workout::{FlatBlock, Phase};
use futures_util::future::BoxFuture;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::mpsc;

pub(crate) struct ManualClock(AtomicU64);
impl Clock for ManualClock {
    fn now(&self) -> MonoTime {
        MonoTime(self.0.load(Ordering::SeqCst))
    }
    fn sleep_until(&self, _: MonoTime) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}
#[derive(Default)]
pub(crate) struct Log {
    started: bool,
    finished: bool,
    rows: Vec<(u32, SecondRow)>,
    fail_after: Option<usize>,
}
pub(crate) struct Recorder(pub(crate) Arc<Mutex<Log>>);
impl RideRecorder for Recorder {
    fn start(&mut self, metrics: &[&str]) -> Result<(), RecordError> {
        assert_eq!(metrics, crate::METRICS);
        self.0.lock().unwrap().started = true;
        Ok(())
    }
    fn append_second(&mut self, s: u32, row: &SecondRow) -> Result<(), RecordError> {
        let mut log = self.0.lock().unwrap();
        if log.fail_after.is_some_and(|n| log.rows.len() >= n) {
            return Err(RecordError("disk".into()));
        }
        log.rows.push((s, *row));
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        self.0.lock().unwrap().finished = true;
        Ok(())
    }
}
pub(crate) struct Rig {
    pub(crate) engine: Engine,
    pub(crate) clock: Arc<ManualClock>,
    pub(crate) events: mpsc::Sender<TrainerEvent>,
    pub(crate) hr: mpsc::Sender<HrEvent>,
    pub(crate) control: mpsc::Receiver<TrainerCommand>,
    pub(crate) log: Arc<Mutex<Log>>,
    pub(crate) gate: Arc<Mutex<Vec<bool>>>,
}
pub(crate) fn block(dur_s: f64, from: f64, to: f64, phase: Phase) -> FlatBlock {
    FlatBlock {
        path: vec![],
        iteration: vec![],
        node_id: String::new(),
        start_s: 0.0,
        dur_s,
        power_from: from,
        power_to: to,
        phase,
        dur_range_s: None,
    }
}
pub(crate) fn rig() -> Rig {
    let (events, event_rx) = mpsc::channel(4096);
    let (control_tx, control) = mpsc::channel(4096);
    let link = TrainerLink {
        info: DeviceInfo::default(),
        caps: TrainerCaps {
            erg: true,
            sim: true,
            resistance: true,
            max_resistance_n: None,
            max_target_w: 2000,
        },
        control: control_tx,
        events: event_rx,
    };
    let (hr, hr_rx) = mpsc::channel(4096);
    let hr_link = HeartRateLink {
        info: DeviceInfo::default(),
        events: hr_rx,
    };
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let gate = Arc::new(Mutex::new(Vec::new()));
    let g = gate.clone();
    let activity = move |a: bool| g.lock().unwrap().push(a);
    let (engine, _handle) =
        Engine::new(clock.clone(), Some(link), Some(hr_link), Arc::new(activity));
    Rig {
        engine,
        clock,
        events,
        hr,
        control,
        log: Arc::default(),
        gate,
    }
}
impl Rig {
    pub(crate) fn load(&mut self, blocks: Vec<FlatBlock>) {
        let rec = Box::new(Recorder(self.log.clone()));
        self.engine
            .command(EngineCmd::Load(RideSetup {
                blocks,
                recorder: rec,
                cap_w: None,
            }))
            .unwrap();
    }
    pub(crate) fn start(&mut self) {
        self.engine.command(EngineCmd::Start).unwrap();
    }
    /// A trainer sample arriving at the start of tick `k`.
    pub(crate) fn sample(&self, k: u64, power: u16, cadence: u8) {
        self.sample_opt(k, Some(power), Some(cadence));
    }
    pub(crate) fn sample_opt(&self, k: u64, power: Option<u16>, cadence: Option<u8>) {
        self.events
            .try_send(TrainerEvent::Sample(TrainerSample {
                at: MonoTime(k * TICK_US + 1),
                power_w: power,
                cadence_rpm: cadence,
                speed_mps: Some(10.0),
                distance_m: None,
                accumulated_power_w: None,
                event_count: None,
                status: TrainerStatus::default(),
                target_limit: TargetLimit::AtTarget,
            }))
            .unwrap();
    }
    pub(crate) fn hr_link(&mut self, s: LinkState) {
        self.hr.try_send(HrEvent::Link(s)).unwrap();
        self.engine.drain();
    }
    pub(crate) fn heart(&self, k: u64, bpm: u8) {
        self.hr
            .try_send(HrEvent::Sample(HrSample {
                at: MonoTime(k * TICK_US + 1),
                bpm: Some(bpm),
                rr_ms: std::iter::empty().collect(),
                contact: None,
            }))
            .unwrap();
    }
    /// A link event received now: stamped with the manual clock, i.e. before the next
    /// unprocessed tick, in order with the samples queued so far.
    pub(crate) fn link(&mut self, s: LinkState) {
        self.events.try_send(TrainerEvent::Link(s)).unwrap();
        self.engine.drain();
    }
    /// The driver starts reconnect attempt `n`, received now (queued in order with the links).
    pub(crate) fn reconnecting(&mut self, n: u32) {
        self.events
            .try_send(TrainerEvent::Reconnecting { attempt: n })
            .unwrap();
        self.engine.drain();
    }
    /// Move the manual clock without processing ticks.
    pub(crate) fn at(&mut self, us: u64) {
        self.clock.0.store(us, Ordering::SeqCst);
    }
    /// Process every tick whose window has closed by the end of tick `k`.
    pub(crate) fn through(&mut self, k: u64) {
        self.clock.0.store((k + 1) * TICK_US, Ordering::SeqCst);
        self.engine.catch_up();
    }
    pub(crate) fn state(&self) -> RideState {
        self.status().state
    }
    pub(crate) fn status(&self) -> crate::RideStatus {
        self.engine.status.borrow().clone()
    }
    /// The driver reports that `mode` reached the trainer.
    pub(crate) fn delivered(&mut self, mode: ControlMode) {
        self.events
            .try_send(TrainerEvent::Delivered { mode })
            .unwrap();
        self.engine.drain();
    }
    pub(crate) fn modes(&mut self) -> Vec<ControlMode> {
        let mut out = vec![];
        while let Ok(TrainerCommand::SetMode(m)) = self.control.try_recv() {
            out.push(m);
        }
        out
    }
    pub(crate) fn rows(&self) -> Vec<(u32, SecondRow)> {
        self.log.lock().unwrap().rows.clone()
    }
}
pub(crate) fn erg(w: u16) -> ControlMode {
    ControlMode::Erg { target_w: w }
}

#[test]
fn window_assigns_tick_by_arrival_time() {
    let w = TickWindow::new(MonoTime(1_000));
    assert_eq!(w.tick_of(MonoTime(999)), None);
    assert_eq!(w.tick_of(MonoTime(1_000)), Some(0));
    assert_eq!(w.tick_of(MonoTime(1_000 + TICK_US - 1)), Some(0));
    assert_eq!(w.tick_of(MonoTime(1_000 + TICK_US)), Some(1));
    assert_eq!(w.end_of(0), MonoTime(1_000 + TICK_US));
}

#[test]
fn hold_lasts_exactly_its_ticks() {
    let mut h = Hold::new(4);
    h.push(10, 7);
    assert_eq!(h.get(9), None);
    for k in 10..=14 {
        assert_eq!(h.get(k), Some(7), "age {}", k - 10);
    }
    assert_eq!(h.get(15), None);
    h.clear();
    assert_eq!(h.get(11), None);
}

#[test]
fn stall_catch_up_assigns_samples_to_their_own_ticks() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    // Nothing is processed while the process "stalls"; every sample is queued with its
    // own arrival time, then all 12 ticks replay in order at once.
    for k in 0..12 {
        r.sample(k, 100 + u16::try_from(k).unwrap() * 10, 90);
    }
    r.through(11);
    let rows = r.rows();
    let power = |i: usize| rows[i].1.power.unwrap();
    assert_eq!(rows.len(), 3);
    // 100 110 120 130 | 140 150 160 170 | 180 190 200 210
    for (i, mean) in [115.0, 155.0, 195.0].into_iter().enumerate() {
        assert!((power(i) - mean).abs() < 1e-4, "second {i}: {}", power(i));
    }
    assert_eq!(r.status().session.0, 12);
    assert_eq!(r.status().late_samples, 0);
}

#[test]
fn newest_sample_in_a_window_wins_and_late_samples_are_counted() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    r.sample(0, 100, 90);
    r.sample(0, 140, 90);
    for k in 1..4 {
        r.sample(k, 140, 90);
    }
    r.through(3);
    assert_eq!(r.rows()[0].1.power, Some(140.0));
    // tick 2 is processed; a sample stamped inside it now arrives too late
    r.sample(2, 999, 90);
    r.sample(4, 100, 90);
    r.through(4);
    assert_eq!(r.status().late_samples, 1);
}

#[test]
fn trainer_values_hold_one_second_then_null() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    // second 1 = ticks 4..8: held 100 at tick 4 (age 4), null at 5, fresh 200 at 6, held at 7
    for (k, p) in [(0, 100), (6, 200)] {
        r.sample(k, p, 90);
    }
    r.sample(8, 200, 90);
    r.sample(10, 200, 90);
    r.through(11);
    let rows = r.rows();
    let p1 = rows[1].1.power.unwrap();
    assert!((p1 - 500.0 / 3.0).abs() < 1e-3, "{p1}");
    // second 0: samples at tick 0 only, held for ticks 1..=3
    assert_eq!(rows[0].1.power, Some(100.0));
}

#[test]
fn hr_values_hold_three_seconds_then_null() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    for k in 0..16 {
        r.sample(k, 150, 90);
    }
    // second 3 = ticks 12..16: held 100 at tick 12 (age 12), null at 13, fresh 200 at 14
    r.heart(0, 100);
    r.heart(14, 200);
    r.through(15);
    let rows = r.rows();
    let hr = rows[3].1.hr.unwrap();
    assert!((hr - 500.0 / 3.0).abs() < 1e-3, "{hr}");
    assert_eq!(rows[0].1.hr, Some(100.0));
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn hr_loss_nulls_hr_but_the_ride_continues() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    for k in 0..24 {
        r.sample(k, 150, 90);
        if k < 8 {
            r.heart(k, 140);
        }
    }
    r.through(23);
    let rows = r.rows();
    assert_eq!(rows[1].1.hr, Some(140.0));
    // held 3 s past tick 7 (to tick 19), then null
    assert_eq!(rows[5].1.hr, None);
    assert!(rows[5].1.power.is_some());
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(r.status().workout.0, 24);
}

#[test]
fn eight_missed_ticks_lose_the_trainer_without_rollback() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    r.sample(0, 150, 90);
    r.through(7);
    // ticks 1..=7 missed: seven is not loss
    assert_eq!(r.state(), RideState::Riding);
    r.through(8);
    // tick 8 is the eighth missed tick: it still counts, the next one is paused
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(r.status().workout.0, 9);
    r.through(11);
    assert_eq!(r.status().workout.0, 9, "workout time frozen");
    assert_eq!(r.status().session.0, 12, "session time keeps running");
    // ticks 5..=8 of the window are null in the record; tick 8 was still workout time
    let rows = r.rows();
    assert_eq!(rows[1].1.power, Some(150.0)); // tick 4 held, 5..=7 null
    assert_eq!(rows[2].1.power, None);
    assert_eq!(rows[2].1.workout_s, Some(2.25));
    // a second made only of paused ticks has no workout position
    r.through(15);
    assert_eq!(r.rows()[3].1.workout_s, None);
}

#[test]
fn seven_missed_ticks_then_a_sample_do_not_lose_the_trainer() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    r.sample(0, 150, 90);
    r.sample(8, 150, 90);
    r.through(12);
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(r.status().workout.0, 13);
}

#[test]
fn link_loss_pauses_and_reconnect_reissues_mode_with_soft_start() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..8 {
        r.sample(k, 150, 90);
    }
    r.through(7);
    // a ride start is a reconnect (#250): the ramp runs from the 3 s power, a step a second
    assert_eq!(r.modes(), vec![erg(150), erg(160)]);
    r.link(LinkState::Lost);
    r.through(9);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(r.status().workout.0, 8);
    // nothing is sent while the link is down
    assert!(r.modes().is_empty());
    r.link(LinkState::Connecting);
    r.through(12);
    assert!(r.modes().is_empty(), "no commands while connecting");
    r.link(LinkState::Ready);
    r.through(13);
    // no pedalling sample yet: the mode is re-issued at 0 W, never the target (#231),
    // and the ride stays paused until a sample arrives
    assert_eq!(r.modes(), vec![erg(0)]);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    r.sample(14, 90, 85);
    r.through(14);
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(
        r.status().workout.0,
        8,
        "the resuming tick itself was paused"
    );
    // the soft start begins with the first sample; the 150 W sampled before the loss
    // is not part of the origin, so it is the 90 W of the new epoch, then five 1 Hz
    // steps to the target
    assert_eq!(r.modes(), vec![erg(90)]);
    for k in 15..40 {
        r.sample(k, 150, 85);
    }
    r.through(39);
    assert_eq!(
        r.modes(),
        vec![erg(112), erg(134), erg(156), erg(178), erg(200)]
    );
}

#[test]
fn resume_waits_for_link_ready_and_the_reapplied_mode_even_when_pedalling() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..4 {
        r.sample(k, 150, 90);
    }
    r.through(3);
    r.link(LinkState::Lost);
    r.through(4);
    assert_eq!(r.modes().len(), 1);
    // pedalling samples arrive while the link is still being re-established
    r.link(LinkState::Connecting);
    r.sample(5, 150, 90);
    r.sample(6, 150, 90);
    r.through(6);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert!(r.modes().is_empty());
    r.link(LinkState::Ready);
    r.sample(7, 150, 90);
    r.through(7);
    assert_eq!(r.modes().len(), 1, "mode re-issued on Ready");
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn stopped_pedalling_pauses_after_twelve_zero_ticks_and_resumes_on_cadence() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    for k in 0..11 {
        r.sample(k, 0, 0);
    }
    r.through(10);
    assert_eq!(r.state(), RideState::Riding, "11 zero ticks are not enough");
    r.sample(11, 0, 0);
    r.through(11);
    assert_eq!(r.state(), RideState::Paused(PauseReason::StoppedPedalling));
    assert_eq!(r.status().workout.0, 12);
    r.sample(12, 0, 0);
    r.sample(13, 0, 0);
    r.through(13);
    assert_eq!(r.status().workout.0, 12);
    r.sample(14, 60, 70);
    r.through(14);
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(r.status().workout.0, 12);
    r.sample(15, 60, 70);
    r.through(15);
    assert_eq!(r.status().workout.0, 13);
}

#[test]
fn one_nonzero_tick_resets_the_stopped_run() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    for k in 0..30 {
        let (p, c) = if k % 8 == 7 { (50, 40) } else { (0, 0) };
        r.sample(k, p, c);
    }
    r.through(29);
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn blocks_sequence_in_workout_time_and_send_on_the_boundary_tick() {
    let mut r = rig();
    r.load(vec![
        block(2.0, 100.0, 100.0, Phase::Warmup),
        block(1.0, 200.0, 200.0, Phase::Work),
    ]);
    r.start();
    for k in 0..12 {
        r.sample(k, 100, 90);
    }
    r.through(6);
    // block 0 = ticks 0..=7; the next target goes out on tick 7 (the boundary tick)
    assert_eq!(r.modes(), vec![erg(100)]);
    assert_eq!(r.status().block_index, Some(0));
    r.through(7);
    // the boundary comes inside the start's soft start, which continues towards the new
    // target: one fifth of the way from the 100 W origin
    assert_eq!(r.modes(), vec![erg(120)]);
    assert_eq!(r.status().block_index, Some(1));
    assert_eq!(r.status().block_elapsed.0, 0);
    r.through(10);
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(r.status().block_elapsed.0, 3);
    r.through(11);
    assert_eq!(r.state(), RideState::Finished);
    assert_eq!(r.status().workout.0, 12);
    assert!(r.log.lock().unwrap().finished);
}

#[test]
fn block_timer_and_block_boundaries_freeze_while_paused() {
    let mut r = rig();
    r.load(vec![
        block(4.0, 100.0, 100.0, Phase::Warmup),
        block(4.0, 200.0, 200.0, Phase::Work),
    ]);
    r.start();
    for k in 0..40 {
        r.sample(k, 100, 90);
    }
    r.through(5);
    r.engine.command(EngineCmd::Pause).unwrap();
    r.through(15);
    assert_eq!(r.state(), RideState::Paused(PauseReason::User));
    assert_eq!(r.status().workout.0, 6);
    assert_eq!(r.status().block_elapsed.0, 6);
    assert_eq!(r.status().session.0, 16);
    r.engine.command(EngineCmd::Resume).unwrap();
    r.through(16);
    assert_eq!(r.status().workout.0, 7);
}

#[test]
fn skip_jumps_to_the_next_block_and_finishing_the_last_ends_the_ride() {
    let mut r = rig();
    r.load(vec![
        block(100.0, 100.0, 100.0, Phase::Warmup),
        block(100.0, 200.0, 200.0, Phase::Work),
    ]);
    r.start();
    for k in 0..8 {
        r.sample(k, 100, 90);
    }
    r.through(3);
    assert_eq!(r.modes(), vec![erg(100)]);
    r.engine.command(EngineCmd::SkipBlock).unwrap();
    assert_eq!(r.status().block_index, Some(1));
    assert_eq!(r.status().block_elapsed.0, 0);
    r.through(4);
    assert_eq!(
        r.modes(),
        vec![erg(120)],
        "the start's ramp goes on to the new target"
    );
    assert_eq!(r.status().workout.0, 5, "skip does not move workout time");
    r.engine.command(EngineCmd::SkipBlock).unwrap();
    assert_eq!(r.state(), RideState::Finished);
}

#[test]
fn user_pause_survives_a_trainer_loss_and_reconnect() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..4 {
        r.sample(k, 150, 90);
    }
    r.through(3);
    r.engine.command(EngineCmd::Pause).unwrap();
    r.link(LinkState::Lost);
    r.through(5);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    r.link(LinkState::Ready);
    r.sample(7, 100, 80);
    r.through(7);
    assert_eq!(r.state(), RideState::Paused(PauseReason::User));
    r.engine.command(EngineCmd::Resume).unwrap();
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn ride_gate_follows_load_finish_and_unload() {
    let mut r = rig();
    assert!(r.gate.lock().unwrap().is_empty());
    r.load(vec![block(1.0, 100.0, 100.0, Phase::Work)]);
    assert_eq!(*r.gate.lock().unwrap(), vec![true]);
    // unload before start clears it
    r.engine.command(EngineCmd::Stop).unwrap();
    assert_eq!(*r.gate.lock().unwrap(), vec![true, false]);
    assert!(!r.status().loaded);
    // a ride that finishes by itself clears it
    r.load(vec![block(1.0, 100.0, 100.0, Phase::Work)]);
    r.start();
    for k in 0..4 {
        r.sample(k, 100, 90);
    }
    r.through(3);
    assert_eq!(r.state(), RideState::Finished);
    // the end mode (ADR-0002 §7.9, #266) is out before the gate clears, and the gate
    // is held until the driver reports it delivered
    assert_eq!(
        r.modes().last(),
        Some(&ControlMode::Resistance { pct: 0.0 })
    );
    assert_eq!(*r.gate.lock().unwrap(), vec![true, false, true]);
    r.delivered(ControlMode::Resistance { pct: 0.0 });
    assert_eq!(*r.gate.lock().unwrap(), vec![true, false, true, false]);
    // a rejected command does not touch the gate
    assert_eq!(
        r.engine.command(EngineCmd::Stop),
        Err(CommandError::WrongState)
    );
    assert_eq!(r.gate.lock().unwrap().len(), 4);
}

#[test]
fn ride_gate_stays_active_while_paused_and_clears_on_stop() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    r.engine.command(EngineCmd::Pause).unwrap();
    assert_eq!(r.gate.lock().unwrap().last(), Some(&true));
    r.engine.command(EngineCmd::Stop).unwrap();
    assert_eq!(r.state(), RideState::Finished);
    assert_eq!(
        r.gate.lock().unwrap().last(),
        Some(&true),
        "end mode in flight"
    );
    r.delivered(ControlMode::Resistance { pct: 0.0 });
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
}

#[test]
fn commands_are_checked_against_the_state() {
    let mut r = rig();
    assert_eq!(
        r.engine.command(EngineCmd::Start),
        Err(CommandError::NotLoaded)
    );
    assert_eq!(
        r.engine.command(EngineCmd::Pause),
        Err(CommandError::NotLoaded)
    );
    r.load(vec![]);
    assert_eq!(
        r.engine.command(EngineCmd::Pause),
        Err(CommandError::WrongState)
    );
    r.start();
    assert_eq!(
        r.engine.command(EngineCmd::Start),
        Err(CommandError::WrongState)
    );
    assert_eq!(
        r.engine.command(EngineCmd::Resume),
        Err(CommandError::WrongState)
    );
    assert_eq!(
        r.engine.command(EngineCmd::Load(RideSetup {
            blocks: vec![],
            recorder: Box::new(Recorder(r.log.clone())),
            cap_w: None,
        })),
        Err(CommandError::WrongState)
    );
}

#[test]
fn record_failure_is_reported_and_stops_recording_not_the_ride() {
    let mut r = rig();
    r.log.lock().unwrap().fail_after = Some(1);
    r.load(vec![]);
    r.start();
    for k in 0..12 {
        r.sample(k, 100, 90);
    }
    r.through(11);
    assert_eq!(r.rows().len(), 1);
    assert_eq!(r.status().record_error.as_deref(), Some("record: disk"));
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(r.status().workout.0, 12);
}

#[test]
fn ramp_target_moves_once_per_second_and_clamps_to_the_trainer_maximum() {
    let ctx = |elapsed, max| PolicyContext {
        block: None,
        block_index: 0,
        block_elapsed_ticks: elapsed,
        tick: 0,
        link_epoch: 0,
        flow_start: Some(0),
        power_3s_w: None,
        cadence_rpm: Some(90),
        last_sent: None,
        riding: true,
        user_paused: false,
        following: TrainerFollowing::Unknown,
        max_target_w: max,
        cap_w: None,
    };
    let b = block(100.0, 100.0, 200.0, Phase::Work);
    let at = |elapsed, max| {
        let mut c = ctx(elapsed, max);
        c.block = Some(&b);
        ErgPolicy::settled().desired(&c)
    };
    assert_eq!(at(0, 2000), Some(erg(100)));
    assert_eq!(
        at(3, 2000),
        Some(erg(100)),
        "sub-second ticks do not move it"
    );
    assert_eq!(at(4 * 50, 2000), Some(erg(150)));
    assert_eq!(at(4 * 100, 2000), Some(erg(200)));
    assert_eq!(at(4 * 100, 120), Some(erg(120)));
    assert_eq!(
        ErgPolicy::new().desired(&ctx(0, 2000)),
        Some(ControlMode::Free),
        "no block: explicit free ride"
    );
}

#[test]
fn a_sample_from_before_the_loss_cannot_resume_the_ride() {
    // Counterexample of the review of 3d92dc0: a sample in tick 9, then Lost,
    // Connecting and Ready inside the same window and no new sample.
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..9 {
        r.sample(k, 150, 90);
    }
    r.through(8);
    r.at(9 * TICK_US + 100_000);
    r.sample(9, 150, 90);
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    r.through(14);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(r.status().workout.0, 9);
    // a cadence sample received after Ready is the resume signal
    r.sample(15, 80, 85);
    r.through(15);
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn a_sample_received_while_connecting_is_not_a_resume_signal() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..4 {
        r.sample(k, 150, 90);
    }
    r.through(3);
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.sample(5, 150, 90);
    r.through(6);
    r.link(LinkState::Ready);
    r.through(9);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    r.sample(10, 150, 90);
    r.through(10);
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn lost_clears_the_trainer_hold() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..5 {
        r.sample(k, 150, 90);
    }
    r.through(4);
    // Lost and an immediate Ready: the held cadence of tick 4 must not carry over
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    r.through(6);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
}

/// Samples of 100 W at ticks `from..to`.
fn feed(r: &Rig, from: u64, to: u64, w: u16) {
    for k in from..to {
        r.sample(k, w, 90);
    }
}

#[test]
fn the_soft_start_is_not_spent_in_silence_and_starts_from_current_power() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 150);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(150), erg(160)]);
    // 33 silent ticks: lost after 8, the mode is re-issued once, no ramp is played
    r.through(40);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(r.modes(), vec![erg(170), erg(0)]);
    // samples return at 100 W; 150 W from 30 ticks ago is not "the current power"
    feed(&r, 41, 70, 100);
    r.through(41);
    assert_eq!(r.modes(), vec![erg(100)]);
    r.through(69);
    assert_eq!(
        r.modes(),
        vec![erg(120), erg(140), erg(160), erg(180), erg(200)]
    );
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn the_ramp_restarts_when_samples_stop_mid_ramp_and_return() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 150);
    r.through(40);
    feed(&r, 41, 51, 100); // ramp from 100 W: engaged at tick 41, step 2 at tick 49
    r.through(50);
    // start ramp from 150 W, re-issue on loss, then the ramp from 100 W
    assert_eq!(
        r.modes(),
        vec![
            erg(150),
            erg(160),
            erg(170),
            erg(0),
            erg(100),
            erg(120),
            erg(140)
        ]
    );
    // 6 ticks without samples (a hold of 4 ticks, no loss yet): the ramp holds its last
    // step (160 W at tick 53), then 140 W returns. The 3 s power at tick 57 is
    // (5 x 100 W from ticks 46..=50 + 140 W) / 6 = 106.7 W: the ramp restarts there.
    feed(&r, 57, 85, 140);
    r.through(57);
    assert_eq!(r.modes(), vec![erg(160), erg(107)]);
    r.through(80);
    assert_eq!(
        r.modes(),
        vec![erg(125), erg(144), erg(163), erg(181), erg(200)]
    );
}

#[test]
fn start_needs_a_ready_trainer() {
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let (mut engine, _h) = Engine::new(clock, None, None, Arc::new(|_: bool| {}));
    let setup = || RideSetup {
        blocks: vec![],
        recorder: Box::new(Recorder(Arc::default())),
        cap_w: None,
    };
    engine.command(EngineCmd::Load(setup())).unwrap();
    assert_eq!(
        engine.command(EngineCmd::Start),
        Err(CommandError::NoTrainer)
    );
    assert_eq!(engine.status.borrow().state, RideState::Idle);

    let mut r = rig();
    r.load(vec![]);
    r.link(LinkState::Connecting);
    assert_eq!(
        r.engine.command(EngineCmd::Start),
        Err(CommandError::TrainerNotReady)
    );
    r.link(LinkState::Ready);
    r.start();
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn after_ready_only_zero_watts_is_sent_until_a_pedalling_sample_then_the_ramp_starts() {
    let mut r = rig();
    r.load(vec![block(600.0, 250.0, 250.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 180);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(180), erg(194)]);
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    // 20 ticks, no samples at all: only 0 W (re-sent when the 8-tick silence is a new
    // epoch), and never the target
    r.through(27);
    assert_eq!(r.modes(), vec![erg(0), erg(0)]);
    // samples with cadence 0 (not pedalling) do not start the ramp either
    for k in 28..32 {
        r.sample(k, 0, 0);
    }
    r.through(31);
    assert!(r.modes().is_empty());
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    // first pedalling sample: the 3 s power is (0 x 4 + 100 W) / 5 = 20 W, then 1 Hz
    // steps to 250 W
    feed(&r, 32, 60, 100);
    r.through(32);
    assert_eq!(r.modes(), vec![erg(20)]);
    r.through(59);
    assert_eq!(
        r.modes(),
        vec![erg(66), erg(112), erg(158), erg(204), erg(250)]
    );
}

#[test]
fn pre_loss_power_is_not_a_soft_start_origin_on_the_link_path() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(300), erg(280)]);
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    r.through(8);
    assert_eq!(r.modes(), vec![erg(0)]);
    // the rider is back at 50 W: the origin is 50 W, not an average with the 300 W
    feed(&r, 9, 20, 50);
    r.through(9);
    assert_eq!(r.modes(), vec![erg(50)]);
}

#[test]
fn pre_loss_power_is_not_a_soft_start_origin_on_the_silence_path() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    // 8 silent ticks lose the trainer; 300 W from ticks 3..=7 is still inside 3 s
    r.through(15);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    feed(&r, 16, 30, 50);
    r.through(16);
    assert_eq!(
        r.modes(),
        vec![erg(300), erg(280), erg(260), erg(0), erg(50)]
    );
}

#[test]
fn a_pedalling_sample_without_power_ramps_from_zero_and_never_jumps() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 150);
    r.through(7);
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    r.through(9);
    assert_eq!(r.modes(), vec![erg(150), erg(160), erg(0)]);
    for k in 10..40 {
        r.events
            .try_send(TrainerEvent::Sample(TrainerSample {
                at: MonoTime(k * TICK_US + 1),
                power_w: None,
                cadence_rpm: Some(80),
                speed_mps: Some(8.0),
                distance_m: None,
                accumulated_power_w: None,
                event_count: None,
                status: TrainerStatus::default(),
                target_limit: TargetLimit::AtTarget,
            }))
            .unwrap();
    }
    r.through(10);
    // origin 0 W (no power in the new epoch), then the normal ramp: 0, 40, ... 200
    assert_eq!(r.modes(), vec![]);
    r.through(34);
    assert_eq!(
        r.modes(),
        vec![erg(40), erg(80), erg(120), erg(160), erg(200)]
    );
}

#[test]
fn an_hr_sample_from_before_an_hr_link_transition_does_not_enter_the_hold() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    feed(&r, 0, 12, 150);
    r.through(3);
    // HR sample in tick 4, then the strap link drops and returns inside the same window
    r.at(4 * TICK_US + 100_000);
    r.heart(4, 150);
    r.hr_link(LinkState::Lost);
    r.hr_link(LinkState::Ready);
    r.through(11);
    let rows = r.rows();
    assert_eq!(rows[1].1.hr, None, "stale HR sample must not be held");
    assert_eq!(rows[2].1.hr, None);
}

#[test]
fn soft_start_state_a_long_flow_does_not_survive_lost_and_ready() {
    // A: long-running flow (300 W through tick 39), Lost and Ready in tick 40, 50 W after
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 40, 300);
    r.through(39);
    assert_eq!(
        r.modes(),
        vec![erg(300), erg(280), erg(260), erg(240), erg(220), erg(200)]
    );
    r.at(40 * TICK_US + 100_000);
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    feed(&r, 40, 70, 50);
    r.through(40);
    assert_eq!(
        r.modes(),
        vec![erg(50)],
        "A: origin 50 W, not a jump to the target"
    );
    r.through(69);
    assert_eq!(
        r.modes(),
        vec![erg(80), erg(110), erg(140), erg(170), erg(200)]
    );
}

#[test]
fn soft_start_state_b_short_flow_does_not_survive_lost_and_ready() {
    // B: flow for 8 ticks only, Lost/Ready, 50 W in tick 8
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    r.through(7);
    r.modes();
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    feed(&r, 8, 30, 50);
    r.through(8);
    assert_eq!(r.modes(), vec![erg(50)], "B");
}

#[test]
fn soft_start_state_c_samples_while_connecting_do_not_survive_ready() {
    // C: 300 W samples while Connecting, Ready later, then 50 W
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    r.through(7);
    r.modes();
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    feed(&r, 8, 12, 300);
    r.through(11);
    assert!(r.modes().is_empty());
    r.link(LinkState::Ready);
    feed(&r, 12, 40, 50);
    r.through(12);
    assert_eq!(r.modes(), vec![erg(50)], "C");
}

#[test]
fn p1_a_second_silence_while_already_lost_resets_the_soft_start() {
    // 300 W, Lost/Ready, power-only samples (no cadence), 8 silent ticks, then 50 W
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    r.through(7);
    r.modes();
    r.link(LinkState::Lost);
    r.link(LinkState::Ready);
    for k in 8..12 {
        r.sample_opt(k, Some(300), None);
    }
    r.through(19);
    r.modes();
    feed(&r, 20, 40, 50);
    r.through(20);
    assert_eq!(r.modes().last(), Some(&erg(50)));
}

#[test]
fn p1b_the_same_through_silence_losses_only() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 300);
    r.through(15); // 8 silent ticks: lost
    r.modes();
    for k in 16..20 {
        r.sample_opt(k, Some(300), None);
    }
    r.through(27); // a second 8-tick silence while still paused
    r.modes();
    feed(&r, 28, 50, 50);
    r.through(28);
    assert_eq!(r.modes().last(), Some(&erg(50)));
}

#[test]
fn connecting_then_ready_without_lost_is_still_a_soft_start() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 150);
    r.through(7);
    assert_eq!(r.modes(), vec![erg(150), erg(160)]);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    feed(&r, 8, 30, 50);
    r.through(8);
    assert_eq!(
        r.modes(),
        vec![erg(50)],
        "ramp from 50 W, not the target at once"
    );
    // and with no sample at all after Ready: 0 W only
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    feed(&r, 0, 8, 150);
    r.through(7);
    r.modes();
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    r.through(12);
    assert_eq!(r.modes(), vec![erg(0)]);
}

mod soft_start_property {
    use super::*;
    use proptest::prelude::*;

    /// One generated step. `Stale` is a sample stamped before a link event that is
    /// delivered after it was sent (#239 N1): queued, then the link, then the tick.
    #[derive(Clone, Debug)]
    enum Op {
        Sample {
            power: Option<u16>,
            cadence: Option<u8>,
        },
        Stale {
            link: u8,
            power: Option<u16>,
            cadence: Option<u8>,
        },
        Hr(u8),
        Link(u8),
        Silence(u8),
        Skip,
        Pause,
        Resume,
        /// `ticks` pedalling samples in a row at one power and cadence: long enough for a
        /// soft start to be repaid.
        Ride {
            ticks: u8,
            power: u16,
            cadence: u8,
        },
    }
    fn op() -> impl Strategy<Value = Op> {
        let cadence = prop_oneof![
            Just(None),
            Just(Some(0_u8)),
            (1_u8..=120).prop_map(Some),
            (60_u8..=120).prop_map(Some),
        ];
        let power = prop::option::weighted(0.8, 0_u16..=400);
        prop_oneof![
            6 => (power.clone(), cadence.clone())
                .prop_map(|(power, cadence)| Op::Sample { power, cadence }),
            2 => (0_u8..3, power, cadence)
                .prop_map(|(link, power, cadence)| Op::Stale { link, power, cadence }),
            1 => (40_u8..180).prop_map(Op::Hr),
            2 => (0_u8..3).prop_map(Op::Link),
            2 => (0_u8..=20).prop_map(Op::Silence),
            2 => Just(Op::Skip),
            2 => Just(Op::Pause),
            2 => Just(Op::Resume),
            3 => (4_u8..=40, 0_u16..=400, 60_u8..=110).prop_map(|(ticks, power, cadence)| {
                Op::Ride { ticks, power, cadence }
            }),
        ]
    }
    /// A 600 s block (longer than any generated ride segment): free, flat or a ramp.
    #[derive(Clone, Copy, Debug)]
    enum Blk {
        Free,
        Flat(u16),
        Ramp(u16, u16),
    }
    impl Blk {
        fn flat_block(self) -> FlatBlock {
            match self {
                Blk::Free => block(600.0, 0.0, 0.0, Phase::Free),
                Blk::Flat(w) => block(600.0, f64::from(w), f64::from(w), Phase::Work),
                Blk::Ramp(a, b) => block(600.0, f64::from(a), f64::from(b), Phase::Work),
            }
        }
        fn peak(self) -> u16 {
            match self {
                Blk::Free => 0,
                Blk::Flat(w) => w,
                Blk::Ramp(a, b) => a.max(b),
            }
        }
    }
    fn blk() -> impl Strategy<Value = Blk> {
        prop_oneof![
            1 => Just(Blk::Free),
            1 => Just(Blk::Flat(0)),
            5 => (60_u16..=320).prop_map(Blk::Flat),
            2 => (60_u16..=320, 60_u16..=320).prop_map(|(a, b)| Blk::Ramp(a, b)),
        ]
    }
    /// A ride: its blocks and an optional health cap (DESIGN §15.2).
    fn ride() -> impl Strategy<Value = (Vec<Blk>, Option<u16>)> {
        (
            prop::collection::vec(blk(), 2..=6),
            prop::option::weighted(0.4, 120_u16..=400),
        )
    }

    struct Ramp {
        start: u64,
        origin: f64,
    }
    /// What ADR-0002 §7.4–7.5 and DESIGN §15.2 allow, restated without the engine's
    /// state. One epoch per applied link event or 8-tick silence; a soft start is owed
    /// after one, after leaving a free block and after a release, and is repaid only by a
    /// completed ERG ramp. A ramp block's target is only known as an interval (workout
    /// time freezes in pauses the oracle does not model): between its value at the block's
    /// start and at the block's session age.
    #[expect(
        clippy::struct_excessive_bools,
        reason = "an oracle: one flag per rule of the ADR it restates"
    )]
    struct Model {
        blocks: Vec<Blk>,
        cap: Option<u16>,
        block: usize,
        /// Session ticks since the current block was entered (an upper bound of its timer).
        age: u64,
        ready: bool,
        armed: bool,
        silent: u32,
        epoch: Vec<(u64, Option<u16>, Option<u8>)>,
        flow: Option<u64>,
        ramp: Option<Ramp>,
        released: bool,
        low: u32,
        high: u32,
        /// The rider's pause (DESIGN §5.2): 0 % resistance, a soft start owed on resume.
        user_paused: bool,
        /// The last mode the engine sent in this epoch (the engine forgets it at an epoch).
        current: Option<ControlMode>,
    }
    impl Model {
        fn new(blocks: Vec<Blk>, cap: Option<u16>) -> Self {
            Self {
                blocks,
                cap,
                block: 0,
                age: 0,
                ready: true,
                // a ride start is a reconnect (#250): the soft start is owed from tick 0
                armed: true,
                silent: 0,
                epoch: vec![],
                flow: None,
                ramp: None,
                released: false,
                low: 0,
                high: 0,
                user_paused: false,
                current: None,
            }
        }
        fn pause(&mut self) {
            self.user_paused = true;
        }
        fn resume(&mut self) {
            self.user_paused = false;
        }
        fn skipped(&mut self) {
            self.block += 1;
            self.age = 0;
        }
        /// A link epoch (§7.5, #231): an applied link transition or 8 silent ticks. What
        /// was sampled before it is forgotten and a soft start is owed. Nothing re-engages
        /// ERG by itself: a release is kept (§7.4 leaves it only after 60 rpm for 2 s).
        fn new_epoch(&mut self) {
            self.epoch.clear();
            self.flow = None;
            self.ramp = None;
            self.low = 0;
            self.high = 0;
            self.current = None;
            self.armed = true;
        }
        fn sample(&mut self, k: u64, power: Option<u16>, cadence: Option<u8>) {
            self.epoch.retain(|(j, ..)| *j != k);
            self.epoch.push((k, power, cadence));
        }
        fn link(&mut self, state: LinkState) {
            self.ready = state == LinkState::Ready;
            self.new_epoch();
        }
        fn held_cadence(&self, k: u64) -> Option<u8> {
            self.epoch
                .iter()
                .filter(|(j, ..)| *j <= k && k - j <= 4)
                .max_by_key(|(j, ..)| *j)
                .and_then(|(_, _, c)| *c)
        }
        /// The block target as an interval: value at the block start, value at its age.
        fn target(&self) -> Option<(f64, f64)> {
            Some(match *self.blocks.get(self.block)? {
                Blk::Free => return None,
                Blk::Flat(w) => (f64::from(w), f64::from(w)),
                Blk::Ramp(a, b) => {
                    #[expect(clippy::cast_precision_loss, reason = "tiny")]
                    let f = (self.age.min(2400) as f64) / 2400.0;
                    let end = f64::from(a) + (f64::from(b) - f64::from(a)) * f;
                    (f64::from(a).min(end), f64::from(a).max(end))
                }
            })
        }
        /// Check what the engine sent in the tick just processed.
        fn check(&self, k: u64, modes: &[ControlMode], want: &Want) -> Result<(), TestCaseError> {
            match want {
                Want::Nothing => prop_assert!(modes.is_empty(), "k={k}: {modes:?} on a dead link"),
                Want::Over => prop_assert!(
                    modes
                        .iter()
                        .all(|m| *m == ControlMode::Resistance { pct: 0.0 }),
                    "k={k}: {modes:?} after the ride's end"
                ),
                Want::Mode(m) => {
                    for x in modes {
                        prop_assert_eq!(x, m, "k={}", k);
                    }
                    prop_assert_eq!(self.current.as_ref(), Some(m), "k={}", k);
                }
                Want::Hold => {
                    for x in modes {
                        prop_assert_eq!(*x, erg(0), "k={} not pedalling", k);
                    }
                    if !matches!(self.current, Some(ControlMode::Erg { .. })) {
                        prop_assert_eq!(
                            self.current,
                            Some(erg(0)),
                            "k={}: owed soft start, not pedalling, must hold 0 W",
                            k
                        );
                    }
                }
                Want::Erg(lo, hi) => {
                    let ok = |m: &ControlMode| {
                        matches!(*m, ControlMode::Erg { target_w }
                            if (*lo - 1.0..=*hi + 1.0).contains(&f64::from(target_w)))
                    };
                    for m in modes {
                        prop_assert!(ok(m), "k={k}: {m:?} outside [{lo}, {hi}]");
                    }
                    prop_assert!(
                        self.current.as_ref().is_some_and(ok),
                        "k={k}: engine holds {:?}, rules expect [{lo}, {hi}] (armed {}, flow {:?})",
                        self.current,
                        self.armed,
                        self.flow
                    );
                }
            }
            Ok(())
        }
        /// After tick `k` was processed; `modes` are the commands it sent.
        fn tick(
            &mut self,
            k: u64,
            had_sample: bool,
            modes: &[ControlMode],
        ) -> Result<(), TestCaseError> {
            if had_sample {
                self.silent = 0;
            } else {
                self.silent += 1;
                if self.silent == 8 {
                    self.new_epoch();
                }
            }
            let cadence = self.held_cadence(k);
            if cadence.is_some_and(|c| c > 0) {
                self.flow.get_or_insert(k);
            } else {
                self.flow = None;
            }
            let applied = self.current; // what was on the trainer when the tick began
            self.current = modes.last().copied().or(self.current);
            self.age += 1;
            let want = self.want(k, applied);
            self.check(k, modes, &want)
        }
        /// What the rules require at tick `k` (state updated as a side effect).
        fn want(&mut self, k: u64, applied: Option<ControlMode>) -> Want {
            if self.block >= self.blocks.len() {
                return Want::Over; // finished: only the end mode may still be drained
            }
            if !self.ready {
                return Want::Nothing;
            }
            let cadence = self.held_cadence(k);
            // §7.4: a release is left only by 60 rpm for 2 s (in any block kind, and it
            // survives link epochs and free blocks), and then the §7.5 soft start is owed.
            if self.released {
                self.high = if cadence.is_some_and(|c| c >= 60) {
                    self.high + 1
                } else {
                    0
                };
                if self.high >= 8 {
                    self.released = false;
                    self.armed = true;
                    self.ramp = None;
                    self.low = 0;
                }
            }
            if self.user_paused {
                // DESIGN §5.2 (#264): 0 % resistance, and the soft start is owed on resume
                self.armed = true;
                self.ramp = None;
                self.low = 0;
                return Want::Mode(ControlMode::Resistance { pct: 0.0 });
            }
            let Some((lo, hi)) = self.target() else {
                // §7.9: a free block is explicit flat-road SIM; ERG afterwards owes a soft start
                self.armed = true;
                self.ramp = None;
                self.low = 0;
                return Want::Mode(ControlMode::Free);
            };
            let _ = hi;
            // §7.4 (architect decision, round 5): judged against what is applied on the
            // trainer. A non-zero ERG on the trainer and cadence < 40 rpm (unknown is 0)
            // for 2 s releases, whatever the policy's own target, debt or flow.
            if !self.released {
                let torque = matches!(applied, Some(ControlMode::Erg { target_w }) if target_w > 0);
                self.low = if torque && cadence.is_none_or(|c| c < 40) {
                    self.low + 1
                } else {
                    0
                };
                if self.low >= 8 {
                    self.released = true;
                    self.high = 0;
                }
            }
            if self.released {
                return Want::Mode(ControlMode::Resistance { pct: 0.0 });
            }
            let clamp = |w: f64| self.cap.map_or(w, |c| w.min(f64::from(c)));
            if !self.armed {
                return Want::Erg(clamp(lo), clamp(hi));
            }
            let Some(flow) = self.flow else {
                self.ramp = None;
                return Want::Hold;
            };
            let _ = flow;
            if self.ramp.is_none() {
                let live: Vec<f64> = self
                    .epoch
                    .iter()
                    .filter(|(j, ..)| k - j < 12)
                    .filter_map(|(_, p, _)| p.map(f64::from))
                    .collect();
                #[expect(clippy::cast_precision_loss, reason = "tiny")]
                let origin = if live.is_empty() {
                    0.0
                } else {
                    live.iter().sum::<f64>() / live.len() as f64
                };
                self.ramp = Some(Ramp { start: k, origin });
            }
            let r = self.ramp.as_ref().expect("just set");
            if k - r.start >= 20 {
                // five 1 Hz steps are done: the debt is repaid
                self.armed = false;
                self.ramp = None;
                return Want::Erg(clamp(lo), clamp(hi));
            }
            // one step per second, five steps, from the origin: no jump, and a target that
            // changes mid-ramp is followed
            #[expect(clippy::cast_precision_loss, reason = "tiny")]
            let step = ((k - r.start) / 4) as f64;
            let at = |t: f64| clamp(r.origin + (t - r.origin) * step / 5.0);
            Want::Erg(at(lo).min(at(hi)), at(lo).max(at(hi)))
        }
    }
    /// An output monitor that knows nothing of the policy: it sees only the command
    /// stream and the inputs (samples, link events, user pause and resume, block kinds, the
    /// cap) and asserts the safety properties of ADR-0002 §7.4–7.5, §7.9 and DESIGN
    /// §5.2 / §15.2. It exists because an oracle that shares the policy's structure shares
    /// its holes (round 5, the stop-in-soft-start hole): this one cannot. It has no
    /// exemptions: the ride start is an engagement like a reconnect (#250), a pause is
    /// 0 % resistance (#264) and the end leaves no torque on the trainer (#266).
    #[expect(
        clippy::struct_excessive_bools,
        reason = "an oracle: one flag per rule of the ADR it restates"
    )]
    struct Monitor {
        cap: Option<u16>,
        ready: bool,
        applied: Option<ControlMode>,
        hold: Option<(u64, Option<u16>, Option<u8>)>,
        powers: Vec<(u64, u16)>,
        pedalled: bool,
        silent: u32,
        low: u32,
        high: u32,
        released: bool,
        last_block: Option<usize>,
        /// The rider's pause, and the 3 s of held zero power and cadence of an auto-pause.
        user_paused: bool,
        zero_run: u32,
        stopped: bool,
        /// The last tick processed, and whether it was paused.
        now: Option<u64>,
        paused_before: bool,
        /// The next non-zero ERG command is the first of an engagement: a ride start, an
        /// epoch, a pause, or a return from `Free`, SIM or resistance.
        engaging: bool,
        /// The block the engagement began in: a block change inside a soft start is a
        /// discontinuity by design (ADR-0002 §7.8 a; the ramp continues at its progress, DESIGN §17.3),
        /// so the origin bound is not asked of it.
        engage_block: Option<usize>,
    }
    /// What may stand on the trainer when no torque is allowed: no power target, no
    /// resistance.
    fn no_torque(m: Option<ControlMode>) -> bool {
        match m {
            Some(ControlMode::Erg { target_w }) => target_w == 0,
            Some(ControlMode::Resistance { pct }) => pct == 0.0,
            Some(_) => true,
            None => false,
        }
    }
    impl Monitor {
        fn new(cap: Option<u16>) -> Self {
            Self {
                cap,
                ready: true,
                applied: None,
                hold: None,
                powers: vec![],
                // a ride start is a reconnect (#250): no torque before a pedalling sample
                pedalled: false,
                silent: 0,
                low: 0,
                high: 0,
                released: false,
                last_block: None,
                user_paused: false,
                zero_run: 0,
                stopped: false,
                now: None,
                paused_before: false,
                engaging: true,
                engage_block: None,
            }
        }

        /// An applied link transition, or 8 silent ticks: the old link is forgotten.
        fn epoch(&mut self) {
            self.applied = None;
            self.hold = None;
            self.powers.clear();
            self.pedalled = false;
            self.low = 0;
            // cadence of the old link does not count towards a re-engagement (§7.4)
            self.high = 0;
            self.engaging = true;
        }
        fn link(&mut self, state: LinkState) {
            self.ready = state == LinkState::Ready;
            self.epoch();
        }
        fn pause(&mut self) {
            self.user_paused = true;
        }
        fn resume(&mut self) {
            self.user_paused = false;
        }
        fn absorb(&mut self, modes: &[ControlMode]) {
            if let Some(m) = modes.last() {
                self.applied = Some(*m);
            }
        }
        /// The ride is over (finished by itself or stopped): the last command that reached
        /// a ready link is not torque (ADR-0002 §7.9, #266).
        fn end(&mut self, modes: &[ControlMode]) -> Result<(), TestCaseError> {
            self.absorb(modes);
            if self.ready {
                prop_assert!(
                    no_torque(self.applied),
                    "ride over, {:?} left on the trainer",
                    self.applied
                );
            }
            Ok(())
        }
        /// Take in what the tick brought: its valid sample (a stale one is not), the epoch of
        /// 8 silent ticks and the auto-pause of 12 held zeros. Returns the held cadence.
        fn ingest(&mut self, k: u64, sample: Option<(Option<u16>, Option<u8>)>) -> u8 {
            self.now = Some(k);
            if let Some((power, cadence)) = sample {
                self.silent = 0;
                self.hold = Some((k, power, cadence));
                if let Some(p) = power {
                    self.powers.push((k, p));
                }
                if cadence.is_some_and(|c| c > 0) {
                    self.pedalled = true;
                }
            } else {
                self.silent += 1;
                if self.silent == 8 {
                    self.epoch();
                }
            }
            let held = self.hold.filter(|(j, ..)| k - j <= 4);
            let cadence = held.and_then(|(_, _, c)| c).unwrap_or(0);
            // auto-pause (ADR-0001 §5): 3 s of held power 0 and cadence 0
            if held.is_some_and(|(_, p, c)| p == Some(0) && c == Some(0)) {
                self.zero_run += 1;
            } else {
                self.zero_run = 0;
            }
            if self.zero_run >= 12 && !self.stopped && !self.user_paused {
                self.stopped = true;
                self.engaging = true;
            }
            if self.stopped && cadence > 0 {
                self.stopped = false;
                self.zero_run = 0;
            }
            cadence
        }
        /// What must hold on the trainer after the tick, whatever was sent in it.
        fn judge_state(&self, k: u64, paused: bool) -> Result<(), TestCaseError> {
            let torque = |m: Option<ControlMode>| matches!(m, Some(ControlMode::Erg { target_w }) if target_w > 0);
            // 1: 2 s below 40 rpm with torque applied ⇒ released by the end of the window
            if self.low >= 8 {
                prop_assert!(
                    !torque(self.applied),
                    "k={k}: {:?} still applied after {} ticks below 40 rpm",
                    self.applied,
                    self.low
                );
            }
            // 6: a pause (the rider's or the stopped rider's) leaves no torque on a ready
            // link, and the rider's pause is exactly 0 % resistance
            if paused && self.ready {
                prop_assert!(
                    no_torque(self.applied),
                    "k={k}: paused with {:?} applied",
                    self.applied
                );
            }
            if self.user_paused && self.ready {
                prop_assert_eq!(
                    self.applied,
                    Some(ControlMode::Resistance { pct: 0.0 }),
                    "k={}: the rider's pause is 0 % resistance",
                    k
                );
            }
            Ok(())
        }
        /// The origin of a soft start: the mean of the 3 s power of the new epoch.
        fn origin(&self, k: u64) -> f64 {
            let live: Vec<f64> = self
                .powers
                .iter()
                .filter(|(j, _)| k - j < 12)
                .map(|(_, p)| f64::from(*p))
                .collect();
            #[expect(clippy::cast_precision_loss, reason = "tiny")]
            let mean = live.iter().sum::<f64>() / live.len() as f64;
            if live.is_empty() { 0.0 } else { mean }
        }
        /// One processed tick. `sample` is a *valid* sample of the tick (a stale one is not).
        fn tick(
            &mut self,
            k: u64,
            sample: Option<(Option<u16>, Option<u8>)>,
            block: Option<(usize, Blk)>,
            modes: &[ControlMode],
        ) -> Result<(), TestCaseError> {
            if block.is_none() {
                // finished: the end mode is the only thing sent; `end` judges it
                self.absorb(modes);
                return Ok(());
            }
            let cadence = self.ingest(k, sample);
            let before = self.applied;
            // rule 1 bookkeeping: ticks on which non-zero ERG was applied at low cadence
            let torque = |m: Option<ControlMode>| matches!(m, Some(ControlMode::Erg { target_w }) if target_w > 0);
            self.low = if torque(before) && cadence < 40 {
                self.low + 1
            } else {
                0
            };
            // the engine judges a release only on ticks with a ready link (§7.4): cadence
            // that arrives while the link is down is not counted
            self.high = if cadence >= 60 && self.ready {
                self.high + 1
            } else {
                0
            };
            let was_released = self.released;
            if self.released && self.high >= 8 {
                self.released = false;
            }
            let origin = self.origin(k);
            let peak = block.map_or(0, |(_, b)| b.peak());
            let step = f64::from(peak).div_euclid(5.0) + 2.0;
            let paused = self.user_paused || self.stopped;
            // the first tick of a pause and the first one after it begin an engagement
            // (a pause and resume between two ticks change nothing)
            if paused || self.paused_before {
                self.engaging = true;
                // the soft start is owed again: the rider must be seen pedalling now
                self.pedalled = cadence > 0;
            }
            self.paused_before = paused;
            if self.engaging && self.engage_block.is_none() {
                self.engage_block = block.map(|(i, _)| i);
            }
            let mut prev = before;
            for m in modes {
                if let ControlMode::Erg { target_w } = *m
                    && target_w > 0
                {
                    // 6 (DESIGN §5.2, #264): no torque while paused
                    prop_assert!(!paused, "k={k}: {m:?} while paused");
                    // 5: while released, nothing until 60 rpm for 2 s
                    prop_assert!(
                        !self.released,
                        "k={k}: {m:?} while released ({was_released})"
                    );
                    // 2: not before the rider pedalled since the epoch / release / start
                    prop_assert!(self.pedalled, "k={k}: {m:?} before any pedalling sample");
                    // 3: the cap
                    if let Some(c) = self.cap {
                        prop_assert!(target_w <= c, "k={k}: {m:?} above cap {c}");
                    }
                    // 4: the first command of an engagement (ride start, epoch, pause or
                    // resume, return from free/SIM/resistance) is the 3 s mean of the new
                    // epoch's power plus at most one ramp step: never the block target
                    if self.engaging {
                        if self.engage_block == block.map(|(i, _)| i) {
                            prop_assert!(
                                f64::from(target_w) <= origin + step,
                                "k={k}: {m:?} engages more than a step ({step}) above the \
                                 3 s mean {origin}"
                            );
                        }
                        self.engaging = false;
                        self.engage_block = None;
                    }
                    // 4: no jump afterwards: one ramp step above what was applied (or above
                    // the rider's own power); a block change is a discontinuity by design
                    let same_block = self.last_block == block.map(|(i, _)| i);
                    if k > 0 && same_block {
                        // a ramp restarts from the rider's current power (§7.5), so that
                        // is as good a base as the previous command
                        let base = match prev {
                            Some(ControlMode::Erg { target_w: w }) if w > 0 => {
                                f64::from(w).max(origin)
                            }
                            _ => origin,
                        };
                        prop_assert!(
                            f64::from(target_w) <= base + step,
                            "k={k}: {m:?} is more than a step ({step}) above {base}"
                        );
                    }
                }
                if matches!(m, ControlMode::Resistance { .. }) && !paused && !self.released {
                    // a release (not a re-issue of one after an epoch)
                    self.released = true;
                    self.high = 0;
                    self.pedalled = false;
                }
                if !matches!(m, ControlMode::Erg { .. }) {
                    self.engaging = true;
                }
                prev = Some(*m);
            }
            if !modes.is_empty() {
                self.applied = prev;
            }
            self.last_block = block.map(|(i, _)| i);
            self.judge_state(k, paused)?;
            Ok(())
        }
    }
    enum Want {
        Nothing,
        Over,
        Mode(ControlMode),
        Hold,
        Erg(f64, f64),
    }

    fn state(i: u8) -> LinkState {
        [LinkState::Lost, LinkState::Connecting, LinkState::Ready][usize::from(i)]
    }
    /// Run several rides on one engine, so nothing may leak from one to the next.
    #[expect(
        clippy::too_many_lines,
        reason = "one pass over a ride's ops, with the oracle and the monitor beside the engine"
    )]
    fn execute(rides: &[(Vec<Blk>, Option<u16>, Vec<Op>)]) -> Result<(), TestCaseError> {
        let mut r = rig();
        let mut k = 0_u64; // absolute tick, kept across rides
        for (blocks, cap, ops) in rides {
            r.link(LinkState::Ready);
            r.at(k * TICK_US);
            r.engine
                .command(EngineCmd::Load(RideSetup {
                    blocks: blocks.iter().map(|b| b.flat_block()).collect(),
                    recorder: Box::new(Recorder(r.log.clone())),
                    cap_w: *cap,
                }))
                .expect("load");
            let expected = match cap {
                Some(_) if blocks.iter().any(|b| matches!(b, Blk::Free)) => {
                    Some(CommandError::CapRequiresErg)
                }
                Some(c) if blocks.iter().any(|b| b.peak() > *c) => {
                    Some(CommandError::CapExceededTarget)
                }
                _ => None,
            };
            let started = r.engine.command(EngineCmd::Start);
            let refused = started.is_err();
            prop_assert_eq!(started.err(), expected);
            if refused {
                r.engine.command(EngineCmd::Stop).expect("unload");
                continue;
            }
            let mut m = Model::new(blocks.clone(), *cap);
            let mut mon = Monitor::new(*cap);
            // `valid`: the tick's valid sample, which the monitor sees as an input
            let step = |r: &mut Rig,
                        m: &mut Model,
                        mon: &mut Monitor,
                        k: &mut u64,
                        valid: Option<(Option<u16>, Option<u8>)>| {
                r.through(*k);
                let modes = r.modes();
                let block = m.blocks.get(m.block).map(|b| (m.block, *b));
                let out = m
                    .tick(*k, valid.is_some(), &modes)
                    .and_then(|()| mon.tick(*k, valid, block, &modes));
                *k += 1;
                out
            };
            let mut over = false;
            for op in ops {
                match op {
                    Op::Sample { power, cadence } => {
                        r.sample_opt(k, *power, *cadence);
                        m.sample(k, *power, *cadence);
                        step(&mut r, &mut m, &mut mon, &mut k, Some((*power, *cadence)))?;
                    }
                    Op::Ride {
                        ticks,
                        power,
                        cadence,
                    } => {
                        for _ in 0..*ticks {
                            r.sample(k, *power, *cadence);
                            m.sample(k, Some(*power), Some(*cadence));
                            step(
                                &mut r,
                                &mut m,
                                &mut mon,
                                &mut k,
                                Some((Some(*power), Some(*cadence))),
                            )?;
                        }
                    }
                    Op::Stale {
                        link,
                        power,
                        cadence,
                    } => {
                        // sent before the link event, processed after it: dropped as stale
                        r.sample_opt(k, *power, *cadence);
                        r.link(state(*link));
                        m.link(state(*link));
                        mon.link(state(*link));
                        step(&mut r, &mut m, &mut mon, &mut k, None)?;
                    }
                    Op::Silence(n) => {
                        for _ in 0..*n {
                            step(&mut r, &mut m, &mut mon, &mut k, None)?;
                        }
                    }
                    Op::Link(i) => {
                        r.link(state(*i));
                        m.link(state(*i));
                        mon.link(state(*i));
                    }
                    Op::Hr(bpm) => r.heart(k, *bpm),
                    Op::Skip => {
                        if r.engine.command(EngineCmd::SkipBlock).is_ok() {
                            m.skipped();
                        }
                    }
                    Op::Pause => {
                        if r.engine.command(EngineCmd::Pause).is_ok() {
                            m.pause();
                            mon.pause();
                        }
                    }
                    Op::Resume => {
                        if r.engine.command(EngineCmd::Resume).is_ok() {
                            m.resume();
                            mon.resume();
                        }
                    }
                }
                // the ride just ended by itself (last block, or a skip past it): judged
                // now, before later link events can be mistaken for part of it
                if !over && r.status().state == RideState::Finished {
                    over = true;
                    mon.end(&r.modes())?;
                }
            }
            // every second of the ride is in exactly one compliance bucket (§7.8)
            let st = r.status();
            prop_assert_eq!(
                u64::from(st.compliance.counted_s + st.compliance.excluded_s),
                st.session.0 / 4,
                "{:?}",
                st.compliance
            );
            // the ride ends (Stop, or already finished by itself): no torque is left on the
            // trainer (ADR-0002 §7.9, #266). The next ride begins with the engine in
            // whatever state this one left it.
            let _ = r.engine.command(EngineCmd::Stop);
            if !over {
                mon.end(&r.modes())?;
            }
            // the driver confirms the end mode; until it does the next ride sends nothing (#279)
            r.delivered(ControlMode::Resistance { pct: 0.0 });
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(5000))]
        #[test]
        fn the_soft_start_never_jumps_and_never_uses_old_epoch_power(
            rides in prop::collection::vec(
                (ride(), prop::collection::vec(op(), 1..60))
                    .prop_map(|((b, c), o)| (b, c, o)),
                1..=3,
            )
        ) {
            execute(&rides)?;
        }
    }
}

// ---- #329: the trainer-loss fields of the status ---------------------------------

/// The five loss fields as one comparable tuple:
/// (`user_paused`, lost since, attempt, soft-start target, (last gap, reconnected at)).
type LossFields = (
    bool,
    Option<u64>,
    Option<u32>,
    Option<u16>,
    (Option<u64>, Option<u64>),
);
fn loss_fields(r: &Rig) -> LossFields {
    let st = r.status();
    (
        st.user_paused,
        st.trainer_lost_since.map(|t| t.0),
        st.reconnect_attempt,
        st.soft_start_target_w,
        (st.last_gap.map(|t| t.0), st.reconnected_at.map(|t| t.0)),
    )
}

/// A rig with one 200 W work block, ridden for 8 ticks at 150 W / 90 rpm (session 8).
fn riding_rig() -> Rig {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..8 {
        r.sample(k, 150, 90);
    }
    r.through(7);
    r
}
const NONE_YET: LossFields = (false, None, None, None, (None, None));

#[test]
fn the_loss_fields_follow_lose_attempts_reconnect_soft_start_and_steady() {
    let mut r = riding_rig();
    // steady, and the soft start every ride begins with is not a "reconnected" state (#250)
    assert_eq!(loss_fields(&r), NONE_YET);

    // lost at the tick boundary where the engine applies the link event (session tick 8)
    r.link(LinkState::Lost);
    r.through(9);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(loss_fields(&r), (false, Some(8), None, None, (None, None)));

    // the driver's attempts, in order; `lost_since` does not move while the loss lasts
    r.reconnecting(1);
    r.through(11);
    assert_eq!(
        loss_fields(&r),
        (false, Some(8), Some(1), None, (None, None))
    );
    r.reconnecting(2);
    r.link(LinkState::Connecting);
    r.through(12);
    assert_eq!(
        loss_fields(&r),
        (false, Some(8), Some(2), None, (None, None))
    );

    // link ready but nothing has flowed yet: still lost, no soft start is shown yet
    r.link(LinkState::Ready);
    r.through(13);
    assert_eq!(
        loss_fields(&r),
        (false, Some(8), Some(2), None, (None, None))
    );

    // the first pedalling sample of the new epoch: back after a gap of 7 ticks (8 -> 15),
    // the soft start shows the target it is easing back to (the block's 200 W)
    r.sample(14, 90, 85);
    r.through(14);
    assert_eq!(
        loss_fields(&r),
        (false, None, None, Some(200), (Some(7), Some(15)))
    );

    // the soft start runs its 20 ticks, then the target goes; the gap and its end stay
    for k in 15..40 {
        r.sample(k, 150, 85);
    }
    r.through(34);
    assert_eq!(r.status().soft_start_target_w, Some(200), "still easing in");
    r.through(39);
    assert_eq!(
        loss_fields(&r),
        (false, None, None, None, (Some(7), Some(15))),
        "steady again"
    );

    // the next loss starts clean: the previous gap is gone
    r.link(LinkState::Lost);
    r.through(41);
    let st = loss_fields(&r);
    assert_eq!(st.2, None);
    assert_eq!(st.4, (None, None));
    assert!(st.1.is_some_and(|t| t >= 40));
}

#[test]
fn an_attempt_arriving_right_after_the_lost_it_belongs_to_is_not_wiped_by_it() {
    // Both are received before the tick that applies them; the attempt must apply after the
    // loss (which starts the count afresh), or attempt 1 would never show.
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.reconnecting(1);
    r.through(9);
    assert_eq!(r.status().reconnect_attempt, Some(1));
    assert_eq!(r.status().trainer_lost_since, Some(SessionTicks(8)));
}

#[test]
fn a_long_loss_keeps_its_first_stamp_through_later_silence_epochs() {
    // 8 silent ticks start a new epoch on their own, also while already lost: the stamp the
    // ">= 30 s" rows count from must stay at the first moment of the loss.
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    assert_eq!(r.status().trainer_lost_since, Some(SessionTicks(8)));
    r.through(40);
    assert_eq!(r.status().trainer_lost_since, Some(SessionTicks(8)));
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
}

#[test]
fn a_silence_loss_is_stamped_and_ends_when_samples_return() {
    // No link event at all: 8 silent ticks decide the loss; a sample ends it.
    let mut r = riding_rig();
    r.through(15);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    let (_, lost_since, attempt, soft, gap) = loss_fields(&r);
    assert!(lost_since.is_some(), "silence is a loss");
    assert_eq!((attempt, soft, gap), (None, None, (None, None)));
    r.sample(16, 90, 85);
    r.through(16);
    let st = r.status();
    assert_eq!(st.trainer_lost_since, None);
    assert_eq!(
        st.last_gap.map(|g| g.0),
        Some(st.reconnected_at.unwrap().0 - lost_since.unwrap())
    );
    assert!(st.last_gap.unwrap().0 > 0);
}

#[test]
fn a_rider_pause_stays_visible_under_a_trainer_loss_and_after_it() {
    let mut r = riding_rig();
    assert!(!r.status().user_paused);
    r.engine.command(EngineCmd::Pause).unwrap();
    assert_eq!(r.state(), RideState::Paused(PauseReason::User));
    assert!(r.status().user_paused);
    // the loss outranks the pause in `state`, but the rider's pause is still theirs to resume
    r.link(LinkState::Lost);
    r.through(9);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert!(
        r.status().user_paused,
        "masked in `state`, not in `user_paused`"
    );
    r.link(LinkState::Ready);
    r.sample(11, 100, 80);
    r.through(11);
    assert_eq!(r.state(), RideState::Paused(PauseReason::User));
    assert!(r.status().user_paused);
    // a rider pause is not a soft start owed for a reconnect: nothing to ease back to while paused
    assert_eq!(r.status().soft_start_target_w, None);
    r.engine.command(EngineCmd::Resume).unwrap();
    assert!(!r.status().user_paused);
    assert_eq!(r.state(), RideState::Riding);
}

#[test]
fn a_loss_without_a_rider_pause_is_not_user_paused() {
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert!(!r.status().user_paused);
}

#[test]
fn no_loss_field_is_set_for_a_ride_that_never_lost_the_trainer() {
    let mut r = riding_rig();
    for k in 8..30 {
        r.sample(k, 150, 90);
    }
    r.through(29);
    // including the soft start owed at the ride's start (#250), which is not a reconnect
    assert_eq!(loss_fields(&r), NONE_YET);
}

#[test]
fn a_soft_start_owed_on_resume_is_not_shown_as_a_reconnected_state() {
    // Loss, back, and the soft start runs out: the "reconnected" state is over. A later rider
    // pause owes another soft start on resume, but that is not a reconnect and has no target row.
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    r.link(LinkState::Ready);
    r.through(12);
    for k in 14..60 {
        r.sample(k, 150, 90);
    }
    r.through(45);
    assert_eq!(
        r.status().soft_start_target_w,
        None,
        "the soft start is over"
    );
    r.engine.command(EngineCmd::Pause).unwrap();
    r.through(46);
    r.engine.command(EngineCmd::Resume).unwrap();
    for k in 47..58 {
        r.through(k);
        assert_eq!(
            r.status().soft_start_target_w,
            None,
            "tick {k}: a resume's soft start is not a reconnect"
        );
    }
    // and it did ease back in: a ramp ran (the trainer got several different ERG targets)
    assert!(r.modes().len() > 2);
}

// ---- #329 review: a ride that ends reports no loss state, and the edge cases of the loss tracking ----

/// Every loss field in its "nothing" value.
fn assert_no_loss_state(r: &Rig, when: &str) {
    assert_eq!(loss_fields(r), NONE_YET, "{when}");
}

#[test]
fn a_ride_stopped_under_a_rider_pause_and_a_loss_reports_no_loss_state() {
    let mut r = riding_rig();
    r.engine.command(EngineCmd::Pause).unwrap();
    r.link(LinkState::Lost);
    r.reconnecting(1);
    r.reconnecting(2);
    r.through(11);
    let (user, lost, attempt, ..) = loss_fields(&r);
    assert!(
        (user, lost.is_some(), attempt) == (true, true, Some(2)),
        "setup"
    );
    r.engine.command(EngineCmd::Stop).unwrap();
    assert_eq!(r.state(), RideState::Finished);
    assert_no_loss_state(&r, "finished under a pause and a loss");
}

#[test]
fn a_ride_stopped_during_a_loss_reports_no_loss_state() {
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.reconnecting(1);
    r.through(10);
    assert!(r.status().trainer_lost_since.is_some(), "setup");
    r.engine.command(EngineCmd::Stop).unwrap();
    assert_eq!(r.state(), RideState::Finished);
    assert_no_loss_state(&r, "stopped while lost");
}

#[test]
fn a_ride_stopped_in_the_soft_start_after_a_loss_reports_no_loss_state() {
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    r.link(LinkState::Ready);
    r.sample(11, 90, 85);
    r.through(11);
    assert_eq!(r.status().soft_start_target_w, Some(200), "setup");
    assert!(r.status().last_gap.is_some(), "setup");
    r.engine.command(EngineCmd::Stop).unwrap();
    assert_eq!(r.state(), RideState::Finished);
    assert_no_loss_state(&r, "stopped in the soft start");
}

#[test]
fn a_ride_that_finishes_by_itself_in_the_soft_start_reports_no_loss_state() {
    // A 5 s block is 20 ticks of workout time. Ticks 0-3 are ridden before the loss; the trainer is lost at
    // tick 4, link Ready is applied at tick 6, the first pedalling sample of the new epoch (tick 7) is the
    // trainer being back, and riding resumes at tick 8. The block therefore ends at tick 23, inside the soft
    // start, which runs 20 ticks from that first sample (ticks 7 to 27).
    let mut r = rig();
    r.load(vec![block(5.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    for k in 0..4 {
        r.sample(k, 150, 90);
    }
    r.through(3);
    r.link(LinkState::Lost);
    r.through(5);
    r.link(LinkState::Ready);
    r.through(6);
    for k in 7..40 {
        r.sample(k, 150, 90);
    }
    // one tick before the end the ride is still riding and the soft start is still running
    r.through(22);
    assert_eq!(r.state(), RideState::Riding);
    assert_eq!(
        r.status().soft_start_target_w,
        Some(200),
        "the soft start is still running when the ride is about to end"
    );
    r.through(39);
    assert_eq!(r.state(), RideState::Finished);
    assert_no_loss_state(&r, "finished by itself in the soft start");
}

#[test]
fn a_sample_while_the_link_is_not_ready_is_not_the_trainer_being_back() {
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    r.link(LinkState::Connecting);
    // a pedalling sample of the new epoch, but the link is only connecting
    r.sample(10, 90, 85);
    r.sample(11, 90, 85);
    r.through(11);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(
        r.status().trainer_lost_since,
        Some(SessionTicks(8)),
        "still lost"
    );
    assert_eq!(r.status().last_gap, None);
    // ready, and the next sample: now it is back
    r.link(LinkState::Ready);
    r.sample(12, 90, 85);
    r.through(12);
    assert_eq!(r.status().trainer_lost_since, None);
    assert!(r.status().last_gap.is_some());
}

#[test]
fn a_second_loss_in_the_middle_of_a_soft_start_starts_clean() {
    let mut r = riding_rig();
    r.link(LinkState::Lost);
    r.through(9);
    r.link(LinkState::Ready);
    r.sample(11, 90, 85);
    r.through(11);
    for k in 12..20 {
        r.sample(k, 150, 90);
    }
    r.through(19);
    assert_eq!(r.status().soft_start_target_w, Some(200), "mid soft start");
    assert!(r.status().last_gap.is_some());
    // the trainer drops again while the ramp is running
    r.link(LinkState::Lost);
    r.reconnecting(1);
    r.through(21);
    let st = loss_fields(&r);
    assert_eq!(st.3, None, "no soft start to show while lost again");
    assert_eq!(
        st.4,
        (None, None),
        "the previous gap belongs to the previous loss"
    );
    assert!(st.1.is_some_and(|t| t >= 20), "a new stamp");
    assert_eq!(st.2, Some(1));
    // and it recovers into a fresh soft start with its own gap
    r.link(LinkState::Ready);
    r.sample(24, 90, 85);
    r.through(24);
    let st = loss_fields(&r);
    assert_eq!((st.1, st.2, st.3), (None, None, Some(200)));
    assert!(
        st.4.0.is_some_and(|g| g < 8),
        "the second, short gap: {:?}",
        st.4
    );
}
