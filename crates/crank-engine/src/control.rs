//! The control seam and the ride-active signal.
//!
//! The engine decides *when* control is sent and re-sent (ADR-0002 §6: on every
//! `Link(Ready)` the driver restores nothing, the engine re-issues the mode); *what* is
//! sent is a [`TargetPolicy`]. [`BlockTarget`] is the plain policy of this part: the
//! block's target, ramped at 1 Hz, with the soft start of ADR-0002 §7.5 after a
//! reconnect. The ERG policy proper (offsets, release, HR steering, health cap) is #218
//! and replaces this trait object; nothing else in the engine changes. Open-ended
//! block end rules are #224, not this policy.
use crank_devices::ControlMode;
use crank_domain::workout::{FlatBlock, Phase};

/// Soft start length after a reconnect: five 1 Hz updates (ADR-0002 §7.4–7.5).
pub const SOFT_START_TICKS: u64 = 20;

/// What a policy may look at when the engine asks for the desired mode on a tick.
#[derive(Clone, Copy, Debug)]
pub struct PolicyContext<'a> {
    /// The current block; `None` for a ride without blocks (free ride).
    pub block: Option<&'a FlatBlock>,
    /// Workout ticks spent in the current block.
    pub block_elapsed_ticks: u64,
    /// Ticks since trainer samples last started flowing (0 while none are).
    pub engaged_ticks: u64,
    /// A trainer loss has not yet been followed by a complete soft start.
    pub after_loss: bool,
    /// Mean trainer power over the last 3 s (12 ticks) at the tick the samples last
    /// started flowing: the soft-start origin. `None` while no trainer value is present;
    /// the policy then has no origin and sends the target as is (design question #231).
    pub engage_power_w: Option<f64>,
    /// Largest target the driver can represent.
    pub max_target_w: u16,
}

/// The desired control mode for a tick; `None` leaves the trainer untouched.
///
/// The engine sends the result only when it differs from the last mode it sent, and
/// at most one `SetMode` per tick (ADR-0002 §4.4).
pub trait TargetPolicy: Send + 'static {
    /// Desired mode for this tick.
    fn desired(&mut self, ctx: &PolicyContext<'_>) -> Option<ControlMode>;
}

/// Plain block-target policy (the seam #218 replaces).
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockTarget;

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value is rounded and clamped into 0..=u16::MAX first"
)]
fn watts(w: f64, max: u16) -> u16 {
    w.round().clamp(0.0, f64::from(max)) as u16
}
fn secs(ticks: u64) -> f64 {
    f64::from(u32::try_from(ticks / crate::ticks::TICKS_PER_SECOND).unwrap_or(u32::MAX))
}

impl TargetPolicy for BlockTarget {
    fn desired(&mut self, ctx: &PolicyContext<'_>) -> Option<ControlMode> {
        let Some(block) = ctx.block else {
            return Some(ControlMode::Free);
        };
        if block.phase == Phase::Free {
            return Some(ControlMode::Free);
        }
        // Ramps move along the line once per second (ADR-0002 §7.3).
        let frac = if block.dur_s > 0.0 {
            (secs(ctx.block_elapsed_ticks) / block.dur_s).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let target = block.power_from + (block.power_to - block.power_from) * frac;
        let target = match ctx.engage_power_w {
            // Soft start: from the current 3 s power to the target over 5 s, 1 Hz.
            Some(from) if ctx.after_loss && ctx.engaged_ticks < SOFT_START_TICKS => {
                let step = secs(ctx.engaged_ticks).floor();
                from + (target - from) * (step / 5.0)
            }
            _ => target,
        };
        Some(ControlMode::Erg {
            target_w: watts(target, ctx.max_target_w),
        })
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
