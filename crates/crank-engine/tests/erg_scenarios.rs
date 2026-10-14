//! ERG policy scenarios (#218): `SimTrainer` faults on a virtual clock, the named
//! regressions of #73 (echo lag, stuck echo, silent page 71, ERG spiral), release and
//! re-engagement, and the page-51 gap of a free ride. Asserts on `RideStatus` and on the
//! commands the engine sent.
use crank_devices::sim::{EchoFault, SimComponent, SimConfig, SimFault, SimRig};
use crank_devices::{
    Clock, ControlMode, DeviceRef, HeartRateDriver, RiderConfig, TrainerCommand, TrainerDriver,
};
use crank_domain::workout::{FlatBlock, Phase};
use crank_engine::{
    Component, Engine, EngineCmd, EngineHandle, METRICS, RecordError, RideRecorder, RideSetup,
    RideState, SecondRow, TrainerFollowing, VirtualClock,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::sleep;

struct Null;
impl RideRecorder for Null {
    fn start(&mut self, m: &[&str]) -> Result<(), RecordError> {
        assert_eq!(m, METRICS);
        Ok(())
    }
    fn append_second(&mut self, _: u32, _: &SecondRow) -> Result<(), RecordError> {
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        Ok(())
    }
}
struct Ride {
    handle: EngineHandle,
    /// `(ms since the clock's epoch, mode)` for every `SetMode`.
    sent: Arc<Mutex<Vec<(u64, ControlMode)>>>,
}
impl Ride {
    fn erg_targets(&self) -> Vec<u16> {
        self.sent
            .lock()
            .expect("test mutex")
            .iter()
            .filter_map(|(_, m)| match m {
                ControlMode::Erg { target_w } => Some(*target_w),
                _ => None,
            })
            .collect()
    }
    fn count(&self, f: impl Fn(&ControlMode) -> bool) -> usize {
        self.sent
            .lock()
            .expect("test mutex")
            .iter()
            .filter(|(_, m)| f(m))
            .count()
    }
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
fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}
async fn launch(blocks: Vec<FlatBlock>, faults: Vec<SimFault>) -> Ride {
    let clock = Arc::new(VirtualClock::new(1));
    let rig = SimRig::new(
        SimConfig {
            faults,
            ..SimConfig::default()
        },
        clock.clone(),
    );
    let rider = RiderConfig {
        rider_kg: 72.0,
        bike_kg: 9.0,
        wheel_m: 2.096,
    };
    let mut trainer = rig
        .trainer()
        .connect(&DeviceRef("sim".into()), rider)
        .await
        .expect("sim setup");
    let hr = rig
        .heart()
        .connect(&DeviceRef("sim".into()))
        .await
        .expect("sim setup");
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let real = std::mem::replace(&mut trainer.control, tx);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let c = clock.clone();
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            if let TrainerCommand::SetMode(m) = cmd {
                log.lock().expect("test mutex").push((c.now().0 / 1000, m));
            }
            if real.send(cmd).await.is_err() {
                break;
            }
        }
    });
    let (engine, handle) = Engine::new(clock, Some(trainer), Some(hr), Arc::new(|_: bool| {}));
    tokio::spawn(engine.run());
    handle
        .send(EngineCmd::Load(RideSetup {
            blocks,
            recorder: Box::new(Null),
            cap_w: None,
        }))
        .await
        .expect("sim setup");
    sleep(Duration::from_millis(100)).await;
    handle.send(EngineCmd::Start).await.expect("sim setup");
    Ride { handle, sent }
}
fn echo(component: SimComponent, fault: EchoFault, from: u64, until: u64) -> SimFault {
    SimFault::Echo {
        component,
        fault,
        from: secs(from),
        until: secs(until),
    }
}
/// A 100 → 200 W ramp over 300 s: the target moves every second.
fn ramp() -> Vec<FlatBlock> {
    vec![block(300.0, 100.0, 200.0, Phase::Work)]
}

#[tokio::test(start_paused = true)]
async fn a_healthy_ramp_is_verified_and_counts_toward_compliance() {
    let r = launch(ramp(), vec![]).await;
    sleep(secs(10)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following
    );
    sleep(secs(110)).await;
    let st = r.handle.status();
    assert_eq!(st.trainer_following, TrainerFollowing::Following);
    // the ride start is an engagement (#250): its 10 s of settle are the only exclusion
    assert_eq!(st.compliance.excluded_s, 10, "{:?}", st.compliance);
    assert!(st.compliance.counted_s >= 105, "{:?}", st.compliance);
}

#[tokio::test(start_paused = true)]
async fn echo_lag_below_l_stays_following_and_lag_above_l_does_not() {
    // 1.5 s of lag on a ramp that moves every second: lagging echoes agree (R2, L = 2 s)
    let r = launch(
        ramp(),
        vec![echo(
            SimComponent::Erg,
            EchoFault::Lag(Duration::from_millis(1500)),
            20,
            100,
        )],
    )
    .await;
    sleep(secs(60)).await;
    let st = r.handle.status();
    assert_eq!(st.trainer_following, TrainerFollowing::Following);
    assert_eq!(st.compliance.excluded_s, 10, "{:?}", st.compliance);
    // 4 s of lag is a stale echo beyond L: Disagree, then not_following
    let r = launch(
        ramp(),
        vec![echo(SimComponent::Erg, EchoFault::Lag(secs(4)), 20, 100)],
    )
    .await;
    sleep(secs(40)).await;
    let st = r.handle.status();
    assert_eq!(st.trainer_following, TrainerFollowing::NotFollowing);
    assert_eq!(st.not_following_component, Some(Component::Erg));
}

