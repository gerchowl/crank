//! The vocabulary of the 1 Hz ride record (ADR-0009 §2.3.2) and the interval statistics over
//! its seconds.
//!
//! The engine writes the record (`crank_engine::record`); this module defines what its
//! per-second codes mean, so the engine, the store's readers and the analysis share one
//! definition. [`interval_stats`] is the one definition of an interval's statistics: the
//! engine's live lap statistics must equal it on the same seconds (ADR-0009 §2.3.2, the
//! engine/domain lap tie test in `crank-engine`).
use crate::workout::normalized_power;

/// Version of a ride's record (`ride.record_version`, ADR-0009 §2.3.2): which metrics it holds.
///
/// * **1**: `power`, `cadence`, `speed`, `hr`, `workout_s` and `block`.
/// * **2**: version 1 plus `target_w`, `compliance`, `adj_rider_pct`, `adj_coach_pct` and
///   `block_end`, all five together.
///
/// A ride's version never changes after it is recorded (a version-1 ride never becomes
/// version 2 by re-analysis).
pub const RECORD_VERSION: u32 = 2;

/// The compliance class of one record second (ADR-0009 §2.3.2, amended for class 9 in #717).
///
/// The live counts (`StreamCompliance`, ADR-0002 §7.8) are sums of these classes:
/// `within_s` = class 1, `counted_s` = classes 1–3, `excluded_s` = classes 4–9. Class 0 is in
/// none of them: a second without an ERG target is not an ERG second.
///
/// When several excluded causes hold in one second the precedence is 6, 8, 7, 5, 4 (pause and
/// trainer verdicts first), so a release during a trainer fault is recorded as the fault;
/// class 9 applies only to a second none of them excludes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComplianceClass {
    /// 0: no ERG target this second (a free, SIM or resistance block, a ride without blocks, a
    /// tick on which no control could be sent).
    NoTarget,
    /// 1: counted, within ±5 % of the controlled target.
    Within,
    /// 2: counted, below −5 %.
    Below,
    /// 3: counted, above +5 %.
    Above,
    /// 4: excluded, settling after a target change or an ERG (re-)engagement (ADR-0002 §7.8).
    Settling,
    /// 5: excluded, ERG released for low cadence (ADR-0002 §7.4), until its re-engage settles.
    Released,
    /// 6: excluded, paused by the rider, by the engine on a trainer loss, or for stopped
    /// pedalling.
    Paused,
    /// 7: excluded, trainer verdict `suspect` (ADR-0002 A5).
    Suspect,
    /// 8: excluded, trainer verdict `not_following` (ADR-0002 A3).
    NotFollowing,
    /// 9: excluded, no 3 s power: an ERG second that nothing above excludes, without a power
    /// sample in the last 3 s (the trainer sends page 25 with an invalid power field, so the
    /// ride is not paused for a loss). There is nothing to compare with the target.
    NoPower,
}
impl ComplianceClass {
    /// Every class, in code order.
    pub const ALL: [Self; 10] = [
        Self::NoTarget,
        Self::Within,
        Self::Below,
        Self::Above,
        Self::Settling,
        Self::Released,
        Self::Paused,
        Self::Suspect,
        Self::NotFollowing,
        Self::NoPower,
    ];
    /// The stored code (the `compliance` metric's value).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::NoTarget => 0,
            Self::Within => 1,
            Self::Below => 2,
            Self::Above => 3,
            Self::Settling => 4,
            Self::Released => 5,
            Self::Paused => 6,
            Self::Suspect => 7,
            Self::NotFollowing => 8,
            Self::NoPower => 9,
        }
    }
    /// The class of a stored code; `None` for any other value.
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }
    /// Counted toward compliance (classes 1–3).
    #[must_use]
    pub fn counted(self) -> bool {
        matches!(self, Self::Within | Self::Below | Self::Above)
    }
    /// Excluded from compliance (classes 4–9).
    #[must_use]
    pub fn excluded(self) -> bool {
        self.code() >= 4
    }
    /// The class of a second the target applies to and nothing excludes: within, below or above
    /// ±5 % of `target_w` by its 3 s power (ADR-0002 §7.8), [`Self::NoPower`] without one.
    #[must_use]
    pub fn of_power(power_3s_w: Option<f64>, target_w: f64) -> Self {
        const TOLERANCE: f64 = 0.05;
        match power_3s_w {
            None => Self::NoPower,
            Some(p) if (p - target_w).abs() <= TOLERANCE * target_w => Self::Within,
            Some(p) if p < target_w => Self::Below,
            Some(_) => Self::Above,
        }
    }
}

