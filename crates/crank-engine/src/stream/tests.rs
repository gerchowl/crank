//! Flow-control, frame-size, rate and subscription rules, on explicit microsecond
//! time (no wall clock, no runtime for the pure parts). Engine-level behaviour with the
//! simulator on a virtual clock is in `tests/streams.rs`.
#![expect(
    clippy::cast_precision_loss,
    reason = "small test counters and indices are exact in f64"
)]
use super::flow::{FRAME_BUDGET_BYTES, Flow, HEARTBEAT_US, MAX_UNACKED, MIN_FRAME_INTERVAL_US};
use super::hub::{Hub, MAX_SUBSCRIPTIONS};
use super::types::{
    LinkDevice, LinkStatus, MAX_RECORD_ERROR_BYTES, MetricId, StreamError, StreamFrame,
    StreamRideState, StreamStatus, SubId, SubSample, SubscribeOpts,
};
use crank_domain::filters::Filter;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

fn status(state: StreamRideState, marker: u64) -> StreamStatus {
    StreamStatus {
        state,
        loaded: true,
        session_s: 0.0,
        workout_s: 0.0,
        block_index: Some(0),
        block_elapsed_s: 0.0,
        late_samples: marker,
        record_error: None,
    }
}
fn riding(marker: u64) -> StreamStatus {
    status(StreamRideState::Riding, marker)
}
fn json(f: &StreamFrame) -> Vec<u8> {
    serde_json::to_vec(f).expect("frame serializes")
}
fn sample(sub: u64, t: f64, v: f64) -> SubSample {
    SubSample {
        sub: SubId(sub),
        t,
        v: Some(v),
    }
}
fn opts(rate_hz: f64, filters: Vec<Filter>) -> SubscribeOpts {
    SubscribeOpts { rate_hz, filters }
}

// ---- flow control: property tests over arbitrary interleavings -------------------

#[derive(Clone, Debug)]
enum Op {
    Sample(u8),
    Status,
    Link(bool, u8),
    Advance(u32),
    /// Acknowledge the in-flight frame at this index modulo the in-flight count (and
    /// every frame before it).
    Ack(u8),
    AckAll,
    /// The consumer drops the content of the next frame it receives (still acks it).
    Discard,
}
fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0u8..6).prop_map(Op::Sample),
        2 => Just(Op::Status),
        1 => (any::<bool>(), 0u8..3).prop_map(|(d, s)| Op::Link(d, s)),
        3 => (0u32..400_000).prop_map(Op::Advance),
        2 => any::<u8>().prop_map(Op::Ack),
        1 => Just(Op::AckAll),
        1 => Just(Op::Discard),
    ]
}
fn link(s: u8) -> LinkStatus {
    [LinkStatus::Connecting, LinkStatus::Ready, LinkStatus::Lost][usize::from(s % 3)]
}

