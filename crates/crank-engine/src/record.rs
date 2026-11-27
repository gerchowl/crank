//! The 1 Hz ride record (ADR-0001 §5–6): per-second means by session second.
//! Gap filling from accumulated power (`estimated`) is #223; rows carry no flag yet.
use crank_domain::record::{BlockEnd, ComplianceClass};
use std::collections::BTreeMap;
use thiserror::Error;

pub use crank_domain::record::RECORD_VERSION;

/// Metric ids written to the record, the store's catalogue names, in [`SecondRow::values`]
/// order. The first five are the version-1 record ([`METRICS_V1`]); the last five came with
/// version 2 (ADR-0009 §2.3.2, [`RECORD_VERSION`]).
pub const METRICS: [&str; 10] = [
    "power",
    "cadence",
    "speed",
    "hr",
    "workout_s",
    "target_w",
    "compliance",
    "adj_rider_pct",
    "adj_coach_pct",
    "block_end",
];
/// The engine metrics of a version-1 record.
pub const METRICS_V1: [&str; 5] = ["power", "cadence", "speed", "hr", "workout_s"];
/// The record metric holding [`SecondRow::block`], in every version. Not in [`METRICS`]: the
/// recorder adds it.
pub const BLOCK_METRIC: &str = "block";

/// Recording failed; the ride continues and the status carries the message.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("record: {0}")]
pub struct RecordError(pub String);

/// What a version-2 record holds of the control of one second (ADR-0009 §2.3.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecondControl {
    /// The controlled target at the last riding tick of the second, W: the target the rider
    /// was given, the one compliance compares with (ADR-0002 §7.8; during a soft start or a
    /// release, the block's target, not the ramp step or the 0 % resistance). `None` (NaN)
    /// outside ERG and without a riding tick.
    pub target_w: Option<f32>,
    /// The second's compliance class.
    pub compliance: ComplianceClass,
    /// The rider's cumulative ± adjustment of the target at that second, % of the planned
    /// target. Always 0: the engine has no rider ± yet (#761); 0 is what holds without one.
    pub adj_rider_pct: f32,
    /// The coach's cumulative adjustment of the target at that second, % of the planned
    /// target. Always 0: the engine applies no coach change yet (#248).
    pub adj_coach_pct: f32,
    /// How the block ended, on the block's last riding second only; `None` (NaN) elsewhere
    /// and in a ride without blocks. The engine appends the second without it and sets it
    /// with [`RideRecorder::set_block_end`] once the block has ended. [`BlockEnd::Edited`] is not written until the engine has
    /// live edits (#240).
    pub block_end: Option<BlockEnd>,
}

/// One session second, aligned with [`METRICS`]. `None` is a gap, never interpolated.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SecondRow {
    /// Mean trainer power, W.
    pub power: Option<f32>,
    /// Mean cadence, rpm.
    pub cadence: Option<f32>,
    /// Mean speed, m/s.
    pub speed: Option<f32>,
    /// Mean heart rate, bpm.
    pub hr: Option<f32>,
    /// Workout position at the end of the second; `None` if no tick of it was riding.
    pub workout_s: Option<f32>,
    /// Index of the block the engine was in at the last riding tick of the second; `None`
    /// if no tick of it was riding. A skip moves it while `workout_s` keeps counting, so
    /// per-block statistics must use this, never `workout_s` against the planned ranges.
    /// Not in [`METRICS`]: the sink stores it as [`BLOCK_METRIC`].
    pub block: Option<u32>,
    /// The version-2 metrics; the engine always writes them, and only a version-1 record
    /// read back ([`decode_record`]) has `None`.
    pub control: Option<SecondControl>,
}
impl SecondRow {
    /// Values in [`METRICS`] order (the version-2 ones `None` without [`SecondRow::control`]).
    #[must_use]
    pub fn values(&self) -> [Option<f32>; 10] {
        let c = self.control;
        [
            self.power,
            self.cadence,
            self.speed,
            self.hr,
            self.workout_s,
            c.and_then(|c| c.target_w),
            c.map(|c| f32::from(c.compliance.code())),
            c.map(|c| c.adj_rider_pct),
            c.map(|c| c.adj_coach_pct),
            c.and_then(|c| c.block_end).map(|e| f32::from(e.code())),
        ]
    }
    /// What the domain's interval statistics read of this second
    /// ([`crank_domain::record::interval_stats`]).
    #[must_use]
    pub fn interval_second(&self) -> crank_domain::record::IntervalSecond {
        crank_domain::record::IntervalSecond {
            riding: self.workout_s.is_some(),
            power: self.power.map(f64::from),
            cadence: self.cadence.map(f64::from),
            hr: self.hr.map(f64::from),
            compliance: self.control.map(|c| c.compliance),
        }
    }
}

/// A stored code (`compliance`, `block_end`) as its integer, if it is one in `0..=255`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "checked to be an integer in 0..=255 first"
)]
fn code(v: f32) -> Option<u8> {
    (v.fract() == 0.0 && (0.0..=255.0).contains(&v)).then_some(v as u8)
}

