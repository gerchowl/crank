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
        atomic::{AtomicBool, Ordering},
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
        let events = receive(MonoTime(at), &bytes);
        prop_assert!(events.len() <= 2);
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
                    status: Some(status.into()),
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
struct Fake {
    sender: mpsc::UnboundedSender<Notification>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Notification>>>,
    writes: Mutex<Vec<(MonoTime, [u8; 13])>>,
    last: Mutex<Option<[u8; 13]>>,
    clock: Arc<dyn Clock>,
    fail: AtomicBool,
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
            let notification = if bytes[4] == 70 && bytes[10] == 54 {
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
        Box::pin(async { Ok(()) })
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
        expected.push((at, CommandStatus::from(status)));
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
    tracing::subscriber::set_global_default(Trace).unwrap();
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
