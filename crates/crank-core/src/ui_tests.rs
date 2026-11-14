//! The snapshot channel (ADR-0001 §4.3) against a scripted ride: the test appends points and
//! writes the status under the store's lock as the engine does, and drives the keeper's state,
//! the loads and the acks. Each test names the §4.3.9 replacement it is.
use super::*;
use crate::dto::{DeviceLinkView, DeviceRole, LinkSample, LinkView};
use crank_engine::{
    Engine, PauseReason, SessionTicks, StreamCap, StreamCommanded, StreamCompliance,
    StreamComponent, StreamFollowing, StreamPauseReason, StreamRideState, VirtualClock,
};
use proptest::prelude::*;
use tokio::{
    sync::mpsc,
    time::{Instant, sleep},
};

const MS: fn(u64) -> Duration = Duration::from_millis;
/// The design's least frame spacing (§4.3.3: at most 10 Hz), not the constant under test.
const DESIGN_SPACING: Duration = Duration::from_millis(100);
/// The whole-frame bound of §4.3.3.
const FRAME_BYTES: usize = 32 * 1024;

fn idle() -> RideStatus {
    let clock = Arc::new(VirtualClock::new(1));
    Engine::new(clock, None, None, Arc::new(|_: bool| {}))
        .1
        .status()
}

/// What a frame was sent with: the frame, the snapshot at its send, and when.
type Sent = (UiFrame, UiState, Instant);
struct Rig {
    ui: Arc<Ui>,
    keeper: watch::Sender<LinkFrame>,
    status: watch::Sender<RideStatus>,
    store: SeriesStore,
    rides: u32,
}
fn rig() -> Rig {
    let keeper = watch::Sender::new(LinkFrame::default());
    Rig {
        ui: Arc::new(Ui::new(
            keeper.subscribe(),
            Arc::new(VirtualClock::new(1)),
            Duration::from_secs(1),
        )),
        keeper,
        status: watch::Sender::new(idle()),
        store: SeriesStore::default(),
        rides: 0,
    }
}
impl Rig {
    /// A load installs a new ride: its own store and status.
    fn install(&mut self) {
        self.rides += 1;
        self.store = SeriesStore::default();
        self.status = watch::Sender::new(idle());
        self.ui.install(RideSlot::new(
            format!("ride-{}", self.rides),
            None,
            self.status.subscribe(),
            self.store.clone(),
        ));
    }
    /// One riding tick as the engine publishes it: the points, the live values and the status,
    /// under the store's lock.
    fn tick(&self) {
        let mut guard = self.store.lock().unwrap();
        let s = &mut *guard;
        let k = self.status.borrow().session.0 + 1;
        #[expect(clippy::cast_precision_loss, reason = "small tick counts")]
        let t = k as f64 / 4.0;
        let power = u16::try_from(k % 400).unwrap();
        s.power.t.push(t);
        s.power.w.push(t);
        s.power.v.push(power);
        if k.is_multiple_of(2) {
            for track in [&mut s.cadence, &mut s.hr] {
                track.t.push(t);
                track.w.push(t);
                track.v.push(90);
            }
        }
        s.live = crank_engine::Live {
            power_w: Some(power),
            cadence_rpm: Some(90),
            hr_bpm: Some(90),
        };
        self.status.send_modify(|st| {
            st.state = RideState::Riding;
            st.loaded = true;
            st.session = SessionTicks(k);
        });
    }
    fn open(&self, window: &str) -> (UiChannelId, mpsc::UnboundedReceiver<Sent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let ui = Arc::downgrade(&self.ui);
        let id = self
            .ui
            .open(Some(window.into()), move |f: UiFrame| {
                let snapshot = ui.upgrade().map(|ui| ui.snapshot());
                let snapshot = snapshot.ok_or(SinkClosed)?;
                tx.send((f, snapshot, Instant::now()))
                    .map_err(|_| SinkClosed)
            })
            .unwrap();
        (id, rx)
    }
    fn series(&self) -> [Track; 3] {
        let s = self.store.lock().unwrap();
        [s.power.clone(), s.cadence.clone(), s.hr.clone()]
    }
}

