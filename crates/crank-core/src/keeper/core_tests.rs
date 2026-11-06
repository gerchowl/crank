//! The link keeper through the core (ADR-0002 §5.1.8): the simulator's latency model and the
//! test radio under the real facade, on paused tokio time. Each test names the §5.1.8 bullet
//! it covers.
use super::tests::{FakeRadio, TICK, fake, found, latency, latent_core, open};
use super::*;
use crate::keeper::{GRACE, Phase, SCAN_CONTINUOUS, SCAN_PERIOD, SCAN_WINDOW, T_ABANDON, T_COOL};
use crank_devices::{
    DeviceInfo, MonoTime,
    sim::{SimFault, SimLatency},
};
use std::sync::atomic::Ordering::SeqCst;
use tokio::time::{Instant, sleep};

const fn likely(ready: bool) -> LinkIntent {
    LinkIntent::Likely { ready }
}
const fn release(reason: ReleaseReason) -> LinkIntent {
    LinkIntent::Release { reason }
}
const S: fn(u64) -> Duration = Duration::from_secs;
async fn phases(core: &Core) -> [Option<Phase>; 2] {
    core.keeper().unwrap().phases().await
}
fn connects(radio: &FakeRadio) -> [usize; 2] {
    [
        radio.connects[0].load(SeqCst),
        radio.connects[1].load(SeqCst),
    ]
}
fn disconnects(radio: &FakeRadio) -> usize {
    radio.disconnects.lock().unwrap().len()
}
fn straps_released(radio: &FakeRadio) -> usize {
    radio.hr_released.load(SeqCst)
}
/// One second before `deadline`.
fn before(deadline: Duration) -> Duration {
    deadline.saturating_sub(S(1))
}
/// Long enough for a release to finish (the strap's release polls every 20 ms).
const SETTLE: Duration = Duration::from_millis(100);
/// A core whose trainer and strap connect at once, both warm after a `Likely`.
async fn warm_core(dir: &std::path::Path) -> (Core, FakeRadio) {
    let (core, radio) = latent_core(dir, SimLatency::default(), SimLatency::default()).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);
    (core, radio)
}
/// A core with a loaded ride over warm links.
async fn loaded_core(dir: &std::path::Path) -> (Core, FakeRadio) {
    let (core, radio) = warm_core(dir).await;
    core.ride_load(None).await.unwrap();
    (core, radio)
}
async fn loaded(core: &Core) -> bool {
    core.ride_status().await.unwrap().status.loaded
}

/// §5.1.8 "Warm start under 1 s": latencies of 3 s (trainer) and 2 s (strap); `Likely`,
/// then both are warm, and `ride_load` answers within 1 s with zero `connect` calls.
#[tokio::test(start_paused = true)]
async fn a_warm_start_answers_within_a_second_with_no_connect() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = latent_core(dir.path(), latency(1, 1, 1), latency(1, 1, 0)).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(3) + TICK).await;
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);
    assert_eq!(connects(&radio), [1, 1]);
    let asked = Instant::now();
    let loaded = core.ride_load(None).await.unwrap();
    assert!(asked.elapsed() < S(1), "{:?}", asked.elapsed());
    assert!(loaded.hr_linked);
    assert_eq!(connects(&radio), [1, 1], "no connect at the load");
    assert_eq!(phases(&core).await, [Some(Phase::Riding); 2]);
}

/// §5.1.8 "Warm start under 1 s": `searching` / `connecting` at the load awaits the running
/// attempt (connect count unchanged).
#[tokio::test(start_paused = true)]
async fn a_load_awaits_the_running_attempts_and_connects_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = latent_core(dir.path(), latency(1, 1, 1), latency(1, 1, 0)).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(Duration::from_millis(1500)).await;
    assert_eq!(phases(&core).await, [Some(Phase::Connecting); 2]);
    let asked = Instant::now();
    assert!(core.ride_load(None).await.unwrap().hr_linked);
    let took = asked.elapsed();
    assert!(took >= Duration::from_millis(1500) && took <= Duration::from_millis(1500) + TICK);
    assert_eq!(connects(&radio), [1, 1]);
}

