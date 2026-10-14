//! [`ErgPolicy`]: the control policy of ADR-0002 §7, as decided.
//!
//! Per tick the policy answers one question, "what mode do we want now?":
//!
//! 1. **Free / no block** (§7.9): `Free` (explicit flat-road SIM). Leaving it owes a soft start.
//! 2. **Release** (§7.4): cadence < 40 rpm for 2 s with a target > 0 → `Resistance { 0 }`;
//!    cadence ≥ 60 rpm for 2 s re-engages with a soft start. The block timer is not touched.
//! 3. **Soft start** (§7.5, #231): owed after a loss epoch, after leaving free/SIM and after a
//!    release. Until a pedalling sample: `Erg { 0 }`; then five 1 Hz steps from the 3 s power
//!    to the block target. A running ramp holds its last step while samples are missing and
//!    restarts from the then-current 3 s power. The debt survives a Free block: a loss during
//!    `Free` is repaid when the next ERG block starts (§7.5 item 5, #218).
//! 4. **Target**: the block line (ramps move once per second, §7.3), rounded to 1 W.
//! 5. **Health cap** (DESIGN §15.2): every ERG command is clamped to `capW`, the ramp from an
//!    above-cap origin included: the cap is on actual power, so the soft start may not
//!    command what the rider happens to be producing.
use crate::control::{PolicyContext, PolicyStatus, SOFT_START_TICKS, TargetPolicy};
use crate::erg::compliance::{Accumulator, BLOCK_SETTLE_TICKS, ENGAGE_SETTLE_TICKS, TickFacts};
use crate::ticks::TICKS_PER_SECOND;
use crank_devices::ControlMode;
use crank_domain::workout::Phase;

/// Cadence below which ERG is released (rpm).
pub const RELEASE_RPM: u8 = 40;
/// Cadence at or above which a release re-engages (rpm).
pub const REENGAGE_RPM: u8 = 60;
/// Both conditions must hold this long: 2 s.
pub const RELEASE_TICKS: u64 = 8;

#[derive(Clone, Copy, Debug)]
struct Ramp {
    start: u64,
    origin: f64,
}
/// The ERG policy. Stateful: one per engine.
#[derive(Debug, Default)]
pub struct ErgPolicy {
    link_epoch: u64,
    loss_epoch: u64,
    /// A soft start is owed (see the module docs).
    owed: bool,
    ramp: Option<Ramp>,
    released: bool,
    low: u64,
    high: u64,
    block: Option<(usize, f64)>,
    /// Settle seconds are excluded from compliance: ticks `settle.0 .. settle.1`.
    settle: (u64, u64),
    compliance: Accumulator,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value is rounded and clamped into 0..=u16::MAX first"
)]
fn watts(w: f64, max: u16) -> u16 {
    w.round().clamp(0.0, f64::from(max)) as u16
}
#[expect(clippy::cast_precision_loss, reason = "tick counts fit f64 exactly")]
fn seconds(ticks: u64) -> f64 {
    (ticks / TICKS_PER_SECOND) as f64
}

impl ErgPolicy {
    /// A policy with nothing owed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// A discontinuity: ticks `from..until` are settle, joined to a window still running.
    fn settle(&mut self, now: u64, from: u64, until: u64) {
        self.settle = if now < self.settle.1 {
            (self.settle.0.min(from), self.settle.1.max(until))
        } else {
            (from, until)
        };
    }
    /// Release / re-engage on held cadence (§7.4). Not counted while a soft start is waiting
    /// for the first pedalling sample: that commands 0 W already.
    fn update_release(&mut self, ctx: &PolicyContext<'_>, target: f64) {
        let cadence = ctx.cadence_rpm;
        if self.released {
            self.high = if cadence.is_some_and(|c| c >= REENGAGE_RPM) {
                self.high + 1
            } else {
                0
            };
            if self.high >= RELEASE_TICKS {
                self.released = false;
                self.owed = true;
                self.ramp = None;
                self.low = 0;
            }
        } else if target > 0.0 && (!self.owed || ctx.flow_start.is_some()) {
            self.low = if cadence.is_some_and(|c| c < RELEASE_RPM) {
                self.low + 1
            } else {
                0
            };
            if self.low >= RELEASE_TICKS {
                self.released = true;
                self.high = 0;
            }
        }
    }
    fn leave_erg(&mut self) {
        self.owed = true;
        self.ramp = None;
        self.released = false;
        self.low = 0;
        self.high = 0;
    }
    fn account(&mut self, ctx: &PolicyContext<'_>, erg: bool, target_w: f64) {
        let counted = ctx.riding
            && erg
            && !self.released
            && !self.owed
            && !(self.settle.0..self.settle.1).contains(&ctx.tick)
            && !ctx.following.excludes_compliance();
        self.compliance.tick(
            ctx.tick,
            TickFacts {
                excluded: !counted,
                power_3s_w: ctx.power_3s_w,
                target_w,
            },
        );
    }
}