/// The page under the client rule (§4.3.2): it replaces the state, replaces a series on
/// `from: 0` and appends otherwise, and checks that `from` is always its own length.
#[derive(Default)]
struct Page {
    series: [Track; 3],
    state: Option<UiState>,
}
impl Page {
    fn apply(&mut self, f: &UiFrame) {
        let got = [&f.series.power, &f.series.cadence, &f.series.hr];
        for (have, got) in self.series.iter_mut().zip(got) {
            if got.from == 0 {
                *have = Track::default();
            }
            assert_eq!(got.from as usize, have.len(), "from is the page's length");
            assert!(
                got.v.len() <= CHUNK && got.t.len() == got.v.len() && got.w.len() == got.v.len()
            );
            have.t.extend(&got.t);
            have.w.extend(&got.w);
            have.v.extend(&got.v);
        }
        self.state = Some(UiState {
            ride: f.ride.clone(),
            loading: f.loading,
            devices: f.devices.clone(),
        });
    }
}
fn bytes(f: &UiFrame) -> usize {
    serde_json::to_vec(f).unwrap().len()
}
/// Let the channel task run, then take what it sent.
async fn take(rx: &mut mpsc::UnboundedReceiver<Sent>) -> Vec<Sent> {
    sleep(MS(1)).await;
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}
/// Ack every frame as it arrives until nothing more is due, applying each to `page`.
async fn drain(
    rig: &Rig,
    window: &str,
    id: UiChannelId,
    rx: &mut mpsc::UnboundedReceiver<Sent>,
    page: &mut Page,
) -> Vec<UiFrame> {
    let mut frames = Vec::new();
    loop {
        sleep(MS(150)).await;
        let got = take(rx).await;
        let Some((last, ..)) = got.last() else {
            return frames;
        };
        let seq = last.seq;
        for (f, ..) in got {
            page.apply(&f);
            frames.push(f);
        }
        rig.ui.ack(Some(window), id, seq).unwrap();
    }
}

/// §4.3.9: the first frame is the snapshot at the open, sent at once.
#[tokio::test(start_paused = true)]
async fn the_first_frame_is_the_snapshot_at_the_open_sent_at_once() {
    let mut rig = rig();
    rig.install();
    rig.tick();
    let (_, mut rx) = rig.open("main");
    let got = take(&mut rx).await;
    assert_eq!(got.len(), 1);
    let (f, snapshot, _) = &got[0];
    assert_eq!(
        (f.seq, f.series.power.from, f.series.power.v.len()),
        (0, 0, 1)
    );
    assert_eq!(f.ride, snapshot.ride);
    assert_eq!(f.ride.as_ref().unwrap().live.power_w, Some(1));
}

/// §4.3.9 "a channel opened mid-ride backfills the series from index 0, contiguously" (fails
/// on `dev`: there a new page's trace starts empty).
#[tokio::test(start_paused = true)]
async fn a_channel_opened_mid_ride_backfills_from_index_0_contiguously() {
    let mut rig = rig();
    rig.install();
    for _ in 0..1_000 {
        rig.tick();
    }
    let (id, mut rx) = rig.open("main");
    let mut page = Page::default();
    let frames = drain(&rig, "main", id, &mut rx, &mut page).await;
    let froms: Vec<u32> = frames.iter().map(|f| f.series.power.from).collect();
    assert_eq!(froms[..4], [0, 256, 512, 768]);
    assert_eq!(
        page.series,
        rig.series(),
        "the whole series, no hole, no duplicate"
    );
    assert!(frames.iter().all(|f| bytes(f) <= FRAME_BYTES));
}