/// §5.1.4: a load cancelled while its attempts run leaves them running, and the next load
/// awaits them, never failing on the wait the cancelled one left.
#[tokio::test(start_paused = true)]
async fn a_load_after_a_cancelled_one_awaits_the_same_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    radio.connect_delay = S(10);
    let core = open(dir.path(), &radio);
    let cancelled = tokio::time::timeout(S(2), core.ride_load(None)).await;
    assert!(cancelled.is_err(), "the caller gave up");
    let asked = Instant::now();
    assert!(core.ride_load(None).await.unwrap().hr_linked);
    assert_eq!(asked.elapsed(), S(8), "the rest of the first attempt");
    assert_eq!(connects(&radio), [1, 1]);
}

/// §5.1.8 "Idle cool-down disconnects": after `Left` both are still warm at `T_cool` − 1 s
/// and released at `T_cool` (driver disconnect observed, phase `cold`).
#[tokio::test(start_paused = true)]
async fn left_releases_both_at_t_cool_and_not_before() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(LinkIntent::Left).await.unwrap();
    sleep(before(T_COOL)).await;
    assert_eq!((disconnects(&radio), straps_released(&radio)), (0, 0));
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);
    sleep(S(1) + SETTLE).await;
    assert_eq!((disconnects(&radio), straps_released(&radio)), (1, 1));
    assert_eq!(phases(&core).await, [Some(Phase::Cold); 2]);
    assert_eq!(
        *radio.commands.lock().unwrap(),
        [TrainerCommand::Disconnect],
        "the release is the only command of a warm period"
    );
}

/// §5.1.8 "Idle cool-down disconnects": a `Likely` between `Left` and `T_cool` restarts the
/// timer.
#[tokio::test(start_paused = true)]
async fn a_likely_restarts_the_idle_timer() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(LinkIntent::Left).await.unwrap();
    sleep(S(200)).await;
    core.links_intent(likely(false)).await.unwrap();
    core.links_intent(LinkIntent::Left).await.unwrap();
    sleep(before(T_COOL)).await;
    assert_eq!(disconnects(&radio), 0);
    sleep(S(1) + SETTLE).await;
    assert_eq!(disconnects(&radio), 1);
}

/// §5.1.8 "Idle cool-down disconnects": every `Release` reason disconnects at once, an attempt
/// still connecting included (its half-open peripheral is released).
#[tokio::test(start_paused = true)]
async fn every_release_reason_disconnects_at_once_mid_attempt_included() {
    use ReleaseReason::{Devices, Hidden, RiderRelease, Sleep};
    for reason in [Hidden, Sleep, Devices, RiderRelease] {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut radio) =
            latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
        radio.connect_delay = S(10);
        let core = open(dir.path(), &radio);
        core.links_intent(likely(false)).await.unwrap();
        sleep(S(1)).await;
        assert_eq!(
            phases(&core).await,
            [Some(Phase::Searching), Some(Phase::Warm)]
        );
        let asked = Instant::now();
        core.links_intent(release(reason)).await.unwrap();
        assert!(asked.elapsed() < S(1), "{reason:?}: at once");
        // (answered once the releases are done: no settling wait, N5)
        assert_eq!(straps_released(&radio), 1, "{reason:?}");
        assert_eq!(
            radio.half_open.load(SeqCst),
            1,
            "{reason:?}: the attempt is dropped"
        );
        assert_eq!(phases(&core).await, [Some(Phase::Cold); 2], "{reason:?}");
    }
}

/// §5.1.8 "Idle cool-down disconnects": the latch blocks `Likely` for the trainer until Retry
/// or "Ride this".
#[tokio::test(start_paused = true)]
async fn the_latch_blocks_likely_until_retry_or_ride_this() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(release(ReleaseReason::RiderRelease))
        .await
        .unwrap();
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(60)).await;
    assert_eq!(connects(&radio), [1, 2], "the strap only");
    core.links_intent(LinkIntent::Retry).await.unwrap();
    sleep(TICK).await;
    assert_eq!(connects(&radio), [2, 2]);
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);

    core.links_intent(release(ReleaseReason::RiderRelease))
        .await
        .unwrap();
    core.links_intent(likely(false)).await.unwrap();
    assert!(core.ride_load(None).await.unwrap().hr_linked, "Ride this");
    assert_eq!(connects(&radio)[0], 3);
}

