//! Compliance (ADR-0002 §7.8): the share of ERG seconds with `power_3s` within ±5 % of
//! the controlled target, excluding settle, released, paused and `suspect` /
//! `not_following` seconds.
//!
//! Each second gets one [`ComplianceClass`] (ADR-0009 §2.3.2), recorded as the `compliance`
//! metric; the running [`Compliance`] tally is the sum of those classes.

/// Settle after a block boundary that is a discontinuity, in ticks (5 s).
pub const BLOCK_SETTLE_TICKS: u64 = 20;
/// Settle after an ERG (re-)engagement: the 5 s soft start plus 5 s, in ticks.
pub const ENGAGE_SETTLE_TICKS: u64 = 40;
use crate::ticks::TICKS_PER_SECOND;
pub use crank_domain::record::ComplianceClass;

/// The running tally, in whole seconds: sums of the seconds' [`ComplianceClass`]es.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compliance {
    /// Seconds that count (classes 1–3: ERG, riding, settled, followed, with power).
    pub counted_s: u32,
    /// Of those, seconds within ±5 % of the controlled target (class 1).
    pub within_s: u32,
    /// Seconds excluded for any rule above (classes 4–9). A second without an ERG target
    /// (class 0) is in none of the three counts.
    pub excluded_s: u32,
}
impl Compliance {
    /// `within / counted`, `None` before any second counted.
    #[must_use]
    pub fn share(&self) -> Option<f64> {
        (self.counted_s > 0).then(|| f64::from(self.within_s) / f64::from(self.counted_s))
    }
    fn add(&mut self, class: ComplianceClass) {
        if class.counted() {
            self.counted_s += 1;
            self.within_s += u32::from(class == ComplianceClass::Within);
        } else if class.excluded() {
            self.excluded_s += 1;
        }
    }
}

/// What holds on one tick, from the policy's state. The variants are in ascending
/// precedence: a second takes the highest cause among its ticks (ADR-0009 §2.3.2: 6, 8, 7,
/// 5, 4, then no target, then the power classes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TickCause {
    /// An ERG tick nothing excludes: the second's power decides its class.
    Erg,
    /// No ERG target (class 0).
    NoTarget,
    /// Settling (class 4).
    Settling,
    /// Released, or its re-engage not yet settled (class 5).
    Released,
    /// Trainer verdict `suspect` (class 7).
    Suspect,
    /// Trainer verdict `not_following` (class 8).
    NotFollowing,
    /// Not a riding tick (class 6).
    Paused,
}

/// What one tick tells compliance. `power_3s_w` is that of the second's last tick.
#[derive(Clone, Copy, Debug)]
pub struct TickFacts {
    /// What holds on the tick.
    pub cause: TickCause,
    /// Mean power over the last 3 s, if any.
    pub power_3s_w: Option<f64>,
    /// The controlled target (never smoothed, never derived from measured power); `None`
    /// on a tick without an ERG target. Read only on riding ticks.
    pub target_w: Option<f64>,
}

/// One closed second: its class and the controlled target at its last riding tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecondCompliance {
    /// The second's class.
    pub class: ComplianceClass,
    /// The controlled target at the last riding tick of the second; `None` without a riding
    /// tick, when that tick had no ERG target, and always for class 0.
    pub target_w: Option<f64>,
}

