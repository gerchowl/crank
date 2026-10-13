//! Required #43 tests, using a virtual Tokio executor and a faithful transport.
use super::*;
use crate::{
    pacing::{Pacer, pages},
    telemetry::{echo, receive},
};
use crank_devices::*;
use futures_util::{future::BoxFuture, stream};
use proptest::prelude::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

fn frame(page: [u8; 8]) -> Vec<u8> {
    let mut frame = vec![0xa4, 9, 0x4e, 5];
    frame.extend(page);
    frame.push(frame.iter().fold(0, |xor, b| xor ^ b));
    frame
}
fn reply(control: [u8; 13], status: u8) -> Vec<u8> {
    frame([
        71,
        control[4],
        0,
        status,
        control[8],
        control[9],
        control[10],
        control[11],
    ])
}
fn sim(grade_pct: f32) -> ControlMode {
    ControlMode::Sim {
        grade_pct,
        crr: 0.004,
        wind: Wind::default(),
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn echo_grid_cannot_drift(target in 0..=16383u16, resistance in 0.0..100.0f32,
        grade in -200.0..200.0f32, crr in 0.0..0.01275f32,
        cw in 0.0..1.86f32, speed in -127.0..127.0f32, drafting in 0.0..1.0f32) {
        for mode in [ControlMode::Erg { target_w: target }, ControlMode::Resistance { pct: resistance },
            ControlMode::Sim { grade_pct: grade, crr, wind: Wind { cw_kg_per_m: cw, speed_kmh: speed, drafting } }, ControlMode::Free] {
            let actual: Vec<_> = pages(mode).unwrap().into_iter().map(|p| echo(p[4], p[8..12].try_into().unwrap())).collect();
            prop_assert_eq!(actual, mode.grid_key().unwrap());
        }
    }
    #[test]
    fn arbitrary_notify_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..128), at in any::<u64>()) {
        runtime().block_on(async {
            let (fake, mut link) = link().await;
            while link.events.try_recv().is_ok() {}
            fake.sender.send(Ok((MonoTime(at), bytes))).unwrap();
            settle().await;
            let mut count = 0;
            while link.events.try_recv().is_ok() { count += 1; }
            assert!(count <= 2);
            // A valid successor proves that the real reader survived the bytes.
            fake.sender.send(Ok((MonoTime(at), reply(pages(sim(0.0)).unwrap()[0], 0)))).unwrap();
            settle().await;
            assert!(matches!(link.events.try_recv(), Ok(TrainerEvent::Observed(_))));
            link.control.send(TrainerCommand::Disconnect).await.unwrap();
            tick().await;
        });
    }
    #[test]
    fn poll_cadence_and_latency_under_arbitrary_erg_changes(changes in prop::collection::vec(prop::option::of(0..=16383u16), 8..128)) {
        let mut pacer = Pacer::default();
        for tick in 0..8 { prop_assert!(pacer.tick(tick).unwrap().is_none()); }
        pacer.set(ControlMode::Erg { target_w: 100 }, 8).unwrap();
        let mut previous_poll = None;
        let mut pending = None;
        for (index, target) in changes.into_iter().enumerate() {
            let tick = u64::try_from(index).unwrap() + 8;
            if let Some(target_w) = target {
                pacer.set(ControlMode::Erg { target_w }, tick).unwrap();
                pending = Some((tick, target_w));
            }
            if let Some(written) = pacer.tick(tick).unwrap() {
                if written[4] == 70 {
                    prop_assert_eq!(written[10], 71);
                    prop_assert_eq!((tick - 8) % 4, 0);
                    if let Some(last) = previous_poll { prop_assert_eq!(tick - last, 4); }
                    previous_poll = Some(tick);
                } else if let Some((since, target)) = pending.take() {
                    prop_assert!(tick - since <= 1);
                    prop_assert_eq!(u16::from_le_bytes([written[10], written[11]]), target * 4);
                }
            }
            if let Some((since, _)) = pending { prop_assert!(tick - since <= 1); }
        }
    }
}
#[test]
fn adversarial_new_erg_target_before_every_tick_still_polls() {
    let mut pacer = Pacer::default();
    for tick in 0..100u16 {
        pacer
            .set(ControlMode::Erg { target_w: tick }, u64::from(tick))
            .unwrap();
        let p = pacer.tick(u64::from(tick)).unwrap().unwrap();
        if tick > 0 && tick % 4 == 0 {
            assert_eq!(p[4], 70);
        } else {
            assert_eq!(p[4], 49);
            assert_eq!(u16::from_le_bytes([p[10], p[11]]), tick * 4);
        }
    }
}
#[test]
fn sim_pair_is_atomic_even_when_poll_is_due() {
    let mut pacer = Pacer::default();
    pacer.set(sim(0.0), 0).unwrap();
    for tick in 0..4 {
        pacer.tick(tick).unwrap();
    }
    pacer
        .set(
            ControlMode::Sim {
                grade_pct: 3.0,
                crr: 0.005,
                wind: Wind {
                    speed_kmh: 2.0,
                    ..Wind::default()
                },
            },
            4,
        )
        .unwrap();
    assert_eq!(pacer.tick(4).unwrap().unwrap()[4], 50);
    assert_eq!(pacer.tick(5).unwrap().unwrap()[4], 51);
    assert_eq!(pacer.tick(6).unwrap().unwrap()[4], 70);
    assert_eq!(pacer.tick(8).unwrap().unwrap()[4], 70); // nominal grid, no drift
}
#[test]
fn coalescing_never_forgets_an_unsent_value_or_repeats_a_sent_value() {
    let mut pacer = Pacer::default();
    pacer.set(sim(0.0), 0).unwrap();
    assert_eq!(pacer.tick(0).unwrap().unwrap()[4], 50);
    pacer.set(sim(1.0), 1).unwrap();
    pacer.set(sim(1.0), 1).unwrap();
    assert_eq!(pacer.tick(1).unwrap().unwrap(), pages(sim(1.0)).unwrap()[1]);
    assert!(pacer.tick(2).unwrap().is_none());
}
#[test]
fn every_reply_is_observed_once_in_arrival_order_with_all_status_codes() {
    let control = pages(ControlMode::Erg { target_w: 200 }).unwrap()[0];
    for status in 0..=u8::MAX {
        let at = MonoTime(u64::from(status / 2)); // duplicate arrival times also survive
        for _ in 0..2 {
            let events = receive(at, &reply(control, status));
            assert_eq!(
                events,
                vec![TrainerEvent::Observed(TrainerObservation {
                    at,
                    source: ObservationSource::Page71Echo,
                    control: Some(ObservedControl::Erg { quarter_w: 800 }),
                    status: Some(expected_status(status)),
                    target_limit: None
                })]
            );
        }
    }
    for page in [55, 0, 255] {
        assert_eq!(echo(page, [0; 4]), ObservedControl::NotControl(page));
    }
}
type Notification = Result<(MonoTime, Vec<u8>), DeviceError>;
fn expected_status(byte: u8) -> CommandStatus {
    match byte {
        0 => CommandStatus::Pass,
        1 => CommandStatus::Fail,
        2 => CommandStatus::NotSupported,
        3 => CommandStatus::Rejected,
        4 => CommandStatus::Pending,
        other => CommandStatus::Other(other),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
}
struct Fake {
    sender: mpsc::UnboundedSender<Notification>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Notification>>>,
    writes: Mutex<Vec<(MonoTime, [u8; 13])>>,
    last: Mutex<Option<[u8; 13]>>,
    clock: Arc<dyn Clock>,
    fail: AtomicBool,
    no_caps: AtomicBool,
    disconnect_fail: AtomicBool,
    disconnects: AtomicUsize,
}
impl Fake {
    fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        Arc::new(Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            writes: Mutex::default(),
            last: Mutex::default(),
            clock,
            fail: AtomicBool::new(false),
            no_caps: AtomicBool::new(false),
            disconnect_fail: AtomicBool::new(false),
            disconnects: AtomicUsize::new(0),
        })
    }
}
impl FecTransport for Arc<Fake> {
    fn connect(
        &self,
        _: DeviceRef,
        _: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        let rx = self.receiver.lock().unwrap().take().unwrap();
        Box::pin(async move {
            let notifications: Notifications = Box::pin(stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|v| (v, rx))
            }));
            Ok((DeviceInfo::default(), notifications))
        })
    }
    fn write(&self, bytes: [u8; 13]) -> BoxFuture<'static, Result<(), DeviceError>> {
        let fake = self.clone();
        Box::pin(async move {
            if fake.fail.swap(false, Ordering::SeqCst) {
                return Err(DeviceError::Gatt("write rejected".into()));
            }
            fake.writes.lock().unwrap().push((fake.clock.now(), bytes));
            let notification =
                if bytes[4] == 70 && bytes[10] == 54 && !fake.no_caps.load(Ordering::SeqCst) {
                    Some(frame([54, 255, 255, 255, 255, 1, 0, 7]))
                } else if bytes[4] == 70 && bytes[10] == 71 {
                    fake.last.lock().unwrap().map(|last| reply(last, 0))
                } else {
                    *fake.last.lock().unwrap() = Some(bytes);
                    None
                };
            if let Some(bytes) = notification {
                fake.sender.send(Ok((fake.clock.now(), bytes))).unwrap();
            }
            Ok(())
        })
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        let fake = self.clone();
        Box::pin(async move {
            fake.disconnects.fetch_add(1, Ordering::SeqCst);
            if fake.disconnect_fail.load(Ordering::SeqCst) {
                Err(DeviceError::Gatt("disconnect rejected".into()))
            } else {
                Ok(())
            }
        })
    }
}
fn rider() -> RiderConfig {
    RiderConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.7,
    }
}
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}
async fn tick() {
    tokio::time::advance(Duration::from_millis(250)).await;
    settle().await;
}
async fn link() -> (Arc<Fake>, TrainerLink) {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let fake = Fake::new(clock.clone());
    let driver = FecTrainer::new(Arc::new(fake.clone()), clock);
    let link = driver
        .connect(&DeviceRef("fake".into()), rider())
        .await
        .unwrap();
    settle().await;
    (fake, link)
}
#[tokio::test(start_paused = true)]
async fn faithful_sim_transport_alternates_echoes_and_respects_four_hz() {
    let (fake, mut link) = link().await;
    assert!(link.caps.erg && link.caps.sim && link.caps.resistance);
    assert_eq!(
        fake.writes
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p[4])
            .collect::<Vec<_>>(),
        [55, 70]
    );
    link.control
        .send(TrainerCommand::SetMode(sim(2.0)))
        .await
        .unwrap();
    for _ in 0..40 {
        tick().await;
    }
    let writes = fake.writes.lock().unwrap().clone();
    for pair in writes.windows(2) {
        assert!(pair[1].0.0 - pair[0].0.0 >= 250_000);
    }
    let controls: Vec<_> = writes
        .iter()
        .skip(4)
        .filter(|(_, p)| p[4] == 50 || p[4] == 51)
        .collect();
    for pair in controls.windows(2) {
        assert_ne!(pair[0].1[4], pair[1].1[4]);
        assert_eq!(pair[1].0.0 - pair[0].0.0, 1_000_000);
    }
    let mut replies = Vec::new();
    while let Ok(event) = link.events.try_recv() {
        if let TrainerEvent::Observed(o) = event {
            replies.push(o.control.unwrap());
        }
    }
    assert!(replies.len() >= 8);
    for pair in replies.windows(2) {
        assert!(matches!(
            (pair[0], pair[1]),
            (ObservedControl::Wind { .. }, ObservedControl::Track { .. })
                | (ObservedControl::Track { .. }, ObservedControl::Wind { .. })
        ));
    }
}
#[tokio::test(start_paused = true)]
async fn write_failure_faults_without_delivery_and_link_loss_stops_writer() {
    let (fake, mut link) = link().await;
    while link.events.try_recv().is_ok() {}
    fake.fail.store(true, Ordering::SeqCst);
    link.control
        .send(TrainerCommand::SetMode(ControlMode::Erg { target_w: 200 }))
        .await
        .unwrap();
    tick().await;
    let mut events = Vec::new();
    while let Ok(event) = link.events.try_recv() {
        events.push(event);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrainerEvent::Fault(DeviceError::Gatt(_))))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, TrainerEvent::Delivered { .. }))
    );
    assert!(events.contains(&TrainerEvent::Link(LinkState::Lost)));
    let count = fake.writes.lock().unwrap().len();
    for _ in 0..12 {
        tick().await;
    }
    assert_eq!(fake.writes.lock().unwrap().len(), count);
}
#[cfg(feature = "replay")]
#[tokio::test(start_paused = true)]
async fn synthetic_fecap_replays_through_real_driver_on_injected_clock() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let replay = ReplayTrainer::new(
        include_str!("../../crank-fec/tests/fixtures/synthetic.fecap"),
        clock,
    )
    .unwrap();
    let mut link = replay
        .connect(&DeviceRef("replay".into()), rider())
        .await
        .unwrap();
    let mut observed = Vec::new();
    while let Some(event) = link.events.recv().await {
        if let TrainerEvent::Observed(o) = event {
            observed.push(o);
        }
    }
    let echoes: Vec<_> = observed
        .iter()
        .filter(|o| o.source == ObservationSource::Page71Echo)
        .collect();
    assert_eq!(echoes.len(), 1);
    assert_eq!(echoes[0].at, MonoTime(1_250_000));
    assert_eq!(
        echoes[0].control,
        Some(ObservedControl::Erg { quarter_w: 801 })
    );
    assert_eq!(echoes[0].status, Some(CommandStatus::Pending));
}