/// The oracle and the consumer, written independently of `Flow`: it counts in-flight
/// frames itself and keeps its own idea of what is pending and when a resync is owed.
#[derive(Default)]
struct World {
    now: u64,
    counter: u64,
    // what the engine side has been told, latest per key
    latest: BTreeMap<u64, SubSample>,
    latest_status: Option<StreamStatus>,
    latest_links: BTreeMap<LinkDevice, LinkStatus>,
    // oracle's pending sets and owed resync
    pend_samples: BTreeSet<u64>,
    pend_status: bool,
    pend_links: BTreeSet<LinkDevice>,
    owed_resync: bool,
    // consumer
    inflight: Vec<u64>,
    expected_seq: u64,
    discard_next: bool,
    view: BTreeMap<u64, SubSample>,
    view_status: Option<StreamStatus>,
    view_links: BTreeMap<LinkDevice, LinkStatus>,
    last_emit: Option<u64>,
}
impl World {
    fn blocked(&self) -> bool {
        self.inflight.len() >= MAX_UNACKED
    }
    /// The window re-reads durable state: `stream_snapshot`.
    fn snapshot(&mut self) {
        self.view = self.latest.clone();
        self.view_status = self.latest_status.clone();
        self.view_links = self.latest_links.clone();
    }
    fn offer_sample(&mut self, flow: &mut Flow, sub: u64) {
        self.counter += 1;
        #[expect(clippy::cast_precision_loss, reason = "small test counter")]
        let s = sample(sub, self.counter as f64, self.counter as f64);
        if !self.pend_samples.insert(sub) && self.blocked() {
            self.owed_resync = true;
        }
        self.latest.insert(sub, s);
        flow.offer_sample(s);
    }
    fn offer_status(&mut self, flow: &mut Flow) {
        self.counter += 1;
        let s = riding(self.counter);
        if self.pend_status && self.blocked() {
            self.owed_resync = true;
        }
        self.pend_status = true;
        self.latest_status = Some(s.clone());
        flow.offer_status(s);
    }
    fn offer_link(&mut self, flow: &mut Flow, device: LinkDevice, state: LinkStatus) {
        if !self.pend_links.insert(device) && self.blocked() {
            self.owed_resync = true;
        }
        self.latest_links.insert(device, state);
        flow.offer_link(device, state);
    }
    fn ack(&mut self, flow: &mut Flow, seq: u64) {
        self.inflight.retain(|&s| s > seq);
        flow.ack(seq);
    }
    /// Poll the flow at `now` and run the consumer over what it sends.
    fn poll(&mut self, flow: &mut Flow) {
        let frames = flow.drain(self.now);
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.seq, self.expected_seq, "consecutive seq");
            assert!(json(f).len() <= FRAME_BUDGET_BYTES, "frame over budget");
            // Resync is owed exactly once, on the first frame of the batch.
            assert_eq!(f.resync, self.owed_resync && i == 0, "resync flag");
            if let Some(last) = self.last_emit.filter(|_| i == 0) {
                assert!(self.now - last >= MIN_FRAME_INTERVAL_US, "10 Hz bound");
            }
            self.expected_seq += 1;
            self.inflight.push(f.seq);
            assert!(self.inflight.len() <= MAX_UNACKED, "more than 3 unacked");
            if self.discard_next {
                // the window dropped the frame and re-reads durable state at once
                self.discard_next = false;
                self.snapshot();
                continue;
            }
            if f.resync {
                self.snapshot();
            }
            for s in &f.samples {
                self.view.insert(s.sub.0, *s);
            }
            if f.status.is_some() {
                self.view_status.clone_from(&f.status);
            }
            for l in &f.links {
                self.view_links.insert(l.device, l.state);
            }
        }
        if !frames.is_empty() {
            self.last_emit = Some(self.now);
            self.owed_resync = false;
            self.pend_samples.clear();
            self.pend_status = false;
            self.pend_links.clear();
        }
        assert_eq!(flow.unacked(), self.inflight.len(), "window agrees");
        assert!(flow.pending_len() <= 6 + 1 + 2, "pending state is bounded");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn flow_control_holds_over_arbitrary_interleavings(ops in prop::collection::vec(op(), 0..160)) {
        let mut flow = Flow::new(FRAME_BUDGET_BYTES);
        let mut w = World::default();
        for op in ops {
            match op {
                Op::Sample(sub) => w.offer_sample(&mut flow, u64::from(sub)),
                Op::Status => w.offer_status(&mut flow),
                Op::Link(trainer, s) => {
                    let d = if trainer { LinkDevice::Trainer } else { LinkDevice::Hr };
                    w.offer_link(&mut flow, d, link(s));
                }
                Op::Advance(us) => w.now += u64::from(us),
                Op::Ack(i) => {
                    if !w.inflight.is_empty() {
                        let seq = w.inflight[usize::from(i) % w.inflight.len()];
                        w.ack(&mut flow, seq);
                    }
                }
                Op::AckAll => {
                    if let Some(&seq) = w.inflight.last() {
                        w.ack(&mut flow, seq);
                    }
                }
                Op::Discard => w.discard_next = true,
            }
            // While the window is full the pending sample of a subscription is its
            // latest: coalescing never keeps a stale value.
            if w.blocked() {
                for sub in &w.pend_samples {
                    prop_assert_eq!(
                        flow.pending_sample(SubId(*sub)),
                        w.latest.get(sub).copied()
                    );
                }
            }
            w.poll(&mut flow);
        }
        // Quiesce: acknowledge everything and let time pass; the consumer's view must
        // then be the latest state, whatever was dropped on the way (resync).
        for _ in 0..8 {
            if let Some(&seq) = w.inflight.last() {
                w.ack(&mut flow, seq);
            }
            w.now += MIN_FRAME_INTERVAL_US;
            w.poll(&mut flow);
        }
        prop_assert_eq!(flow.pending_len(), 0);
        prop_assert_eq!(&w.view, &w.latest);
        prop_assert_eq!(&w.view_status, &w.latest_status);
        prop_assert_eq!(&w.view_links, &w.latest_links);
    }
}