/// §5.1.8 "Background grace": still warm at 59 s, released at 60 s; a `Likely` at 30 s
/// cancels the release.
#[tokio::test(start_paused = true)]
async fn the_background_grace_releases_at_sixty_seconds_unless_likely_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(LinkIntent::Background).await.unwrap();
    sleep(before(GRACE)).await;
    assert_eq!(disconnects(&radio), 0);
    sleep(S(1) + SETTLE).await;
    assert_eq!((disconnects(&radio), straps_released(&radio)), (1, 1));

    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(LinkIntent::Background).await.unwrap();
    sleep(S(30)).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(90)).await;
    assert_eq!(disconnects(&radio), 0);
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);
}

/// §5.1.8 "The Ready sheet holds": with a failed load, `Likely { ready: true }` and no input
/// for 30 min of virtual time, the keeper's links stay warm.
#[tokio::test(start_paused = true)]
async fn the_ready_sheet_over_a_failed_load_holds_the_keepers_links() {
    let dir = tempfile::tempdir().unwrap();
    let (_, radio) = latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    radio.fail_connect.store(true, SeqCst);
    let core = open(dir.path(), &radio);
    let failed = core.ride_load(None).await.unwrap_err();
    assert_eq!(failed.code, ErrorCode::DeviceUnavailable);
    core.links_intent(likely(true)).await.unwrap();
    sleep(S(30 * 60)).await;
    assert_eq!(straps_released(&radio), 0);
    assert_eq!(
        phases(&core).await,
        [Some(Phase::NotFound), Some(Phase::Warm)]
    );
}

/// §5.1.8 "A loaded ride keeps its links": with a loaded, unstarted ride, `Hidden`,
/// `Background` past 60 s and `T_cool` past 5 min leave the links connected, and nothing is
/// written (no command after `Ready`).
#[tokio::test(start_paused = true)]
async fn a_loaded_ride_keeps_its_links_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    core.links_intent(release(ReleaseReason::Hidden))
        .await
        .unwrap();
    core.links_intent(LinkIntent::Background).await.unwrap();
    sleep(GRACE + S(1)).await;
    core.links_intent(LinkIntent::Left).await.unwrap();
    sleep(T_COOL + S(1)).await;
    assert!(loaded(&core).await);
    assert_eq!((disconnects(&radio), straps_released(&radio)), (0, 0));
    assert!(radio.commands.lock().unwrap().is_empty(), "nothing written");
    assert_eq!(phases(&core).await, [Some(Phase::Riding); 2]);
}

/// §5.1.8 "An abandoned load releases": a ride loaded but unstarted, with no input, is
/// discarded at `T_abandon` (links released, phase `cold`).
#[tokio::test(start_paused = true)]
async fn an_abandoned_load_is_discarded_at_t_abandon() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    sleep(before(T_ABANDON)).await;
    assert!(loaded(&core).await);
    sleep(S(1) + SETTLE).await;
    assert!(!loaded(&core).await);
    assert_eq!((disconnects(&radio), straps_released(&radio)), (1, 1));
    assert_eq!(phases(&core).await, [Some(Phase::Cold); 2]);
    assert!(
        core.rides_list().await.unwrap().is_empty(),
        "nothing recorded"
    );
}

/// §5.1.8 "An abandoned load releases": input at 9 min restarts the timer.
#[tokio::test(start_paused = true)]
async fn rider_input_restarts_t_abandon() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = loaded_core(dir.path()).await;
    sleep(S(9 * 60)).await;
    core.links_intent(likely(false)).await.unwrap();
    sleep(before(T_ABANDON)).await;
    assert!(loaded(&core).await);
    sleep(S(1) + SETTLE).await;
    assert!(!loaded(&core).await);
}

/// §5.1.8 "Sleep discards an unstarted ride": `Release(Sleep)` with a loaded, unstarted ride
/// discards it and releases its links; with a started ride, the ride and its links are
/// untouched.
#[tokio::test(start_paused = true)]
async fn sleep_discards_an_unstarted_ride_and_leaves_a_started_one() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    core.links_intent(release(ReleaseReason::Sleep))
        .await
        .unwrap();
    assert!(!loaded(&core).await, "answered once discarded and released");
    assert_eq!(disconnects(&radio), 1);

    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    core.ride_start().await.unwrap();
    core.links_intent(release(ReleaseReason::Sleep))
        .await
        .unwrap();
    sleep(S(5)).await;
    assert!(core.ride_status().await.unwrap().status.in_progress());
    assert_eq!(disconnects(&radio), 0);
}