#[path = "../../crank-fec/tests/support/mod.rs"]
mod capture_reader;
#[test]
fn capture_writer_roundtrips_with_existing_codec_fixture_reader() {
    let path = std::env::temp_dir().join(format!("crank-43-{}.fecap", std::process::id()));
    let capture = Capture::create(&path, MonoTime(100)).unwrap();
    let records = [
        (MonoTime(100), true, vec![0, 1, 255]),
        (
            MonoTime(100),
            false,
            pages(ControlMode::Erg { target_w: 201 }).unwrap()[0].to_vec(),
        ),
        (
            MonoTime(200),
            true,
            reply(pages(ControlMode::Erg { target_w: 201 }).unwrap()[0], 0),
        ),
    ];
    for (at, rx, bytes) in &records {
        capture.record(*at, *rx, bytes).unwrap();
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let expected: Vec<_> = records
        .iter()
        .map(|(at, rx, bytes)| (at.0 - 100, *rx, crank_fec::decode(bytes)))
        .collect();
    assert_eq!(capture_reader::replay(&text), expected);
    assert!(capture.record(MonoTime(199), true, &[0]).is_err());
    assert!(Capture::create(&path, MonoTime(0)).is_err());
    std::fs::remove_file(path).unwrap();
}
#[tokio::test(start_paused = true)]
async fn receive_pipeline_preserves_duplicate_replies_and_transport_loss() {
    let (fake, mut link) = link().await;
    let control = pages(ControlMode::Resistance { pct: 10.0 }).unwrap()[0];
    let mut expected = Vec::new();
    for status in [4, 0, 255, 3, 0] {
        let at = MonoTime(123);
        fake.sender.send(Ok((at, reply(control, status)))).unwrap();
        expected.push((at, expected_status(status)));
    }
    settle().await;
    let mut actual = Vec::new();
    while let Ok(event) = link.events.try_recv() {
        if let TrainerEvent::Observed(o) = event {
            actual.push((o.at, o.status.unwrap()));
        }
    }
    assert_eq!(actual, expected);
    fake.sender.send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    assert!(link.control.is_closed());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn sim_poll_lateness_is_bounded_during_pair_supersession(values in prop::collection::vec((0..100u16, 0..100u16), 32..128)) {
        let mut pacer = Pacer::default();
        let mut polls = 0u64;
        for (tick, (grade, wind)) in values.into_iter().enumerate() {
            let tick = u64::try_from(tick).unwrap();
            pacer.set(ControlMode::Sim { grade_pct: f32::from(grade), crr: 0.004,
                wind: Wind { speed_kmh: f32::from(wind), ..Wind::default() } }, tick).unwrap();
            if let Some(p) = pacer.tick(tick).unwrap()
                && p[4] == 70 {
                    polls += 1;
                    let due = polls * 4;
                    prop_assert!((due..=due + 2).contains(&tick));
                }
            prop_assert!(tick <= (polls + 1) * 4 + 2);
        }
    }
    #[test]
    fn unsuperseded_sim_pair_reaches_both_writes_within_two_ticks(at in 0..32u64) {
        let mut pacer = Pacer::default();
        pacer.set(sim(0.0), 0).unwrap();
        for tick in 0..at { pacer.tick(tick).unwrap(); }
        let mode = ControlMode::Sim { grade_pct: 3.0, crr: 0.005, wind: Wind { speed_kmh: 5.0, ..Wind::default() } };
        pacer.set(mode, at).unwrap();
        let mut writes: Vec<_> = (at..at + 2).filter_map(|tick| pacer.tick(tick).unwrap()).collect();
        writes.sort_by_key(|p| p[4]);
        prop_assert_eq!(writes, pages(mode).unwrap());
    }
}
#[test]
fn malformed_notifications_fault_but_do_not_consume_valid_successors() {
    let control = pages(ControlMode::Erg { target_w: 200 }).unwrap()[0];
    let mut corrupt = reply(control, 0);
    corrupt[12] ^= 1;
    assert!(matches!(
        receive(MonoTime(0), &corrupt).as_slice(),
        [TrainerEvent::Fault(DeviceError::Protocol {
            kind: ProtocolErrorKind::Checksum,
            ..
        })]
    ));
    assert!(matches!(
        receive(MonoTime(1), &[0]).as_slice(),
        [TrainerEvent::Fault(DeviceError::Protocol {
            kind: ProtocolErrorKind::Framing,
            ..
        })]
    ));
    assert!(matches!(
        receive(MonoTime(2), &reply(control, 0)).as_slice(),
        [TrainerEvent::Observed(_)]
    ));
}

#[tokio::test(start_paused = true)]
async fn real_writer_coalesces_adversarial_erg_changes_with_two_tick_latency() {
    let (fake, link) = link().await;
    for target_w in 100..200u16 {
        // Two updates before each tick: only the newest value may be written.
        for watts in [target_w + 1, target_w] {
            link.control
                .send(TrainerCommand::SetMode(ControlMode::Erg {
                    target_w: watts,
                }))
                .await
                .unwrap();
        }
        let before = fake.writes.lock().unwrap().len();
        tick().await;
        let writes = fake.writes.lock().unwrap();
        assert_eq!(writes.len(), before + 1);
        let last = writes.last().unwrap().1;
        if last[4] == 49 {
            assert_eq!(u16::from_le_bytes([last[10], last[11]]), target_w * 4);
        } else {
            assert_eq!(last[4], 70);
        }
    }
}

struct Trace;
impl tracing::Subscriber for Trace {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
#[tokio::test(start_paused = true)]
async fn trace_capture_failure_is_reported_without_misreporting_gatt_or_stopping_control() {
    enable_trace();
    let path = std::env::temp_dir().join(format!(
        "crank-43-capture-fault-{}.fecap",
        std::process::id()
    ));
    let capture = Arc::new(Capture::create(&path, MonoTime(0)).unwrap());
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let fake = Fake::new(clock.clone());
    let driver = FecTrainer::new(Arc::new(fake.clone()), clock).with_capture(capture.clone());
    let mut link = driver
        .connect(&DeviceRef("fake".into()), rider())
        .await
        .unwrap();
    settle().await;
    // Make subsequent append timestamps invalid to exercise capture's I/O error
    // path without relying on host filesystem permissions or a Linux /dev/full.
    capture.record(MonoTime(1_000_000_000), true, &[]).unwrap();
    link.control
        .send(TrainerCommand::SetMode(ControlMode::Erg { target_w: 200 }))
        .await
        .unwrap();
    tick().await;
    let mut events = Vec::new();
    while let Ok(event) = link.events.try_recv() {
        events.push(event);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrainerEvent::Fault(DeviceError::Capture(_))))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrainerEvent::Delivered { .. }))
    );
    assert!(!events.contains(&TrainerEvent::Link(LinkState::Lost)));
    assert_eq!(fake.writes.lock().unwrap().last().unwrap().1[4], 49);
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    tick().await;
    std::fs::remove_file(path).unwrap();
}

