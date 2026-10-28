//! Flow-control, frame-size, rate and subscription rules, on explicit microsecond
//! time (no wall clock, no runtime for the pure parts). Engine-level behaviour with the
//! simulator on a virtual clock is in `tests/streams.rs`.
#![expect(
    clippy::cast_precision_loss,
    reason = "small test counters and indices are exact in f64"
)]
use super::flow::{FRAME_BUDGET_BYTES, Flow, HEARTBEAT_US, MAX_UNACKED, MIN_FRAME_INTERVAL_US};
use super::hub::{Hub, MAX_STREAMS, MAX_SUBSCRIPTIONS, fresh_id};
use super::types::{
    LinkDevice, LinkStatus, MAX_RECORD_ERROR_BYTES, MetricId, StreamError, StreamFrame, StreamId,
    StreamRideState, StreamStatus, SubId, SubSample, SubscribeOpts,
};
use crank_domain::filters::Filter;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

fn status(state: StreamRideState, marker: u32) -> StreamStatus {
    StreamStatus {
        state,
        loaded: true,
        session_s: 0.0,
        workout_s: 0.0,
        block_index: Some(0),
        block_elapsed_s: 0.0,
        late_samples: marker,
        record_error: None,
        trainer_following: super::types::StreamFollowing::Unknown,
        not_following_component: None,
        erg_released: false,
        compliance: super::types::StreamCompliance::default(),
        cap: None,
    }
}
fn riding(marker: u32) -> StreamStatus {
    status(StreamRideState::Riding, marker)
}
fn json(f: &StreamFrame) -> Vec<u8> {
    serde_json::to_vec(f).expect("frame serializes")
}
fn sample(sub: u32, t: f64, v: f64) -> SubSample {
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
        5 => (0u8..10).prop_map(Op::Sample),
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

/// The oracle and the consumer, written from the ADR-0001 §4.3 text and independent of
/// `Flow`: it counts in-flight frames itself, keeps its own pending set and decides from
/// the ADR when a frame is due, what it must carry and when a resync is owed.
struct World {
    budget: usize,
    now: u64,
    counter: u32,
    // the latest state offered, per key
    latest: BTreeMap<u32, SubSample>,
    latest_status: Option<StreamStatus>,
    latest_links: BTreeMap<LinkDevice, LinkStatus>,
    // ADR: "bounded pending state": the oracle's own copy of what is waiting
    pend_samples: BTreeSet<u32>,
    pend_status: Option<StreamStatus>,
    pend_links: BTreeMap<LinkDevice, LinkStatus>,
    // ADR: the next frame after the window reopens carries `resync` iff something was
    // coalesced while the window was full
    owed_resync: bool,
    // consumer
    inflight: Vec<u32>,
    expected_seq: u32,
    discard_next: bool,
    view: BTreeMap<u32, SubSample>,
    view_status: Option<StreamStatus>,
    view_links: BTreeMap<LinkDevice, LinkStatus>,
    last_emit: Option<u64>,
    split_batches: u32,
    bare_frames: u32,
}
impl World {
    /// A world whose first frame carries `seq`: the property must also run across the
    /// `u32` wrap (#272).
    fn new(budget: usize, seq: u32) -> Self {
        Self {
            budget,
            now: 0,
            counter: 0,
            latest: BTreeMap::new(),
            latest_status: None,
            latest_links: BTreeMap::new(),
            pend_samples: BTreeSet::new(),
            pend_status: None,
            pend_links: BTreeMap::new(),
            owed_resync: false,
            inflight: Vec::new(),
            expected_seq: seq,
            discard_next: false,
            view: BTreeMap::new(),
            view_status: None,
            view_links: BTreeMap::new(),
            last_emit: None,
            split_batches: 0,
            bare_frames: 0,
        }
    }
    /// The window is full: three frames sent and not acknowledged.
    fn blocked(&self) -> bool {
        self.inflight.len() >= MAX_UNACKED
    }
    /// The window re-reads durable state: `stream_snapshot`.
    fn snapshot(&mut self) {
        self.view = self.latest.clone();
        self.view_status = self.latest_status.clone();
        self.view_links = self.latest_links.clone();
    }
    fn offer_sample(&mut self, flow: &mut Flow, sub: u32) {
        self.counter += 1;
        let s = sample(sub, f64::from(self.counter), f64::from(self.counter));
        if !self.pend_samples.insert(sub) && self.blocked() {
            self.owed_resync = true;
        }
        self.latest.insert(sub, s);
        flow.offer_sample(s);
    }
    fn offer_status(&mut self, flow: &mut Flow) {
        self.counter += 1;
        let s = riding(self.counter);
        if self.pend_status.is_some() && self.blocked() {
            self.owed_resync = true;
        }
        self.pend_status = Some(s.clone());
        self.latest_status = Some(s.clone());
        flow.offer_status(s);
    }
    fn offer_link(&mut self, flow: &mut Flow, device: LinkDevice, state: LinkStatus) {
        if self.pend_links.insert(device, state).is_some() && self.blocked() {
            self.owed_resync = true;
        }
        self.latest_links.insert(device, state);
        flow.offer_link(device, state);
    }
    fn ack(&mut self, flow: &mut Flow, seq: u32) {
        // Frames are acknowledged in send order, so by position, not by comparing ids
        // (seq wraps); a seq that is not in flight changes nothing.
        if let Some(i) = self.inflight.iter().position(|&s| s == seq) {
            self.inflight.drain(..=i);
        }
        flow.ack(seq);
    }
    fn anything_pending(&self) -> bool {
        !self.pend_samples.is_empty()
            || self.pend_status.is_some()
            || !self.pend_links.is_empty()
            || self.owed_resync
    }
    /// Poll the flow at `now`, check the batch against the ADR, run the consumer.
    fn poll(&mut self, flow: &mut Flow) {
        let frames = flow.drain(self.now);
        // ≤ 10 Hz and "sends nothing while the window is full".
        let due = self.anything_pending()
            && !self.blocked()
            && self
                .last_emit
                .is_none_or(|l| self.now - l >= MIN_FRAME_INTERVAL_US);
        assert_eq!(!frames.is_empty(), due, "a frame iff something is due");
        if frames.is_empty() {
            assert_eq!(flow.unacked(), self.inflight.len(), "window agrees");
            return;
        }
        // Discrete state alone over budget: the bare resync frame (step 3).
        let links: Vec<_> = self
            .pend_links
            .iter()
            .map(|(&device, &state)| super::types::LinkChange { device, state })
            .collect();
        let head = StreamFrame {
            seq: self.expected_seq,
            resync: self.owed_resync,
            samples: Vec::new(),
            status: self.pend_status.clone(),
            links: links.clone(),
        };
        let bare = json(&head).len() > self.budget;
        if bare {
            self.bare_frames += 1;
            assert_eq!(frames.len(), 1, "bare frame alone");
            let f = &frames[0];
            assert!(f.resync && f.samples.is_empty() && f.status.is_none() && f.links.is_empty());
            self.pend_status = None;
            self.pend_links.clear();
        } else {
            assert_eq!(frames[0].resync, self.owed_resync, "resync owed once");
            assert_eq!(frames[0].status, self.pend_status.take());
            assert_eq!(frames[0].links, links);
            self.pend_links.clear();
        }
        if frames.len() > 1 {
            self.split_batches += 1;
        }
        let mut carried = BTreeSet::new();
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.seq, self.expected_seq, "consecutive seq");
            assert!(bare || json(f).len() <= self.budget, "frame over budget");
            assert!(bare || i == 0 || (!f.resync && f.status.is_none() && f.links.is_empty()));
            self.expected_seq = self.expected_seq.wrapping_add(1);
            self.inflight.push(f.seq);
            assert!(self.inflight.len() <= MAX_UNACKED, "more than 3 unacked");
            for s in &f.samples {
                // one per subscription per batch, always the latest, and it was pending
                assert!(carried.insert(s.sub.0), "one sample per subscription");
                assert!(self.pend_samples.contains(&s.sub.0));
                assert_eq!(Some(s), self.latest.get(&s.sub.0), "latest wins");
            }
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
        for sub in &carried {
            self.pend_samples.remove(sub);
        }
        if !self.pend_samples.is_empty() && !bare {
            assert!(
                self.blocked(),
                "samples are left over only when the window is full"
            );
        }
        self.last_emit = Some(self.now);
        self.owed_resync = false;
        assert_eq!(flow.unacked(), self.inflight.len(), "window agrees");
        assert!(flow.pending_len() <= 10 + 1 + 2, "pending state is bounded");
    }
}