#[test]
fn a_full_window_sends_nothing_and_coalesces_to_the_latest() {
    let mut flow = Flow::new(FRAME_BUDGET_BYTES);
    let mut now = 0;
    for i in 0..3_u64 {
        flow.offer_sample(sample(1, 0.0, 0.0));
        assert_eq!(flow.drain(now).len(), 1, "frame {i}");
        now += MIN_FRAME_INTERVAL_US;
    }
    assert!(flow.blocked());
    for v in 10..1_000 {
        flow.offer_sample(sample(1, 0.0, f64::from(v)));
        assert!(
            flow.drain(now + 10_000_000).is_empty(),
            "blocked: nothing sent"
        );
    }
    assert_eq!(flow.pending_len(), 1, "no unbounded queue");
    assert_eq!(flow.pending_sample(SubId(1)), Some(sample(1, 0.0, 999.0)));
    assert!(flow.ack(0));
    let frames = flow.drain(now + 10_000_000);
    assert_eq!(frames.len(), 1);
    assert!(frames[0].resync, "frames were coalesced");
    assert_eq!(frames[0].samples, [sample(1, 0.0, 999.0)]);
    assert_eq!(frames[0].seq, 3);
}

#[test]
fn an_ack_for_a_frame_never_sent_is_ignored() {
    let mut flow = Flow::new(FRAME_BUDGET_BYTES);
    flow.offer_status(riding(1));
    assert_eq!(flow.drain(0).len(), 1);
    assert!(!flow.ack(5));
    assert_eq!(flow.unacked(), 1);
    assert!(flow.ack(0));
    assert_eq!(flow.unacked(), 0);
}

// ---- frame size ------------------------------------------------------------------

/// A status at its caps: the longest record error of 4-byte characters, widest numbers.
fn worst_status() -> StreamStatus {
    StreamStatus {
        state: StreamRideState::Paused {
            reason: super::types::StreamPauseReason::StoppedPedalling,
        },
        loaded: true,
        session_s: f64::MIN,
        workout_s: f64::MIN,
        block_index: Some(u32::MAX),
        block_elapsed_s: f64::MIN,
        late_samples: u64::MAX,
        record_error: Some("𝔘".repeat(1_000)),
    }
}

