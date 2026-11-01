//! #147 tests: discovery, HRS, reconnect and pairing through the transport seams.
use crate::*;
use crank_devices::*;
use futures_util::{StreamExt, future::BoxFuture, stream};
use proptest::prelude::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

fn at(us: u64) -> MonoTime {
    MonoTime(us)
}
async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}
async fn advance(d: Duration) {
    tokio::time::advance(d).await;
    settle().await;
}
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

// ---- HRS parsing (Bluetooth HRS spec §3.106 vectors) ------------------------------------

#[test]
fn hrs_spec_vectors() {
    // uint8 value, no contact feature, no RR.
    let s = parse_heart_rate(at(1), &[0x00, 72]).unwrap();
    assert_eq!((s.at, s.bpm, s.contact), (at(1), Some(72), None));
    assert!(s.rr_ms.is_empty());
    // uint16 value.
    assert_eq!(
        parse_heart_rate(at(0), &[0x01, 0x48, 0x00]).unwrap().bpm,
        Some(72)
    );
    // A 16-bit value above u8 range is not representable: no bpm, not a wrap.
    assert_eq!(
        parse_heart_rate(at(0), &[0x01, 0x2c, 0x01]).unwrap().bpm,
        None
    );
    // Zero is "no value".
    assert_eq!(parse_heart_rate(at(0), &[0x00, 0]).unwrap().bpm, None);
}
#[test]
fn hrs_sensor_contact_bits() {
    let contact = |flags| parse_heart_rate(at(0), &[flags, 60]).unwrap().contact;
    assert_eq!(contact(0b000), None); // not supported, detected bit ignored
    assert_eq!(contact(0b010), None);
    assert_eq!(contact(0b100), Some(false)); // supported, no contact
    assert_eq!(contact(0b110), Some(true)); // supported and detected
}
#[test]
fn hrs_rr_intervals_are_converted_from_1024ths_of_a_second() {
    // 1024 → 1000 ms, 512 → 500 ms, 870 → 849.6 → 850 ms (rounded), 0 → 0.
    let bytes = [0x10, 60, 0x00, 0x04, 0x00, 0x02, 0x66, 0x03, 0x00, 0x00];
    let s = parse_heart_rate(at(0), &bytes).unwrap();
    assert_eq!(s.rr_ms.as_slice(), [1000, 500, 850, 0]);
    // Energy expended (flag bit 3) is skipped before the RR field, uint16 HR.
    let bytes = [0x19, 0x3c, 0x00, 0x10, 0x00, 0x00, 0x04];
    let s = parse_heart_rate(at(0), &bytes).unwrap();
    assert_eq!((s.bpm, s.rr_ms.as_slice()), (Some(60), [1000].as_slice()));
    // RR bytes without the RR flag are ignored, not misread.
    assert!(
        parse_heart_rate(at(0), &[0x00, 60, 1, 2])
            .unwrap()
            .rr_ms
            .is_empty()
    );
}
#[test]
fn hrs_truncated_payloads_are_framing_errors_not_panics() {
    for bytes in [
        &[][..],
        &[0x00],
        &[0x01, 0x48],
        &[0x08, 60, 0x00],
        &[0x10, 60, 0x00], // odd RR length
    ] {
        assert!(
            matches!(
                parse_heart_rate(at(0), bytes),
                Err(DeviceError::Protocol {
                    kind: ProtocolErrorKind::Framing,
                    ..
                })
            ),
            "{bytes:?}"
        );
    }
}
proptest! {
    #[test]
    fn hrs_parser_never_panics_and_rr_count_matches_length(
        bytes in prop::collection::vec(any::<u8>(), 0..64), t in any::<u64>()) {
        if let Ok(s) = parse_heart_rate(at(t), &bytes) {
            let header = 1 + if bytes[0] & 1 == 0 { 1 } else { 2 } + if bytes[0] & 8 != 0 { 2 } else { 0 };
            let expected = if bytes[0] & 0x10 != 0 { (bytes.len() - header) / 2 } else { 0 };
            prop_assert_eq!(s.rr_ms.len(), expected);
            prop_assert_eq!(s.at, at(t));
        }
    }
}

// ---- HRS driver over a fake transport ----------------------------------------------------