/// §4.3.9 "the page converges after a stall": the page stalls 60 s mid-ride, holding one
/// frame; its ack brings the state now and series that start exactly where its own ended
/// (fails on `dev`: a hole in the trace, #557 item 4).
#[tokio::test(start_paused = true)]
async fn the_page_converges_after_a_sixty_second_stall() {
    let mut rig = rig();
    rig.install();
    for _ in 0..40 {
        rig.tick();
    }
    let (id, mut rx) = rig.open("main");
    let mut page = Page::default();
    drain(&rig, "main", id, &mut rx, &mut page).await;
    // the stall: the ride goes on, the page acks nothing
    let mut held = Vec::new();
    for _ in 0..240 {
        rig.tick();
        sleep(MS(250)).await;
        held.extend(take(&mut rx).await);
    }
    assert_eq!(
        held.len(),
        1,
        "one frame in flight, nothing past the window"
    );
    let (stalled, ..) = held.remove(0);
    page.apply(&stalled);
    rig.ui.ack(Some("main"), id, stalled.seq).unwrap();
    let frames = drain(&rig, "main", id, &mut rx, &mut page).await;
    assert_eq!(
        frames[0].series.power.from as usize,
        stalled.series.power.from as usize + stalled.series.power.v.len(),
        "the next frame starts where the page's series ended"
    );
    assert_eq!(page.series, rig.series());
    let status = rig.status.borrow().clone();
    let shown = &page.state.as_ref().unwrap().ride.as_ref().unwrap().status;
    assert_eq!(shown, &StreamStatus::from(&status), "the state now");
}

/// §4.3.9 "the overlay": a riding device shows the ride's link, a lost trainer shows `lost` in
/// the same frame as `trainer_lost_since_s`, an unlinked strap keeps its own phase; before the
/// load installs its ride, a riding device is `connecting`.
#[tokio::test(start_paused = true)]
async fn a_lost_trainer_shows_riding_and_lost_in_the_frame_of_its_loss() {
    let mut rig = rig();
    let device = |role| PairedDevice {
        role,
        id: "id".into(),
        name: None,
        serial: None,
        model: None,
    };
    rig.ui.set_paired([
        Some(device(DeviceRole::Trainer)),
        Some(device(DeviceRole::HeartRate)),
    ]);
    let phase = |phase| {
        Some(DeviceLinkView {
            phase,
            sample: None,
        })
    };
    rig.keeper.send_replace(LinkFrame {
        seq: 1,
        links: LinkView {
            trainer: phase(LinkPhase::Riding),
            strap: phase(LinkPhase::NotFound),
        },
    });
    rig.ui.load_started();
    let loading = rig.ui.snapshot();
    assert!(loading.loading && loading.ride.is_none());
    assert_eq!(loading.devices.trainer.link, Some(LinkStatus::Connecting));
    rig.install();
    rig.tick();
    rig.status
        .send_modify(|st| st.trainer_link = Some(LinkState::Ready));
    let (id, mut rx) = rig.open("main");
    let first = take(&mut rx).await.remove(0).0;
    assert_eq!(first.devices.trainer.link, Some(LinkStatus::Ready));
    rig.ui.ack(Some("main"), id, first.seq).unwrap();
    // the engine publishes the loss and its pause in one status
    rig.status.send_modify(|st| {
        st.state = RideState::Paused(PauseReason::TrainerLost);
        st.trainer_link = Some(LinkState::Lost);
        st.trainer_lost_since = Some(SessionTicks(9));
    });
    sleep(MS(150)).await;
    let f = take(&mut rx).await.remove(0).0;
    let status = &f.ride.as_ref().unwrap().status;
    assert_eq!(status.trainer_lost_since_s, Some(2.25));
    assert_eq!(
        (f.devices.trainer.phase, f.devices.trainer.link),
        (Some(LinkPhase::Riding), Some(LinkStatus::Lost))
    );
    assert_eq!(
        (f.devices.strap.phase, f.devices.strap.link),
        (Some(LinkPhase::NotFound), None),
        "the unlinked strap is the keeper's"
    );
}