#[test]
fn every_frame_stays_under_the_byte_budget_at_every_cap() {
    let mut flow = Flow::new(FRAME_BUDGET_BYTES);
    for sub in 0..MAX_SUBSCRIPTIONS as u64 {
        flow.offer_sample(SubSample {
            sub: SubId(u64::MAX - sub),
            t: f64::MIN,
            v: Some(-1.797_693_134_862_315_7e308),
        });
    }
    let mut ride = crate::engine::RideStatus {
        state: crate::engine::RideState::Riding,
        loaded: true,
        session: crate::ticks::SessionTicks(u64::MAX),
        workout: crate::ticks::WorkoutTicks(u64::MAX),
        block_index: Some(usize::MAX),
        block_elapsed: crate::ticks::WorkoutTicks(u64::MAX),
        trainer_link: None,
        hr_link: None,
        late_samples: u64::MAX,
        record_error: Some("𝔘".repeat(1_000)),
    };
    ride.state = crate::engine::RideState::Paused(crate::engine::PauseReason::StoppedPedalling);
    let from_engine = StreamStatus::from(&ride);
    assert_eq!(
        from_engine.record_error.as_ref().map(String::len),
        Some(MAX_RECORD_ERROR_BYTES),
        "cut on a character boundary at the cap"
    );
    flow.offer_status(from_engine);
    flow.offer_link(LinkDevice::Trainer, LinkStatus::Connecting);
    flow.offer_link(LinkDevice::Hr, LinkStatus::Connecting);
    let frames = flow.drain(0);
    let sent: usize = frames.iter().map(|f| f.samples.len()).sum();
    assert_eq!((sent, flow.pending_len()), (MAX_SUBSCRIPTIONS, 0));
    for f in &frames {
        let n = json(f).len();
        assert!(n <= FRAME_BUDGET_BYTES, "{n} bytes");
    }
    // The uncut worst status is itself far under the budget: the cap is not what
    // keeps frames small, only the measurement is the guarantee.
    assert!(serde_json::to_vec(&worst_status()).expect("status").len() < FRAME_BUDGET_BYTES);
}

#[test]
fn oversized_samples_split_over_consecutive_frames_and_all_arrive() {
    // A small budget forces splitting: ~30 bytes per sample, a few per frame.
    let mut flow = Flow::new(260);
    for sub in 0..10_u64 {
        flow.offer_sample(sample(sub, 1.5, 100.0 + sub as f64));
    }
    flow.offer_status(riding(1));
    let frames = flow.drain(0);
    assert!(frames.len() > 1);
    let mut got = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.seq, i as u64, "consecutive seq");
        assert!(json(f).len() <= 260);
        assert_eq!(
            f.status.is_some(),
            i == 0,
            "discrete state rides the first frame"
        );
        got.extend(f.samples.iter().map(|s| s.sub.0));
    }
    assert_eq!(got, (0..10).collect::<Vec<_>>(), "every sample, in order");
    assert_eq!(flow.unacked(), frames.len());
}

#[test]
fn a_split_stops_at_the_window_and_leaves_the_rest_pending() {
    let mut flow = Flow::new(150);
    for sub in 0..20_u64 {
        flow.offer_sample(sample(sub, 1.5, 1.0));
    }
    let frames = flow.drain(0);
    assert_eq!(frames.len(), MAX_UNACKED, "never more than the window");
    let sent: usize = frames.iter().map(|f| f.samples.len()).sum();
    assert_eq!(flow.pending_len(), 20 - sent);
    assert!(flow.pending_len() > 0);
}

#[test]
fn discrete_state_over_budget_becomes_the_bare_resync_frame() {
    let mut flow = Flow::new(120);
    flow.offer_status(worst_status());
    flow.offer_sample(sample(1, 1.0, 2.0));
    let frames = flow.drain(0);
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert!(f.resync && f.status.is_none() && f.links.is_empty() && f.samples.is_empty());
    assert_eq!(
        String::from_utf8(json(f)).expect("utf8"),
        r#"{"seq":0,"resync":true,"samples":[],"status":null,"links":[]}"#
    );
    assert!(json(f).len() < 100, "fixed shape under 100 bytes");
    assert_eq!(flow.pending_len(), 1, "the sample is not lost");
}

// ---- hub: subscriptions, rates, heartbeat, cleanup -------------------------------

