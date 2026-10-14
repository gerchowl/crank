//! Rule-level tests: each drives the actor synchronously on a manual clock, so every
//! assertion is about exact ticks. Scenario tests with the simulator are in
//! `tests/scenarios.rs`.
use crate::control::{BlockTarget, PolicyContext, TargetPolicy};
use crate::engine::{CommandError, Engine, EngineCmd, PauseReason, RideSetup, RideState};
use crate::record::{RecordError, RideRecorder, SecondRow};
use crate::ticks::{Hold, TICK_US, TickWindow};
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

struct ManualClock(AtomicU64);
impl Clock for ManualClock {
    fn now(&self) -> MonoTime {
        MonoTime(self.0.load(Ordering::SeqCst))
    }
    fn sleep_until(&self, _: MonoTime) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}
#[derive(Default)]
struct Log {
    started: bool,
    finished: bool,
    rows: Vec<(u32, SecondRow)>,
    fail_after: Option<usize>,
}
struct Recorder(Arc<Mutex<Log>>);
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
struct Rig {
    engine: Engine,
    clock: Arc<ManualClock>,
    events: mpsc::Sender<TrainerEvent>,
    hr: mpsc::Sender<HrEvent>,
    control: mpsc::Receiver<TrainerCommand>,
    log: Arc<Mutex<Log>>,
    gate: Arc<Mutex<Vec<bool>>>,
}
fn block(dur_s: f64, from: f64, to: f64, phase: Phase) -> FlatBlock {
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
fn rig() -> Rig {
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
    fn load(&mut self, blocks: Vec<FlatBlock>) {
        let rec = Box::new(Recorder(self.log.clone()));
        self.engine
            .command(EngineCmd::Load(RideSetup {
                blocks,
                recorder: rec,
            }))
            .unwrap();
    }
    fn start(&mut self) {
        self.engine.command(EngineCmd::Start).unwrap();
    }
    /// A trainer sample arriving at the start of tick `k`.
    fn sample(&self, k: u64, power: u16, cadence: u8) {
        self.events
            .try_send(TrainerEvent::Sample(TrainerSample {
                at: MonoTime(k * TICK_US + 1),
                power_w: Some(power),
                cadence_rpm: Some(cadence),
                speed_mps: Some(10.0),
                distance_m: None,
                accumulated_power_w: None,
                event_count: None,
                status: TrainerStatus::default(),
                target_limit: TargetLimit::AtTarget,
            }))
            .unwrap();
    }
    fn heart(&self, k: u64, bpm: u8) {
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
    fn link(&mut self, s: LinkState) {
        self.events.try_send(TrainerEvent::Link(s)).unwrap();
        self.engine.drain();
    }
    /// Move the manual clock without processing ticks.
    fn at(&mut self, us: u64) {
        self.clock.0.store(us, Ordering::SeqCst);
    }
    /// Process every tick whose window has closed by the end of tick `k`.
    fn through(&mut self, k: u64) {
        self.clock.0.store((k + 1) * TICK_US, Ordering::SeqCst);
        self.engine.catch_up();
    }
    fn state(&self) -> RideState {
        self.status().state
    }
    fn status(&self) -> crate::RideStatus {
        self.engine.status.borrow().clone()
    }
    fn modes(&mut self) -> Vec<ControlMode> {
        let mut out = vec![];
        while let Ok(TrainerCommand::SetMode(m)) = self.control.try_recv() {
            out.push(m);
        }
        out
    }
    fn rows(&self) -> Vec<(u32, SecondRow)> {
        self.log.lock().unwrap().rows.clone()
    }
}
fn erg(w: u16) -> ControlMode {
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
    assert_eq!(r.modes(), vec![erg(200)], "mode sent once, not per tick");
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
    // the soft start begins with the first sample: the 3 s power is (5 x 150 W from
    // ticks 3..=7, 90 W now) / 6 = 140 W, then five 1 Hz steps to the target
    assert_eq!(r.modes(), vec![erg(140)]);
    for k in 15..40 {
        r.sample(k, 150, 85);
    }
    r.through(39);
    assert_eq!(
        r.modes(),
        vec![erg(152), erg(164), erg(176), erg(188), erg(200)]
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
    assert_eq!(r.modes(), vec![erg(200)]);
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
    assert_eq!(r.modes(), vec![erg(200)]);
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
    assert_eq!(r.gate.lock().unwrap().last(), Some(&false));
    assert_eq!(r.state(), RideState::Finished);
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
            recorder: Box::new(Recorder(r.log.clone()))
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
        block_elapsed_ticks: elapsed,
        engaged_ticks: 0,
        after_loss: false,
        engage_power_w: None,
        max_target_w: max,
    };
    let b = block(100.0, 100.0, 200.0, Phase::Work);
    let at = |elapsed, max| {
        let mut c = ctx(elapsed, max);
        c.block = Some(&b);
        BlockTarget.desired(&c)
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
        BlockTarget.desired(&ctx(0, 2000)),
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
    assert_eq!(r.modes(), vec![erg(200)]);
    // 33 silent ticks: lost after 8, the mode is re-issued once, no ramp is played
    r.through(40);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(r.modes(), vec![erg(0)]);
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
    // start, re-issue on loss, then the ramp from 100 W
    assert_eq!(
        r.modes(),
        vec![erg(200), erg(0), erg(100), erg(120), erg(140)]
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
    assert_eq!(r.modes(), vec![erg(250)]);
    r.link(LinkState::Lost);
    r.link(LinkState::Connecting);
    r.link(LinkState::Ready);
    // 20 ticks, no samples at all: exactly one command, 0 W, and never the target
    r.through(27);
    assert_eq!(r.modes(), vec![erg(0)]);
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
