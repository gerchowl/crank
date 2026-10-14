//! Whole-ride scenarios: `SimTrainer`/`SimHeart` on a virtual clock (paused tokio
//! time, no wall time), asserting exact tick counts and record rows (#217).
use crank_devices::sim::{SimConfig, SimFault, SimRig};
use crank_devices::{
    ControlMode, DeviceRef, HeartRateDriver, LinkState, RiderConfig, TrainerCommand, TrainerDriver,
    TrainerEvent,
};
use crank_domain::workout::{FlatBlock, Phase};
use crank_engine::{
    Engine, EngineCmd, EngineHandle, METRICS, PauseReason, RecordError, RideRecorder, RideSetup,
    RideState, SecondRow, VirtualClock,
};
use crank_store::{
    CompiledJson, JsonColumn, NewRide, RideState as StoredState, RideSummaryJson, RideWorkoutJson,
    Sample, Store,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::sleep;

const RIDER: RiderConfig = RiderConfig {
    rider_kg: 72.0,
    bike_kg: 9.0,
    wheel_m: 2.096,
};
#[derive(Default)]
struct Rec {
    rows: Vec<(u32, SecondRow)>,
    started: bool,
    finished: bool,
}
struct Recorder(Arc<Mutex<Rec>>);
impl RideRecorder for Recorder {
    fn start(&mut self, _: &[&str]) -> Result<(), RecordError> {
        self.0.lock().expect("test mutex").started = true;
        Ok(())
    }
    fn append_second(&mut self, s: u32, row: &SecondRow) -> Result<(), RecordError> {
        self.0.lock().expect("test mutex").rows.push((s, *row));
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        self.0.lock().expect("test mutex").finished = true;
        Ok(())
    }
}
struct Ride {
    handle: EngineHandle,
    rec: Arc<Mutex<Rec>>,
    /// Every `SetMode` the engine sent: (virtual ms since start, mode).
    sent: Arc<Mutex<Vec<(u64, ControlMode)>>>,
    gate: Arc<Mutex<Vec<bool>>>,
}
impl Ride {
    fn rows(&self) -> Vec<(u32, SecondRow)> {
        self.rec.lock().expect("test mutex").rows.clone()
    }
    fn modes(&self) -> Vec<(u64, u16)> {
        self.sent
            .lock()
            .expect("sim setup")
            .iter()
            .filter_map(|(t, m)| match m {
                ControlMode::Erg { target_w } => Some((*t, *target_w)),
                // the end mode of a finished ride (ADR-0002 §7.9, #266)
                ControlMode::Free => None,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }
}
fn block(dur_s: f64, w: f64, phase: Phase) -> FlatBlock {
    FlatBlock {
        path: vec![],
        iteration: vec![],
        node_id: String::new(),
        start_s: 0.0,
        dur_s,
        power_from: w,
        power_to: w,
        phase,
        dur_range_s: None,
    }
}
async fn launch(blocks: Vec<FlatBlock>, faults: Vec<SimFault>) -> Ride {
    launch_with(blocks, faults, false, None).await
}
/// `hide_lost` swallows `Link(Lost)`, leaving the engine only the silence (packet loss
/// without a disconnect event). `recorder` replaces the in-memory record.
async fn launch_with(
    blocks: Vec<FlatBlock>,
    faults: Vec<SimFault>,
    hide_lost: bool,
    recorder: Option<Box<dyn RideRecorder>>,
) -> Ride {
    let clock = Arc::new(VirtualClock::new(1));
    let rig = SimRig::new(
        SimConfig {
            faults,
            ..SimConfig::default()
        },
        clock.clone(),
    );
    let mut trainer = rig
        .trainer()
        .connect(&DeviceRef("sim".into()), RIDER)
        .await
        .expect("sim setup");
    let hr = rig
        .heart()
        .connect(&DeviceRef("sim".into()))
        .await
        .expect("sim setup");
    if hide_lost {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let mut real = std::mem::replace(&mut trainer.events, rx);
        tokio::spawn(async move {
            while let Some(e) = real.recv().await {
                if e == TrainerEvent::Link(LinkState::Lost) {
                    continue;
                }
                if tx.send(e).await.is_err() {
                    break;
                }
            }
        });
    }
    // Tee the engine's commands so the test sees exactly what it sent and when.
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let real = std::mem::replace(&mut trainer.control, tx);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let c = clock.clone();
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            if let TrainerCommand::SetMode(m) = cmd {
                let now = crank_devices::Clock::now(&*c).0 / 1000;
                log.lock().expect("test mutex").push((now, m));
            }
            if real.send(cmd).await.is_err() {
                break;
            }
        }
    });
    let gate = Arc::new(Mutex::new(Vec::new()));
    let g = gate.clone();
    let (engine, handle) = Engine::new(
        clock,
        Some(trainer),
        Some(hr),
        Arc::new(move |a: bool| g.lock().expect("test mutex").push(a)),
    );
    tokio::spawn(engine.run());
    let rec = Arc::new(Mutex::new(Rec::default()));
    handle
        .send(EngineCmd::Load(RideSetup {
            blocks,
            recorder: recorder.unwrap_or_else(|| Box::new(Recorder(rec.clone()))),
            cap_w: None,
        }))
        .await
        .expect("sim setup");
    // Start off the simulator's 250 ms grid, so no sample sits on a window edge.
    sleep(Duration::from_millis(100)).await;
    handle.send(EngineCmd::Start).await.expect("sim setup");
    Ride {
        handle,
        rec,
        sent,
        gate,
    }
}
fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn paused_only(rows: &[(u32, SecondRow)]) -> Vec<u32> {
    rows.iter()
        .filter(|(_, r)| r.workout_s.is_none())
        .map(|(s, _)| *s)
        .collect()
}
/// Ride start is 100 ms after the simulator's grid origin; tick `k` ends at 100 + 250(k+1) ms.
const T0_MS: u64 = 100;