fn enable_trace() {
    static TRACE: std::sync::Once = std::sync::Once::new();
    TRACE.call_once(|| tracing::subscriber::set_global_default(Trace).unwrap());
}

#[tokio::test(start_paused = true)]
async fn cancelled_setup_disconnects_and_failed_cleanup_preserves_both_errors() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let fake = Fake::new(clock.clone());
    let driver = FecTrainer::new(Arc::new(fake.clone()), clock.clone());
    let connecting = tokio::spawn(driver.connect(&DeviceRef("fake".into()), rider()));
    settle().await;
    assert_eq!(fake.writes.lock().unwrap().len(), 1); // connected, waiting after page 55
    connecting.abort();
    assert!(connecting.await.err().unwrap().is_cancelled());
    settle().await;
    assert_eq!(fake.disconnects.load(Ordering::SeqCst), 1);

    let fake = Fake::new(clock.clone());
    fake.no_caps.store(true, Ordering::SeqCst);
    fake.disconnect_fail.store(true, Ordering::SeqCst);
    let result = FecTrainer::new(Arc::new(fake.clone()), clock)
        .connect(&DeviceRef("fake".into()), rider())
        .await;
    let error = result.err().unwrap().to_string();
    assert!(error.contains("page 54 timed out"));
    assert!(error.contains("disconnect rejected"));
    assert_eq!(fake.disconnects.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn startup_capture_fault_is_nonfatal_and_rx_capture_keeps_arrival_time() {
    enable_trace();
    let path = std::env::temp_dir().join(format!("crank-43-arrival-{}.fecap", std::process::id()));
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let capture = Arc::new(Capture::create(&path, MonoTime(0)).unwrap());
    let fake = Fake::new(clock.clone());
    let driver =
        FecTrainer::new(Arc::new(fake.clone()), clock.clone()).with_capture(capture.clone());
    let mut link = driver
        .connect(&DeviceRef("fake".into()), rider())
        .await
        .unwrap();
    while link.events.try_recv().is_ok() {}
    let at = clock.now();
    tokio::time::advance(Duration::from_millis(20)).await;
    let bytes = reply(pages(sim(0.0)).unwrap()[0], 0);
    fake.sender.send(Ok((at, bytes.clone()))).unwrap();
    settle().await;
    assert!(matches!(link.events.try_recv(), Ok(TrainerEvent::Observed(o)) if o.at == at));
    let records = capture_reader::replay(&std::fs::read_to_string(&path).unwrap());
    assert_eq!(
        records.last().unwrap(),
        &(at.0, true, crank_fec::decode(&bytes))
    );
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    tick().await;
    std::fs::remove_file(&path).unwrap();

    // An invalid capture epoch fails the first setup append as well as later appends.
    let capture = Arc::new(Capture::create(&path, MonoTime(u64::MAX)).unwrap());
    let fake = Fake::new(clock.clone());
    let mut link = FecTrainer::new(Arc::new(fake.clone()), clock)
        .with_capture(capture)
        .connect(&DeviceRef("fake".into()), rider())
        .await
        .unwrap();
    assert_eq!(fake.writes.lock().unwrap().len(), 2);
    let mut faults = 0;
    while let Ok(event) = link.events.try_recv() {
        if matches!(event, TrainerEvent::Fault(DeviceError::Capture(_))) {
            faults += 1;
        }
    }
    assert_eq!(faults, 3); // page 55, request 54, reply 54
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    tick().await;
    std::fs::remove_file(path).unwrap();
}

struct WakeClock {
    base: MonotonicClock,
    early: AtomicBool,
    late_us: u64,
}
impl Clock for WakeClock {
    fn now(&self) -> MonoTime {
        self.base.now()
    }
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()> {
        if self.early.swap(false, Ordering::SeqCst) {
            Box::pin(async {})
        } else {
            self.base.sleep_until(MonoTime(at.0 + self.late_us))
        }
    }
}
#[tokio::test(start_paused = true)]
async fn early_wake_cannot_underflow_and_late_wakes_do_not_drift_the_tick_grid() {
    let clock = Arc::new(WakeClock {
        base: MonotonicClock::default(),
        early: AtomicBool::new(false),
        late_us: 20_000,
    });
    let fake = Fake::new(clock.clone());
    let link = FecTrainer::new(Arc::new(fake.clone()), clock.clone())
        .connect(&DeviceRef("fake".into()), rider())
        .await
        .unwrap();
    clock.early.store(true, Ordering::SeqCst);
    settle().await;
    let epoch = clock.now().0;
    for i in 0..12u16 {
        link.control
            .send(TrainerCommand::SetMode(ControlMode::Erg { target_w: i }))
            .await
            .unwrap();
        tokio::time::advance(Duration::from_micros(if i == 0 {
            270_000
        } else {
            250_000
        }))
        .await;
        settle().await;
        assert_eq!(
            fake.writes.lock().unwrap().last().unwrap().0.0,
            epoch + (u64::from(i) + 1) * 250_000 + 20_000
        );
    }
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    tick().await;
}

async fn writer_sequence(modes: Vec<ControlMode>) -> usize {
    let (fake, link) = link().await;
    let epoch = fake.clock.now().0;
    let mut pending = std::collections::BTreeMap::new();
    let mut sent = std::collections::BTreeMap::new();
    let mut kind = None;
    let mut previous_desired = Vec::new();
    let mut due = epoch + 1_250_000;
    let mut last_poll = None;
    let mut third_ticks = 0;
    for mode in modes.into_iter().map(Some).chain([None, None, None]) {
        if let Some(mode) = mode {
            let desired = pages(mode).unwrap();
            // A new desired mode supersedes the entire older coalescing slot.
            // Repeating an identical target must not restart its latency budget.
            if desired != previous_desired {
                for (_, issued, deferred) in pending.values_mut() {
                    *issued = fake.clock.now().0;
                    *deferred = None;
                }
            }
            previous_desired.clone_from(&desired);
            if kind != Some(desired[0][4]) {
                sent.clear();
            }
            kind = Some(desired[0][4]);
            pending.retain(|page, _| desired.iter().any(|p| p[4] == *page));
            for page in desired {
                if sent.get(&page[4]) == Some(&page) {
                    pending.remove(&page[4]);
                    continue;
                }
                // Superseding a value replaces its deadline; identical values retain it.
                let entry = pending
                    .entry(page[4])
                    .or_insert((page, fake.clock.now().0, None));
                if entry.0 != page {
                    *entry = (page, fake.clock.now().0, None);
                }
            }
            link.control
                .send(TrainerCommand::SetMode(mode))
                .await
                .unwrap();
        }
        let before = fake.writes.lock().unwrap().len();
        tick().await;
        let writes = fake.writes.lock().unwrap();
        let current = fake.clock.now().0;
        for (at, page) in &writes[before..] {
            if page[4] == 70 {
                let deferred_pair = deferred_pair_poll(&writes[..before], at.0, due, last_poll);
                last_poll = Some(at.0);
                if deferred_pair {
                    // Only a poll already deferred by a pair permits the extra slot.
                    for (control, issued, deferred) in pending.values_mut() {
                        if matches!(control[4], 50 | 51)
                            && (at.0 == *issued + 250_000 || at.0 == *issued + 500_000)
                        {
                            *deferred = Some(at.0);
                        }
                    }
                }
                due += 1_000_000;
            } else {
                if let Some((desired, issued, deferred)) = pending.get(&page[4]) {
                    assert_eq!(page, desired); // writer must use the newest component
                    let budget = sim_budget(page[4], *issued, at.0, *deferred);
                    assert!(
                        at.0 - issued <= budget,
                        "page {}, issued {issued}, written {at:?}",
                        page[4]
                    );
                    third_ticks += usize::from(at.0 - issued > 500_000);
                    pending.remove(&page[4]);
                }
                sent.insert(page[4], *page);
            }
        }
        for (page, issued, deferred) in pending.values() {
            assert!(
                current - issued < sim_budget(page[4], *issued, current + 250_000, *deferred),
                "pending {pending:?}, now {current}, recent {:?}",
                writes.iter().rev().take(5).collect::<Vec<_>>()
            );
        }
        assert!(current <= due + 500_000, "now {current}, due {due}");
    }
    // Flush all three possible slots; the checks above enforce the conditional bound.
    assert!(pending.is_empty());
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    tick().await;
    third_ticks
}
fn deferred_pair_poll(
    previous: &[(MonoTime, [u8; 13])],
    at: u64,
    due: u64,
    last: Option<u64>,
) -> bool {
    assert!(
        (due.saturating_sub(250_000)..=due + 500_000).contains(&at),
        "poll at {at}, due {due}"
    );
    if let Some(last) = last {
        assert!(at - last >= 500_000);
    }
    if at <= due {
        return false;
    }
    let pair = &previous[previous.len() - 2..];
    matches!([pair[0].1[4], pair[1].1[4]], [50, 51] | [51, 50])
        && pair[1].0.0 - pair[0].0.0 == 250_000
        && at - pair[1].0.0 == 250_000
}
#[tokio::test(start_paused = true)]
async fn deferred_poll_exercises_exactly_one_extra_sim_slot() {
    let mode = |value| ControlMode::Sim {
        grade_pct: value,
        crr: 0.004,
        wind: Wind {
            speed_kmh: value,
            ..Wind::default()
        },
    };
    let modes = [mode(0.0); 4]
        .into_iter()
        .chain([mode(1.0); 2])
        .chain([mode(2.0); 4])
        .collect();
    assert!(writer_sequence(modes).await > 0);
}
fn sim_budget(page: u8, issued: u64, written: u64, deferred: Option<u64>) -> u64 {
    if matches!(page, 50 | 51)
        && deferred.is_some_and(|poll| poll == issued + 250_000 || poll + 250_000 == written)
    {
        750_000
    } else {
        500_000
    }
}
#[test]
fn third_tick_requires_sim_and_a_deferred_poll_in_the_first_or_previous_slot() {
    assert_eq!(sim_budget(50, 0, 750_000, Some(250_000)), 750_000);
    assert_eq!(sim_budget(51, 0, 750_000, Some(500_000)), 750_000);
    for page in [48, 49] {
        assert_eq!(sim_budget(page, 0, 750_000, Some(250_000)), 500_000);
    }
    for poll in [None, Some(0), Some(750_000)] {
        assert_eq!(sim_budget(50, 0, 750_000, poll), 500_000);
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn real_writer_new_sim_every_tick_preserves_latency_and_poll_grid(values in prop::collection::vec((0..100u16, 0..100u16), 16..64)) {
        let modes = values.into_iter().map(|(grade, wind)| ControlMode::Sim {
            grade_pct: f32::from(grade), crr: 0.004,
            wind: Wind { speed_kmh: f32::from(wind), ..Wind::default() },
        }).collect();
        runtime().block_on(async { writer_sequence(modes).await; });
    }
    #[test]
    fn real_writer_mixed_modes_preserve_latency_and_poll_grid(values in prop::collection::vec((0..4u8, 0..100u16, 0..100u16), 16..64)) {
        let modes = values.into_iter().map(|(kind, a, b)| match kind {
            0 => ControlMode::Erg { target_w: a },
            1 => ControlMode::Resistance { pct: f32::from(a) },
            2 => ControlMode::Sim { grade_pct: f32::from(a), crr: 0.004, wind: Wind { speed_kmh: f32::from(b), ..Wind::default() } },
            _ => ControlMode::Free,
        }).collect();
        runtime().block_on(async { writer_sequence(modes).await; });
    }
}

#[test]
fn grade_updates_on_poll_ticks_do_not_reset_the_sim_component_phase() {
    let mut pacer = Pacer::default();
    let mut last = None;
    let mut replies = Vec::new();
    for tick in 0..48u64 {
        if tick % 4 == 0 {
            pacer
                .set(sim(f32::from(u16::try_from(tick).unwrap())), tick)
                .unwrap();
        }
        if let Some(p) = pacer.tick(tick).unwrap() {
            if p[4] == 70 {
                replies.push(last.unwrap());
            } else {
                last = Some(p[4]);
            }
        }
    }
    assert!(replies.len() >= 10);
    for pair in replies.windows(2) {
        assert_ne!(pair[0], pair[1]);
    }
}
