//! Streams on a whole engine: simulator rig, virtual clock (paused tokio time, no wall
//! time). Pure flow-control properties are in `src/stream/tests.rs` (#219).
#![expect(
    clippy::duration_suboptimal_units,
    clippy::unchecked_time_subtraction,
    reason = "virtual-time test arithmetic: instants are monotonic and spans are literals"
)]
use crank_devices::sim::{SimConfig, SimRig};
use crank_devices::{DeviceRef, HeartRateDriver, RiderConfig, TrainerDriver};
use crank_domain::filters::Filter;
use crank_domain::workout::{FlatBlock, Phase};
use crank_engine::{
    Engine, EngineCmd, EngineHandle, MetricId, RecordError, RideRecorder, RideSetup, SecondRow,
    StreamFrame, StreamRideState, SubscribeOpts, VirtualClock,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::mpsc,
    time::{Instant, sleep},
};

#[derive(Default)]
struct Rows(Arc<Mutex<Vec<u32>>>);
impl RideRecorder for Rows {
    fn start(&mut self, _: &[&str]) -> Result<(), RecordError> {
        Ok(())
    }
    fn append_second(&mut self, s: u32, _: &SecondRow) -> Result<(), RecordError> {
        self.0.lock().expect("test mutex").push(s);
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        Ok(())
    }
}
fn work(dur_s: f64) -> FlatBlock {
    FlatBlock {
        path: vec![],
        iteration: vec![],
        node_id: String::new(),
        start_s: 0.0,
        dur_s,
        power_from: 200.0,
        power_to: 200.0,
        phase: Phase::Work,
        dur_range_s: None,
    }
}
async fn launch(dur_s: f64) -> (EngineHandle, Arc<Mutex<Vec<u32>>>) {
    launch_at(dur_s, 1).await
}
/// `speed`× simulated: session time runs `speed`× faster than the timer.
/// Status changes the ride start's soft start adds on top of the beat: `Erg { 0 }` while the
/// rider is not pedalling, then one new commanded target per 1 Hz step (#324).
const SOFT_START_STATUS_CHANGES: usize = 6;
async fn launch_at(dur_s: f64, speed: u32) -> (EngineHandle, Arc<Mutex<Vec<u32>>>) {
    let clock = Arc::new(VirtualClock::new(speed));
    let rig = SimRig::new(SimConfig::default(), clock.clone());
    let rider = RiderConfig {
        rider_kg: 72.0,
        bike_kg: 9.0,
        wheel_m: 2.096,
    };
    let trainer = rig
        .trainer()
        .connect(&DeviceRef("sim".into()), rider)
        .await
        .expect("sim setup");
    let hr = rig
        .heart()
        .connect(&DeviceRef("sim".into()))
        .await
        .expect("sim setup");
    let (engine, handle) = Engine::new(clock, Some(trainer), Some(hr), Arc::new(|_: bool| {}));
    tokio::spawn(engine.run());
    let rows = Rows::default();
    let seen = rows.0.clone();
    handle
        .send(EngineCmd::Load(RideSetup {
            blocks: vec![work(dur_s)],
            recorder: Box::new(rows),
            cap_w: None,
        }))
        .await
        .expect("sim setup");
    sleep(Duration::from_millis(100)).await;
    handle.send(EngineCmd::Start).await.expect("sim setup");
    (handle, seen)
}
fn opts(rate_hz: f64) -> SubscribeOpts {
    SubscribeOpts {
        rate_hz,
        filters: vec![Filter::Sma { window_s: 3.0 }],
    }
}
/// A consumer that applies and acknowledges every frame at once; yields each frame with
/// the virtual instant it arrived at.
fn consume(
    handle: EngineHandle,
    stream: crank_engine::StreamId,
    mut rx: mpsc::Receiver<StreamFrame>,
) -> Arc<Mutex<Vec<(Duration, StreamFrame)>>> {
    let out = Arc::new(Mutex::new(Vec::new()));
    let log = out.clone();
    let t0 = Instant::now();
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            let seq = f.seq;
            log.lock().expect("test mutex").push((t0.elapsed(), f));
            if handle.stream_ack(stream, seq).await.is_err() {
                break;
            }
        }
    });
    out
}