/// The architect's decision on #435 (ADR-0002 §5.1.3): "Release trainer" on the Ready sheet
/// with a ride loaded but not started unloads it and releases its links, and the trainer is
/// latched (no warm-up until a new load or Retry).
#[tokio::test(start_paused = true)]
async fn release_trainer_with_an_unstarted_ride_unloads_it_and_latches() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    core.links_intent(release(ReleaseReason::RiderRelease))
        .await
        .unwrap();
    assert!(!loaded(&core).await, "answered once discarded and released");
    assert_eq!((disconnects(&radio), straps_released(&radio)), (1, 1));
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(10)).await;
    assert_eq!(
        connects(&radio),
        [1, 2],
        "no silent re-acquire of the trainer"
    );
}

/// The architect's decision on #435 (ADR-0002 §5.1.3): during a started ride "Release
/// trainer" is refused, and the ride and its links are untouched.
#[tokio::test(start_paused = true)]
async fn release_trainer_during_a_started_ride_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    core.ride_start().await.unwrap();
    let refused = core
        .links_intent(release(ReleaseReason::RiderRelease))
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    sleep(S(5)).await;
    assert!(core.ride_status().await.unwrap().status.in_progress());
    assert_eq!(disconnects(&radio), 0);
    // paused by the rider, it is still started
    core.ride_pause().await.unwrap();
    assert_eq!(
        core.links_intent(release(ReleaseReason::RiderRelease))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

/// §5.1.8 "`Likely` during a ride warms nothing": with a ride loaded or active and the strap
/// not linked, `Likely` from any view starts no attempt (connect count unchanged).
#[tokio::test(start_paused = true)]
async fn likely_during_a_ride_starts_no_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let absent = SimLatency {
        not_advertising: true,
        ..latency(1, 0, 0)
    };
    let (core, radio) = latent_core(dir.path(), SimLatency::default(), absent).await;
    assert!(!core.ride_load(None).await.unwrap().hr_linked);
    for started in [false, true] {
        if started {
            core.ride_start().await.unwrap();
        }
        let before = connects(&radio);
        core.links_intent(likely(true)).await.unwrap();
        core.links_intent(LinkIntent::Retry).await.unwrap();
        core.links_intent(likely(false)).await.unwrap();
        sleep(S(60)).await;
        assert_eq!(connects(&radio), before, "started: {started}");
    }
    assert!(radio.scans.lock().unwrap().is_empty(), "no retry scan");
}

/// The scans a radio saw, as (start, end) offsets from `t0`, in whole seconds.
fn scan_spans(radio: &FakeRadio, t0: Instant) -> Vec<(u64, Option<u64>)> {
    let mut spans = Vec::new();
    for (what, at) in radio.scans.lock().unwrap().iter() {
        let s = (*at - t0).as_secs();
        match *what {
            "start" => spans.push((s, None)),
            _ => {
                if let Some(last) = spans.last_mut() {
                    last.1 = Some(s);
                }
            }
        }
    }
    spans
}