// Budgets start at 120 bytes: an empty frame is 62 and a sample at most ~40, so a lone
// sample always fits; real budgets are 8 000.
/// Where a stream's seq starts: usually at 0, often a few frames short of the `u32` wrap.
fn start_seq() -> impl Strategy<Value = u32> {
    prop_oneof![1 => Just(0), 3 => (0u32..8).prop_map(|k| u32::MAX - k)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn flow_control_holds_over_arbitrary_interleavings(
        budget in prop_oneof![1 => Just(FRAME_BUDGET_BYTES), 3 => 120usize..400],
        start in start_seq(),
        ops in prop::collection::vec(op(), 0..160),
    ) {
        let mut flow = Flow::starting_at(budget, start);
        let mut w = World::new(budget, start);
        for op in ops {
            match op {
                Op::Sample(sub) => w.offer_sample(&mut flow, u32::from(sub)),
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
        for _ in 0..24 {
            if let Some(&seq) = w.inflight.last() {
                w.ack(&mut flow, seq);
            }
            w.now += MIN_FRAME_INTERVAL_US;
            w.poll(&mut flow);
        }
        prop_assert_eq!(flow.pending_len(), 0);
        prop_assert_eq!(&w.view, &w.latest);
        // A bare resync drops discrete state: the consumer re-read it by query.
        prop_assert!(w.bare_frames > 0 || w.view_status == w.latest_status);
        prop_assert!(w.bare_frames > 0 || w.view_links == w.latest_links);
    }
}

/// The generator must actually reach the split and bare paths, or the property proves
/// nothing about them (checked on a fixed seed sweep).
#[test]
fn the_generator_reaches_the_split_and_bare_paths() {
    use proptest::test_runner::{Config, TestRunner};
    let mut runner = TestRunner::new(Config::with_cases(300));
    let (split, bare) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
    let strategy = (
        120usize..400,
        start_seq(),
        prop::collection::vec(op(), 0..160),
    );
    runner
        .run(&strategy, |(budget, start, ops)| {
            let mut flow = Flow::starting_at(budget, start);
            let mut w = World::new(budget, start);
            for op in ops {
                match op {
                    Op::Sample(sub) => w.offer_sample(&mut flow, u32::from(sub)),
                    Op::Status => w.offer_status(&mut flow),
                    Op::Advance(us) => w.now += u64::from(us),
                    Op::AckAll => {
                        if let Some(&seq) = w.inflight.last() {
                            w.ack(&mut flow, seq);
                        }
                    }
                    _ => {}
                }
                w.poll(&mut flow);
            }
            split.set(split.get() + w.split_batches);
            bare.set(bare.get() + w.bare_frames);
            Ok(())
        })
        .expect("model holds");
    assert!(split.get() > 50, "split batches: {}", split.get());
    assert!(bare.get() > 5, "bare frames: {}", bare.get());
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
        late_samples: u32::MAX,
        record_error: Some("𝔘".repeat(1_000)),
        trainer_following: super::types::StreamFollowing::NotFollowing,
        not_following_component: Some(super::types::StreamComponent::Resistance),
        erg_released: true,
        compliance: super::types::StreamCompliance {
            counted_s: u32::MAX,
            within_s: u32::MAX,
            excluded_s: u32::MAX,
        },
        cap: Some(super::types::StreamCap {
            cap_w: u16::MAX,
            erg_only: true,
        }),
    }
}

#[test]
fn every_frame_stays_under_the_byte_budget_at_every_cap() {
    let mut flow = Flow::new(FRAME_BUDGET_BYTES);
    for sub in 0..u32::try_from(MAX_SUBSCRIPTIONS).unwrap() {
        flow.offer_sample(SubSample {
            sub: SubId(u32::MAX - sub),
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
        trainer_following: crate::erg::TrainerFollowing::NotFollowing,
        not_following_component: Some(crate::erg::Component::Resistance),
        erg_released: true,
        compliance: crate::erg::Compliance {
            counted_s: u32::MAX,
            within_s: u32::MAX,
            excluded_s: u32::MAX,
        },
        cap: Some(crate::engine::CapState {
            cap_w: u16::MAX,
            erg_only: true,
        }),
    };
    ride.state = crate::engine::RideState::Paused(crate::engine::PauseReason::StoppedPedalling);
    let from_engine = StreamStatus::from(&ride);
    assert_eq!(
        from_engine.late_samples,
        u32::MAX,
        "the engine's u64 counter saturates on the wire"
    );
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
    // A small budget forces splitting: ~30 bytes per sample, a few per frame. The status
    // alone is ~285 bytes since #218 (following, compliance, cap), so the budget is
    // sized so that 12 samples split over exactly three frames.
    const BUDGET: usize = 360;
    let mut flow = Flow::new(BUDGET);
    for sub in 0..12_u32 {
        flow.offer_sample(sample(sub, 1.5, 100.0 + f64::from(sub)));
    }
    flow.offer_status(riding(1));
    let frames = flow.drain(0);
    // the status (~285 bytes) rides the first frame with a few samples: three frames, the
    // whole window (MAX_UNACKED), carry the 12 samples
    assert_eq!(frames.len(), 3);
    let mut got = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(u64::from(f.seq), i as u64, "consecutive seq");
        assert!(json(f).len() <= BUDGET);
        assert_eq!(
            f.status.is_some(),
            i == 0,
            "discrete state rides the first frame"
        );
        got.extend(f.samples.iter().map(|s| s.sub.0));
    }
    assert_eq!(got, (0..12).collect::<Vec<_>>(), "every sample, in order");
    assert_eq!(flow.unacked(), frames.len());
}

#[test]
fn a_split_stops_at_the_window_and_leaves_the_rest_pending() {
    let mut flow = Flow::new(150);
    for sub in 0..20_u32 {
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
    let (id, rx) = hub.open(Some("main".into()), 0).expect("open");
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
        hub.on_status(riding(u32::try_from(step).unwrap()), now);
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
    let (_, mut rx) = hub.open(None, 0).expect("open");
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
    let (a, mut rx_a) = hub.open(Some("w1".into()), 0).expect("open");
    let (b, _rx_b) = hub.open(Some("w2".into()), 0).expect("open");
    assert_eq!(hub.stream_count(), 2);
    // a page reload: same label, the previous stream goes
    let (a2, rx_a2) = hub.open(Some("w1".into()), 0).expect("open");
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
        hub.on_status(riding(u32::try_from(step).unwrap()), step * 10 * MS);
        hub.poll(step * 10 * MS);
    }
    let flow = hub.flow(id).expect("stream alive");
    assert_eq!(flow.unacked(), MAX_UNACKED);
    assert!(flow.pending_len() <= MAX_SUBSCRIPTIONS + 3);
}

// ---- review round 2: caps, reaping, pacing anchors, gaps ---------------------------

#[test]
fn a_status_change_restarts_the_heartbeat_period() {
    // Mutant: `last_status_us` not reset on a change would beat again 200 ms later.
    let (mut hub, id, mut rx) = hub_with_stream();
    let mut at = Vec::new();
    for step in 0..=20_u64 {
        let t = step * 100 * MS;
        // a discrete change at 300 ms, clock fields advancing all the time
        let mut s = riding(u32::from(t >= 300 * MS));
        s.session_s = step as f64 / 10.0;
        hub.on_status(s, t);
        hub.poll(t);
        at.extend(status_times(&mut hub, id, &mut rx, t));
    }
    assert_eq!(at[..2], [0, 300 * MS], "first status, then the change");
    assert_eq!(
        at[2],
        800 * MS,
        "next beat 500 ms after the change, not after the first"
    );
}

#[test]
fn unlabelled_streams_never_evict_each_other() {
    // Mutant: `if window.is_some()` → `if true` would drop the first `None` stream.
    let mut hub = Hub::new();
    let (a, _ra) = hub.open(None, 0).expect("open");
    let (b, _rb) = hub.open(None, 0).expect("open");
    assert_eq!(hub.stream_count(), 2);
    assert!(hub.ack(a, 0).is_ok() && hub.ack(b, 0).is_ok());
}

#[test]
fn a_blocked_stream_neither_beats_nor_flags_resync_after_the_ack() {
    // Mutant: heartbeat offered while blocked replaces the pending status → resync.
    let (mut hub, id, mut rx) = hub_with_stream();
    // three discrete changes fill the window
    for k in 0..3_u64 {
        hub.on_status(riding(u32::try_from(k).unwrap()), k * 100 * MS);
        hub.poll(k * 100 * MS);
    }
    assert_eq!(drain(&mut rx).len(), 3);
    for k in 0..20_u64 {
        let t = 1_000 * MS + k * 500 * MS;
        let mut s = riding(2);
        s.session_s = k as f64;
        hub.on_status(s, t);
        hub.poll(t);
    }
    assert!(drain(&mut rx).is_empty(), "blocked: nothing sent");
    hub.ack(id, 2).expect("ack");
    for k in 0..4_u64 {
        let t = 12_000 * MS + k * 500 * MS;
        hub.on_status(riding(2), t);
        hub.poll(t);
    }
    let frames = drain(&mut rx);
    assert!(!frames.is_empty());
    assert!(
        frames.iter().all(|f| !f.resync),
        "nothing was coalesced: {frames:?}"
    );
}

#[test]
fn filter_chains_are_capped_and_the_caps_themselves_are_accepted() {
    use super::types::{MAX_FILTER_SECONDS, MAX_FILTERS_PER_CHAIN, MAX_MEDIAN_SAMPLES};
    let (mut hub, id, _rx) = hub_with_stream();
    let at_cap = vec![
        Filter::Median {
            n: MAX_MEDIAN_SAMPLES,
        },
        Filter::Sma {
            window_s: MAX_FILTER_SECONDS,
        },
        Filter::Ema {
            tau_s: MAX_FILTER_SECONDS,
        },
        Filter::OneEuro {
            min_cutoff: 1.0,
            beta: 0.1,
        },
    ];
    assert_eq!(at_cap.len(), MAX_FILTERS_PER_CHAIN);
    hub.subscribe(id, MetricId::Power, opts(10.0, at_cap.clone()))
        .expect("at the caps");
    let mut too_long = at_cap;
    too_long.push(Filter::Hold);
    for bad in [
        too_long,
        vec![Filter::Median {
            n: MAX_MEDIAN_SAMPLES + 1.0,
        }],
        vec![Filter::Median { n: 1e12 }],
        vec![Filter::Sma {
            window_s: MAX_FILTER_SECONDS + 0.1,
        }],
        vec![Filter::Sma { window_s: 1e300 }],
        vec![Filter::Ema {
            tau_s: MAX_FILTER_SECONDS + 0.1,
        }],
    ] {
        assert!(
            matches!(
                hub.subscribe(id, MetricId::Power, opts(1.0, bad.clone())),
                Err(StreamError::InvalidFilter(_))
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn the_tick_cost_stays_bounded_at_every_cap() {
    use super::types::{MAX_FILTER_SECONDS, MAX_MEDIAN_SAMPLES};
    // Every cap at once: 16 streams of 64 subscriptions, each with the longest chain.
    let mut hub = Hub::new();
    let mut rxs = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..MAX_STREAMS {
        let (id, rx) = hub.open(None, 0).expect("open");
        ids.push(id);
        rxs.push(rx);
    }
    for (id, _) in ids
        .iter()
        .flat_map(|id| (0..MAX_SUBSCRIPTIONS).map(move |n| (*id, n)))
    {
        hub.subscribe(
            id,
            MetricId::Power,
            opts(
                10.0,
                vec![
                    Filter::Median {
                        n: MAX_MEDIAN_SAMPLES,
                    },
                    Filter::Sma {
                        window_s: MAX_FILTER_SECONDS,
                    },
                    Filter::Ema {
                        tau_s: MAX_FILTER_SECONDS,
                    },
                    Filter::OneEuro {
                        min_cutoff: 1.0,
                        beta: 0.1,
                    },
                ],
            ),
        )
        .expect("at the caps");
    }
    // Past every window length (1 200 samples of the longest SMA). The cost is counted, not
    // timed (#293): one tick must push each chain once and ship at most one frame per
    // subscription, so work per tick is linear in the subscriptions and cannot flake.
    let subscriptions = MAX_STREAMS * MAX_SUBSCRIPTIONS;
    let steps = 3_000_u64;
    let mut frames = 0_usize;
    for k in 1..=steps {
        let t = k as f64 / 4.0;
        let before = hub.chain_pushes();
        hub.on_source(t, &[Some((k % 400) as f64), None, None, None, None]);
        assert_eq!(
            hub.chain_pushes() - before,
            subscriptions,
            "chain pushes in tick {k}: each subscription's chain runs exactly once"
        );
        hub.poll(k * 250 * MS);
        // The consumers keep up, as the UI does.
        for (id, rx) in ids.iter().zip(&mut rxs) {
            for f in drain(rx) {
                frames += 1;
                hub.ack(*id, f.seq).expect("ack");
            }
        }
    }
    // Ten per second against four ticks a second: at most one sample frame per
    // subscription per tick, plus a bounded number of status frames per stream.
    let per_tick_bound = subscriptions * usize::try_from(steps).expect("small");
    assert!(
        frames <= per_tick_bound + MAX_STREAMS * 8,
        "{frames} frames over {steps} ticks exceeds one per subscription per tick"
    );
}

#[test]
fn closed_receivers_are_reaped_without_anything_to_send() {
    // Mutant: reaping only on a failed send never frees a blocked or idle stream.
    let mut hub = Hub::new();
    let mut rxs = Vec::new();
    for _ in 0..super::hub::MAX_STREAMS {
        rxs.push(hub.open(None, 0).expect("open").1);
    }
    assert_eq!(hub.open(None, 0).err(), Some(StreamError::TooManyStreams));
    drop(rxs);
    for k in 0..10_u64 {
        hub.poll(k * 100 * MS);
    }
    assert_eq!(
        hub.stream_count(),
        0,
        "reaped with no status, no samples, no beat"
    );
    assert!(hub.open(None, 0).is_ok());
}

#[test]
fn a_blocked_dead_stream_is_reaped() {
    let mut hub = Hub::new();
    let (_, rx) = hub.open(None, 0).expect("open");
    for k in 0..3_u64 {
        hub.on_status(riding(u32::try_from(k).unwrap()), k * 100 * MS);
        hub.poll(k * 100 * MS);
    }
    drop(rx);
    // window full: nothing is attempted any more, yet the stream must go
    hub.poll(1_000 * MS);
    assert_eq!(hub.stream_count(), 0);
}

#[test]
fn non_finite_values_are_gaps_and_do_not_poison_the_chain() {
    // ADR-0001 §5: gaps are null, never interpolated. NaN would serialize as `null`
    // anyway (indistinguishable) and poison an exponential mean for the rest of the ride.
    let (mut hub, id, mut rx) = hub_with_stream();
    let ema = hub
        .subscribe(
            id,
            MetricId::Speed,
            opts(10.0, vec![Filter::Ema { tau_s: 1.0 }]),
        )
        .expect("sub");
    let mut out = Vec::new();
    for (k, v) in [100.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 100.0]
        .into_iter()
        .enumerate()
    {
        let t = (k + 1) as f64 / 4.0;
        hub.on_source(t, &[None, None, Some(v), None, None]);
        hub.poll((k as u64 + 1) * 250 * MS);
        for f in drain(&mut rx) {
            hub.ack(id, f.seq).expect("ack");
            out.extend(f.samples.into_iter().filter(|s| s.sub == ema).map(|s| s.v));
        }
    }
    // EMA with no new value holds nothing: non-finite inputs are missing samples.
    assert_eq!(out[0], Some(100.0));
    assert_eq!(out[1..4], [None, None, None]);
    assert_eq!(out[4], Some(100.0), "the chain recovered");
}

#[test]
fn a_chain_that_overflows_to_infinity_emits_a_gap() {
    // The finite input can still make a filter overflow (the sum of an SMA): the chain's
    // output is then non-finite, which is a gap too (ADR-0001 §5), never `inf`/`null`
    // by accident. Mutant: dropping the output filter in `Hub::on_source`.
    let (mut hub, id, mut rx) = hub_with_stream();
    let sma = hub
        .subscribe(
            id,
            MetricId::Speed,
            opts(10.0, vec![Filter::Sma { window_s: 4.0 }]),
        )
        .expect("sub");
    let mut out = Vec::new();
    for k in 1..=3_u64 {
        hub.on_source(k as f64 / 4.0, &[None, None, Some(f64::MAX), None, None]);
        hub.poll(k * 250 * MS);
        for f in drain(&mut rx) {
            hub.ack(id, f.seq).expect("ack");
            out.extend(f.samples.into_iter().filter(|s| s.sub == sma).map(|s| s.v));
        }
    }
    assert!(out.iter().all(|v| v.is_none_or(f64::is_finite)), "{out:?}");
    assert!(out.contains(&None), "the overflow is a gap: {out:?}");
}

#[test]
fn opening_a_stream_frees_the_closed_ones_before_the_cap_check() {
    // Unlabelled streams never evict, and nothing has polled since their receivers went:
    // only `open` itself can free the slots. Mutant: removing `reap()` from `open`.
    let mut hub = Hub::new();
    let rxs: Vec<_> = (0..MAX_STREAMS)
        .map(|_| hub.open(None, 0).expect("open").1)
        .collect();
    drop(rxs);
    assert!(
        hub.open(None, 0).is_ok(),
        "slots of dead receivers are free"
    );
    assert_eq!(hub.stream_count(), 1);
}

#[test]
fn the_worst_status_with_the_erg_fields_has_a_measured_size() {
    // The discrete part at every cap with #218's fields (following, component, released,
    // compliance at u32::MAX, cap) and the record error cut at its cap (256 bytes of
    // 4-byte characters): measured, against the 8 000-byte frame budget.
    let mut st = worst_status();
    st.record_error = Some("𝔘".repeat(MAX_RECORD_ERROR_BYTES / 4));
    let n = serde_json::to_vec(&st).expect("status").len();
    eprintln!("worst status: {n} bytes");
    assert!(n < 800, "worst status is {n} bytes");
    assert!(n < FRAME_BUDGET_BYTES / 10);
}

#[test]
fn the_wire_status_carries_the_erg_fields_with_their_meaning() {
    use crate::engine::{CapState, RideState, RideStatus};
    use crate::erg::{Compliance, Component, TrainerFollowing};
    use crate::ticks::{SessionTicks, WorkoutTicks};
    let ride = RideStatus {
        state: RideState::Riding,
        loaded: true,
        session: SessionTicks(0),
        workout: WorkoutTicks(0),
        block_index: None,
        block_elapsed: WorkoutTicks(0),
        trainer_link: None,
        hr_link: None,
        late_samples: 0,
        record_error: None,
        trainer_following: TrainerFollowing::NotFollowing,
        not_following_component: Some(Component::Track),
        erg_released: true,
        compliance: Compliance {
            counted_s: 3,
            within_s: 2,
            excluded_s: 1,
        },
        cap: Some(CapState {
            cap_w: 180,
            erg_only: true,
        }),
    };
    let v = serde_json::to_value(StreamStatus::from(&ride)).expect("status");
    assert_eq!(v["trainerFollowing"], "not_following");
    assert_eq!(v["notFollowingComponent"], "track");
    assert_eq!(v["ergReleased"], true);
    assert_eq!(v["compliance"]["countedS"], 3);
    assert_eq!(v["compliance"]["withinS"], 2);
    assert_eq!(v["compliance"]["excludedS"], 1);
    assert_eq!(v["cap"]["capW"], 180);
    assert_eq!(v["cap"]["ergOnly"], true);
    for (c, name) in [
        (Component::Erg, "erg"),
        (Component::Resistance, "resistance"),
        (Component::Wind, "wind"),
    ] {
        let mut r = ride.clone();
        r.not_following_component = Some(c);
        let v = serde_json::to_value(StreamStatus::from(&r)).expect("status");
        assert_eq!(v["notFollowingComponent"], name);
    }
    let mut calm = ride;
    calm.not_following_component = None;
    calm.erg_released = false;
    calm.cap = None;
    calm.trainer_following = TrainerFollowing::Checking;
    let v = serde_json::to_value(StreamStatus::from(&calm)).expect("status");
    assert_eq!(
        (
            &v["trainerFollowing"],
            &v["notFollowingComponent"],
            &v["ergReleased"],
            &v["cap"]
        ),
        (
            &"checking".into(),
            &serde_json::Value::Null,
            &false.into(),
            &serde_json::Value::Null
        )
    );
}

// ---- u32 ids and seq (#271) -----------------------------------------------------

#[test]
fn seq_wraps_at_u32_max_and_the_ack_window_follows() {
    let mut flow = Flow::starting_at(FRAME_BUDGET_BYTES, u32::MAX - 1);
    let mut seqs = Vec::new();
    // Three frames cross the wrap: MAX-1, MAX, 0. Each drain is one frame (a status).
    for k in 0..3_u64 {
        flow.offer_status(riding(u32::try_from(k).unwrap()));
        let f = flow.drain(k * MIN_FRAME_INTERVAL_US);
        assert_eq!(f.len(), 1);
        seqs.push(f[0].seq);
    }
    assert_eq!(
        seqs,
        [u32::MAX - 1, u32::MAX, 0],
        "consecutive across the wrap"
    );
    assert!(
        flow.blocked(),
        "three unacknowledged frames fill the window"
    );
    // A seq that was never sent is ignored, wherever it sits relative to the wrap.
    assert!(!flow.ack(1));
    assert!(!flow.ack(u32::MAX - 2));
    assert_eq!(flow.unacked(), 3);
    // Cumulative ack of the oldest, then of the frame after the wrap: the numerically
    // smallest seq (0) is the newest, so a `<`/`>` comparison would get this wrong.
    assert!(flow.ack(u32::MAX - 1));
    assert_eq!(flow.unacked(), 2);
    assert!(flow.ack(0));
    assert_eq!(flow.unacked(), 0, "acking 0 acknowledges MAX and 0");
    assert!(!flow.ack(0), "already acknowledged");
    // The next frame is 1.
    flow.offer_status(riding(9));
    assert_eq!(flow.drain(10 * MIN_FRAME_INTERVAL_US)[0].seq, 1);
}

#[test]
fn a_split_across_the_wrap_numbers_its_frames_consecutively() {
    let mut flow = Flow::starting_at(360, u32::MAX);
    for sub in 0..12_u32 {
        flow.offer_sample(sample(sub, 1.5, 100.0));
    }
    flow.offer_status(riding(1));
    let seqs: Vec<u32> = flow.drain(0).iter().map(|f| f.seq).collect();
    assert_eq!(seqs, [u32::MAX, 0, 1]);
}

#[test]
fn ids_skip_zero_and_live_ids_when_the_counter_wraps() {
    let mut next = u32::MAX;
    let live = |id: u32| id == 1 || id == 2;
    assert_eq!(fresh_id(&mut next, live), u32::MAX);
    assert_eq!(
        fresh_id(&mut next, live),
        3,
        "0 is never issued; live 1 and 2 are skipped"
    );
    assert_eq!(fresh_id(&mut next, live), 4);
}

#[test]
fn stream_and_subscription_ids_stay_unique_across_a_counter_wrap() {
    let mut hub = Hub::new();
    hub.counters_at(u32::MAX, u32::MAX);
    let (a, _ra) = hub.open(None, 0).unwrap();
    let (b, _rb) = hub.open(None, 0).unwrap();
    assert_eq!((a, b), (StreamId(u32::MAX), StreamId(1)));
    let opts = || SubscribeOpts {
        rate_hz: 1.0,
        filters: Vec::new(),
    };
    let s1 = hub.subscribe(a, MetricId::Power, opts()).unwrap();
    let s2 = hub.subscribe(b, MetricId::Power, opts()).unwrap();
    let s3 = hub.subscribe(a, MetricId::Hr, opts()).unwrap();
    assert_eq!((s1, s2, s3), (SubId(u32::MAX), SubId(1), SubId(2)));
    // Close a stream and wrap the counter again: a live id is never reissued.
    hub.counters_at(1, 1);
    let (c, _rc) = hub.open(None, 0).unwrap();
    assert_eq!(c, StreamId(2), "stream 1 is live");
    let s4 = hub.subscribe(c, MetricId::Cadence, opts()).unwrap();
    assert_eq!(s4, SubId(3), "subscriptions 1 and 2 are live");
}
