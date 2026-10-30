//! Laps (#12; decided in #243 Q1, Q2, Q5): one lap per [`FlatBlock`] as ridden.
//!
//! * A lap is keyed by its block's `(node_id, path, iteration)` (ADR-0001 §6 `lap`, with the
//!   `iteration` the #243 decision adds to match `recovery`).
//! * It ends when its block ends, at the tick of a skip (DESIGN §2 "Lap / skip", DSL.md
//!   `lap`), or when the ride ends. A free ride is one lap; there the rider's lap/skip action
//!   closes it and opens the next.
//! * Its span is in session time and includes pauses; its workout span is in workout time.
//! * Its statistics come from the 1 Hz record ([`SecondRow`]), second by second as each is
//!   appended, so paused and lost seconds (null) are excluded from every mean. A record second
//!   belongs to the lap whose block it records ([`SecondRow::block`]): a second a boundary
//!   splits goes to the lap of its last riding tick.
//! * Compliance is the ERG policy's own per-second classification (ADR-0002 §7.8, the #218
//!   following machine and settle rules): the lap gets the change of the ride's tally over its
//!   seconds, so the laps' tallies add up to the ride's.
//!
//! Laps are served by the `laps` query; the status carries only how many have closed
//! (`laps_closed`), and the live lap's power is the `power_lap` stream metric.
use crate::erg::Compliance;
use crate::record::SecondRow;
use crate::stream::types::StreamCompliance;
use crate::ticks::ticks_to_seconds;
use crank_domain::workout::{FlatBlock, normalized_power};
use serde::{Deserialize, Serialize};

/// The block a lap rides: its position and its source step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LapBlock {
    /// Index of the block in the compiled workout.
    pub index: u32,
    /// Source step identity.
    pub node_id: String,
    /// Zero-based child indices from the root to the source step.
    pub path: Vec<u32>,
    /// Zero-based enclosing repeat iterations.
    pub iteration: Vec<u32>,
}
impl LapBlock {
    fn new(index: usize, b: &FlatBlock) -> Self {
        let to_u32 = |v: &[usize]| v.iter().map(|&i| wire(i)).collect();
        Self {
            index: wire(index),
            node_id: b.node_id.clone(),
            path: to_u32(&b.path),
            iteration: to_u32(&b.iteration),
        }
    }
}
/// A workout position as `u32` (TypeScript cannot carry `usize`, #271). A compiled workout is
/// bounded far below it (`MAX_SECONDS` of blocks), so the saturation never applies.
fn wire(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// How a lap ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum LapEnd {
    /// Its block's time ran out.
    Completed,
    /// The rider's lap/skip action (`S`) ended it.
    Skipped,
    /// The ride ended during it.
    Stopped,
}

/// Riding seconds without a value, per sensor (DESIGN §5.5 "data gaps for each sensor").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LapGaps {
    /// Seconds without power.
    pub power_s: u32,
    /// Seconds without cadence.
    pub cadence_s: u32,
    /// Seconds without heart rate.
    pub hr_s: u32,
}

/// A lap's statistics, from its record seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LapStats {
    /// Mean power over the seconds with power, W; `null` without any.
    pub avg_power_w: Option<f64>,
    /// Normalized power over the seconds with power (30 s rolling fourth-power mean; under
    /// 30 s, their mean), W; `null` without any.
    pub np_w: Option<f64>,
    /// Mean heart rate over the seconds with one, bpm.
    pub avg_hr_bpm: Option<f64>,
    /// Mean cadence over the seconds with one, rpm.
    pub avg_cadence_rpm: Option<f64>,
    /// Record seconds in which the ride was paused throughout (any reason, a trainer loss
    /// included).
    pub paused_s: u32,
    /// Riding seconds without a value, per sensor.
    pub gap_s: LapGaps,
    /// The lap's share of the ride's compliance tally (ADR-0002 §7.8).
    pub compliance: StreamCompliance,
}

/// One closed lap (ADR-0001 §6 `lap`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Lap {
    /// Zero-based, in ride order.
    pub idx: u32,
    /// Its block; `null` in a free ride.
    pub block: Option<LapBlock>,
    /// How it ended.
    pub end: LapEnd,
    /// Session second at which it began.
    pub start_s: f64,
    /// Session second at which it ended.
    pub end_s: f64,
    /// Workout second at which it began.
    pub workout_start_s: f64,
    /// Workout second at which it ended.
    pub workout_end_s: f64,
    /// Its statistics.
    pub stats: LapStats,
}