#[tokio::test(start_paused = true)]
async fn full_45_minute_workout() {
    // 5 min warm-up + 5 x (5 min work + 3 min recovery) = 45 min of workout time.
    // With seed 1 the simulated trainer also goes silent once, without a link event,
    // for a few seconds around 915 s (SimCore's "link blip"): the 8-tick rule fires
    // inside this very ride, so session time ends 5 ticks after workout time.
    let mut blocks = vec![block(300.0, 120.0, Phase::Warmup)];
    for _ in 0..5 {
        blocks.push(block(300.0, 200.0, Phase::Work));
        blocks.push(block(180.0, 100.0, Phase::Recovery));
    }
    let r = launch(blocks, vec![]).await;
    assert_eq!(*r.gate.lock().expect("test mutex"), vec![true]);
    sleep(secs(2702)).await;
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Finished);
    assert_eq!(
        st.workout.0, 10_800,
        "45 min of workout time = 10 800 ticks"
    );
    assert_eq!(st.session.0, 10_805, "plus the 5 paused ticks of the blip");
    assert_eq!(st.block_index, None);
    assert_eq!((st.late_samples, st.record_error), (0, None));
    assert_eq!(*r.gate.lock().expect("test mutex"), vec![true, false]);
    let rows = r.rows();
    assert_eq!(
        rows.len(),
        2701,
        "complete seconds only; the last 1/4 s is dropped"
    );
    for (i, (s, row)) in rows.iter().enumerate() {
        assert_eq!(*s as usize, i, "consecutive session seconds");
        let at = f32::from(u16::try_from(i + 1).unwrap());
        match i {
            ..915 => assert_eq!(row.workout_s, Some(at)),
            // two riding ticks inside the 2 s window (null values), the rest paused
            915 => assert_eq!((row.workout_s, row.power), (Some(915.5), None)),
            916 => assert_eq!(row.workout_s, Some(915.75)),
            _ => assert_eq!(row.workout_s, Some(at - 1.25)),
        }
        if i != 915 {
            assert!(row.power.is_some() && row.cadence.is_some() && row.speed.is_some());
        }
    }
    // Controls: one mode per block on its boundary tick (ADR-0002 §7.2). The blip is
    // detected at 915.6 s and the mode re-issued then at 0 W (no pedalling sample yet,
    // #231); the soft start runs once samples flow again, from the then-current 3 s
    // power (71 W: the trainer is spinning up) to the target, one step per second. The ride
    // start is a reconnect too (#250): the first ramp runs from the rider's 29 W to the first
    // block's 120 W. Every later boundary lies 1.25 s later in session time.
    assert_eq!(
        r.modes(),
        [
            (T0_MS + 250, 29),
            (1_350, 47),
            (2_350, 65),
            (3_350, 84),
            (4_350, 102),
            (5_350, 120),
            (300_100, 200),
            (600_100, 100),
            (780_100, 200),
            (915_600, 0),
            (916_850, 71),
            (917_850, 97),
            (918_850, 123),
            (919_850, 148),
            (920_850, 174),
            (921_850, 200),
            (1_081_350, 100),
            (1_261_350, 200),
            (1_561_350, 100),
            (1_741_350, 200),
            (2_041_350, 100),
            (2_221_350, 200),
            (2_521_350, 100),
        ]
    );
    let rec = r.rec.lock().expect("test mutex");
    assert!(rec.started && rec.finished);
}