/// §4.3.9 "lifecycle by id" (§4.3.5): a failed send, a page-load event and a reopen each close
/// exactly their channels; a page-load event closes only the ids the window holds at the event,
/// never one opened after it; at most 16 channels.
#[tokio::test(start_paused = true)]
async fn lifecycle_by_channel_id() {
    let rig = rig();
    let (old, _old_rx) = rig.open("main");
    let (other, _other_rx) = rig.open("other");
    // a reopen closes the window's older channel only
    let (new, _new_rx) = rig.open("main");
    let gone = rig.ui.ack(Some("main"), old, 0).unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
    rig.ui.ack(Some("other"), other, 0).unwrap();
    // another window's channel is not this window's
    let foreign = rig.ui.ack(Some("main"), other, 0).unwrap_err();
    assert_eq!(foreign.code, ErrorCode::NotFound);
    // the page-load event closes what the window holds now; the next page's channel lives
    rig.ui.close_window("main");
    let (next, mut next_rx) = rig.open("main");
    assert_eq!(
        rig.ui.ack(Some("main"), new, 0).unwrap_err().code,
        ErrorCode::NotFound
    );
    let f = take(&mut next_rx).await.remove(0).0;
    rig.ui.ack(Some("main"), next, f.seq).unwrap();
    sleep(HEARTBEAT).await;
    assert_eq!(
        take(&mut next_rx).await.len(),
        1,
        "alive: the heartbeat after the ack"
    );
    // a failed send closes the channel
    let failing = rig
        .ui
        .open(Some("dead".into()), |_| Err(SinkClosed))
        .unwrap();
    sleep(MS(1)).await;
    assert_eq!(
        rig.ui.ack(Some("dead"), failing, 0).unwrap_err().code,
        ErrorCode::NotFound
    );
    // 16 per process: a 17th window is refused, a reopen of an open window is not
    let mut rxs = Vec::new();
    for n in 0..14 {
        rxs.push(rig.open(&format!("w{n}")));
    }
    let refused = rig.ui.open(Some("w99".into()), |_| Ok(())).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    rig.open("w0");
    let ids: Vec<u32> = rxs.iter().map(|(id, _)| id.0).collect();
    assert!(ids.windows(2).all(|w| w[1] > w[0]), "ids are never reused");
}

/// §4.3.9 rate: a page that acks at once gets frames at least 100 ms apart during a ride, and
/// an idle core sends one frame a second, no more.
#[tokio::test(start_paused = true)]
async fn frames_are_spaced_and_an_idle_core_beats_once_a_second() {
    let mut rig = rig();
    rig.install();
    let (id, mut rx) = rig.open("main");
    let mut at = Vec::new();
    for _ in 0..40 {
        rig.tick();
        for _ in 0..250 {
            for (f, _, when) in take(&mut rx).await {
                rig.ui.ack(Some("main"), id, f.seq).unwrap();
                at.push(when);
            }
        }
    }
    assert!(at.windows(2).all(|w| w[1] - w[0] >= DESIGN_SPACING));
    assert!(at.len() >= 38, "{} frames for 40 ticks", at.len());
    // idle: the ride is over, nothing changes
    rig.status.send_modify(|st| st.state = RideState::Finished);
    sleep(MS(200)).await;
    for (f, ..) in take(&mut rx).await {
        rig.ui.ack(Some("main"), id, f.seq).unwrap();
    }
    let mut beats = 0;
    for _ in 0..100 {
        sleep(MS(100)).await;
        for (f, ..) in take(&mut rx).await {
            assert_eq!(f.ride.as_ref().unwrap().ended, Some(RideEnding::Finished));
            rig.ui.ack(Some("main"), id, f.seq).unwrap();
            beats += 1;
        }
    }
    assert!((9..=11).contains(&beats), "{beats} frames in 10 s idle");
}