type HrReceiver = mpsc::UnboundedReceiver<Result<(MonoTime, Vec<u8>), DeviceError>>;
struct FakeHr {
    streams: Mutex<VecDeque<Result<HrReceiver, DeviceError>>>,
    connects: Mutex<Vec<MonoTime>>,
    clock: Arc<dyn Clock>,
    disconnects: Mutex<usize>,
    /// While set, `disconnect` starts (and counts) but does not finish until a permit is added.
    hold_disconnect: std::sync::atomic::AtomicBool,
    disconnect_gate: tokio::sync::Semaphore,
    /// The next `connect` is connected to the peripheral but hangs at this setup stage
    /// (service discovery, subscribe) for ever, as a connect that gets cancelled mid-way.
    hang_connect: Mutex<Option<&'static str>>,
    hung_at: Mutex<Vec<&'static str>>,
}
impl HrTransport for Arc<FakeHr> {
    fn connect(
        &self,
        _: DeviceRef,
        _: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        self.connects.lock().unwrap().push(self.clock.now());
        if let Some(stage) = self.hang_connect.lock().unwrap().take() {
            self.hung_at.lock().unwrap().push(stage);
            return Box::pin(std::future::pending());
        }
        let next = self.streams.lock().unwrap().pop_front();
        Box::pin(async move {
            let rx = next.ok_or(DeviceError::NotFound)??;
            let notifications: Notifications = Box::pin(stream::unfold(rx, |mut rx| async move {
                let n = rx.recv().await?;
                Some((n, rx))
            }));
            Ok((
                DeviceInfo {
                    name: Some("Strap".into()),
                    ..DeviceInfo::default()
                },
                notifications,
            ))
        })
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        *self.disconnects.lock().unwrap() += 1;
        let fake = self.clone();
        Box::pin(async move {
            if fake
                .hold_disconnect
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                fake.disconnect_gate.acquire().await.unwrap().forget();
            }
            Ok(())
        })
    }
}
type HrSender = mpsc::UnboundedSender<Result<(MonoTime, Vec<u8>), DeviceError>>;
fn fake_hr(clock: &Arc<dyn Clock>, connections: usize) -> (Arc<FakeHr>, Vec<HrSender>) {
    let mut senders = Vec::new();
    let streams = (0..connections)
        .map(|_| {
            let (tx, rx) = mpsc::unbounded_channel();
            senders.push(tx);
            Ok(rx)
        })
        .collect();
    let fake = Arc::new(FakeHr {
        streams: Mutex::new(streams),
        connects: Mutex::default(),
        clock: clock.clone(),
        disconnects: Mutex::new(0),
        hold_disconnect: std::sync::atomic::AtomicBool::new(false),
        disconnect_gate: tokio::sync::Semaphore::new(0),
        hang_connect: Mutex::default(),
        hung_at: Mutex::default(),
    });
    (fake, senders)
}
fn drain<T>(rx: &mut mpsc::Receiver<T>) -> Vec<T> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[tokio::test(start_paused = true)]
async fn hrs_driver_streams_samples_faults_on_garbage_and_reports_loss() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, senders) = fake_hr(&clock, 1);
    let driver = HrsDriver::new(Arc::new(fake.clone()), clock);
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    assert_eq!(link.info.name.as_deref(), Some("Strap"));
    senders[0]
        .send(Ok((at(7), vec![0x16, 64, 0x00, 0x04])))
        .unwrap();
    senders[0].send(Ok((at(8), vec![0x00]))).unwrap(); // truncated
    senders[0].send(Ok((at(9), vec![0x00, 65]))).unwrap(); // survives the bad frame
    settle().await;
    let events = drain(&mut link.events);
    assert_eq!(
        events[..2],
        [
            HrEvent::Link(LinkState::Connecting),
            HrEvent::Link(LinkState::Ready)
        ]
    );
    assert!(matches!(&events[2], HrEvent::Sample(s)
        if s.bpm == Some(64) && s.contact == Some(true) && s.rr_ms.as_slice() == [1000]));
    assert!(matches!(
        &events[3],
        HrEvent::Fault(DeviceError::Protocol { .. })
    ));
    assert!(matches!(&events[4], HrEvent::Sample(s) if s.bpm == Some(65) && s.at == at(9)));
    senders[0].send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    let events = drain(&mut link.events);
    assert_eq!(events.last(), Some(&HrEvent::Link(LinkState::Lost)));
    assert_eq!(*fake.disconnects.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn hr_strap_dropout_reconnects_on_the_backoff_schedule() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, senders) = fake_hr(&clock, 2);
    // Connection 2 is only reachable after two refused attempts.
    fake.streams
        .lock()
        .unwrap()
        .insert(1, Err(DeviceError::NotFound));
    fake.streams
        .lock()
        .unwrap()
        .insert(1, Err(DeviceError::NotFound));
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock.clone(), RideGate::new(true));
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    settle().await;
    let t0 = clock.now();
    senders[0].send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    advance(ms(500)).await;
    advance(ms(1000)).await;
    advance(ms(2000)).await;
    settle().await;
    let connects = fake.connects.lock().unwrap().clone();
    let offsets: Vec<_> = connects.iter().map(|t| t.0 - t0.0).collect();
    // Initial connect at t0 (before the loss), then +0.5 s, +1.5 s, +3.5 s.
    assert_eq!(offsets, [0, 500_000, 1_500_000, 3_500_000]);
    senders[1].send(Ok((at(1), vec![0x00, 80]))).unwrap();
    settle().await;
    let events = drain(&mut link.events);
    let attempts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            HrEvent::Reconnecting { attempt } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, [1, 2, 3]);
    assert!(events.contains(&HrEvent::Link(LinkState::Lost)));
    assert!(matches!(events.last(), Some(HrEvent::Sample(s)) if s.bpm == Some(80)));
}