const MS: u64 = 1_000;
fn hub_with_stream() -> (
    Hub,
    super::types::StreamId,
    tokio::sync::mpsc::Receiver<StreamFrame>,
) {
    let mut hub = Hub::new();
    let (id, rx) = hub.open(Some("main".into()), 0);
    (hub, id, rx)
}
fn drain(rx: &mut tokio::sync::mpsc::Receiver<StreamFrame>) -> Vec<StreamFrame> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[test]
fn subscription_chain_sees_every_source_sample_and_delivery_follows_the_rate() {
    let (mut hub, id, mut rx) = hub_with_stream();
    // 1 Hz delivery of a 1 s mean over a 4 Hz source: the chain must see all four.
    let sub = hub
        .subscribe(
            id,
            MetricId::Power,
            opts(1.0, vec![Filter::Sma { window_s: 1.0 }]),
        )
        .expect("subscribe");
    let mut delivered = Vec::new();
    for k in 1..=16_u64 {
        let t = k as f64 / 4.0;
        // 100 W for the first three of every four samples, 200 W on the fourth
        let p = if k % 4 == 0 { 200.0 } else { 100.0 };
        hub.on_source(t, &[Some(p), None, None, None, None]);
        hub.poll(k * 250 * MS);
        for f in drain(&mut rx) {
            hub.ack(id, f.seq).expect("ack");
            delivered.extend(f.samples);
        }
    }
    let ts: Vec<f64> = delivered.iter().map(|s| s.t).collect();
    assert_eq!(ts, [0.25, 1.25, 2.25, 3.25], "one per second");
    assert!(delivered.iter().all(|s| s.sub == sub));
    // SMA over (t - 1 s, t]: at 1.25 s it holds the samples at 0.5, 0.75, 1.0, 1.25.
    assert_eq!(delivered[1].v, Some((100.0 + 100.0 + 200.0 + 100.0) / 4.0));
}

#[test]
fn each_subscription_has_its_own_chain() {
    let (mut hub, id, mut rx) = hub_with_stream();
    let raw = hub
        .subscribe(id, MetricId::Hr, opts(10.0, vec![]))
        .expect("raw");
    let slow = hub
        .subscribe(
            id,
            MetricId::Hr,
            opts(10.0, vec![Filter::Ema { tau_s: 10.0 }]),
        )
        .expect("ema");
    hub.on_source(0.25, &[None, None, None, Some(100.0), None]);
    hub.on_source(0.5, &[None, None, None, Some(200.0), None]);
    hub.poll(500 * MS);
    let samples: Vec<_> = drain(&mut rx).into_iter().flat_map(|f| f.samples).collect();
    let last = |sub| {
        samples
            .iter()
            .rev()
            .find(|s| s.sub == sub)
            .and_then(|s| s.v)
    };
    assert_eq!(last(raw), Some(200.0));
    let ema = last(slow).expect("ema value");
    assert!(ema > 100.0 && ema < 200.0, "ema {ema} is filtered");
}

#[test]
fn subscriptions_are_bounded_and_validated() {
    let (mut hub, id, _rx) = hub_with_stream();
    for _ in 0..MAX_SUBSCRIPTIONS {
        hub.subscribe(id, MetricId::Power, opts(1.0, vec![]))
            .expect("within cap");
    }
    assert_eq!(
        hub.subscribe(id, MetricId::Power, opts(1.0, vec![])),
        Err(StreamError::TooManySubscriptions)
    );
    let (mut hub, id, _rx) = hub_with_stream();
    for bad in [0.0, 0.09, 10.1, f64::NAN, f64::INFINITY] {
        assert_eq!(
            hub.subscribe(id, MetricId::Power, opts(bad, vec![])),
            Err(StreamError::InvalidRate),
            "{bad}"
        );
    }
    assert!(matches!(
        hub.subscribe(
            id,
            MetricId::Power,
            opts(1.0, vec![Filter::Ema { tau_s: f64::NAN }])
        ),
        Err(StreamError::InvalidFilter(_))
    ));
    assert_eq!(
        hub.subscribe(
            super::types::StreamId(99),
            MetricId::Power,
            opts(1.0, vec![])
        ),
        Err(StreamError::UnknownStream)
    );
}

