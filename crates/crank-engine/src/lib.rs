//! Ride engine core (ADR-0001 §4.3, §5; ADR-0002 §3, §6).
//!
//! * [`ticks`]: the 4 Hz tick grid, the two time axes, sample holding and the loss
//!   thresholds of ADR-0001 §5.
//! * [`clock`]: [`MonotonicClock`] and [`VirtualClock`] over the `Clock` trait of
//!   `crank-devices`.
//! * [`engine`]: the actor: states, tick assignment, trainer/HR loss, reconnect,
//!   block sequencing, the ride record.
//! * [`control`]: the [`TargetPolicy`] seam and [`RideActivity`], through which the
//!   engine drives the BLE supervisors' `RideGate` (#208).
//! * [`erg`]: the ERG policy (ADR-0002 §7), the trainer-following machine (§4.4.1),
//!   compliance and the health-cap clamp (#218).
//! * [`record`]: the 1 Hz record sink; core adapts it to `crank-store`.
//!
//! * [`stream`]: subscriptions with filter chains, the multiplexed `StreamFrame`, flow
//!   control and the status heartbeat (#219).
//!
//! Not here: laps, thresholds, the cursor's actual block start, nudges (#243).
pub mod clock;
pub mod control;
pub mod engine;
pub mod erg;
pub mod record;
pub mod stream;
pub mod ticks;

pub use clock::{MonotonicClock, VirtualClock};
pub use control::{PolicyContext, PolicyStatus, RideActivity, TargetPolicy};
pub use engine::{
    CapState, CommandError, Engine, EngineCmd, EngineHandle, PauseReason, RideSetup, RideState,
    RideStatus,
};
pub use erg::{Compliance, Component, ErgPolicy, TrainerFollowing};
pub use record::{METRICS, RecordError, RideRecorder, SecondRow};
pub use stream::types::{
    StreamCap, StreamCompliance, StreamComponent, StreamError, StreamFollowing, StreamPauseReason,
};
pub use stream::{
    LinkChange, LinkDevice, LinkStatus, MetricId, StreamFrame, StreamId, StreamRideState,
    StreamSnapshot, StreamStatus, SubId, SubSample, SubscribeOpts,
};
pub use ticks::{SessionTicks, WorkoutTicks};

#[cfg(test)]
mod erg_tests;
#[cfg(test)]
mod tests;