#[tokio::test(start_paused = true)]
async fn outside_a_ride_a_strap_loss_is_surfaced_and_not_retried() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    // A second connection exists, so a retry would succeed and show up in `connects`.
    let (fake, senders) = fake_hr(&clock, 2);
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock, RideGate::new(false));
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    settle().await;
    drain(&mut link.events);
    senders[0].send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        fake.connects.lock().unwrap().len(),
        1,
        "no reconnect attempt"
    );
    assert_eq!(
        drain(&mut link.events),
        [
            HrEvent::Fault(DeviceError::Disconnected),
            HrEvent::Link(LinkState::Lost)
        ]
    );
    assert!(
        link.events.recv().await.is_none(),
        "stream ends; rediscovery takes over"
    );
}

#[tokio::test(start_paused = true)]
async fn ride_ending_during_strap_backoff_stops_the_loop() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    // Only the first connection exists: attempt 1 fails and the loop backs off.
    let (fake, senders) = fake_hr(&clock, 1);
    let gate = RideGate::new(true);
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock, gate.clone());
    let _link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    settle().await;
    senders[0].send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    advance(ms(500)).await; // attempt 1 fails; now backing off 1 s
    assert_eq!(fake.connects.lock().unwrap().len(), 2);
    gate.set_active(false);
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        fake.connects.lock().unwrap().len(),
        2,
        "no attempts after the ride ended"
    );
}

/// #237: a ride that ends while a strap reconnect is in flight (the `gate.inactive()` arm of
/// the connect select, not the backoff one) abandons the attempt: the hung connect is
/// dropped and cleaned up, no further attempt follows, and the event stream ends.
#[tokio::test(start_paused = true)]
async fn ride_ending_during_a_strap_reconnect_attempt_abandons_it() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, senders) = fake_hr(&clock, 2);
    let gate = RideGate::new(true);
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock, gate.clone());
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    settle().await;
    *fake.hang_connect.lock().unwrap() = Some("mid-setup");
    senders[0].send(Err(DeviceError::Disconnected)).unwrap();
    settle().await;
    advance(ms(500)).await; // attempt 1 starts and hangs mid-setup
    assert_eq!(fake.connects.lock().unwrap().len(), 2);
    assert_eq!(*fake.hung_at.lock().unwrap(), ["mid-setup"]);
    let disconnects = *fake.disconnects.lock().unwrap();
    gate.set_active(false);
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        fake.connects.lock().unwrap().len(),
        2,
        "no attempts after the ride ended"
    );
    assert_eq!(
        *fake.disconnects.lock().unwrap(),
        disconnects + 1,
        "the abandoned attempt was disconnected"
    );
    let events = drain(&mut link.events);
    assert_eq!(
        events.last(),
        Some(&HrEvent::Reconnecting { attempt: 1 }),
        "nothing follows the abandoned attempt"
    );
    assert!(
        link.events.recv().await.is_none(),
        "the stream ends with the supervisor"
    );
}

// ---- discovery ---------------------------------------------------------------------------

fn short(n: u16) -> Uuid {
    Uuid::from_u128((u128::from(n) << 96) | 0x0000_1000_8000_0080_5f9b_34fb)
}
use uuid::Uuid;

