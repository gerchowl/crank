//! The streams of one engine: subscriptions with filter chains, window cleanup, status
//! heartbeat (ADR-0001 §4.3).
//!
//! Everything here is synchronous and called from the engine task. A frame goes out
//! through `try_send` on a channel that holds more than the ack window, so a slow or
//! dead consumer is bounded by [`super::flow`] and can never block a tick or the record
//! writer; a failed send drops the stream.
use super::flow::{FRAME_BUDGET_BYTES, Flow, HEARTBEAT_US};
use super::types::{
    LinkChange, LinkDevice, LinkStatus, MAX_FILTER_SECONDS, MAX_FILTERS_PER_CHAIN,
    MAX_MEDIAN_SAMPLES, MetricId, StreamError, StreamFrame, StreamId, StreamSnapshot, StreamStatus,
    SubId, SubSample, SubscribeOpts,
};
use super::work::{Counted, Work};
use crank_domain::filters::{Filter, FilterChain};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

/// Subscriptions per stream; `subscribe` returns `Conflict` beyond it.
pub(crate) const MAX_SUBSCRIPTIONS: usize = 64;
/// Live streams per engine: a main window, a few secondary windows and headroom for a
/// reload racing its predecessor's cleanup.
pub(crate) const MAX_STREAMS: usize = 16;
/// Source metrics published per tick, in [`crate::METRICS`] order.
pub(crate) const SOURCES: usize = 6;

struct Sub {
    metric: MetricId,
    period_s: f64,
    chain: FilterChain,
    last_sent_t: Option<f64>,
    latest: Option<SubSample>,
}
struct Stream {
    window: Option<String>,
    sink: mpsc::Sender<StreamFrame>,
    subs: Counted<SubId, Sub>,
    flow: Flow,
    last_status_us: u64,
}

/// The next id of a per-process `u32` counter that starts at 1: it never issues 0 and,
/// when the counter wraps, skips every id for which `live` is true, so ids that are live
/// are unique. Terminates because at most 16 streams and 64 subscriptions each are live.
pub(crate) fn fresh_id(next: &mut u32, live: impl Fn(u32) -> bool) -> u32 {
    loop {
        let id = *next;
        *next = id.checked_add(1).unwrap_or(1);
        if id != 0 && !live(id) {
            return id;
        }
    }
}