/// How a block ended (the `block_end` metric, on the block's last riding second; ADR-0009
/// §2.3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockEnd {
    /// 1: its time ran out.
    Completed,
    /// 2: the rider skipped it.
    Skipped,
    /// 3: the ride ended during it.
    Stopped,
    /// 4: a live end-set edit ended it (DESIGN §17). Not written yet: the engine has no live
    /// edits (#240).
    Edited,
}
impl BlockEnd {
    /// Every end, in code order.
    pub const ALL: [Self; 4] = [Self::Completed, Self::Skipped, Self::Stopped, Self::Edited];
    /// The stored code.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::Completed => 1,
            Self::Skipped => 2,
            Self::Stopped => 3,
            Self::Edited => 4,
        }
    }
    /// The end of a stored code; `None` for any other value.
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.code() == code)
    }
}

/// What [`interval_stats`] reads of one record second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IntervalSecond {
    /// The second has a riding tick (its `workout_s` is finite); a paused second has none.
    pub riding: bool,
    /// Mean power, W.
    pub power: Option<f64>,
    /// Mean cadence, rpm.
    pub cadence: Option<f64>,
    /// Mean heart rate, bpm.
    pub hr: Option<f64>,
    /// Its compliance class; `None` in a version-1 record.
    pub compliance: Option<ComplianceClass>,
}

/// The statistics of an interval's seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntervalStats {
    /// Mean power over the riding seconds with power, W.
    pub avg_power_w: Option<f64>,
    /// Normalized power over the riding seconds with power, joined end to end (the gap rule of
    /// ADR-0009 §2.3.1.1, run-wise NP, replaces it in issue 6, #718), W.
    pub np_w: Option<f64>,
    /// Mean heart rate over the riding seconds with one, bpm.
    pub avg_hr_bpm: Option<f64>,
    /// Mean cadence over the riding seconds with one, rpm.
    pub avg_cadence_rpm: Option<f64>,
    /// Paused seconds.
    pub paused_s: u32,
    /// Riding seconds without power.
    pub power_gap_s: u32,
    /// Riding seconds without cadence.
    pub cadence_gap_s: u32,
    /// Riding seconds without heart rate.
    pub hr_gap_s: u32,
    /// Seconds of class 1–3.
    pub counted_s: u32,
    /// Seconds of class 1.
    pub within_s: u32,
    /// Seconds of class 4–9.
    pub excluded_s: u32,
}

#[derive(Clone, Copy, Default)]
struct Mean {
    sum: f64,
    n: u32,
}
impl Mean {
    fn add(&mut self, v: f64) {
        self.sum += v;
        self.n += 1;
    }
    fn get(self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / f64::from(self.n))
    }
}