#[tokio::test(start_paused = true)]
async fn dropout_mid_interval_freezes_ride_time_and_the_block_timer() {
    let r = launch(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::Dropout {
            at: secs(100),
            down: secs(10),
        }],
    )
    .await;
    sleep(secs(99)).await;
    assert_eq!(r.handle.status().state, RideState::Riding);
    sleep(ms(1300)).await;
    // The link event arrives at 100.0 s (window of tick 399 is still open): ticks
    // 0..=398 were workout time, the loss freezes everything from tick 399.
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Paused(PauseReason::TrainerLost));
    assert_eq!((st.workout.0, st.block_elapsed.0), (399, 399));
    assert_eq!(st.session.0, 401, "session time keeps running");
    sleep(secs(10)).await;
    let st = r.handle.status();
    assert_eq!(
        (st.workout.0, st.block_elapsed.0),
        (399, 399),
        "frozen for the outage"
    );
    assert_eq!(st.session.0, 441);
    assert_eq!(st.trainer_link, Some(LinkState::Lost));
    sleep(secs(20)).await;
    let rows = r.rows();
    // seconds 100..=111 are entirely paused: null record values, no workout position
    assert_eq!(paused_only(&rows), (100..=111).collect::<Vec<_>>());
    for (s, row) in rows.iter().filter(|(s, _)| (100..=111).contains(s)) {
        assert_eq!(
            *row,
            SecondRow::default(),
            "second {s}: gaps, never interpolated"
        );
    }
    // second 99 holds tick 399, paused: workout position is 99.75 s
    assert_eq!(rows[99].1.workout_s, Some(99.75));
    // the link is back at 112.5 s: ticks 399..=449 (51) were paused in total
    let st = r.handle.status();
    assert_eq!(st.session.0 - st.workout.0, 51);
    assert_eq!(st.state, RideState::Riding);
    assert_eq!(
        *r.gate.lock().expect("test mutex"),
        vec![true],
        "gate stays active in a loss"
    );
}

#[tokio::test(start_paused = true)]
async fn silent_dropout_keeps_the_two_second_window_in_workout_time() {
    let r = launch_with(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::Dropout {
            at: secs(100),
            down: secs(10),
        }],
        true,
        None,
    )
    .await;
    sleep(secs(104)).await;
    // last sample at 99.75 s is in tick 398; ticks 399..=406 are the 8 missed ticks.
    // They are workout time (no rollback); the ride is paused from tick 407.
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Paused(PauseReason::TrainerLost));
    assert_eq!(st.workout.0, 407);
    assert_eq!(
        st.session.0, 415,
        "(104 - 0.1) s / 250 ms = 415 complete ticks"
    );
    let rows = r.rows();
    // second 100 (ticks 400..=403) and 101 are in the window: riding, values null
    // after the 1 s hold (tick 398 held through tick 402)
    assert_eq!(rows[100].1.workout_s, Some(101.0));
    assert!(
        rows[100].1.power.is_some(),
        "ticks 400..=402 are still held"
    );
    assert_eq!(rows[101].1.power, None);
    assert_eq!(
        rows[101].1.workout_s,
        Some(101.75),
        "tick 407 is the first paused"
    );
    // second 102 holds ticks 408..=411: all paused
    assert_eq!(rows[102].1, SecondRow::default());
}

