//! The 1 Hz ride record (ADR-0001 §5–6): per-second means by session second.
//! Gap filling from accumulated power (`estimated`) is #223; rows carry no flag yet.
use thiserror::Error;

/// Metric ids written to the record, the store's catalogue names.
pub const METRICS: [&str; 5] = ["power", "cadence", "speed", "hr", "workout_s"];

/// Recording failed; the ride continues and the status carries the message.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("record: {0}")]
pub struct RecordError(pub String);

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
}
impl SecondRow {
    /// Values in [`METRICS`] order.
    #[must_use]
    pub fn values(&self) -> [Option<f32>; 5] {
        [
            self.power,
            self.cadence,
            self.speed,
            self.hr,
            self.workout_s,
        ]
    }
}

/// Where a ride's record goes; core implements it over `crank-store` (the engine does
/// not depend on the store, ADR-0001 §2). Calls are synchronous on the engine task.
pub trait RideRecorder: Send + 'static {
    /// The ride started; `metrics` is the fixed set of the record.
    ///
    /// # Errors
    /// Any failure to open the ride record.
    fn start(&mut self, metrics: &[&str]) -> Result<(), RecordError>;
    /// Append session second `session_s` (consecutive from 0, paused seconds included).
    ///
    /// # Errors
    /// Any failure to persist the row.
    fn append_second(&mut self, session_s: u32, row: &SecondRow) -> Result<(), RecordError>;
    /// The ride finished normally.
    ///
    /// # Errors
    /// Any failure to close the ride record.
    fn finish(&mut self) -> Result<(), RecordError>;
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