#[tokio::test(start_paused = true)]
async fn frames_stay_within_ten_hertz_and_status_beats_at_two_hertz() {
    let (h, _) = launch(600.0).await;
    let (id, rx) = h.open_stream(Some("main".into())).await.expect("open");
    for rate in [10.0, 4.0, 1.0] {
        h.stream_subscribe(id, MetricId::Power, opts(rate))
            .await
            .expect("subscribe");
    }
    let frames = consume(h.clone(), id, rx);
    sleep(Duration::from_secs(60)).await;
    let log = frames.lock().expect("test mutex");
    assert!(log.len() > 100, "frames flow: {}", log.len());
    for pair in log.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= Duration::from_millis(100),
            "≤ 10 Hz: {:?} then {:?}",
            pair[0].0,
            pair[1].0
        );
        assert_eq!(
            pair[1].1.seq,
            pair[0].1.seq.wrapping_add(1),
            "consecutive seq"
        );
    }
    // Steady riding, nothing but the clock changing: status on the 500 ms beat, and once per
    // soft-start step (the commanded target moves in 1 Hz steps for 5 s, then it is steady, #324).
    let beats: Vec<Duration> = log
        .iter()
        .skip(1)
        .filter(|(_, f)| f.status.is_some())
        .map(|(t, _)| *t)
        .collect();
    assert!(
        (110..=121 + SOFT_START_STATUS_CHANGES).contains(&beats.len()),
        "2 Hz over 60 s: {}",
        beats.len()
    );
    // Past the soft start (5 s of ride time) only the beat sends a status.
    let steady: Vec<_> = beats
        .iter()
        .filter(|t| **t > Duration::from_secs(7))
        .collect();
    for pair in steady.windows(2) {
        assert!(
            *pair[1] - *pair[0] >= Duration::from_millis(500),
            "beat spacing {pair:?}"
        );
    }
    // Rates: the 1 Hz subscription is delivered once per second of session time.
    let slow = log
        .iter()
        .flat_map(|(_, f)| f.samples.iter())
        .filter(|s| s.sub.0 == 3)
        .count();
    assert!((58..=61).contains(&slow), "1 Hz over 60 s: {slow}");
    let last = log.last().expect("frames").1.status.clone();
    assert!(
        log.iter().any(|(_, f)| f
            .status
            .as_ref()
            .is_some_and(|s| s.state == StreamRideState::Riding && s.session_s > 50.0)),
        "{last:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_consumer_never_blocks_the_ride_or_the_record() {
    let (h, rows) = launch(600.0).await;
    let (id, mut rx) = h.open_stream(None).await.expect("open");
    h.stream_subscribe(id, MetricId::Power, opts(10.0))
        .await
        .expect("subscribe");
    // No reads, no acks: 60 s of ride.
    sleep(Duration::from_secs(60)).await;
    let st = h.status();
    assert!(st.session.0 >= 238, "ticks kept running: {}", st.session.0);
    assert!(
        rows.lock().expect("test mutex").len() >= 59,
        "record kept up"
    );
    let mut got = Vec::new();
    while let Ok(f) = rx.try_recv() {
        got.push(f);
    }
    assert_eq!(got.len(), 3, "at most 3 unacked frames: {}", got.len());
    // The window comes back: the next frame is flagged and carries the latest state.
    h.stream_ack(id, 2).await.expect("ack");
    sleep(Duration::from_millis(300)).await;
    let f = rx.try_recv().expect("frame after the ack");
    assert!(f.resync, "coalesced while blocked");
    assert_eq!(f.seq, 3);
    let snap = h.stream_snapshot(id).await.expect("snapshot");
    assert_eq!(snap.samples.len(), 1);
    assert!(snap.status.is_some());
}

#[tokio::test(start_paused = true)]
async fn a_dead_subscriber_is_dropped_and_the_ride_carries_on() {
    let (h, rows) = launch(600.0).await;
    let (id, rx) = h.open_stream(None).await.expect("open");
    h.stream_subscribe(id, MetricId::Power, opts(10.0))
        .await
        .expect("subscribe");
    drop(rx);
    sleep(Duration::from_secs(10)).await;
    assert_eq!(
        h.stream_snapshot(id).await,
        Err(crank_engine::StreamError::UnknownStream),
        "a failed send drops the stream"
    );
    assert!(rows.lock().expect("test mutex").len() >= 9);
}

#[tokio::test(start_paused = true)]
async fn opening_a_window_again_drops_its_previous_streams() {
    let (h, _) = launch(600.0).await;
    let (first, mut rx1) = h.open_stream(Some("main".into())).await.expect("open");
    let (second, _rx2) = h.open_stream(Some("main".into())).await.expect("reload");
    assert_ne!(first, second);
    // whatever was already queued, then the channel is closed
    while rx1.recv().await.is_some() {}
    assert!(h.stream_snapshot(second).await.is_ok());
    h.close_window("main".into()).await.expect("destroyed");
    assert_eq!(
        h.stream_snapshot(second).await,
        Err(crank_engine::StreamError::UnknownStream)
    );
}