/// §5.1.8 "Scan cap": with the trainer never advertising and intent `Likely` for 10 min, the
/// scan is continuous for `SCAN_CONTINUOUS`, then on for `RESOLVE_SCAN` in every
/// `SCAN_PERIOD`; it stops on `Left`, and an advertisement in a window starts the attempt.
#[tokio::test(start_paused = true)]
async fn the_retry_scan_is_capped_and_an_advertisement_starts_the_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let absent = SimLatency {
        not_advertising: true,
        ..latency(4, 0, 0)
    };
    let (_, mut radio) = latent_core(dir.path(), absent, SimLatency::default()).await;
    radio.never_ends = true;
    let core = open(dir.path(), &radio);
    let t0 = Instant::now();
    core.links_intent(likely(false)).await.unwrap();
    for _ in 0..20 {
        sleep(S(30)).await;
        core.links_intent(likely(false)).await.unwrap();
    }
    assert_eq!(phases(&core).await[0], Some(Phase::NotFound));
    let (cont, period, window) = (
        SCAN_CONTINUOUS.as_secs(),
        SCAN_PERIOD.as_secs(),
        SCAN_WINDOW.as_secs(),
    );
    // not found after the 4 s scan of its attempt: continuous to 34 s, then windows
    let mut expect = vec![(4, Some(4 + cont))];
    let mut start = 4 + cont + period;
    while start < 600 {
        expect.push((start, Some(start + window)));
        start += period;
    }
    assert_eq!(scan_spans(&radio, t0), expect);
    assert_eq!(
        connects(&radio)[0],
        1,
        "no connect loop against an absent trainer"
    );

    // the trainer comes back: the next window sees it, and the attempt warms it up
    radio.rig.set_latency(true, SimLatency::default());
    radio
        .ads
        .lock()
        .unwrap()
        .push(found("neo", None, true, false));
    sleep(S(30)).await;
    assert_eq!(connects(&radio)[0], 2);
    assert_eq!(phases(&core).await, [Some(Phase::Warm); 2]);

    // `Left` stops a scan that runs
    radio.rig.set_latency(true, absent);
    radio.ads.lock().unwrap().clear();
    core.links_intent(LinkIntent::Retry).await.unwrap();
    sleep(S(1)).await;
    core.links_intent(release(ReleaseReason::Hidden))
        .await
        .unwrap();
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(10)).await;
    let scans = scan_spans(&radio, t0).len();
    core.links_intent(LinkIntent::Left).await.unwrap();
    sleep(SETTLE).await;
    assert!(
        scan_spans(&radio, t0).last().is_some_and(|s| s.1.is_some()),
        "stopped on Left"
    );
    sleep(S(120)).await;
    assert_eq!(scan_spans(&radio, t0).len(), scans, "no scan after Left");
}

/// §5.1.8 "Quit while warm": `Core::shutdown` with both devices warm disconnects both, writes
/// nothing else, and returns within 3 s even when a disconnect hangs.
#[tokio::test(start_paused = true)]
async fn a_quit_while_warm_releases_both_within_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    radio.disconnect_delay = S(30);
    let core = open(dir.path(), &radio);
    core.links_intent(likely(false)).await.unwrap();
    sleep(TICK).await;
    let asked = Instant::now();
    assert_eq!(core.shutdown(S(3)).await, Shutdown::Idle);
    let took = asked.elapsed();
    assert!(took <= S(3), "{took:?}");
    assert_eq!(straps_released(&radio), 1);
    sleep(S(30)).await;
    assert_eq!(disconnects(&radio), 1, "the hung disconnect was sent");
    assert_eq!(
        *radio.commands.lock().unwrap(),
        [TrainerCommand::Disconnect]
    );
    assert_eq!(phases(&core).await, [Some(Phase::Cold); 2]);
}

/// §5.1.8 "Quit while warm": with the trainer still connecting, the quit cancels its attempt
/// (the half-open peripheral is released).
#[tokio::test(start_paused = true)]
async fn a_quit_cancels_an_attempt_still_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    radio.connect_delay = S(10);
    let core = open(dir.path(), &radio);
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(1)).await;
    let asked = Instant::now();
    assert_eq!(core.shutdown(S(3)).await, Shutdown::Idle);
    assert!(asked.elapsed() <= S(3));
    assert_eq!(
        radio.half_open.load(SeqCst),
        1,
        "the quit awaits the cancelled attempt"
    );
    assert_eq!(straps_released(&radio), 1);
    sleep(S(20)).await;
    assert_eq!(connects(&radio), [1, 1], "no attempt after the quit");
}

/// §5.1.8 "Nothing is written while warm", at core level: no `TrainerCommand` reaches the
/// control channel before the handover; the first is the session's first `SetMode`.
#[tokio::test(start_paused = true)]
async fn no_command_reaches_a_warm_trainer_before_the_handover() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = warm_core(dir.path()).await;
    core.links_intent(likely(true)).await.unwrap();
    sleep(S(10 * 60)).await;
    assert!(radio.commands.lock().unwrap().is_empty());
    core.ride_load(None).await.unwrap();
    sleep(S(5)).await;
    assert!(
        radio.commands.lock().unwrap().is_empty(),
        "nothing before Start"
    );
    core.ride_start().await.unwrap();
    sleep(S(1)).await;
    let first = radio.commands.lock().unwrap().first().copied();
    assert!(
        matches!(first, Some(TrainerCommand::SetMode { .. })),
        "{first:?}"
    );
}