#[test]
fn unsubscribe_drops_the_pending_sample() {
    let (mut hub, id, mut rx) = hub_with_stream();
    let sub = hub
        .subscribe(id, MetricId::Power, opts(10.0, vec![]))
        .expect("sub");
    hub.on_source(0.25, &[Some(1.0), None, None, None, None]);
    hub.unsubscribe(id, sub).expect("unsub");
    hub.poll(0);
    assert!(drain(&mut rx).iter().all(|f| f.samples.is_empty()));
    assert_eq!(hub.unsubscribe(id, sub), Err(StreamError::UnknownSub));
}

#[test]
fn frames_never_exceed_ten_per_second_however_fast_state_changes() {
    let (mut hub, id, mut rx) = hub_with_stream();
    hub.subscribe(id, MetricId::Power, opts(10.0, vec![]))
        .expect("sub");
    let mut frames_at = Vec::new();
    // sources every 10 ms of clock, a new status marker every time, consumer acks at once
    for step in 1..=3_000_u64 {
        let now = step * 10 * MS;
        hub.on_source(
            step as f64 * 0.1,
            &[Some(step as f64), None, None, None, None],
        );
        hub.on_status(riding(step), now);
        hub.poll(now);
        for f in drain(&mut rx) {
            frames_at.push(now);
            hub.ack(id, f.seq).expect("ack");
        }
    }
    assert!(frames_at.len() > 250, "frames do flow: {}", frames_at.len());
    for pair in frames_at.windows(2) {
        assert!(pair[1] - pair[0] >= MIN_FRAME_INTERVAL_US, "{pair:?}");
    }
    let per_second = frames_at.iter().filter(|&&t| t < 1_000_000).count();
    assert!(per_second <= 10, "{per_second} frames in the first second");
}

/// `t` once per frame that carries a status; acknowledges everything.
fn status_times(
    hub: &mut Hub,
    id: super::types::StreamId,
    rx: &mut tokio::sync::mpsc::Receiver<StreamFrame>,
    t: u64,
) -> Vec<u64> {
    let mut at = Vec::new();
    for f in drain(rx) {
        if f.status.is_some() {
            at.push(t);
        }
        hub.ack(id, f.seq).expect("ack");
    }
    at
}

#[test]
fn status_is_sent_on_change_and_otherwise_at_two_hertz() {
    let (mut hub, id, mut rx) = hub_with_stream();
    let mut status_at = Vec::new();
    let mut t = 0;
    // 10 s riding: only the clock fields advance, every 250 ms
    for k in 0..=40_u64 {
        t = k * 250 * MS;
        let mut s = riding(0);
        s.session_s = k as f64 / 4.0;
        hub.on_status(s, t);
        hub.poll(t);
        status_at.extend(status_times(&mut hub, id, &mut rx, t));
    }
    assert_eq!(status_at[0], 0, "first status at once");
    for pair in status_at.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            HEARTBEAT_US,
            "2 Hz heartbeat: {status_at:?}"
        );
    }
    assert_eq!(status_at.len(), 21, "0, 0.5 … 10 s");
    // A change goes out in the same poll, not at the next beat.
    let before = status_at.len();
    t += 100 * MS;
    hub.on_status(
        status(
            StreamRideState::Paused {
                reason: super::types::StreamPauseReason::User,
            },
            0,
        ),
        t,
    );
    hub.poll(t);
    status_at.extend(status_times(&mut hub, id, &mut rx, t));
    assert_eq!(status_at.len(), before + 1);
    assert_eq!(*status_at.last().expect("sent"), t);
}

