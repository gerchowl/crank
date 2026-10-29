//! Outbound streams (ADR-0001 §4.3, §5): subscriptions with filter chains, the
//! multiplexed [`StreamFrame`], flow control, status heartbeat (#219).
//!
//! * [`types`]: the contract types a window sees.
//! * `flow`: per-stream flow control, a pure state machine (≤ 3 unacked frames,
//!   coalescing, resync, the 8 000-byte frame budget).
//! * `hub`: the streams of an engine: subscriptions, filter chains, window cleanup,
//!   heartbeat. The engine calls it synchronously and never awaits a subscriber.
//!
//! Not here: laps (#12), thresholds and flags (#13), the cursor's actual block start
//! (#14), nudges (the judge hook): all #243. The IPC transport is #34.
pub(crate) mod flow;
pub(crate) mod hub;
pub mod types;
pub(crate) mod work;

pub use types::{
    LinkChange, LinkDevice, LinkStatus, MetricId, StreamFrame, StreamId, StreamRideState,
    StreamSnapshot, StreamStatus, SubId, SubSample, SubscribeOpts,
};

#[cfg(test)]
mod tests;