/// An FE-C peripheral under the real driver: it answers the page-70 request for page 54,
/// sends a page-25 frame every 250 ms while connected, and records every write.
/// Where a connected peripheral's notifications go.
type Notify = std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<(MonoTime, Vec<u8>)>>>;
struct FecPeripheral {
    clock: Arc<dyn Clock>,
    writes: std::sync::Mutex<Vec<[u8; 13]>>,
    notify: Notify,
    disconnects: std::sync::atomic::AtomicUsize,
}
fn fec_frame(page: [u8; 8]) -> Vec<u8> {
    let mut frame = vec![0xa4, 9, 0x4e, 5];
    frame.extend(page);
    frame.push(frame.iter().fold(0, |xor, b| xor ^ b));
    frame
}
impl crank_ble::FecTransport for FecPeripheral {
    fn connect(
        &self,
        _: DeviceRef,
        _: Arc<dyn Clock>,
    ) -> BoxFuture<
        'static,
        Result<(DeviceInfo, crank_ble::Notifications), crank_devices::DeviceError>,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.notify.lock().unwrap() = Some(tx.clone());
        let clock = self.clock.clone();
        tokio::spawn(async move {
            for n in 0u8.. {
                clock.sleep_until(MonoTime(clock.now().0 + 250_000)).await;
                let page25 = fec_frame([25, n, 90, 0, 0, 150, 0, 0]);
                if tx.send((clock.now(), page25)).is_err() {
                    return;
                }
            }
        });
        let notifications: crank_ble::Notifications = tokio_stream_of(rx).map(Ok).boxed();
        async { Ok((DeviceInfo::default(), notifications)) }.boxed()
    }
    fn write(&self, bytes: [u8; 13]) -> BoxFuture<'static, Result<(), crank_devices::DeviceError>> {
        self.writes.lock().unwrap().push(bytes);
        if bytes[4] == 70
            && bytes[10] == 54
            && let Some(tx) = self.notify.lock().unwrap().as_ref()
        {
            let _ = tx.send((
                self.clock.now(),
                fec_frame([54, 255, 255, 255, 255, 200, 0, 7]),
            ));
        }
        async { Ok(()) }.boxed()
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), crank_devices::DeviceError>> {
        self.disconnects.fetch_add(1, SeqCst);
        self.notify.lock().unwrap().take();
        async { Ok(()) }.boxed()
    }
}
/// A stream over an unbounded receiver.
fn tokio_stream_of<T: Send + 'static>(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<T>,
) -> BoxStream<'static, T> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)).boxed()
}

/// §5.1.8 "Nothing is written while warm" and "Handover" (a full event channel never faults
/// a warm link): the real FE-C driver over a recording transport is held warm for 10 min of
/// virtual time at 4 notifications a second, more than its 256-event channel. Its write log
/// holds only the connect's page 55 and the page-70 request for page 54, the link is never
/// faulted or dropped, and the first control page is the session's first `SetMode`.
#[tokio::test(start_paused = true)]
async fn a_warm_fec_trainer_is_written_nothing_and_never_faults() {
    let dir = tempfile::tempdir().unwrap();
    let mut radio = fake(vec![], false);
    let clock: Arc<dyn Clock> = Arc::new(crank_engine::VirtualClock::new(1));
    let peripheral = Arc::new(FecPeripheral {
        clock,
        writes: std::sync::Mutex::default(),
        notify: std::sync::Mutex::default(),
        disconnects: std::sync::atomic::AtomicUsize::default(),
    });
    radio.fec = Some(peripheral.clone());
    let core = open(dir.path(), &radio);
    core.device_pair("neo".into(), DeviceRole::Trainer)
        .await
        .unwrap();
    let paired = peripheral.writes.lock().unwrap().len();
    core.links_intent(likely(true)).await.unwrap();
    sleep(S(10 * 60)).await;
    assert_eq!(phases(&core).await[0], Some(Phase::Warm), "never faulted");
    assert_eq!(peripheral.disconnects.load(SeqCst), 1, "only the pairing's");
    let pages = |from: usize| -> Vec<(u8, u8)> {
        peripheral.writes.lock().unwrap()[from..]
            .iter()
            .map(|w| (w[4], w[10]))
            .collect()
    };
    let warm = pages(paired);
    assert_eq!(warm.len(), 2, "{warm:?}");
    assert_eq!(warm[0].0, 55, "page 55 at connect");
    assert_eq!(warm[1], (70, 54), "the page-70 request for page 54");
    let written = peripheral.writes.lock().unwrap().len();
    core.ride_load(None).await.unwrap();
    core.ride_start().await.unwrap();
    sleep(S(1)).await;
    let control = pages(written);
    assert!(
        control.first().is_some_and(|p| (48..=51).contains(&p.0)),
        "the first write after the handover is a control page: {control:?}"
    );
}