/// §4.3.9 "the frame-bound test" (§4.3.3): with every part at its cap the state is ≤ 4 000 B
/// and the frame ≤ 32 KB; the channel never puts more than 256 points of a series in a frame.
/// The state with every part at its cap (§4.3.3).
fn worst_state() -> UiState {
    let widest = "𝔘".repeat(1_000);
    let device = |role| {
        cut(PairedDevice {
            role,
            id: widest.clone(),
            name: Some(widest.clone()),
            serial: Some(widest.clone()),
            model: Some(widest.clone()),
        })
    };
    let view = |role, sample| DeviceView {
        paired: Some(device(role)),
        phase: Some(LinkPhase::NotFound),
        link: Some(LinkStatus::Connecting),
        sample: Some(sample),
    };
    let status = StreamStatus {
        state: StreamRideState::Paused {
            reason: StreamPauseReason::StoppedPedalling,
        },
        loaded: true,
        session_s: f64::MIN,
        workout_s: f64::MIN,
        block_index: Some(u32::MAX),
        block_elapsed_s: f64::MIN,
        block_start_session_s: Some(f64::MIN),
        block_start_workout_s: Some(f64::MIN),
        laps_closed: u32::MAX,
        late_samples: u32::MAX,
        record_error: Some("𝔘".repeat(64)),
        trainer_following: StreamFollowing::NotFollowing,
        not_following_component: Some(StreamComponent::Resistance),
        erg_released: true,
        compliance: StreamCompliance {
            counted_s: u32::MAX,
            within_s: u32::MAX,
            excluded_s: u32::MAX,
        },
        cap: Some(StreamCap {
            cap_w: u16::MAX,
            erg_only: true,
        }),
        user_paused: true,
        trainer_lost_since_s: Some(f64::MIN),
        reconnect_attempt: Some(u32::MAX),
        soft_start_target_w: Some(u16::MAX),
        last_gap_s: Some(f64::MIN),
        reconnected_at_s: Some(f64::MIN),
        commanded: Some(StreamCommanded::Resistance { pct: f32::MIN }),
    };
    UiState {
        ride: Some(RideView {
            id: format!("ride-{:x}", u128::MAX),
            workout_id: Some(widest[..64].to_owned()),
            status,
            live: LiveValues {
                power_w: Some(u16::MAX),
                cadence_rpm: Some(u16::MAX),
                hr_bpm: Some(u16::MAX),
            },
            ended: Some(RideEnding::Finished),
            engine_unresponsive: true,
        }),
        loading: true,
        devices: DevicesView {
            trainer: view(
                DeviceRole::Trainer,
                LinkSample::Trainer {
                    power_w: Some(u16::MAX),
                    cadence_rpm: Some(u8::MAX),
                },
            ),
            strap: view(
                DeviceRole::HeartRate,
                LinkSample::HeartRate { bpm: Some(u8::MAX) },
            ),
        },
    }
}

#[tokio::test(start_paused = true)]
async fn every_frame_is_within_the_bound_at_every_cap() {
    let state = worst_state();
    let state_bytes = serde_json::to_vec(&state).unwrap().len();
    assert!(state_bytes <= 4_000, "state {state_bytes} B");
    let points = Points {
        from: u32::MAX,
        t: vec![999_999.75; CHUNK],
        w: vec![999_999.75; CHUNK],
        v: vec![u16::MAX; CHUNK],
    };
    let frame = UiFrame {
        channel: UiChannelId(u32::MAX),
        seq: u32::MAX,
        ride: state.ride,
        loading: true,
        devices: state.devices,
        series: SeriesDelta {
            power: points.clone(),
            cadence: points.clone(),
            hr: points,
        },
    };
    let n = bytes(&frame);
    eprintln!("state {state_bytes} B, frame {n} B");
    assert!(n <= FRAME_BYTES, "frame {n} B");

    // the channel's own frames: a long backlog of every series goes 256 points at a time
    let mut rig = rig();
    rig.install();
    for _ in 0..2_000 {
        rig.tick();
    }
    let (id, mut rx) = rig.open("main");
    let frames = drain(&rig, "main", id, &mut rx, &mut Page::default()).await;
    for f in &frames {
        let s = &f.series;
        assert!(s.power.v.len() <= CHUNK && s.cadence.v.len() <= CHUNK && s.hr.v.len() <= CHUNK);
        assert!(bytes(f) <= FRAME_BYTES);
    }
    assert_eq!(frames[0].series.power.v.len(), CHUNK);
}