#[test]
fn classification_matrix() {
    assert_eq!(
        classify(&[FEC_SERVICE, short(0x1818)]),
        Some((Some(DriverKind::FeC), false))
    );
    assert_eq!(
        classify(&[short(0x1826), short(0x1818)]),
        Some((Some(DriverKind::Ftms), false))
    );
    // FE-C wins when a trainer advertises both.
    assert_eq!(
        classify(&[short(0x1826), FEC_SERVICE]),
        Some((Some(DriverKind::FeC), false))
    );
    assert_eq!(classify(&[short(0x180d)]), Some((None, true)));
    assert_eq!(
        classify(&[FEC_SERVICE, short(0x180d)]),
        Some((Some(DriverKind::FeC), true))
    );
    // Cycling-Power-only and unrelated devices are ignored in v1.
    assert_eq!(classify(&[short(0x1818)]), None);
    assert_eq!(classify(&[short(0x180f)]), None);
    assert_eq!(classify(&[]), None);
}
#[test]
fn scan_filter_lists_exactly_the_four_services() {
    assert_eq!(
        SCAN_SERVICES,
        [FEC_SERVICE, short(0x1826), short(0x1818), short(0x180d)]
    );
}
struct Ads(Mutex<Option<Vec<Advertisement>>>);
impl Advertisements for Ads {
    fn advertisements(&self) -> futures_util::stream::BoxStream<'static, Advertisement> {
        stream::iter(self.0.lock().unwrap().take().unwrap()).boxed()
    }
}
fn ad(id: &str, name: Option<&str>, services: &[Uuid]) -> Advertisement {
    Advertisement {
        device: DeviceRef(id.into()),
        name: name.map(Into::into),
        services: services.to_vec(),
    }
}
#[tokio::test]
async fn scanner_post_filters_classifies_and_reports_changes_once() {
    let scanner = Scanner(Ads(Mutex::new(Some(vec![
        ad("cps", Some("Pedals"), &[short(0x1818)]), // post-filtered (Linux ignores the OS filter)
        ad("neo", None, &[]),                        // discovered, services not known yet
        ad("neo", None, &[FEC_SERVICE]),             // classified
        ad("neo", None, &[FEC_SERVICE]),             // duplicate: suppressed
        ad("neo", Some("Tacx NEO"), &[FEC_SERVICE]), // name arrived: reported again
        ad("hr", Some("Strap"), &[short(0x180d)]),
        ad("kick", Some("KICKR"), &[short(0x1826)]),
    ]))));
    let found: Vec<_> = scanner.scan().collect().await;
    let summary: Vec<_> = found
        .iter()
        .map(|d| {
            (
                d.device.0.as_str(),
                d.info.name.as_deref(),
                d.trainer,
                d.heart_rate,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("neo", None, Some(DriverKind::FeC), false),
            ("neo", Some("Tacx NEO"), Some(DriverKind::FeC), false),
            ("hr", Some("Strap"), None, true),
            ("kick", Some("KICKR"), Some(DriverKind::Ftms), false),
        ]
    );
}

// ---- pairing memory ----------------------------------------------------------------------

#[test]
fn pairing_matches_by_platform_id_then_name_and_verifies_serial() {
    let info = DeviceInfo {
        name: Some("Tacx NEO".into()),
        serial: Some("55624".into()),
        model: Some("T2800".into()),
        ..DeviceInfo::default()
    };
    let pairing = Pairing::from_connection(&DeviceRef("uuid-1".into()), &info);
    let found = |id: &str, name: Option<&str>| DiscoveredDevice {
        device: DeviceRef(id.into()),
        info: DeviceInfo {
            name: name.map(Into::into),
            ..DeviceInfo::default()
        },
        trainer: Some(DriverKind::FeC),
        heart_rate: false,
    };
    assert!(pairing.matches(&found("uuid-1", None)));
    assert!(pairing.matches(&found("uuid-2", Some("Tacx NEO")))); // platform id re-assigned
    assert!(!pairing.matches(&found("uuid-2", Some("KICKR"))));
    assert!(!pairing.matches(&found("uuid-2", None)));
    assert!(pairing.verify(&info));
    assert!(pairing.verify(&DeviceInfo::default())); // serial unknown: cannot refute
    assert!(!pairing.verify(&DeviceInfo {
        serial: Some("99999".into()),
        ..DeviceInfo::default()
    }));
    assert!(!pairing.verify(&DeviceInfo {
        model: Some("Other".into()),
        ..DeviceInfo::default()
    }));
    assert_eq!(pairing.device_ref(), DeviceRef("uuid-1".into()));
}
#[test]
fn pairing_round_trips_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let info = DeviceInfo {
        name: Some("Tacx NEO".into()),
        serial: Some("55624".into()),
        model: Some("T2800".into()),
        ..DeviceInfo::default()
    };
    let pairing = Pairing::from_connection(&DeviceRef("uuid-1".into()), &info);
    let store = crank_store::Store::open_owner(dir.path()).unwrap();
    store
        .remember_device(crank_store::Device {
            id: "trainer".into(),
            kind: "trainer".into(),
            name: pairing.name.clone(),
            platform_id: pairing.platform_id.clone(),
            serial: pairing.serial.clone(),
            model: pairing.model.clone(),
            last_seen: 1,
        })
        .unwrap();
    drop(store);
    let stored = crank_store::Store::open_read_only(dir.path())
        .unwrap()
        .devices()
        .unwrap();
    let restored = Pairing {
        platform_id: stored[0].platform_id.clone(),
        name: stored[0].name.clone(),
        serial: stored[0].serial.clone(),
        model: stored[0].model.clone(),
    };
    assert_eq!(restored, pairing);
}