/// §5.1.8 "Handover": the engine's following machine activates on the handed-over `Ready`
/// link without a new `Link` event (it sends its mode as soon as the ride starts), a link
/// lost while warm makes the load connect cold, and a ride's end leaves the phase `cold`.
#[tokio::test(start_paused = true)]
async fn the_handover_activates_on_ready_a_lost_link_connects_cold_and_an_end_leaves_cold() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut radio) =
        latent_core(dir.path(), SimLatency::default(), SimLatency::default()).await;
    let cfg = crank_devices::sim::SimConfig {
        faults: vec![SimFault::Dropout {
            at: S(10),
            down: S(1),
        }],
        ..crank_devices::sim::SimConfig::default()
    };
    radio.rig = crank_devices::sim::SimRig::new(cfg, Arc::new(crank_engine::VirtualClock::new(1)));
    let core = open(dir.path(), &radio);
    core.links_intent(likely(true)).await.unwrap();
    sleep(S(12)).await;
    assert_eq!(
        phases(&core).await[0],
        Some(Phase::NotFound),
        "lost while warm"
    );
    assert_eq!(disconnects(&radio), 1, "and released");
    core.ride_load(None).await.unwrap();
    assert_eq!(connects(&radio)[0], 2, "the load connects it cold");
    core.ride_start().await.unwrap();
    sleep(S(1)).await;
    assert!(
        radio
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, TrainerCommand::SetMode { .. })),
        "the engine took the link as Ready"
    );
    core.ride_end().await.unwrap();
    assert_eq!(phases(&core).await, [Some(Phase::Cold); 2]);
}

/// Load over warm links with the engine stalled, so that the links stay handed out (`Handed`)
/// until the stall is released: the load's `Load` waits for the engine.
async fn handed_core(
    dir: &std::path::Path,
) -> (
    Core,
    FakeRadio,
    tokio::task::JoinHandle<Result<RideLoaded, ApiError>>,
) {
    let (core, radio) = warm_core(dir).await;
    super::tests::stall_engine(&core);
    let load = {
        let core = core.clone();
        tokio::spawn(async move { core.ride_load(None).await })
    };
    sleep(Duration::from_millis(100)).await;
    assert_eq!(phases(&core).await, [Some(Phase::Riding); 2], "handed");
    (core, radio, load)
}

/// B1, probe C1: `Sleep` or "Release trainer" while the links are handed to a load whose
/// engine is stalled is recorded, answered only once applied: when the stall is released the
/// ride loads and is discarded at once, the trainer is released, and "Release trainer" latches.
#[tokio::test(start_paused = true)]
async fn a_release_while_handed_is_applied_once_the_load_resolves() {
    for reason in [ReleaseReason::Sleep, ReleaseReason::RiderRelease] {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio, load) = handed_core(dir.path()).await;
        let intent = {
            let core = core.clone();
            tokio::spawn(async move { core.links_intent(release(reason)).await })
        };
        sleep(Duration::from_millis(400)).await;
        assert!(
            !intent.is_finished(),
            "{reason:?}: not answered before it is applied"
        );
        core.0.stall.release();
        assert!(
            load.await.unwrap().is_ok(),
            "{reason:?}: the load went through"
        );
        intent.await.unwrap().unwrap();
        assert!(!loaded(&core).await, "{reason:?}: discarded");
        assert_eq!(disconnects(&radio), 1, "{reason:?}: the trainer released");
        core.links_intent(likely(false)).await.unwrap();
        sleep(S(5)).await;
        let trainer = if reason == ReleaseReason::RiderRelease {
            1
        } else {
            2
        };
        assert_eq!(connects(&radio)[0], trainer, "{reason:?}: latched or not");
    }
}

