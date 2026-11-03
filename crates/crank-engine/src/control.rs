//! The control seam and the ride-active signal.
//!
//! The engine decides *when* control is sent and re-sent (ADR-0002 §6: on every
//! `Link(Ready)` the driver restores nothing, the engine re-issues the mode); *what* is
//! sent is a [`TargetPolicy`]: [`crate::erg::ErgPolicy`] is the decided ERG policy
//! (ADR-0002 §7). Open-ended block end rules are #224, not this policy.
use crate::erg::{Compliance, TrainerFollowing};
use crank_devices::ControlMode;
use crank_domain::workout::FlatBlock;

/// Soft start length after a reconnect: five 1 Hz updates (ADR-0002 §7.4–7.5).
pub const SOFT_START_TICKS: u64 = 20;

/// What a policy may look at when the engine asks for the desired mode on a tick.
#[derive(Clone, Copy, Debug)]
pub struct PolicyContext<'a> {
    /// The current block; `None` for a ride without blocks (free ride).
    pub block: Option<&'a FlatBlock>,
    /// Index of the current block (changes at every boundary and skip).
    pub block_index: usize,
    /// Workout ticks spent in the current block.
    pub block_elapsed_ticks: u64,
    /// The tick being processed, from the ride's start (session time).
    pub tick: u64,
    /// Increases at every applied link transition and every 8-tick silence: nothing the
    /// policy learned before it (release counters, a running ramp) may be carried over, and
    /// a soft start is owed (ADR-0002 §7.5, #231).
    pub link_epoch: u64,
    /// The tick at which samples last started flowing: a pedalling (cadence > 0) value is
    /// held since then. `None` while none is. Restarts at every applied link transition
    /// and every 8-tick silence, so a ramp that began before one is not continued.
    pub flow_start: Option<u64>,
    /// Mean trainer power over the last 3 s (12 ticks), pruned by tick time; `None`
    /// without a sample in the window. Never carries power from before a loss epoch.
    pub power_3s_w: Option<f64>,
    /// Held trainer cadence at this tick.
    pub cadence_rpm: Option<u8>,
    /// The mode the engine last sent in the current epoch.
    pub last_sent: Option<ControlMode>,
    /// The tick is workout time (nothing paused the ride).
    pub riding: bool,
    /// The rider paused the ride (`EngineCmd::Pause`, not yet resumed): DESIGN §5.2 puts
    /// the trainer at 0 % resistance and re-engages ERG with the soft start (#264).
    pub user_paused: bool,
    /// The trainer-following verdict at this tick (ADR-0002 §4.4.1).
    pub following: TrainerFollowing,
    /// Largest target the driver can represent.
    pub max_target_w: u16,
    /// The ride's health cap, if one is active (DESIGN §15.2).
    pub cap_w: Option<u16>,
}

/// What the policy reports to the engine's status.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PolicyStatus {
    /// ERG is released for low cadence ("ERG released, spin up").
    pub released: bool,
    /// The compliance tally (ADR-0002 §7.8).
    pub compliance: Compliance,
    /// While a soft start is owed and the ride is in an ERG block (not paused, not released):
    /// the block's target in watts the ramp is heading for. The engine decides whether to show
    /// it (only after a recovered trainer loss).
    pub soft_start_target_w: Option<u16>,
}

/// The desired control mode for a tick; `None` leaves the trainer untouched.
///
/// The engine sends the result only when it differs from the last mode it sent, and
/// at most one `SetMode` per tick (ADR-0002 §4.4).
pub trait TargetPolicy: Send + 'static {
    /// Desired mode for this tick.
    fn desired(&mut self, ctx: &PolicyContext<'_>) -> Option<ControlMode>;
    /// A tick on which no control could be sent (the link is not ready): the policy may
    /// still account for the tick.
    fn skipped(&mut self, _ctx: &PolicyContext<'_>) {}
    /// State for the ride status.
    fn status(&self) -> PolicyStatus {
        PolicyStatus::default()
    }
}

/// Receives the engine's ride-active signal (ADR-0002 §6, #208).
///
/// `true` when a ride is loaded, `false` when it finishes or is unloaded. The engine
/// cannot depend on `crank-ble`, so core passes `move |a| gate.set_active(a)` for the
/// `RideGate` it shares with `Reconnecting` and `ReconnectingHr`; opening the ride
/// view before a load is core's call on the same gate (#34).
pub trait RideActivity: Send + Sync + 'static {
    /// Whether a ride is active.
    fn set_active(&self, active: bool);
}
impl<F: Fn(bool) + Send + Sync + 'static> RideActivity for F {
    fn set_active(&self, active: bool) {
        self(active);
    }
}