impl TargetPolicy for ErgPolicy {
    fn desired(&mut self, ctx: &PolicyContext<'_>) -> Option<ControlMode> {
        if ctx.loss_epoch != self.loss_epoch {
            self.loss_epoch = ctx.loss_epoch;
            self.owed = true;
        }
        if ctx.link_epoch != self.link_epoch {
            self.link_epoch = ctx.link_epoch;
            self.ramp = None;
            self.released = false;
            self.low = 0;
            self.high = 0;
        }
        let Some(block) = ctx.block.filter(|b| b.phase != Phase::Free) else {
            if !self.owed || self.ramp.is_some() || self.released {
                self.leave_erg();
            }
            self.account(ctx, false, 0.0);
            return Some(ControlMode::Free);
        };
        // A block boundary is a discontinuity when its start target differs from the
        // previous block's end target (ADR-0002 §7.8 a). The first block has no previous
        // block, and a ride start is not an engagement (§7.8 c: "the soft start's 5 s plus
        // 5 s", and §7.5 starts a soft start only after a reconnect or after leaving
        // free/SIM), so it is never settled. At the boundary tick the engine has already
        // advanced the index (the new target is sent on it, §7.2) but the tick is still the
        // old block's: compliance keeps its target.
        let boundary = ctx.block_elapsed_ticks == 0;
        let mut previous_end = None;
        if let Some((_, end)) = self.block.filter(|(i, _)| *i != ctx.block_index) {
            if (block.power_from - end).abs() >= 0.5 {
                let first = ctx.tick + u64::from(boundary);
                self.settle(ctx.tick, first, first + BLOCK_SETTLE_TICKS);
            }
            previous_end = boundary.then_some(end);
        }
        self.block = Some((ctx.block_index, block.power_to));
        let frac = if block.dur_s > 0.0 {
            (seconds(ctx.block_elapsed_ticks) / block.dur_s).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let line = block.power_from + (block.power_to - block.power_from) * frac;
        let max = ctx
            .cap_w
            .map_or(ctx.max_target_w, |c| c.min(ctx.max_target_w));
        let target = f64::from(watts(line, max));
        let shown = previous_end.map_or(target, |e| e.min(f64::from(max)));

        self.update_release(ctx, target);
        if self.released {
            self.account(ctx, true, shown);
            return Some(ControlMode::Resistance { pct: 0.0 });
        }

        let mut command = target;
        if self.owed {
            if ctx.flow_start.is_none() {
                // No torque for a rider who is not pedalling (§7.4, #231). A ramp already
                // running holds its last step.
                self.ramp = None;
                self.account(ctx, true, shown);
                return if matches!(ctx.last_sent, Some(ControlMode::Erg { .. })) {
                    None
                } else {
                    Some(ControlMode::Erg { target_w: 0 })
                };
            }
            let ramp = if let Some(r) = self.ramp {
                r
            } else {
                self.settle(ctx.tick, ctx.tick, ctx.tick + ENGAGE_SETTLE_TICKS);
                let r = Ramp {
                    start: ctx.tick,
                    origin: ctx.power_3s_w.unwrap_or(0.0),
                };
                self.ramp = Some(r);
                r
            };
            // `saturating_sub`: a ramp starts at or before the current tick by construction (one
            // policy per ride, ticks only grow), but the engine actor never dies on arithmetic.
            let age = ctx.tick.saturating_sub(ramp.start);
            if age >= SOFT_START_TICKS {
                self.owed = false;
                self.ramp = None;
            } else {
                let step = seconds(age).floor();
                command = ramp.origin + (target - ramp.origin) * (step / 5.0);
            }
        }
        self.account(ctx, true, shown);
        Some(ControlMode::Erg {
            target_w: watts(command, max),
        })
    }
    fn status(&self) -> PolicyStatus {
        PolicyStatus {
            released: self.released,
            compliance: self.compliance.tally(),
        }
    }
}