/// All streams of an engine and the latest discrete state they are fed from.
pub(crate) struct Hub {
    budget: usize,
    next_stream: u32,
    next_sub: u32,
    streams: BTreeMap<StreamId, Stream>,
    status: Option<StreamStatus>,
    links: BTreeMap<LinkDevice, LinkStatus>,
    /// Work units counted for the cost tests (#293); zero-sized outside tests.
    work: Work,
}
impl Hub {
    pub(crate) fn new() -> Self {
        Self::with_budget(FRAME_BUDGET_BYTES)
    }
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            next_stream: 1,
            next_sub: 1,
            streams: BTreeMap::new(),
            status: None,
            links: BTreeMap::new(),
            work: Work::default(),
        }
    }
    /// Counters positioned for wrap tests.
    #[cfg(test)]
    pub(crate) fn counters_at(&mut self, stream: u32, sub: u32) {
        self.next_stream = stream;
        self.next_sub = sub;
    }
    /// Open a stream for `window`, dropping that window's previous streams (a page
    /// reload). The new stream starts with the current status and links.
    pub(crate) fn open(
        &mut self,
        window: Option<String>,
        now_us: u64,
    ) -> Result<(StreamId, mpsc::Receiver<StreamFrame>), StreamError> {
        // Only a window label identifies a page; unlabelled streams never evict.
        if window.is_some() {
            self.streams.retain(|_, s| s.window != window);
        }
        self.reap();
        if self.streams.len() >= MAX_STREAMS {
            return Err(StreamError::TooManyStreams);
        }
        let (sink, rx) = mpsc::channel(super::flow::MAX_UNACKED + 1);
        let id = StreamId(fresh_id(&mut self.next_stream, |id| {
            self.streams.contains_key(&StreamId(id))
        }));
        let mut flow = Flow::with_work(self.budget, self.work.clone());
        if let Some(s) = &self.status {
            flow.offer_status(s.clone());
        }
        for (&device, &state) in &self.links {
            flow.offer_link(device, state);
        }
        self.streams.insert(
            id,
            Stream {
                window,
                sink,
                subs: Counted::new(self.work.clone()),
                flow,
                last_status_us: now_us,
            },
        );
        Ok((id, rx))
    }
    /// Drop streams whose receiver is gone, whether or not they have anything to send.
    fn reap(&mut self) {
        self.streams.retain(|_, s| !s.sink.is_closed());
    }
    /// Drop every stream of a window (its `Destroyed` event).
    pub(crate) fn close_window(&mut self, window: &str) {
        self.streams
            .retain(|_, s| s.window.as_deref() != Some(window));
    }
    pub(crate) fn close(&mut self, stream: StreamId) {
        self.streams.remove(&stream);
    }
    pub(crate) fn subscribe(
        &mut self,
        stream: StreamId,
        metric: MetricId,
        opts: SubscribeOpts,
    ) -> Result<SubId, StreamError> {
        let s = self
            .streams
            .get(&stream)
            .ok_or(StreamError::UnknownStream)?;
        if s.subs.len() >= MAX_SUBSCRIPTIONS {
            return Err(StreamError::TooManySubscriptions);
        }
        if !(SubscribeOpts::MIN_RATE_HZ..=SubscribeOpts::MAX_RATE_HZ).contains(&opts.rate_hz) {
            return Err(StreamError::InvalidRate);
        }
        validate_filters(&opts.filters)?;
        let chain = FilterChain::new(opts.filters)
            .map_err(|e| StreamError::InvalidFilter(e.to_string()))?;
        let streams = &self.streams;
        let id = SubId(fresh_id(&mut self.next_sub, |id| {
            streams.values().any(|st| st.subs.contains_key(&SubId(id)))
        }));
        let s = self
            .streams
            .get_mut(&stream)
            .ok_or(StreamError::UnknownStream)?;
        s.subs.insert(
            id,
            Sub {
                metric,
                period_s: 1.0 / opts.rate_hz,
                chain,
                last_sent_t: None,
                latest: None,
            },
        );
        Ok(id)
    }
    pub(crate) fn unsubscribe(&mut self, stream: StreamId, sub: SubId) -> Result<(), StreamError> {
        let s = self
            .streams
            .get_mut(&stream)
            .ok_or(StreamError::UnknownStream)?;
        s.subs.remove(&sub).ok_or(StreamError::UnknownSub)?;
        s.flow.forget_sub(sub);
        Ok(())
    }
    pub(crate) fn ack(&mut self, stream: StreamId, seq: u32) -> Result<(), StreamError> {
        let s = self
            .streams
            .get_mut(&stream)
            .ok_or(StreamError::UnknownStream)?;
        s.flow.ack(seq);
        Ok(())
    }
    /// The durable state behind `resync`: latest value of every subscription, status, links.
    pub(crate) fn snapshot(&self, stream: StreamId) -> Result<StreamSnapshot, StreamError> {
        let s = self
            .streams
            .get(&stream)
            .ok_or(StreamError::UnknownStream)?;
        Ok(StreamSnapshot {
            samples: s.subs.values().filter_map(|b| b.latest).collect(),
            status: self.status.clone(),
            links: self
                .links
                .iter()
                .map(|(&device, &state)| LinkChange { device, state })
                .collect(),
        })
    }
    /// One source sample per metric at session second `t`: every chain sees it; a
    /// subscription's latest value is offered when its rate makes it due.
    pub(crate) fn on_source(&mut self, t: f64, values: &[Option<f64>; SOURCES]) {
        for s in self.streams.values_mut() {
            for (&id, sub) in s.subs.iter_mut() {
                // A non-finite value is a gap (ADR-0001 §5: gaps are null, never
                // interpolated): it would otherwise poison an exponential mean for good
                // and serialize as `null` anyway, indistinguishable from a gap.
                let input = values[sub.metric.index()].filter(|v| v.is_finite());
                self.work.add(1);
                let v = sub.chain.push(t, input).filter(|v| v.is_finite());
                let sample = SubSample { sub: id, t, v };
                sub.latest = Some(sample);
                if sub.last_sent_t.is_none_or(|l| t - l >= sub.period_s - 1e-9) {
                    sub.last_sent_t = Some(t);
                    s.flow.offer_sample(sample);
                }
            }
        }
    }
    /// The engine's status after any change. Sent at once when anything but the clock
    /// fields changed; the clock fields ride on the heartbeat.
    pub(crate) fn on_status(&mut self, status: StreamStatus, now_us: u64) {
        let changed = self
            .status
            .as_ref()
            .is_none_or(|p| !p.same_discrete(&status));
        if changed {
            for s in self.streams.values_mut() {
                s.flow.offer_status(status.clone());
                s.last_status_us = now_us;
            }
        }
        self.status = Some(status);
    }
    /// A link state; sent when it differs from the last one.
    pub(crate) fn on_link(&mut self, device: LinkDevice, state: LinkStatus) {
        if self.links.insert(device, state) != Some(state) {
            for s in self.streams.values_mut() {
                s.flow.offer_link(device, state);
            }
        }
    }
    /// Heartbeat, then send what is due. Streams whose sink is closed or full are dropped.
    pub(crate) fn poll(&mut self, now_us: u64) {
        self.reap();
        let beat = self.status.as_ref().filter(|s| s.in_progress());
        let mut dead = Vec::new();
        for (&id, s) in &mut self.streams {
            if let Some(status) = beat
                && !s.flow.blocked()
                && now_us.saturating_sub(s.last_status_us) >= HEARTBEAT_US
            {
                s.flow.offer_status(status.clone());
                s.last_status_us = now_us;
            }
            for frame in s.flow.drain(now_us) {
                if s.sink.try_send(frame).is_err() {
                    dead.push(id);
                    break;
                }
            }
        }
        for id in dead {
            self.streams.remove(&id);
        }
    }
    /// Earliest instant [`Hub::poll`] has something to do, for the engine's sleep.
    pub(crate) fn next_due_us(&self, now_us: u64) -> Option<u64> {
        let beating = self.status.as_ref().is_some_and(StreamStatus::in_progress);
        self.streams
            .values()
            .flat_map(|s| {
                let emit = s.flow.next_emit_at(now_us);
                let beat = (beating && !s.flow.blocked())
                    .then(|| (s.last_status_us + HEARTBEAT_US).max(now_us));
                [emit, beat]
            })
            .flatten()
            .min()
    }
    #[cfg(test)]
    pub(crate) fn work(&self) -> usize {
        self.work.total()
    }
    #[cfg(test)]
    pub(crate) fn flow(&self, stream: StreamId) -> Option<&Flow> {
        self.streams.get(&stream).map(|s| &s.flow)
    }
    #[cfg(test)]
    pub(crate) fn stream_count(&self) -> usize {
        self.streams.len()
    }
}

/// Chain length and window caps: unbounded `n` or `window_s` would keep every sample and
/// make the tick that writes the record pay for it.
fn validate_filters(filters: &[Filter]) -> Result<(), StreamError> {
    let bad = |what: &str| Err(StreamError::InvalidFilter(what.to_owned()));
    if filters.len() > MAX_FILTERS_PER_CHAIN {
        return bad("chain longer than 4 filters");
    }
    for f in filters {
        match *f {
            Filter::Sma { window_s: s } | Filter::Ema { tau_s: s } if s > MAX_FILTER_SECONDS => {
                return bad("window or time constant over 300 s");
            }
            Filter::Median { n } if n > MAX_MEDIAN_SAMPLES => {
                return bad("median over 256 samples");
            }
            _ => {}
        }
    }
    Ok(())
}