// ---- trainer reconnect supervisor --------------------------------------------------------

struct Device {
    link: TrainerLink,
    events: mpsc::Sender<TrainerEvent>,
    commands: mpsc::Receiver<TrainerCommand>,
}
fn device_link() -> Device {
    let (control, commands) = mpsc::channel(16);
    let (events, receiver) = mpsc::channel(64);
    Device {
        link: TrainerLink {
            seq: SeqSource::default(),
            info: DeviceInfo::default(),
            caps: TrainerCaps {
                erg: true,
                sim: true,
                resistance: true,
                max_resistance_n: None,
                max_target_w: 2000,
            },
            control,
            events: receiver,
        },
        events,
        commands,
    }
}
struct Script {
    clock: Arc<dyn Clock>,
    outcomes: Mutex<VecDeque<Result<TrainerLink, DeviceError>>>,
    attempts: Mutex<Vec<(MonoTime, RiderConfig)>>,
}
struct ScriptDriver(Arc<Script>);
impl TrainerDriver for ScriptDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::FeC
    }
    fn connect(
        &self,
        _: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        self.0
            .attempts
            .lock()
            .unwrap()
            .push((self.0.clock.now(), rider));
        let next = self
            .0
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(DeviceError::NotFound));
        Box::pin(async move { next })
    }
}
fn rider() -> RiderConfig {
    RiderConfig {
        rider_kg: 71.5,
        bike_kg: 9.0,
        wheel_m: 0.68,
    }
}
/// The engine's ERG 200 W command (sequence 1).
fn erg_200() -> TrainerCommand {
    TrainerCommand::SetMode {
        mode: ControlMode::Erg { target_w: 200 },
        seq: CommandSeq(1),
    }
}
fn script(clock: &Arc<dyn Clock>, outcomes: Vec<Result<TrainerLink, DeviceError>>) -> Arc<Script> {
    Arc::new(Script {
        clock: clock.clone(),
        outcomes: Mutex::new(outcomes.into()),
        attempts: Mutex::default(),
    })
}

#[test]
fn backoff_schedule_is_exact() {
    let secs: Vec<f64> = (1..=8).map(|a| backoff(a).as_secs_f64()).collect();
    assert_eq!(secs, [0.5, 1.0, 2.0, 4.0, 5.0, 5.0, 5.0, 5.0]);
}