#[tokio::test(start_paused = true)]
async fn reconnect_reissues_the_mode_with_a_soft_start_and_resumes_on_cadence() {
    let r = launch(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::Dropout {
            at: secs(100),
            down: secs(10),
        }],
    )
    .await;
    sleep(secs(130)).await;
    // attempts at +0.5, +1.5, +3.5, +7.5, +12.5 s; the first one after the 10 s
    // outage succeeds: Link(Ready) at 112.5 s, mode re-issued on the next tick end
    let modes = r.modes();
    // a ride start is a reconnect (#250): the ramp runs from the rider's power at the first
    // sample, one step a second, to the 200 W target
    assert_eq!(modes[0].0, T0_MS + 250, "first engagement");
    assert!(modes[0].1 < 200, "{modes:?}");
    let start = modes.iter().take_while(|(t, _)| *t < 100_000).count();
    assert_eq!(modes[start - 1].1, 200, "the start's ramp is repaid");
    assert_eq!(
        modes[start..],
        [
            (112_600, 191), // from the 3 s power of the first sample after Ready
            (113_600, 193),
            (114_600, 195),
            (115_600, 196),
            (116_600, 198),
            (117_600, 200),
        ],
        "soft start: five 1 Hz steps, then the block target; nothing while down"
    );
    // resumed on the tick that carried the first sample after the re-applied mode
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Riding);
    assert_eq!(st.session.0 - st.workout.0, 51);
    let rows = r.rows();
    assert_eq!(rows[112].1.workout_s, Some(100.25));
    assert_eq!(rows[113].1.workout_s, Some(101.25));
}

#[tokio::test(start_paused = true)]
async fn hr_loss_nulls_hr_while_the_ride_continues() {
    let r = launch(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::HrLoss {
            at: secs(60),
            down: secs(20),
        }],
    )
    .await;
    sleep(secs(90)).await;
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Riding);
    assert_eq!(st.workout.0, st.session.0, "never paused");
    let rows = r.rows();
    // strap link lost at 60.0 s: the Lost event clears the 3 s hold at once
    for (s, row) in rows.iter().filter(|(s, _)| (60..82).contains(s)) {
        assert_eq!(row.hr, None, "second {s}");
        assert!(row.power.is_some() && row.workout_s.is_some());
    }
    assert!(rows[59].1.hr.is_some());
    assert!(rows[82].1.hr.is_some(), "strap back after its reconnect");
}

#[tokio::test(start_paused = true)]
async fn skip_starts_the_next_block_now_without_moving_workout_time() {
    let r = launch(
        vec![
            block(600.0, 150.0, Phase::Warmup),
            block(600.0, 220.0, Phase::Work),
        ],
        vec![],
    )
    .await;
    // wake mid-window (30.15 s): 120 ticks are complete
    sleep(secs(30) + ms(50)).await;
    let before = r.handle.status();
    assert_eq!((before.block_index, before.block_elapsed.0), (Some(0), 120));
    r.handle.send(EngineCmd::SkipBlock).await.unwrap();
    let at = r.handle.status();
    assert_eq!((at.block_index, at.block_elapsed.0), (Some(1), 0));
    assert_eq!(
        at.workout, before.workout,
        "skip does not rewrite workout time"
    );
    sleep(secs(10)).await;
    let st = r.handle.status();
    assert_eq!(st.block_elapsed.0, 40);
    assert_eq!(st.workout.0, before.workout.0 + 40);
    let modes = r.modes();
    // the ride start's soft start (#250): six 1 Hz steps from the rider's power to 150 W,
    // then the skip's 220 W
    assert_eq!(modes.len(), 7, "{modes:?}");
    assert_eq!(modes[5].1, 150);
    assert_eq!(modes[6].1, 220);
    // sent on the first tick processed after the skip: window 120 ends at 30.35 s
    assert_eq!(modes[6], (30_350, 220));
}