/// The lap being ridden.
#[derive(Debug)]
struct OpenLap {
    block: Option<LapBlock>,
    session_start: u64,
    workout_start: u64,
    /// Power of each record second that has one, in order (for NP).
    power: Vec<f64>,
    /// Their running sum: the live mean is read every tick.
    power_sum: Sum,
    cadence: Sum,
    hr: Sum,
    paused_s: u32,
    gaps: LapGaps,
    compliance: Compliance,
}
#[derive(Clone, Copy, Debug, Default)]
struct Sum {
    sum: f64,
    n: u32,
}
impl Sum {
    fn add(&mut self, v: f64) {
        self.sum += v;
        self.n += 1;
    }
    fn mean(self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / f64::from(self.n))
    }
}

/// The laps of one ride, owned by its session.
#[derive(Debug, Default)]
pub(crate) struct Laps {
    closed: Vec<Lap>,
    open: Option<OpenLap>,
    /// The ride's compliance tally when the last second was attributed.
    tally: Compliance,
}
impl Laps {
    /// Open a lap at session tick `session`, workout tick `workout`.
    pub(crate) fn open(&mut self, block: Option<(usize, &FlatBlock)>, session: u64, workout: u64) {
        self.open = Some(OpenLap {
            block: block.map(|(i, b)| LapBlock::new(i, b)),
            session_start: session,
            workout_start: workout,
            power: Vec::new(),
            power_sum: Sum::default(),
            cadence: Sum::default(),
            hr: Sum::default(),
            paused_s: 0,
            gaps: LapGaps::default(),
            compliance: Compliance::default(),
        });
    }
    /// Close the open lap at session tick `session`, workout tick `workout`.
    pub(crate) fn close(&mut self, end: LapEnd, session: u64, workout: u64) {
        let Some(lap) = self.open.take() else {
            return;
        };
        // Record values are means of `u16`/`u8` samples, finite by construction, and a lap is
        // far shorter than `u32::MAX` windows: `normalized_power` cannot fail on them.
        let np = (!lap.power.is_empty())
            .then(|| normalized_power(&lap.power).ok())
            .flatten();
        self.closed.push(Lap {
            idx: wire(self.closed.len()),
            block: lap.block,
            end,
            start_s: ticks_to_seconds(lap.session_start),
            end_s: ticks_to_seconds(session),
            workout_start_s: ticks_to_seconds(lap.workout_start),
            workout_end_s: ticks_to_seconds(workout),
            stats: LapStats {
                avg_power_w: lap.power_sum.mean(),
                np_w: np,
                avg_hr_bpm: lap.hr.mean(),
                avg_cadence_rpm: lap.cadence.mean(),
                paused_s: lap.paused_s,
                gap_s: lap.gaps,
                compliance: StreamCompliance {
                    counted_s: lap.compliance.counted_s,
                    within_s: lap.compliance.within_s,
                    excluded_s: lap.compliance.excluded_s,
                },
            },
        });
    }
    /// A record second was appended: it belongs to the open lap.
    pub(crate) fn on_second(&mut self, row: &SecondRow) {
        let Some(lap) = self.open.as_mut() else {
            return;
        };
        if row.workout_s.is_none() {
            lap.paused_s += 1;
            return;
        }
        match row.power {
            Some(p) => {
                lap.power.push(f64::from(p));
                lap.power_sum.add(f64::from(p));
            }
            None => lap.gaps.power_s += 1,
        }
        match row.cadence {
            Some(c) => lap.cadence.add(f64::from(c)),
            None => lap.gaps.cadence_s += 1,
        }
        match row.hr {
            Some(h) => lap.hr.add(f64::from(h)),
            None => lap.gaps.hr_s += 1,
        }
    }
    /// The policy has classified the second just appended: the change of the ride's tally
    /// goes to the open lap. Called after the policy, on the tick that closed the second.
    pub(crate) fn on_compliance(&mut self, tally: Compliance) {
        let before = std::mem::replace(&mut self.tally, tally);
        if let Some(lap) = self.open.as_mut() {
            let c = &mut lap.compliance;
            c.counted_s += tally.counted_s.saturating_sub(before.counted_s);
            c.within_s += tally.within_s.saturating_sub(before.within_s);
            c.excluded_s += tally.excluded_s.saturating_sub(before.excluded_s);
        }
    }
    /// Mean power of the open lap's record seconds so far: the closed lap's `avg_power_w`
    /// when it ends. `None` without a lap or a second with power.
    pub(crate) fn live_power(&self) -> Option<f64> {
        self.open.as_ref().and_then(|l| l.power_sum.mean())
    }
    /// The closed laps, in order.
    pub(crate) fn closed(&self) -> &[Lap] {
        &self.closed
    }
}