/// The statistics of an interval: every second of it, paused ones included, in order.
///
/// Paused seconds count only toward `paused_s` and their compliance class; a gap in one
/// channel never removes a second from another.
#[must_use]
pub fn interval_stats(seconds: &[IntervalSecond]) -> IntervalStats {
    let mut s = IntervalStats::default();
    let (mut power, mut cadence, mut hr) = (Mean::default(), Mean::default(), Mean::default());
    let mut trace = Vec::new();
    for second in seconds {
        match second.compliance {
            Some(c) if c.counted() => {
                s.counted_s += 1;
                s.within_s += u32::from(c == ComplianceClass::Within);
            }
            Some(c) if c.excluded() => s.excluded_s += 1,
            _ => {}
        }
        if !second.riding {
            s.paused_s += 1;
            continue;
        }
        match second.power {
            Some(p) => {
                power.add(p);
                trace.push(p);
            }
            None => s.power_gap_s += 1,
        }
        match second.cadence {
            Some(c) => cadence.add(c),
            None => s.cadence_gap_s += 1,
        }
        match second.hr {
            Some(h) => hr.add(h),
            None => s.hr_gap_s += 1,
        }
    }
    s.avg_power_w = power.get();
    // Record values are finite means of device samples; a non-finite input is not a power
    // trace, and NP is then absent rather than wrong.
    s.np_w = (!trace.is_empty())
        .then(|| normalized_power(&trace).ok())
        .flatten();
    s.avg_hr_bpm = hr.get();
    s.avg_cadence_rpm = cadence.get();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_partition_the_live_counts() {
        for c in ComplianceClass::ALL {
            assert_eq!(ComplianceClass::from_code(c.code()), Some(c));
            // Exactly one of: no target, counted, excluded.
            let kinds = [c == ComplianceClass::NoTarget, c.counted(), c.excluded()];
            assert_eq!(kinds.iter().filter(|k| **k).count(), 1, "{c:?}");
        }
        assert_eq!(ComplianceClass::from_code(10), None);
        for e in BlockEnd::ALL {
            assert_eq!(BlockEnd::from_code(e.code()), Some(e));
        }
        assert_eq!(BlockEnd::from_code(0), None);
        assert_eq!(BlockEnd::from_code(5), None);
    }

    #[test]
    fn power_classes_at_the_five_percent_edges() {
        let t = 200.0;
        assert_eq!(
            ComplianceClass::of_power(Some(210.0), t),
            ComplianceClass::Within
        );
        assert_eq!(
            ComplianceClass::of_power(Some(190.0), t),
            ComplianceClass::Within
        );
        assert_eq!(
            ComplianceClass::of_power(Some(189.9), t),
            ComplianceClass::Below
        );
        assert_eq!(
            ComplianceClass::of_power(Some(210.1), t),
            ComplianceClass::Above
        );
        assert_eq!(ComplianceClass::of_power(None, t), ComplianceClass::NoPower);
    }

    #[test]
    fn interval_stats_keep_channels_apart_and_count_classes() {
        let riding = |p: Option<f64>, h: Option<f64>, c| IntervalSecond {
            riding: true,
            power: p,
            cadence: Some(90.0),
            hr: h,
            compliance: Some(c),
        };
        let seconds = [
            riding(Some(100.0), Some(120.0), ComplianceClass::Within),
            riding(None, Some(140.0), ComplianceClass::NoPower),
            IntervalSecond {
                compliance: Some(ComplianceClass::Paused),
                ..IntervalSecond::default()
            },
            riding(Some(300.0), None, ComplianceClass::Below),
            riding(Some(200.0), Some(130.0), ComplianceClass::NoTarget),
        ];
        let s = interval_stats(&seconds);
        assert_eq!(s.avg_power_w, Some(200.0));
        assert_eq!(s.avg_hr_bpm, Some(130.0));
        assert_eq!(s.avg_cadence_rpm, Some(90.0));
        assert_eq!((s.paused_s, s.power_gap_s, s.hr_gap_s), (1, 1, 1));
        assert_eq!((s.counted_s, s.within_s, s.excluded_s), (2, 1, 2));
        // Under 30 s NP is the mean of the joined trace.
        assert_eq!(s.np_w, Some(200.0));
        assert_eq!(interval_stats(&[]), IntervalStats::default());
    }
}