#[tokio::test(start_paused = true)]
async fn the_end_mode_is_delivered_before_the_engine_shuts_down() {
    // #266: with every handle gone the actor ends the ride; the end mode must reach the
    // trainer first (ADR-0002 §7.9), and only then the ride-active signal clears
    let r = launch(vec![block(600.0, 200.0, Phase::Work)], vec![]).await;
    sleep(secs(30)).await;
    assert_eq!(r.modes().last(), Some(&(T0_MS + 250 + 5_000, 200)));
    let Ride {
        handle, gate, sent, ..
    } = r;
    drop(handle);
    sleep(secs(3)).await;
    assert_eq!(*gate.lock().expect("test mutex"), vec![true, false]);
    assert_eq!(
        sent.lock().expect("test mutex").last().map(|(_, m)| *m),
        Some(ControlMode::Free)
    );
}

#[tokio::test(start_paused = true)]
async fn manual_pause_freezes_workout_time_and_leaves_gaps() {
    let r = launch(vec![block(600.0, 200.0, Phase::Work)], vec![]).await;
    sleep(secs(60) + ms(50)).await;
    r.handle.send(EngineCmd::Pause).await.unwrap();
    let paused_at = r.handle.status();
    assert_eq!(paused_at.state, RideState::Paused(PauseReason::User));
    sleep(secs(30)).await;
    let st = r.handle.status();
    assert_eq!(st.workout, paused_at.workout);
    assert_eq!(st.block_elapsed, paused_at.block_elapsed);
    assert_eq!(
        st.session.0,
        paused_at.session.0 + 120,
        "30 s of session time"
    );
    r.handle.send(EngineCmd::Resume).await.unwrap();
    sleep(secs(10)).await;
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Riding);
    assert_eq!(st.workout.0, paused_at.workout.0 + 40);
    let rows = r.rows();
    // pause lands inside window 240, resume inside window 360: ticks 240..=359 are
    // paused, which is exactly session seconds 60..=89
    let gaps = paused_only(&rows);
    assert_eq!(gaps, (60..90).collect::<Vec<_>>());
    assert!(
        rows.iter()
            .filter(|(s, _)| gaps.contains(s))
            .all(|(_, r)| r.power.is_none())
    );
    assert!(rows.last().unwrap().1.power.is_some());
}

#[tokio::test(start_paused = true)]
async fn stopped_pedalling_pauses_then_resumes_on_cadence() {
    let r = launch(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::CadenceStall {
            from: secs(60),
            until: secs(80),
        }],
    )
    .await;
    sleep(secs(75) + ms(50)).await;
    // the model's cadence decays to 0 over ~10 s, then 12 ticks of 0 W / 0 rpm
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Paused(PauseReason::StoppedPedalling));
    assert_eq!(
        st.workout.0, 282,
        "ticks 0..=281 were riding, 282 starts the pause"
    );
    sleep(secs(15)).await;
    // cadence is back at 80.0 s, inside window 319: that tick is the last paused one
    let st = r.handle.status();
    assert_eq!(st.state, RideState::Riding);
    assert_eq!(st.session.0 - st.workout.0, 38, "ticks 282..=319");
    let rows = r.rows();
    assert_eq!(paused_only(&rows), (71..80).collect::<Vec<_>>());
    assert_eq!(rows[70].1.workout_s, Some(70.5));
    assert_eq!(rows[70].1.power, Some(0.0));
    assert_eq!(rows[80].1.workout_s, Some(71.5));
}

