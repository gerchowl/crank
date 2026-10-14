//! Contract types of the stream layer (ADR-0001 §4.1, §4.3). Serialized with the
//! conventions of §4.1: camelCase structs, `snake_case` tagged enums, seconds as `f64`,
//! `Option` as `T | null`.
use crate::engine::{PauseReason, RideState, RideStatus};
use crate::ticks::ticks_to_seconds;
use crank_devices::LinkState;
use crank_domain::filters::Filter;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stream: one per window, multiplexing every subscription of that window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StreamId(pub u64);
/// A subscription on a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SubId(pub u64);

/// A source metric the engine publishes every tick: the record's catalogue names
/// ([`crate::METRICS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum MetricId {
    /// Trainer power, W.
    Power,
    /// Cadence, rpm.
    Cadence,
    /// Speed, m/s.
    Speed,
    /// Heart rate, bpm.
    Hr,
    /// Workout position, s (null while paused).
    WorkoutS,
}
impl MetricId {
    /// Index into the per-tick source vector, in [`crate::METRICS`] order.
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Power => 0,
            Self::Cadence => 1,
            Self::Speed => 2,
            Self::Hr => 3,
            Self::WorkoutS => 4,
        }
    }
}

/// Options of one subscription.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SubscribeOpts {
    /// Delivery rate in session time, [`SubscribeOpts::MIN_RATE_HZ`] to
    /// [`SubscribeOpts::MAX_RATE_HZ`] (ADR-0001 §4.3).
    pub rate_hz: f64,
    /// The subscription's own filter chain, built from `crank-domain`'s filters and fed
    /// every source sample, whatever the delivery rate.
    pub filters: Vec<Filter>,
}
/// Filters per subscription chain. DESIGN's widgets use one display filter each
/// (`shared/src/layouts.ts`: a single `ema`; the default is `power_3s`); the cap leaves
/// room for a median in front of a mean and a hold.
pub const MAX_FILTERS_PER_CHAIN: usize = 4;
/// Longest SMA window and EMA time constant, seconds. The longest display filter in
/// the spike is 10 s (`shared/src/layouts.ts`); 300 s is 30× that and keeps an SMA to
/// 1 200 samples at the 4 Hz source.
pub const MAX_FILTER_SECONDS: f64 = 300.0;
/// Largest median sample count: 64 s of the 4 Hz source.
pub const MAX_MEDIAN_SAMPLES: f64 = 256.0;

impl SubscribeOpts {
    /// Lowest delivery rate.
    pub const MIN_RATE_HZ: f64 = 0.1;
    /// Highest delivery rate: the display maximum.
    pub const MAX_RATE_HZ: f64 = 10.0;
}

/// Latest value of one subscription. `t` is session seconds (ADR-0001 §5); `v` is
/// `null` for a gap.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SubSample {
    /// The subscription.
    pub sub: SubId,
    /// Session seconds of the source sample.
    pub t: f64,
    /// Filtered value, `null` for a gap.
    pub v: Option<f64>,
}

/// Ride state as the stream reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamRideState {
    /// Nothing started.
    Idle,
    /// Workout and session time advance.
    Riding,
    /// Workout time is frozen.
    Paused {
        /// Why: `user`, `trainer_lost` or `stopped_pedalling`.
        reason: StreamPauseReason,
    },
    /// Ended.
    Finished,
}
/// Why a ride is paused (ADR-0001 §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StreamPauseReason {
    /// The rider paused.
    User,
    /// The trainer link or its samples were lost.
    TrainerLost,
    /// Cadence and power were 0 for 3 s.
    StoppedPedalling,
}
/// Readiness of a device link (ADR-0002 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    /// Establishing.
    Connecting,
    /// Ready.
    Ready,
    /// Lost or released.
    Lost,
}
impl From<LinkState> for LinkStatus {
    fn from(s: LinkState) -> Self {
        match s {
            LinkState::Connecting => Self::Connecting,
            LinkState::Ready => Self::Ready,
            LinkState::Lost => Self::Lost,
        }
    }
}
/// Which device a link change is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum LinkDevice {
    /// The trainer.
    Trainer,
    /// The heart-rate strap.
    Hr,
}
/// The latest state of one device link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LinkChange {
    /// The device.
    pub device: LinkDevice,
    /// Its link state.
    pub state: LinkStatus,
}

/// Longest `record_error` a frame carries, in bytes; the full message is in
/// [`RideStatus`]. Keeps the discrete part of a frame bounded (ADR-0001 §4.3).
pub const MAX_RECORD_ERROR_BYTES: usize = 256;