#[tokio::test(start_paused = true)]
async fn trainer_reconnects_on_the_exact_schedule_reapplies_rider_and_never_restores_mode() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let mut second = device_link();
    let mut outcomes: Vec<_> = (0..5)
        .map(|_| Err(DeviceError::ConnectFailed("not found".into())))
        .collect();
    outcomes.insert(0, Ok(first.link));
    outcomes.push(Ok(second.link));
    let script = script(&clock, outcomes);
    let driver = Reconnecting::new(
        ScriptDriver(script.clone()),
        clock.clone(),
        RideGate::new(true),
    );
    let mut link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    // A running ride: the engine had issued ERG before the loss.
    link.control.send(erg_200()).await.unwrap();
    settle().await;
    let mut first_commands = first.commands;
    assert_eq!(
        first_commands.try_recv(),
        Ok(TrainerCommand::SetMode {
            mode: ControlMode::Erg { target_w: 200 },
            seq: CommandSeq(1)
        })
    );

    let lost_at = clock.now();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    // Delays 0.5, 1, 2, 4, 5 and a final 5 s: one tick before each deadline nothing
    // happens, on the deadline exactly one attempt is made.
    let mut elapsed = 0;
    for (index, delay) in [500, 1000, 2000, 4000, 5000, 5000].into_iter().enumerate() {
        advance(ms(delay - 1)).await;
        assert_eq!(
            script.attempts.lock().unwrap().len(),
            1 + index,
            "early at {delay}"
        );
        advance(ms(1)).await;
        assert_eq!(
            script.attempts.lock().unwrap().len(),
            2 + index,
            "late at {delay}"
        );
        elapsed += delay;
        let (t, r) = *script.attempts.lock().unwrap().last().unwrap();
        assert_eq!(t.0 - lost_at.0, elapsed * 1000);
        assert_eq!(r, rider(), "rider config is re-applied on every attempt");
    }
    // The consumer-facing link survived: Lost, then numbered attempts and faults, then Ready.
    let mut second_events = vec![
        TrainerEvent::Link(LinkState::Connecting),
        TrainerEvent::Link(LinkState::Ready),
    ];
    for e in second_events.drain(..) {
        second.events.send(e).await.unwrap();
    }
    settle().await;
    let events = drain(&mut link.events);
    assert_eq!(events[0], TrainerEvent::Link(LinkState::Lost));
    let attempts: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let TrainerEvent::Reconnecting { attempt } = e {
                Some(*attempt)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(attempts, [1, 2, 3, 4, 5, 6]);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, TrainerEvent::Fault(_)))
            .count(),
        5
    );
    assert_eq!(events.last(), Some(&TrainerEvent::Link(LinkState::Ready)));
    // The driver restored nothing: the new device saw no command at all.
    assert!(second.commands.try_recv().is_err());
    // Only the engine's explicit SetMode after Ready reaches it.
    link.control.send(erg_200()).await.unwrap();
    settle().await;
    assert_eq!(
        second.commands.try_recv(),
        Ok(TrainerCommand::SetMode {
            mode: ControlMode::Erg { target_w: 200 },
            seq: CommandSeq(1)
        })
    );
}

#[tokio::test(start_paused = true)]
async fn commands_sent_while_disconnected_are_discarded_not_replayed() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let mut second = device_link();
    let script = script(&clock, vec![Ok(first.link), Ok(second.link)]);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, RideGate::new(true));
    let mut link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    link.control
        .send(TrainerCommand::SetMode {
            mode: ControlMode::Free,
            seq: CommandSeq(1),
        })
        .await
        .unwrap();
    advance(ms(500)).await;
    // The discarded command did not end the loop: attempt 1 connected the new device,
    // whose Ready reaches the consumer, and the command was never replayed to it.
    assert_eq!(script.attempts.lock().unwrap().len(), 2);
    second
        .events
        .send(TrainerEvent::Link(LinkState::Ready))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        drain(&mut link.events).last(),
        Some(&TrainerEvent::Link(LinkState::Ready))
    );
    assert!(second.commands.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn outside_a_ride_a_loss_is_surfaced_and_not_retried() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let script = script(&clock, vec![Ok(first.link)]);
    let gate = RideGate::new(false);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, gate.clone());
    let mut link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(script.attempts.lock().unwrap().len(), 1);
    assert_eq!(
        drain(&mut link.events),
        [TrainerEvent::Link(LinkState::Lost)]
    );
    assert!(
        link.events.recv().await.is_none(),
        "stream ends; rediscovery takes over"
    );
}

#[tokio::test(start_paused = true)]
async fn ride_ending_during_backoff_stops_the_loop() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let script = script(&clock, vec![Ok(first.link)]);
    let gate = RideGate::new(true);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, gate.clone());
    let _link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    advance(ms(500)).await; // attempt 1 fails; now backing off 1 s
    assert_eq!(script.attempts.lock().unwrap().len(), 2);
    gate.set_active(false);
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        script.attempts.lock().unwrap().len(),
        2,
        "no attempts after the ride ended"
    );
}