/// Accumulates ticks into seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accumulator {
    tally: Compliance,
    cause: Option<TickCause>,
    target_w: Option<f64>,
    last: Option<SecondCompliance>,
}
impl Accumulator {
    /// The tally so far.
    #[must_use]
    pub fn tally(&self) -> Compliance {
        self.tally
    }
    /// The last closed second.
    #[must_use]
    pub fn last_second(&self) -> Option<SecondCompliance> {
        self.last
    }
    /// Feed tick `tick` (4 ticks per second); closes the second on its last tick.
    pub fn tick(&mut self, tick: u64, f: TickFacts) {
        self.cause = Some(self.cause.map_or(f.cause, |c| c.max(f.cause)));
        if f.cause != TickCause::Paused {
            self.target_w = f.target_w;
        }
        if !(tick + 1).is_multiple_of(TICKS_PER_SECOND) {
            return;
        }
        let cause = self.cause.take().unwrap_or(TickCause::NoTarget);
        let target_w = self.target_w.take();
        let class = match cause {
            TickCause::Paused => ComplianceClass::Paused,
            TickCause::NotFollowing => ComplianceClass::NotFollowing,
            TickCause::Suspect => ComplianceClass::Suspect,
            TickCause::Released => ComplianceClass::Released,
            TickCause::Settling => ComplianceClass::Settling,
            TickCause::NoTarget => ComplianceClass::NoTarget,
            // An ERG tick always carries its target; without one there is nothing to judge.
            TickCause::Erg => target_w.map_or(ComplianceClass::NoTarget, |t| {
                ComplianceClass::of_power(f.power_3s_w, t)
            }),
        };
        self.tally.add(class);
        self.last = Some(SecondCompliance {
            class,
            target_w: target_w.filter(|_| class != ComplianceClass::NoTarget),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TickCause as T;

    fn second(acc: &mut Accumulator, s: u64, ticks: [(TickCause, Option<f64>); 4], p: Option<f64>) {
        for (i, (cause, target_w)) in ticks.into_iter().enumerate() {
            acc.tick(
                s * TICKS_PER_SECOND + i as u64,
                TickFacts {
                    cause,
                    power_3s_w: p,
                    target_w,
                },
            );
        }
    }

    #[test]
    fn a_second_takes_the_highest_cause_of_its_ticks() {
        let t = Some(200.0);
        let mut acc = Accumulator::default();
        // A release during a trainer fault is recorded as the fault (precedence 6, 8, 7, 5, 4).
        let cases = [
            (
                [
                    (T::Released, t),
                    (T::NotFollowing, t),
                    (T::Erg, t),
                    (T::Erg, t),
                ],
                ComplianceClass::NotFollowing,
            ),
            (
                [(T::Suspect, t), (T::Released, t), (T::Erg, t), (T::Erg, t)],
                ComplianceClass::Suspect,
            ),
            (
                [(T::Settling, t), (T::Released, t), (T::Erg, t), (T::Erg, t)],
                ComplianceClass::Released,
            ),
            (
                [
                    (T::Paused, None),
                    (T::NotFollowing, t),
                    (T::Erg, t),
                    (T::Erg, t),
                ],
                ComplianceClass::Paused,
            ),
            (
                [(T::Erg, t), (T::Erg, t), (T::Settling, t), (T::Erg, t)],
                ComplianceClass::Settling,
            ),
            (
                [
                    (T::Erg, t),
                    (T::Erg, t),
                    (T::NoTarget, None),
                    (T::NoTarget, None),
                ],
                ComplianceClass::NoTarget,
            ),
            ([(T::Erg, t); 4], ComplianceClass::Within),
        ];
        for (s, (ticks, want)) in cases.into_iter().enumerate() {
            second(&mut acc, s as u64, ticks, Some(205.0));
            assert_eq!(acc.last_second().unwrap().class, want, "second {s}");
        }
        let c = acc.tally();
        assert_eq!((c.counted_s, c.within_s, c.excluded_s), (1, 1, 5));
    }

    #[test]
    fn the_target_is_that_of_the_last_riding_tick_and_absent_for_class_0() {
        let mut acc = Accumulator::default();
        second(
            &mut acc,
            0,
            [
                (T::Erg, Some(100.0)),
                (T::Erg, Some(101.0)),
                (T::Paused, None),
                (T::Paused, None),
            ],
            Some(100.0),
        );
        let last = acc.last_second().unwrap();
        assert_eq!(
            (last.class, last.target_w),
            (ComplianceClass::Paused, Some(101.0))
        );
        second(&mut acc, 1, [(T::Paused, None); 4], None);
        assert_eq!(acc.last_second().unwrap().target_w, None);
        second(
            &mut acc,
            2,
            [
                (T::Erg, Some(100.0)),
                (T::Erg, Some(100.0)),
                (T::Erg, Some(100.0)),
                (T::NoTarget, None),
            ],
            Some(100.0),
        );
        let last = acc.last_second().unwrap();
        assert_eq!(
            (last.class, last.target_w),
            (ComplianceClass::NoTarget, None)
        );
        second(&mut acc, 3, [(T::Erg, Some(100.0)); 4], None);
        let last = acc.last_second().unwrap();
        assert_eq!(
            (last.class, last.target_w),
            (ComplianceClass::NoPower, Some(100.0))
        );
        second(&mut acc, 4, [(T::Erg, Some(100.0)); 4], Some(80.0));
        assert_eq!(acc.last_second().unwrap().class, ComplianceClass::Below);
        second(&mut acc, 5, [(T::Erg, Some(100.0)); 4], Some(120.0));
        assert_eq!(acc.last_second().unwrap().class, ComplianceClass::Above);
        let c = acc.tally();
        // class 0 counts nowhere; 6 and 9 are excluded
        assert_eq!((c.counted_s, c.within_s, c.excluded_s), (2, 0, 3));
    }
}