#[test]
fn no_heartbeat_while_idle_or_finished() {
    for state in [StreamRideState::Idle, StreamRideState::Finished] {
        let (mut hub, id, mut rx) = hub_with_stream();
        hub.on_status(status(state, 0), 0);
        hub.poll(0);
        assert_eq!(drain(&mut rx).len(), 1, "the change itself");
        hub.ack(id, 0).expect("ack");
        for k in 1..=20 {
            hub.on_status(status(state, 0), k * 500 * MS);
            hub.poll(k * 500 * MS);
        }
        assert!(drain(&mut rx).is_empty());
    }
}

#[test]
fn a_blocked_stream_does_not_beat_into_resync() {
    let (mut hub, _id, mut rx) = hub_with_stream();
    hub.on_status(riding(0), 0);
    for k in 0..40_u64 {
        hub.poll(k * 500 * MS);
    }
    // never acked: 3 frames at most, whatever the beats
    assert_eq!(drain(&mut rx).len(), MAX_UNACKED);
}

#[test]
fn a_new_stream_starts_with_the_current_status_and_links() {
    let mut hub = Hub::new();
    hub.on_status(riding(7), 0);
    hub.on_link(LinkDevice::Trainer, LinkStatus::Ready);
    let (_, mut rx) = hub.open(None, 0);
    hub.poll(0);
    let f = drain(&mut rx).remove(0);
    assert_eq!(f.status, Some(riding(7)));
    assert_eq!(f.links.len(), 1);
}

#[test]
fn link_changes_are_sent_only_when_they_change() {
    let (mut hub, id, mut rx) = hub_with_stream();
    hub.on_link(LinkDevice::Hr, LinkStatus::Ready);
    hub.poll(0);
    hub.ack(id, drain(&mut rx)[0].seq).expect("ack");
    hub.on_link(LinkDevice::Hr, LinkStatus::Ready);
    hub.poll(200 * MS);
    assert!(drain(&mut rx).is_empty(), "same state: nothing");
    hub.on_link(LinkDevice::Hr, LinkStatus::Lost);
    hub.poll(400 * MS);
    let f = drain(&mut rx).remove(0);
    assert_eq!(f.links.len(), 1);
    assert_eq!(f.links[0].state, LinkStatus::Lost);
}

#[test]
fn window_cleanup_drops_streams_and_dead_sinks() {
    let mut hub = Hub::new();
    let (a, mut rx_a) = hub.open(Some("w1".into()), 0);
    let (b, _rx_b) = hub.open(Some("w2".into()), 0);
    assert_eq!(hub.stream_count(), 2);
    // a page reload: same label, the previous stream goes
    let (a2, rx_a2) = hub.open(Some("w1".into()), 0);
    assert_ne!(a, a2);
    assert_eq!(hub.stream_count(), 2);
    assert!(rx_a.try_recv().is_err());
    assert_eq!(hub.ack(a, 0), Err(StreamError::UnknownStream));
    // window destroyed
    hub.close_window("w2");
    assert_eq!(hub.ack(b, 0), Err(StreamError::UnknownStream));
    assert_eq!(hub.stream_count(), 1);
    // a failed send drops the stream, and the engine side carries on
    drop(rx_a2);
    hub.on_status(riding(0), 0);
    hub.poll(0);
    assert_eq!(hub.stream_count(), 0);
}

#[test]
fn a_consumer_that_never_reads_or_acks_is_bounded() {
    let (mut hub, id, _rx) = hub_with_stream();
    for _ in 0..MAX_SUBSCRIPTIONS {
        hub.subscribe(id, MetricId::Power, opts(10.0, vec![]))
            .expect("sub");
    }
    for step in 1..=20_000_u64 {
        hub.on_source(step as f64, &[Some(step as f64), None, None, None, None]);
        hub.on_status(riding(step), step * 10 * MS);
        hub.poll(step * 10 * MS);
    }
    let flow = hub.flow(id).expect("stream alive");
    assert_eq!(flow.unacked(), MAX_UNACKED);
    assert!(flow.pending_len() <= MAX_SUBSCRIPTIONS + 3);
}