#[tokio::test(start_paused = true)]
async fn release_during_backoff_stops_the_loop() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let script = script(&clock, vec![Ok(first.link)]);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, RideGate::new(true));
    let link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    advance(ms(500)).await; // attempt 1 (fails), now waiting 1 s
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        script.attempts.lock().unwrap().len(),
        2,
        "initial connect + attempt 1 only"
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_the_link_stops_reconnecting_and_release_is_forwarded() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let first = device_link();
    let mut first_commands = first.commands;
    let script = script(&clock, vec![Ok(first.link)]);
    let driver = Reconnecting::new(
        ScriptDriver(script.clone()),
        clock.clone(),
        RideGate::new(true),
    );
    let link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    // Release while connected reaches the device.
    link.control.send(TrainerCommand::Disconnect).await.unwrap();
    settle().await;
    assert_eq!(first_commands.try_recv(), Ok(TrainerCommand::Disconnect));
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        script.attempts.lock().unwrap().len(),
        1,
        "a released ride is never reconnected"
    );
    drop(link);

    // Consumer disappears mid-outage.
    let first = device_link();
    let script = self::script(&clock, vec![Ok(first.link)]);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, RideGate::new(true));
    let link = driver
        .connect(&DeviceRef("neo".into()), rider())
        .await
        .unwrap();
    first
        .events
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .unwrap();
    settle().await;
    drop(link);
    settle().await;
    advance(Duration::from_mins(1)).await;
    assert_eq!(
        script.attempts.lock().unwrap().len(),
        1,
        "no attempt once the consumer is gone"
    );
}

#[tokio::test(start_paused = true)]
async fn first_connect_failure_is_returned_not_retried() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let script = script(&clock, vec![Err(DeviceError::NotFound)]);
    let driver = Reconnecting::new(ScriptDriver(script.clone()), clock, RideGate::new(true));
    let result = driver.connect(&DeviceRef("neo".into()), rider()).await;
    assert!(matches!(result, Err(DeviceError::NotFound)));
    advance(Duration::from_secs(30)).await;
    assert_eq!(script.attempts.lock().unwrap().len(), 1);
}

// ---- #201: capture tests leave nothing behind, even when they panic ------------------------

#[test]
fn a_panicking_capture_test_leaves_no_file_behind() {
    let seen = Mutex::new(None);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panics.fecap");
        *seen.lock().unwrap() = Some(dir.path().to_owned());
        Capture::create(&path, MonoTime(0))
            .unwrap()
            .record(MonoTime(0), true, &[1])
            .unwrap();
        assert!(path.exists());
        panic!("injected failure after the capture file exists");
    }));
    assert!(outcome.is_err());
    let dir = seen.lock().unwrap().take().unwrap();
    assert!(
        !dir.exists(),
        "temp dir (and its .fecap) must be removed on unwind"
    );
}

// ---- release: the consumer's end of the stream means "handed back" ------------------------