/// Core's adapter of the record sink over `crank-store` (the engine itself does not
/// depend on the store, ADR-0001 §2).
struct StoreRecorder {
    store: Arc<Store>,
}
impl RideRecorder for StoreRecorder {
    fn start(&mut self, metrics: &[&str]) -> Result<(), RecordError> {
        let ride = NewRide {
            id: "ride".into(),
            started_at: 1_700_000_000,
            workout: RideWorkoutJson::parse(
                r#"{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[]}"#,
            )
            .map_err(|e| RecordError(e.to_string()))?,
            compiled: CompiledJson::parse(r#"{"v":1}"#).map_err(|e| RecordError(e.to_string()))?,
            trainer_device_id: None,
            hr_device_id: None,
            telemetry_source: "sim".into(),
            metrics: metrics.iter().map(|m| (*m).to_owned()).collect(),
        };
        self.store
            .start_ride(ride)
            .map_err(|e| RecordError(e.to_string()))
    }
    fn append_second(&mut self, s: u32, row: &SecondRow) -> Result<(), RecordError> {
        let samples = METRICS
            .iter()
            .zip(row.values())
            .map(|(m, value)| Sample {
                metric: (*m).to_owned(),
                value,
                estimated: false,
            })
            .collect();
        self.store
            .append_second("ride", s, samples)
            .map_err(|e| RecordError(e.to_string()))
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        let summary =
            RideSummaryJson::parse(r#"{"v":1}"#).map_err(|e| RecordError(e.to_string()))?;
        self.store
            .finish_ride("ride", 1_700_000_200, summary)
            .map_err(|e| RecordError(e.to_string()))
    }
}

#[tokio::test(start_paused = true)]
async fn the_record_lands_in_the_store_with_gaps_as_missing_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_owner(dir.path()).unwrap());
    let r = launch_with(
        vec![block(600.0, 200.0, Phase::Work)],
        vec![SimFault::Dropout {
            at: secs(100),
            down: secs(10),
        }],
        false,
        Some(Box::new(StoreRecorder {
            store: store.clone(),
        })),
    )
    .await;
    sleep(secs(130) + ms(50)).await;
    r.handle.send(EngineCmd::Stop).await.unwrap();
    assert_eq!(r.handle.status().state, RideState::Finished);
    assert_eq!(r.handle.status().record_error, None);
    // the end mode is delivered by the sim trainer, then the gate clears (#266)
    sleep(ms(500)).await;
    assert_eq!(*r.gate.lock().expect("test mutex"), vec![true, false]);
    let ride = store.ride("ride").unwrap().unwrap();
    assert_eq!(ride.state, StoredState::Finished);
    let seconds = |metric: &str| -> Vec<Option<f32>> {
        store
            .chunks("ride", metric)
            .unwrap()
            .into_iter()
            .flat_map(|c| c.values)
            .collect()
    };
    let workout = seconds("workout_s");
    let power = seconds("power");
    assert_eq!(
        workout.len(),
        130,
        "520 complete ticks = 130 session seconds"
    );
    for s in 100..=111 {
        assert_eq!((workout[s], power[s]), (None, None), "second {s}");
    }
    assert_eq!(workout[99], Some(99.75));
    assert_eq!(workout[112], Some(100.25));
    assert!(power[98].is_some() && power[112].is_some());
}

#[tokio::test(start_paused = true)]
async fn dropping_every_handle_mid_ride_closes_the_record_and_clears_the_gate() {
    let r = launch(vec![block(600.0, 200.0, Phase::Work)], vec![]).await;
    sleep(secs(10)).await;
    assert!(!r.rec.lock().expect("test mutex").finished);
    let (rec, gate) = (r.rec.clone(), r.gate.clone());
    drop(r.handle);
    sleep(ms(500)).await;
    assert!(rec.lock().expect("test mutex").finished);
    assert_eq!(*gate.lock().expect("test mutex"), vec![true, false]);
    assert_eq!(rec.lock().expect("test mutex").rows.len(), 10);
}
