//! Contract types of the stream layer (ADR-0001 §4.1, §4.3). Serialized with the
//! conventions of §4.1: camelCase structs, `snake_case` tagged enums, seconds as `f64`,
//! `Option` as `T | null`.
use crate::engine::{PauseReason, RideState, RideStatus};
use crate::erg::{Component, TrainerFollowing};
use crate::ticks::ticks_to_seconds;
use crank_devices::{ControlMode, LinkState};
use crank_domain::filters::Filter;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stream: one per window, multiplexing every subscription of that window.
///
/// Ids are `u32` because TypeScript cannot carry a `u64` without precision loss
/// (specta-typescript forbids it, #271). They are per-process counters starting at 1 and
/// never 0. A counter that wraps skips every id still live (at most 16 streams), so live
/// ids are always unique; a stale id from before a wrap would need 2^32 stream opens in
/// one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StreamId(pub u32);
/// A subscription on a stream. A per-process `u32` counter with the same bounds and
/// wrap rule as [`StreamId`] (live ids, at most 16 × 64, are skipped on wrap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SubId(pub u32);

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

/// Whether the trainer follows the commanded mode (ADR-0002 §4.4.1), on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StreamFollowing {
    /// No claim.
    Unknown,
    /// Waiting for evidence.
    Checking,
    /// A disagreement is open.
    Suspect,
    /// Verified.
    Following,
    /// A required component is not following or silent.
    NotFollowing,
}
impl From<TrainerFollowing> for StreamFollowing {
    fn from(f: TrainerFollowing) -> Self {
        match f {
            TrainerFollowing::Unknown => Self::Unknown,
            TrainerFollowing::Checking => Self::Checking,
            TrainerFollowing::Suspect => Self::Suspect,
            TrainerFollowing::Following => Self::Following,
            TrainerFollowing::NotFollowing => Self::NotFollowing,
        }
    }
}
/// A commanded component (the control page the trainer echoes), on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StreamComponent {
    /// Page 49.
    Erg,
    /// Page 48.
    Resistance,
    /// Page 51: grade and Crr.
    Track,
    /// Page 50: wind.
    Wind,
}
impl From<Component> for StreamComponent {
    fn from(c: Component) -> Self {
        match c {
            Component::Erg => Self::Erg,
            Component::Resistance => Self::Resistance,
            Component::Track => Self::Track,
            Component::Wind => Self::Wind,
        }
    }
}
/// The mode the engine has commanded the trainer into, on the wire (#324). The rider-facing
/// reading of why there is no ERG target comes with it: `resistance` with `ergReleased` is a
/// release, with `userPaused` a pause, and with neither the end mode.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamCommanded {
    /// ERG: the trainer holds this power (page 49). `watts` is what was sent (after the soft
    /// start, the health cap and rounding), not the plan.
    Erg {
        /// Target watts.
        watts: u16,
    },
    /// Basic resistance (page 48), percent; 0 for a release, a rider pause and the end mode.
    Resistance {
        /// Resistance percent, 0..=100.
        pct: f32,
    },
    /// Explicit flat-road simulation for a free block (ADR-0002 §7.9).
    Free,
    /// A track simulation. No policy sends it yet; it is here so the mapping is total.
    Sim,
}
impl From<ControlMode> for StreamCommanded {
    fn from(m: ControlMode) -> Self {
        match m {
            ControlMode::Erg { target_w } => Self::Erg { watts: target_w },
            ControlMode::Resistance { pct } => Self::Resistance { pct },
            ControlMode::Free => Self::Free,
            ControlMode::Sim { .. } => Self::Sim,
        }
    }
}
/// The compliance tally in whole seconds (ADR-0002 §7.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamCompliance {
    /// Seconds that count.
    pub counted_s: u32,
    /// Of those, seconds within ±5 % of the controlled target.
    pub within_s: u32,
    /// Seconds excluded (settle, released, paused, suspect or not following).
    pub excluded_s: u32,
}
/// The active health cap (DESIGN §15.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamCap {
    /// Cap in watts.
    pub cap_w: u16,
    /// Only ERG blocks are allowed.
    pub erg_only: bool,
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
    /// Samples that arrived after their tick (dropped), saturating at `u32::MAX` (the
    /// engine counts in `u64`; TypeScript cannot carry it, #271).
    pub late_samples: u32,
    /// First recording failure, cut to [`MAX_RECORD_ERROR_BYTES`].
    pub record_error: Option<String>,
    /// Whether the trainer follows the commanded mode.
    pub trainer_following: StreamFollowing,
    /// The required component that is not following or silent, if any.
    pub not_following_component: Option<StreamComponent>,
    /// ERG is released for low cadence.
    pub erg_released: bool,
    /// The compliance tally.
    pub compliance: StreamCompliance,
    /// The active health cap; `null` when none applies.
    pub cap: Option<StreamCap>,
    /// The loss fields below describe the running ride: after it has finished they are
    /// `false`/`null`.
    ///
    /// The rider has paused the ride and not resumed, also while `state` says `trainer_lost`
    /// (that reason outranks the rider's in `state`; this field does not).
    pub user_paused: bool,
    /// Session second at which the current trainer loss began, `null` while the trainer is not
    /// lost. A timestamp on the ride's clock, not an elapsed time, so the status does not change
    /// every tick: "lost for" is `session_s - trainer_lost_since_s`, from the session time the
    /// heartbeat refreshes (DESIGN §6.5/§6.6).
    ///
    /// **Key "the trainer is lost" on this field, not on `state`.** When the trainer comes back
    /// while the rider is not pedalling, `state` stays `paused(trainer_lost)` (the ride resumes on
    /// the first pedalling sample) while this is already `null` and `last_gap_s` is set.
    pub trainer_lost_since_s: Option<f64>,
    /// The driver's reconnect attempt in the current loss (1-based); `null` when not lost or
    /// before the first attempt.
    pub reconnect_attempt: Option<u32>,
    /// While the soft start after a recovered loss runs: the target, W, the trainer is being
    /// eased back to; `null` otherwise. It follows the block's target line, so on a ramp block it
    /// can change as often as once per tick (at most 4 per second), but only for the soft start's
    /// 20 ticks (5 s): bounded per loss.
    pub soft_start_target_w: Option<u16>,
    /// Length of the last trainer loss, s, from the loss until the trainer was back; `null`
    /// until one has ended and again once the next one begins.
    pub last_gap_s: Option<f64>,
    /// Session second at which the last loss ended (with `last_gap_s`).
    pub reconnected_at_s: Option<f64>,
    /// What the engine has commanded the trainer into (#324): the ERG setpoint actually sent,
    /// or the reason there is none (`resistance`, `free`). `null` outside a running ride and
    /// while the trainer is being re-told after a loss. Not the plan. Changes at most once per
    /// tick (the engine sends at most one mode per tick), and in practice once per second at
    /// most: a ramp block and the soft start move in 1 Hz steps.
    pub commanded: Option<StreamCommanded>,
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
            late_samples: u32::try_from(s.late_samples).unwrap_or(u32::MAX),
            record_error: s.record_error.as_ref().map(|e| {
                let mut end = e.len().min(MAX_RECORD_ERROR_BYTES);
                while !e.is_char_boundary(end) {
                    end -= 1;
                }
                e[..end].to_owned()
            }),
            trainer_following: s.trainer_following.into(),
            not_following_component: s.not_following_component.map(Into::into),
            erg_released: s.erg_released,
            compliance: StreamCompliance {
                counted_s: s.compliance.counted_s,
                within_s: s.compliance.within_s,
                excluded_s: s.compliance.excluded_s,
            },
            cap: s.cap.map(|c| StreamCap {
                cap_w: c.cap_w,
                erg_only: c.erg_only,
            }),
            user_paused: s.user_paused,
            trainer_lost_since_s: s.trainer_lost_since.map(|t| ticks_to_seconds(t.0)),
            reconnect_attempt: s.reconnect_attempt,
            soft_start_target_w: s.soft_start_target_w,
            last_gap_s: s.last_gap.map(|t| ticks_to_seconds(t.0)),
            reconnected_at_s: s.reconnected_at.map(|t| ticks_to_seconds(t.0)),
            commanded: s.commanded.map(StreamCommanded::from),
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
    /// instead of counting as a change. The loss fields are discrete: each changes only at a
    /// transition (a loss, an attempt, the trainer back, the soft start ending, a rider pause),
    /// never per tick, so they are sent on change and add no heartbeat churn (#329).
    #[must_use]
    pub fn same_discrete(&self, other: &Self) -> bool {
        self.state == other.state
            && self.loaded == other.loaded
            && self.block_index == other.block_index
            && self.late_samples == other.late_samples
            && self.record_error == other.record_error
            && self.user_paused == other.user_paused
            && self.trainer_lost_since_s == other.trainer_lost_since_s
            && self.reconnect_attempt == other.reconnect_attempt
            && self.soft_start_target_w == other.soft_start_target_w
            && self.last_gap_s == other.last_gap_s
            && self.reconnected_at_s == other.reconnected_at_s
            && self.commanded == other.commanded
    }
}

/// One message to a window (ADR-0001 §4.3). Everything in a frame happened before
/// everything in the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    /// Per stream, +1 per frame, **wrapping** `u32::MAX` → 0: at the 10 Hz maximum frame
    /// rate that is after about 13.6 years of continuous streaming. Consumers compare
    /// `seq` for equality with the next expected value, never with `<`/`>`.
    pub seq: u32,
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