/// §4.3.5 "engine unresponsive", the silent-status path (#351): a ride in progress whose
/// status watch goes unwritten for 1 s on the core clock is shown `engine_unresponsive`. The
/// flag stays when the status comes back (it stays until the ride ends) and is not shown once
/// the ride has ended.
#[tokio::test(start_paused = true)]
async fn a_ride_whose_status_goes_silent_for_a_second_is_unresponsive_until_it_ends() {
    let mut rig = rig();
    rig.install();
    rig.tick();
    let flag = |rig: &Rig| rig.ui.snapshot().ride.unwrap().engine_unresponsive;
    assert!(!flag(&rig));
    sleep(MS(999)).await;
    assert!(!flag(&rig), "silent for less than 1 s");
    sleep(MS(1)).await;
    assert!(flag(&rig), "silent for 1 s");
    rig.tick();
    assert!(flag(&rig), "kept when the status comes back");
    rig.status.send_modify(|st| st.state = RideState::Finished);
    assert!(!flag(&rig), "an ended ride is not shown unresponsive");
    // a ride that keeps writing its status is never flagged, however long it runs
    rig.install();
    for _ in 0..40 {
        rig.tick();
        sleep(MS(250)).await;
        assert!(!flag(&rig));
    }
    // a ride not in progress (loaded, unstarted) is not timed: it does not tick
    rig.install();
    sleep(Duration::from_secs(5)).await;
    assert!(!flag(&rig));
}

/// §4.3.3: each `PairedDevice` string is cut to 64 bytes on a character boundary when the
/// pairings are set.
#[test]
fn paired_device_strings_are_cut_to_64_bytes_on_a_character_boundary() {
    let rig = rig();
    let long = "é".repeat(100);
    rig.ui.set_paired([
        Some(PairedDevice {
            role: DeviceRole::Trainer,
            id: "x".repeat(65),
            name: Some(long.clone()),
            serial: Some(long.clone()),
            model: Some(format!("a{long}")),
        }),
        None,
    ]);
    let paired = rig.ui.snapshot().devices.trainer.paired.unwrap();
    assert_eq!(paired.id.len(), 64);
    assert_eq!(paired.name.as_deref(), Some("é".repeat(32).as_str()));
    assert_eq!(paired.serial, paired.name);
    assert_eq!(
        paired.model.unwrap().len(),
        63,
        "cut back to the character boundary"
    );
    assert_eq!(rig.ui.snapshot().devices.strap.paired, None);
}

