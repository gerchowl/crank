//! Per-stream flow control (ADR-0001 §4.3): a pure state machine over microsecond
//! monotonic time, so it is tested without an engine or a runtime.
//!
//! The consumer has no backpressure of its own, so the core bounds the outstanding work
//! explicitly:
//!
//! * at most [`MAX_UNACKED`] frames are unacknowledged; while the window is full nothing
//!   is sent and new state **coalesces** into bounded pending state (per subscription
//!   only the latest sample, the latest status, the latest state of each link);
//! * whatever coalesced while blocked is flagged: the next frame carries `resync: true`;
//! * every frame is serialized and measured: over [`FRAME_BUDGET_BYTES`] the samples
//!   are split over consecutive frames, and discrete state that alone exceeds the
//!   budget is dropped for a bare resync frame;
//! * frames are at most [`MIN_FRAME_INTERVAL_US`] apart (≤ 10 Hz).
use super::types::{
    LinkChange, LinkDevice, LinkStatus, StreamFrame, StreamStatus, SubId, SubSample,
};
use std::collections::{BTreeMap, VecDeque};

/// Unacknowledged frames a stream may have in flight.
pub(crate) const MAX_UNACKED: usize = 3;
/// Serialized-JSON byte budget of a frame: below Tauri's 8 192-byte direct-eval
/// threshold, leaving room for the IPC envelope.
pub(crate) const FRAME_BUDGET_BYTES: usize = 8_000;
/// Frames of one stream are at least this far apart: 10 Hz.
pub(crate) const MIN_FRAME_INTERVAL_US: u64 = 100_000;
/// Status is refreshed this often while a ride is in progress: 2 Hz.
pub(crate) const HEARTBEAT_US: u64 = 500_000;

fn json_len<T: serde::Serialize>(v: &T) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// Flow-control state of one stream.
pub(crate) struct Flow {
    budget: usize,
    next_seq: u32,
    unacked: VecDeque<u32>,
    samples: BTreeMap<SubId, SubSample>,
    status: Option<StreamStatus>,
    links: BTreeMap<LinkDevice, LinkStatus>,
    resync: bool,
    last_emit_us: Option<u64>,
}
impl Flow {
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            budget,
            next_seq: 0,
            unacked: VecDeque::new(),
            samples: BTreeMap::new(),
            status: None,
            links: BTreeMap::new(),
            resync: false,
            last_emit_us: None,
        }
    }
    /// A flow whose first frame carries `seq` (wrap tests).
    #[cfg(test)]
    pub(crate) fn starting_at(budget: usize, seq: u32) -> Self {
        Self {
            next_seq: seq,
            ..Self::new(budget)
        }
    }
    /// The ack window is full: nothing is sent and every replacement is a coalescing.
    pub(crate) fn blocked(&self) -> bool {
        self.unacked.len() >= MAX_UNACKED
    }
    /// Frames in flight.
    #[cfg(test)]
    pub(crate) fn unacked(&self) -> usize {
        self.unacked.len()
    }
    /// Pending entries: samples, status, links.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.samples.len() + usize::from(self.status.is_some()) + self.links.len()
    }
    /// A pending sample, for tests.
    #[cfg(test)]
    pub(crate) fn pending_sample(&self, sub: SubId) -> Option<SubSample> {
        self.samples.get(&sub).copied()
    }
    fn has_pending(&self) -> bool {
        self.resync || !self.samples.is_empty() || self.status.is_some() || !self.links.is_empty()
    }
    /// Latest value of a subscription replaces the pending one.
    pub(crate) fn offer_sample(&mut self, s: SubSample) {
        if self.samples.insert(s.sub, s).is_some() && self.blocked() {
            self.resync = true;
        }
    }
    /// Latest status replaces the pending one.
    pub(crate) fn offer_status(&mut self, s: StreamStatus) {
        if self.status.replace(s).is_some() && self.blocked() {
            self.resync = true;
        }
    }
    /// Latest state of a link replaces the pending one.
    pub(crate) fn offer_link(&mut self, device: LinkDevice, state: LinkStatus) {
        if self.links.insert(device, state).is_some() && self.blocked() {
            self.resync = true;
        }
    }
    /// The subscription ended: its pending sample goes with it.
    pub(crate) fn forget_sub(&mut self, sub: SubId) {
        self.samples.remove(&sub);
    }
    /// The consumer applied every frame up to and including `seq`. A `seq` that is not
    /// outstanding (never sent, or already acknowledged) is ignored. Outstanding frames
    /// are found by position in send order, never by comparing sequence numbers, so the
    /// `u32::MAX` → 0 wrap needs no special case. Returns whether anything was
    /// acknowledged.
    pub(crate) fn ack(&mut self, seq: u32) -> bool {
        match self.unacked.iter().position(|&s| s == seq) {
            Some(i) => {
                self.unacked.drain(..=i);
                true
            }
            None => false,
        }
    }
    /// Earliest instant [`Flow::drain`] can emit, `None` when nothing is pending or the
    /// window is full (an ack, not time, reopens it).
    pub(crate) fn next_emit_at(&self, now_us: u64) -> Option<u64> {
        (self.has_pending() && !self.blocked()).then(|| {
            self.last_emit_us
                .map_or(now_us, |l| (l + MIN_FRAME_INTERVAL_US).max(now_us))
        })
    }
    /// The frames to send at `now_us`: none while blocked or inside the 10 Hz interval.
    pub(crate) fn drain(&mut self, now_us: u64) -> Vec<StreamFrame> {
        if self.next_emit_at(now_us) != Some(now_us) {
            return Vec::new();
        }
        let links: Vec<LinkChange> = std::mem::take(&mut self.links)
            .into_iter()
            .map(|(device, state)| LinkChange { device, state })
            .collect();
        let head = StreamFrame {
            seq: self.next_seq,
            resync: self.resync,
            samples: Vec::new(),
            status: self.status.take(),
            links,
        };
        self.resync = false;
        let mut frames = Vec::new();
        let mut size = json_len(&head);
        let mut cur = head;
        if size > self.budget {
            // Discrete state alone is over budget: the fixed-size resync frame, and the
            // discrete state is dropped; the window re-reads it by query.
            cur.resync = true;
            cur.status = None;
            cur.links = Vec::new();
            frames.push(cur);
        } else {
            // Samples fill the first frame and the following ones, each re-measured; a
            // sample that does not fit when the window has no room stays pending.
            let due: Vec<SubSample> = self.samples.values().copied().collect();
            for s in due {
                let add = json_len(&s) + usize::from(!cur.samples.is_empty());
                if size + add > self.budget {
                    if self.unacked.len() + frames.len() + 1 >= MAX_UNACKED {
                        break;
                    }
                    let seq = cur.seq.wrapping_add(1);
                    frames.push(cur);
                    cur = StreamFrame {
                        seq,
                        resync: false,
                        samples: Vec::new(),
                        status: None,
                        links: Vec::new(),
                    };
                    size = json_len(&cur);
                    cur.samples.push(s);
                    size += json_len(&s);
                } else {
                    cur.samples.push(s);
                    size += add;
                }
                self.samples.remove(&s.sub);
            }
            frames.push(cur);
        }
        for f in &frames {
            self.unacked.push_back(f.seq);
        }
        // `seq` wraps by design (see `StreamFrame::seq`); a drain emits a handful of frames.
        self.next_seq = self
            .next_seq
            .wrapping_add(u32::try_from(frames.len()).unwrap_or(u32::MAX));
        self.last_emit_us = Some(now_us);
        frames
    }
}