/// The real two-layer stack of production (`ReconnectingHr` over `HrsDriver` over a fake
/// transport): the stream ends for the consumer only after the transport has disconnected,
/// whether the strap sends every second or nothing at all (#290).
async fn strap_release_waits_for_the_disconnect(sample_every: Option<Duration>) {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, senders) = fake_hr(&clock, 1);
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock, RideGate::new(true));
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    if let Some(every) = sample_every {
        let sender = senders[0].clone();
        tokio::spawn(async move {
            while sender.send(Ok((at(1), vec![0x00, 70]))).is_ok() {
                tokio::time::sleep(every).await;
            }
        });
        advance(Duration::from_millis(2500)).await;
    }
    settle().await;
    assert_eq!(*fake.disconnects.lock().unwrap(), 0, "riding");
    // The disconnect takes a while. What the consumer of the link does when it is done with
    // it must not complete before the transport has finished disconnecting.
    fake.hold_disconnect
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waiter = {
        let released = released.clone();
        tokio::spawn(async move {
            release_events(&mut link.events).await;
            released.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };
    // in small steps: the release poll sleeps between its checks
    for _ in 0..50 {
        advance(ms(20)).await;
    }
    assert_eq!(*fake.disconnects.lock().unwrap(), 1, "disconnecting");
    assert!(
        !released.load(std::sync::atomic::Ordering::SeqCst),
        "released while the transport is still disconnecting"
    );
    fake.disconnect_gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("the strap is released")
        .unwrap();
}
/// A platform disconnect that never completes ends the strap's stream anyway, after the
/// disconnect timeout and not before.
#[tokio::test(start_paused = true)]
async fn a_hanging_strap_disconnect_does_not_hang_the_release() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, _senders) = fake_hr(&clock, 1);
    fake.hold_disconnect
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let inner = HrsDriver::new(Arc::new(fake.clone()), clock.clone());
    let driver = ReconnectingHr::new(inner, clock, RideGate::new(true));
    let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
    settle().await;
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let released = released.clone();
        tokio::spawn(async move {
            release_events(&mut link.events).await;
            released.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    for _ in 0..90 {
        advance(ms(20)).await;
    }
    assert!(!released.load(std::sync::atomic::Ordering::SeqCst), "1.8 s");
    for _ in 0..20 {
        advance(ms(20)).await;
    }
    assert!(released.load(std::sync::atomic::Ordering::SeqCst), "2.2 s");
}
#[tokio::test(start_paused = true)]
async fn a_one_hertz_strap_is_released_only_after_its_disconnect() {
    strap_release_waits_for_the_disconnect(Some(Duration::from_secs(1))).await;
}
#[tokio::test(start_paused = true)]
async fn a_silent_strap_is_released_only_after_its_disconnect() {
    strap_release_waits_for_the_disconnect(None).await;
}

/// #304: a shutdown that cancels a reconnect attempt mid-connect (during service discovery or
/// subscribe, the peripheral already connected) disconnects the peripheral before the strap
/// is released (ADR-0002 §7.9: "released" means disconnected), whatever the disconnect takes.
#[tokio::test(start_paused = true)]
async fn a_shutdown_mid_connect_disconnects_before_the_strap_is_released() {
    {
        let stage = "mid-setup";
        let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
        let (fake, senders) = fake_hr(&clock, 1);
        let cleanup = ConnectCleanup::new();
        let inner =
            HrsDriver::new(Arc::new(fake.clone()), clock.clone()).with_cleanup(cleanup.clone());
        let mut link = ReconnectingHr::new(inner, clock, RideGate::new(true))
            .with_cleanup(cleanup)
            .connect(&DeviceRef("strap".into()))
            .await
            .unwrap();
        settle().await;
        // The strap drops out; the reconnect attempt gets stuck mid-connect.
        *fake.hang_connect.lock().unwrap() = Some(stage);
        drop(senders);
        for _ in 0..60 {
            advance(ms(20)).await;
        }
        assert_eq!(*fake.hung_at.lock().unwrap(), [stage]);
        assert_eq!(*fake.disconnects.lock().unwrap(), 1, "the lost link");
        // The disconnect of the half-open peripheral takes a while.
        fake.hold_disconnect
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let released = released.clone();
            tokio::spawn(async move {
                release_events(&mut link.events).await;
                released.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
        for _ in 0..25 {
            advance(ms(20)).await;
        }
        assert_eq!(
            *fake.disconnects.lock().unwrap(),
            2,
            "{stage}: the half-open peripheral is disconnected"
        );
        assert!(
            !released.load(std::sync::atomic::Ordering::SeqCst),
            "{stage}: released while the disconnect is still running"
        );
        fake.disconnect_gate.add_permits(1);
        for _ in 0..10 {
            advance(ms(20)).await;
        }
        assert!(
            released.load(std::sync::atomic::Ordering::SeqCst),
            "{stage}"
        );
    }
}
/// #304: a cancelled connect whose disconnect hangs frees the driver's admission after the
/// disconnect timeout, so a later connect is not refused with `AlreadyConnected` for ever.
#[tokio::test(start_paused = true)]
async fn a_hung_disconnect_of_a_cancelled_connect_frees_the_admission() {
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::default());
    let (fake, _senders) = fake_hr(&clock, 1);
    fake.hold_disconnect
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let driver = HrsDriver::new(Arc::new(fake.clone()), clock);
    let dev = DeviceRef("strap".into());
    *fake.hang_connect.lock().unwrap() = Some("mid-setup");
    let attempt = tokio::spawn(driver.connect(&dev));
    settle().await;
    attempt.abort();
    advance(ms(100)).await;
    assert_eq!(*fake.disconnects.lock().unwrap(), 1, "cleanup started");
    assert_eq!(
        driver.connect(&dev).await.err(),
        Some(DeviceError::AlreadyConnected),
        "held while the cleanup runs"
    );
    for _ in 0..110 {
        advance(ms(20)).await;
    }
    driver
        .connect(&dev)
        .await
        .expect("the admission is free after the disconnect timeout");
}