/// One step of the property test's script.
#[derive(Clone, Debug)]
enum Op {
    /// Ticks of the ride, 250 ms apart.
    Ticks(u8),
    /// The keeper's state changes.
    Keeper,
    /// A load starts (the series dropped, every cursor back to 0) …
    Load,
    /// … and installs its ride.
    Install,
    /// The ride finishes.
    End,
    /// An unstarted ride is unloaded.
    Unload,
    /// The page acks the frame it holds.
    Ack,
    /// The page acks an older `seq`.
    Stale,
    /// Another window acks this channel.
    Foreign,
    /// Nothing for a while: no ack, no tick.
    Wait(u16),
    /// The page opens a new channel (a remount or the watchdog), mid-backfill or not.
    Reopen,
}
fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (1_u8..60).prop_map(Op::Ticks),
        1 => Just(Op::Keeper),
        1 => Just(Op::Load),
        1 => Just(Op::Install),
        1 => Just(Op::End),
        1 => Just(Op::Unload),
        4 => Just(Op::Ack),
        1 => Just(Op::Stale),
        1 => Just(Op::Foreign),
        1 => (0_u16..3_000).prop_map(Op::Wait),
        1 => Just(Op::Reopen),
    ]
}
/// The page's side of the property test.
struct Client {
    id: UiChannelId,
    rx: mpsc::UnboundedReceiver<Sent>,
    page: Page,
    held: Option<u32>,
    last: Option<Instant>,
}
impl Client {
    fn new(rig: &Rig) -> Self {
        let (id, rx) = rig.open("main");
        Self {
            id,
            rx,
            page: Page::default(),
            held: None,
            last: None,
        }
    }
    /// Take the frames sent, checking each against the properties.
    async fn receive(&mut self) {
        for (f, snapshot, at) in take(&mut self.rx).await {
            assert!(
                self.held.is_none(),
                "a second frame while {:?} is unacked",
                self.held
            );
            assert_eq!(f.channel, self.id);
            let state = UiState {
                ride: f.ride.clone(),
                loading: f.loading,
                devices: f.devices.clone(),
            };
            assert_eq!(
                state, snapshot,
                "the frame's state is the snapshot at its send"
            );
            if let Some(last) = self.last {
                assert!(at - last >= DESIGN_SPACING, "frames {:?} apart", at - last);
            }
            assert!(bytes(&f) <= FRAME_BYTES);
            self.page.apply(&f);
            self.held = Some(f.seq);
            self.last = Some(at);
        }
    }
}
/// Apply one step of the script.
async fn step(rig: &mut Rig, ride: &mut bool, client: &mut Client, op: Op) {
    match op {
        Op::Ticks(n) if *ride && rig.status.borrow().state != RideState::Finished => {
            for _ in 0..n {
                rig.tick();
                sleep(MS(250)).await;
                client.receive().await;
            }
        }
        Op::Ticks(_) => {}
        Op::Keeper => {
            rig.keeper.send_modify(|f| {
                f.seq += 1;
                f.links.strap = Some(DeviceLinkView {
                    phase: LinkPhase::Warm,
                    sample: Some(LinkSample::HeartRate {
                        bpm: Some(u8::try_from(f.seq % 200).unwrap()),
                    }),
                });
            });
        }
        Op::Load => {
            rig.ui.load_started();
            *ride = false;
        }
        Op::Install => {
            rig.install();
            rig.ui.load_over();
            *ride = true;
        }
        Op::End => {
            let _series = rig.store.lock().unwrap();
            rig.status.send_modify(|st| st.state = RideState::Finished);
        }
        Op::Unload => {
            rig.ui.unload(&format!("ride-{}", rig.rides));
            *ride = false;
        }
        Op::Ack => {
            if let Some(seq) = client.held.take() {
                rig.ui.ack(Some("main"), client.id, seq).unwrap();
            }
        }
        Op::Stale => {
            let seq = client.held.map_or(0, |s| s.wrapping_sub(1));
            rig.ui.ack(Some("main"), client.id, seq).unwrap();
        }
        Op::Foreign => {
            let seq = client.held.unwrap_or(0);
            let e = rig.ui.ack(Some("other"), client.id, seq).unwrap_err();
            assert_eq!(e.code, ErrorCode::NotFound);
        }
        Op::Wait(ms) => sleep(MS(u64::from(ms))).await,
        Op::Reopen => {
            let old = client.id;
            *client = Client::new(rig);
            let e = rig.ui.ack(Some("main"), old, 0).unwrap_err();
            assert_eq!(e.code, ErrorCode::NotFound);
        }
    }
}
async fn script(ops: Vec<Op>) {
    let mut rig = rig();
    let mut ride = false;
    let mut client = Client::new(&rig);
    for op in ops {
        step(&mut rig, &mut ride, &mut client, op).await;
        client.receive().await;
    }
    // the page acks again: it converges on the core's series and state
    let expected = |rig: &Rig, ride: bool| {
        if ride {
            rig.series()
        } else {
            Default::default()
        }
    };
    for _ in 0..400 {
        if let Some(seq) = client.held.take() {
            rig.ui.ack(Some("main"), client.id, seq).unwrap();
        }
        sleep(MS(150)).await;
        client.receive().await;
        if client.page.series == expected(&rig, ride)
            && client.page.state == Some(rig.ui.snapshot())
        {
            break;
        }
    }
    assert_eq!(
        client.page.series,
        expected(&rig, ride),
        "no hole, no duplicate"
    );
    assert_eq!(client.page.state, Some(rig.ui.snapshot()));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// §4.3.9 "one property test of the `ui` channel" over arbitrary interleavings of ticks,
    /// keeper changes, loads, ends and acks (right, stale, foreign, none for a long time), and
    /// reopens mid-backfill: at most one frame unacked; each frame's state is the snapshot at
    /// its send (so `ride.live` is the latest tick's); the chunks applied by the client rule
    /// equal the core's series exactly, with `from` always the page's length; frames ≥ 100 ms
    /// apart; every frame within the bound.
    #[test]
    fn the_channel_holds_over_arbitrary_interleavings(ops in prop::collection::vec(op(), 1..40)) {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(script(ops));
    }
}