#[tokio::test(start_paused = true)]
async fn status_reports_a_pause_at_once_and_session_time_keeps_running() {
    let (h, _) = launch(600.0).await;
    let (id, rx) = h.open_stream(None).await.expect("open");
    let frames = consume(h.clone(), id, rx);
    sleep(Duration::from_secs(5)).await;
    h.send(EngineCmd::Pause).await.expect("pause");
    sleep(Duration::from_millis(150)).await;
    let paused_at = {
        let log = frames.lock().expect("test mutex");
        log.iter()
            .find(|(_, f)| {
                f.status
                    .as_ref()
                    .is_some_and(|s| matches!(s.state, StreamRideState::Paused { .. }))
            })
            .map(|(t, _)| *t)
            .expect("pause reported within one frame interval")
    };
    assert!(paused_at >= Duration::from_secs(5));
    sleep(Duration::from_secs(5)).await;
    let log = frames.lock().expect("test mutex");
    let last = log
        .iter()
        .rev()
        .find_map(|(_, f)| f.status.clone())
        .expect("status");
    assert!(last.session_s >= 9.5, "session time runs while paused");
    assert!(
        last.workout_s < 5.5,
        "workout time is frozen: {}",
        last.workout_s
    );
}

/// ADR-0001 §4.3: "At 10× / 60× simulated speed the source rates scale with simulated
/// session time, frame emission does not." Frames and heartbeats are paced in timer
/// time; the source values advance in session time.
async fn pacing_at(speed: u32) {
    let (h, _) = launch_at(7_200.0, speed).await;
    let (id, rx) = h.open_stream(Some("main".into())).await.expect("open");
    h.stream_subscribe(id, MetricId::Power, opts(10.0))
        .await
        .expect("subscribe");
    let frames = consume(h.clone(), id, rx);
    sleep(Duration::from_secs(10)).await;
    let log = frames.lock().expect("test mutex");
    assert!(log.len() >= 20, "{speed}x: frames flow: {}", log.len());
    assert!(
        log.len() <= 102,
        "{speed}x: ≤ 10 Hz in timer time: {}",
        log.len()
    );
    for pair in log.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= Duration::from_millis(100),
            "{speed}x: {:?} then {:?}",
            pair[0].0,
            pair[1].0
        );
    }
    let beats = log.iter().filter(|(_, f)| f.status.is_some()).count();
    assert!(
        (18..=23 + SOFT_START_STATUS_CHANGES).contains(&beats),
        "{speed}x: status 2 Hz in timer time: {beats}"
    );
    let last_t = log
        .iter()
        .flat_map(|(_, f)| f.samples.iter())
        .map(|s| s.t)
        .fold(0.0, f64::max);
    let min = 9.0 * f64::from(speed);
    assert!(
        last_t >= min,
        "{speed}x: source advances in session time: {last_t} < {min}"
    );
}

#[tokio::test(start_paused = true)]
async fn pacing_does_not_scale_with_speed_10x() {
    pacing_at(10).await;
}

#[tokio::test(start_paused = true)]
async fn pacing_does_not_scale_with_speed_60x() {
    pacing_at(60).await;
}

#[tokio::test(start_paused = true)]
async fn paused_ticks_publish_null() {
    // Mutant: publishing the held trainer values while paused instead of the record's gap.
    let (h, _) = launch(600.0).await;
    let (id, mut rx) = h.open_stream(None).await.expect("open");
    let sub = h
        .stream_subscribe(
            id,
            MetricId::Power,
            SubscribeOpts {
                rate_hz: 10.0,
                filters: vec![],
            },
        )
        .await
        .expect("subscribe");
    sleep(Duration::from_secs(5)).await;
    let before = h.stream_snapshot(id).await.expect("snapshot");
    assert!(before.samples[0].v.is_some(), "riding: a power value");
    h.send(EngineCmd::Pause).await.expect("pause");
    sleep(Duration::from_secs(2)).await;
    let after = h.stream_snapshot(id).await.expect("snapshot");
    let s = after.samples.iter().find(|s| s.sub == sub).expect("sub");
    assert_eq!(s.v, None, "paused: a gap, never a held value");
    assert!(
        s.t > before.samples[0].t + 1.5,
        "session time keeps advancing"
    );
    while rx.try_recv().is_ok() {}
}