/// The ride status of a frame: [`RideStatus`] in its wire form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    /// Ride state.
    pub state: StreamRideState,
    /// A ride is loaded.
    pub loaded: bool,
    /// Session time, s.
    pub session_s: f64,
    /// Workout time, s.
    pub workout_s: f64,
    /// Current block, `null` without blocks or after the last.
    pub block_index: Option<u32>,
    /// Workout seconds in the current block.
    pub block_elapsed_s: f64,
    /// Samples that arrived after their tick (dropped).
    pub late_samples: u64,
    /// First recording failure, cut to [`MAX_RECORD_ERROR_BYTES`].
    pub record_error: Option<String>,
}
impl From<&RideStatus> for StreamStatus {
    fn from(s: &RideStatus) -> Self {
        let state = match s.state {
            RideState::Idle => StreamRideState::Idle,
            RideState::Riding => StreamRideState::Riding,
            RideState::Finished => StreamRideState::Finished,
            RideState::Paused(r) => StreamRideState::Paused {
                reason: match r {
                    PauseReason::User => StreamPauseReason::User,
                    PauseReason::TrainerLost => StreamPauseReason::TrainerLost,
                    PauseReason::StoppedPedalling => StreamPauseReason::StoppedPedalling,
                },
            },
        };
        Self {
            state,
            loaded: s.loaded,
            session_s: ticks_to_seconds(s.session.0),
            workout_s: ticks_to_seconds(s.workout.0),
            block_index: s.block_index.and_then(|i| u32::try_from(i).ok()),
            block_elapsed_s: ticks_to_seconds(s.block_elapsed.0),
            late_samples: s.late_samples,
            record_error: s.record_error.as_ref().map(|e| {
                let mut end = e.len().min(MAX_RECORD_ERROR_BYTES);
                while !e.is_char_boundary(end) {
                    end -= 1;
                }
                e[..end].to_owned()
            }),
        }
    }
}
impl StreamStatus {
    /// A ride is in progress (riding or paused): the heartbeat runs.
    #[must_use]
    pub fn in_progress(&self) -> bool {
        matches!(
            self.state,
            StreamRideState::Riding | StreamRideState::Paused { .. }
        )
    }
    /// Equal in everything but the clock fields (`session_s`, `workout_s`,
    /// `block_elapsed_s`), which advance every tick and are refreshed by the heartbeat
    /// instead of counting as a change.
    #[must_use]
    pub fn same_discrete(&self, other: &Self) -> bool {
        self.state == other.state
            && self.loaded == other.loaded
            && self.block_index == other.block_index
            && self.late_samples == other.late_samples
            && self.record_error == other.record_error
    }
}

/// One message to a window (ADR-0001 §4.3). Everything in a frame happened before
/// everything in the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    /// Per stream, +1 per frame.
    pub seq: u64,
    /// Frames were coalesced or discrete state dropped: re-query durable state.
    pub resync: bool,
    /// Latest value of each subscription that is due, at most one per subscription.
    pub samples: Vec<SubSample>,
    /// On change, and every 500 ms while a ride is in progress.
    pub status: Option<StreamStatus>,
    /// Latest state of each device link, if changed.
    pub links: Vec<LinkChange>,
}

/// The durable state a window re-reads after a `resync` or a `seq` gap
/// (`stream_snapshot`, ADR-0001 §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamSnapshot {
    /// Latest value of every subscription.
    pub samples: Vec<SubSample>,
    /// Latest status.
    pub status: Option<StreamStatus>,
    /// Latest state of each device link.
    pub links: Vec<LinkChange>,
}

/// A stream command that cannot apply.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamError {
    /// No such stream (never opened, or dropped with its window).
    #[error("unknown stream")]
    UnknownStream,
    /// No such subscription on the stream.
    #[error("unknown subscription")]
    UnknownSub,
    /// More than 64 subscriptions on the stream (`Conflict`, ADR-0001 §4.3).
    #[error("too many subscriptions")]
    TooManySubscriptions,
    /// More than 16 live streams on the engine (`Conflict`).
    #[error("too many streams")]
    TooManyStreams,
    /// `rate_hz` is outside 0.1–10 Hz or not finite.
    #[error("rate must be within 0.1..=10 Hz")]
    InvalidRate,
    /// A filter parameter is invalid.
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
    /// The engine task has ended.
    #[error("engine stopped")]
    EngineStopped,
}