#[tokio::test(start_paused = true)]
async fn a_stuck_echo_is_not_following_within_the_bound_and_excluded_from_compliance() {
    // stuck from 20 s: the last value agreed is desired until ~21 s; bound
    // L + 2·T_poll + T_mismatch + r = 2 + 2 + 2 + ~1 s ⇒ not_following by 28 s
    let r = launch(
        ramp(),
        vec![echo(SimComponent::Erg, EchoFault::Stuck, 20, 200)],
    )
    .await;
    sleep(secs(20)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following
    );
    sleep(secs(8)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::NotFollowing,
        "detected within the guarantee-1 bound"
    );
    sleep(secs(40)).await;
    let st = r.handle.status();
    assert_eq!(st.trainer_following, TrainerFollowing::NotFollowing);
    assert!(
        st.compliance.excluded_s >= 40,
        "stuck seconds are excluded: {:?}",
        st.compliance
    );
    assert!(st.compliance.counted_s <= 30, "{:?}", st.compliance);
}

#[tokio::test(start_paused = true)]
async fn a_silent_page_71_goes_unknown_after_t_unknown_and_recovers() {
    let r = launch(
        ramp(),
        vec![echo(SimComponent::Erg, EchoFault::Silent, 20, 40)],
    )
    .await;
    sleep(secs(20)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following
    );
    sleep(secs(4)).await;
    assert_ne!(
        r.handle.status().trainer_following,
        TrainerFollowing::Unknown,
        "T_unknown(erg) = 5 s: not yet"
    );
    sleep(secs(3)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Unknown
    );
    sleep(secs(20)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following,
        "evidence returns after the window: verified again"
    );
}

#[tokio::test(start_paused = true)]
async fn an_erg_spiral_is_released_and_re_engaged_with_a_soft_start() {
    // 4 s at 30 rpm in ERG spirals in the sim until the mode leaves ERG (§7.4)
    let r = launch(
        vec![block(300.0, 200.0, 200.0, Phase::Work)],
        vec![SimFault::ErgSpiral {
            from: secs(60),
            dip_rpm: 30.0,
        }],
    )
    .await;
    sleep(secs(59)).await;
    assert!(!r.handle.status().erg_released);
    // cadence crosses 40 rpm at ~62.2 s and stays below for 2 s: released at ~64.4 s
    sleep(secs(5)).await;
    assert!(
        !r.handle.status().erg_released,
        "not before 2 s below 40 rpm"
    );
    sleep(secs(2)).await;
    assert!(
        r.handle.status().erg_released,
        "released 2 s after crossing 40 rpm"
    );
    assert_eq!(r.count(|m| matches!(m, ControlMode::Resistance { .. })), 1);
    sleep(secs(10)).await;
    let st = r.handle.status();
    assert!(!st.erg_released, "cadence is back: re-engaged");
    assert_eq!(st.state, RideState::Riding);
    // after the re-engage the ERG commands ramp: more than one step, ending on the target
    let t = r.erg_targets();
    // the ride start ramps too (#250): its first command is the rider's own power
    assert!(t.first().is_some_and(|w| *w < 200), "{t:?}");
    assert_eq!(t.last(), Some(&200));
    assert!(t.len() >= 5, "{t:?}");
    // the block timer kept running through the release
    assert!(st.block_elapsed.0 >= 4 * 75, "{:?}", st.block_elapsed);
    sleep(secs(20)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following
    );
}

#[tokio::test(start_paused = true)]
async fn a_cadence_stall_releases_and_a_return_re_engages() {
    let r = launch(
        vec![block(300.0, 150.0, 150.0, Phase::Work)],
        vec![SimFault::CadenceStall {
            from: secs(30),
            until: secs(45),
        }],
    )
    .await;
    sleep(secs(50)).await;
    assert!(r.count(|m| matches!(m, ControlMode::Resistance { .. })) >= 1);
    sleep(secs(30)).await;
    let st = r.handle.status();
    assert!(!st.erg_released);
    assert_eq!(st.trainer_following, TrainerFollowing::Following);
}

#[tokio::test(start_paused = true)]
async fn a_free_ride_with_track_page_silent_is_not_following_naming_track() {
    let r = launch(
        vec![
            block(30.0, 150.0, 150.0, Phase::Work),
            block(300.0, 0.0, 0.0, Phase::Free),
        ],
        vec![echo(SimComponent::Track, EchoFault::Silent, 30, 400)],
    )
    .await;
    sleep(secs(25)).await;
    assert_eq!(
        r.handle.status().trainer_following,
        TrainerFollowing::Following
    );
    // the Free block starts at 30 s; track needs T_unknown(track) = 9 s of silence
    sleep(secs(30)).await;
    let st = r.handle.status();
    assert_eq!(st.trainer_following, TrainerFollowing::NotFollowing);
    assert_eq!(st.not_following_component, Some(Component::Track));
    assert!(r.count(|m| matches!(m, ControlMode::Free)) >= 1);
}