/// N1 (kills M9): a discard of an unstarted ride is refused once `ride_start` has marked the
/// ride starting, though the engine has not taken the Start yet (its status still idle).
#[tokio::test(start_paused = true)]
async fn a_discard_leaves_a_ride_whose_start_was_sent() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _radio) = loaded_core(dir.path()).await;
    let ride_id = loaded_id(&core).await;
    super::tests::stall_engine(&core);
    let start = {
        let core = core.clone();
        tokio::spawn(async move { core.ride_start().await })
    };
    sleep(Duration::from_millis(10)).await;
    assert!(
        !core.ride_status().await.unwrap().status.in_progress(),
        "the engine has not taken the Start"
    );
    let refused = core.end_ride(true, Some(&ride_id)).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    core.0.stall.release();
    let _ = start.await.unwrap();
    assert!(loaded(&core).await, "the ride was not discarded");
}

/// N2: "Release trainer" that reaches the keeper while a Start races it (past the core's own
/// check) answers `Conflict`, not `Ok`, and does not latch the trainer.
#[tokio::test(start_paused = true)]
async fn a_rider_release_racing_a_start_is_refused_and_does_not_latch() {
    let dir = tempfile::tempdir().unwrap();
    let (core, radio) = loaded_core(dir.path()).await;
    super::tests::stall_engine(&core);
    let start = {
        let core = core.clone();
        tokio::spawn(async move { core.ride_start().await })
    };
    sleep(Duration::from_millis(10)).await;
    // straight to the keeper, as if the core's check had passed just before the Start
    let refused = core
        .keeper()
        .unwrap()
        .intent(release(ReleaseReason::RiderRelease))
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    core.0.stall.release();
    start.await.unwrap().unwrap();
    core.ride_end().await.unwrap();
    core.links_intent(likely(false)).await.unwrap();
    sleep(S(5)).await;
    assert_eq!(
        connects(&radio)[0],
        2,
        "not latched: the trainer warms up again"
    );
}
async fn loaded_id(core: &Core) -> String {
    core.0
        .state
        .lock()
        .await
        .session
        .as_ref()
        .map(|s| s.ride_id.clone())
        .unwrap()
}

/// X2 at core level, both interleavings: "Release trainer" recorded while the links are handed
/// to a load whose engine never takes it; that load fails (its links released), and the rider
/// asks for the ride again. Whether the keeper sees the release of the failed load before or
/// after the new load (held back with `hold_internal`), the new load connects the trainer
/// again and gets the ride; the release was applied (released first) or superseded (load
/// first).
#[tokio::test(start_paused = true)]
async fn a_load_after_a_recorded_rider_release_gets_the_ride_in_either_order() {
    for released_first in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio, first) = handed_core(dir.path()).await;
        let intent = {
            let core = core.clone();
            tokio::spawn(async move {
                core.links_intent(release(ReleaseReason::RiderRelease))
                    .await
            })
        };
        sleep(Duration::from_millis(10)).await;
        let keeper = core.keeper().unwrap();
        if !released_first {
            keeper.hold_internal(true).await;
        }
        // the stalled engine never takes `Load`: the load fails at its bound
        let failed = first.await.unwrap().unwrap_err();
        assert_eq!(failed.code, ErrorCode::EngineUnresponsive);
        core.0.stall.release();
        if released_first {
            for _ in 0..50 {
                if phases(&core).await[0] != Some(Phase::Riding) {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(phases(&core).await[0], Some(Phase::Cold), "released first");
        } else {
            assert_eq!(
                phases(&core).await[0],
                Some(Phase::Riding),
                "the release is held back"
            );
        }
        let second = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        sleep(Duration::from_millis(10)).await;
        if !released_first {
            keeper.hold_internal(false).await;
        }
        let loaded = second.await.unwrap();
        assert!(
            loaded.is_ok(),
            "released first: {released_first}: {loaded:?}"
        );
        assert_eq!(connects(&radio)[0], 2, "the trainer connected again");
        let answered = intent.await.unwrap();
        if released_first {
            answered.unwrap();
        } else {
            assert_eq!(
                answered.unwrap_err().code,
                ErrorCode::Conflict,
                "superseded"
            );
        }
    }
}