/// Read a ride's record back into rows: `columns` holds each stored metric's values by session
/// second (the store's `Store::record`), `version` the ride's `record_version`.
///
/// A version-1 record yields rows with `control: None`; a version-2 record yields every row
/// with its [`SecondControl`]. Extra metrics are ignored.
///
/// # Errors
/// An unknown version, a missing metric, metrics of different lengths, a `block` that is not
/// a block index, or a `compliance` or `block_end` value that is no code.
pub fn decode_record(
    version: u32,
    columns: &BTreeMap<String, Vec<Option<f32>>>,
) -> Result<Vec<SecondRow>, RecordError> {
    let names: &[&str] = match version {
        1 => &METRICS_V1,
        2 => &METRICS,
        v => return Err(RecordError(format!("unknown record version {v}"))),
    };
    let column = |name: &str| {
        columns
            .get(name)
            .ok_or_else(|| RecordError(format!("version {version} record without `{name}`")))
    };
    let block = column(BLOCK_METRIC)?;
    let values: Vec<&Vec<Option<f32>>> =
        names.iter().map(|n| column(n)).collect::<Result<_, _>>()?;
    let n = block.len();
    if values.iter().any(|v| v.len() != n) {
        return Err(RecordError("record metrics of different lengths".into()));
    }
    let bad = |what: &str, s: usize| RecordError(format!("second {s}: invalid {what}"));
    (0..n)
        .map(|s| {
            let at = |i: usize| values[i][s];
            let control = if version >= 2 {
                let compliance = at(6)
                    .and_then(code)
                    .and_then(ComplianceClass::from_code)
                    .ok_or_else(|| bad("compliance", s))?;
                let block_end = at(9)
                    .map(|v| {
                        code(v)
                            .and_then(BlockEnd::from_code)
                            .ok_or_else(|| bad("block_end", s))
                    })
                    .transpose()?;
                Some(SecondControl {
                    target_w: at(5),
                    compliance,
                    adj_rider_pct: at(7).ok_or_else(|| bad("adj_rider_pct", s))?,
                    adj_coach_pct: at(8).ok_or_else(|| bad("adj_coach_pct", s))?,
                    block_end,
                })
            } else {
                None
            };
            Ok(SecondRow {
                power: at(0),
                cadence: at(1),
                speed: at(2),
                hr: at(3),
                workout_s: at(4),
                block: block[s]
                    .map(|b| {
                        (b.fract() == 0.0 && (0.0..16_777_216.0).contains(&b))
                            .then(|| {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_sign_loss,
                                    reason = "checked to be an integer in 0..2^24 first"
                                )]
                                let i = b as u32;
                                i
                            })
                            .ok_or_else(|| bad("block", s))
                    })
                    .transpose()?,
                control,
            })
        })
        .collect()
}

/// How the engine finished a ride, for the record's end cause (ADR-0009 §2.3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RideFinish {
    /// The workout ran out: its last block's time, or a skip of its last block.
    Completed,
    /// `EngineCmd::Stop` ended it (the rider's End, or the app quitting).
    Stopped,
}

/// Where a ride's record goes; core implements it over `crank-store` (the engine does
/// not depend on the store, ADR-0001 §2). Calls are synchronous on the engine task.
pub trait RideRecorder: Send + 'static {
    /// The ride started; `metrics` is the fixed set of the record, of [`RECORD_VERSION`].
    ///
    /// # Errors
    /// Any failure to open the ride record.
    fn start(&mut self, metrics: &[&str]) -> Result<(), RecordError>;
    /// Append session second `session_s` (consecutive from 0, paused seconds included).
    ///
    /// # Errors
    /// Any failure to persist the row.
    fn append_second(&mut self, session_s: u32, row: &SecondRow) -> Result<(), RecordError>;
    /// The block of second `session_s`, already appended, ended there as `end`: set that
    /// second's `block_end` (ADR-0009 §2.3.2). Called once per block, after its last riding
    /// second, when the block's end is known.
    ///
    /// # Errors
    /// Any failure to persist the change.
    fn set_block_end(&mut self, session_s: u32, end: BlockEnd) -> Result<(), RecordError>;
    /// The ride finished normally, as `how` says.
    ///
    /// # Errors
    /// Any failure to close the ride record.
    fn finish(&mut self, how: RideFinish) -> Result<(), RecordError>;
}

/// Mean of the non-null tick values of one second.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Mean {
    sum: f64,
    n: u32,
}
impl Mean {
    pub(crate) fn add(&mut self, v: Option<f64>) {
        if let Some(v) = v {
            self.sum += v;
            self.n += 1;
        }
    }
    #[expect(clippy::cast_possible_truncation, reason = "the record is f32")]
    pub(crate) fn mean(&self) -> Option<f32> {
        (self.n > 0).then(|| (self.sum / f64::from(self.n)) as f32)
    }
}
