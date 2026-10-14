//! The ride record on `crank-store`, and the summary written when a ride finishes
//! (ADR-0001 §5–6). The engine calls the recorder synchronously on its task.
use crate::dto::{BlockSummary, RideSummary};
use crank_domain::workout::{CompiledWorkout, FlatBlock, Workout, normalized_power};
use crank_engine::{METRICS, RecordError, RideRecorder, SecondRow};
use crank_store::{
    CompiledJson, JsonColumn, NewRide, RideSummaryJson, RideWorkoutJson, Sample, Store, StoreError,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// UTC Unix seconds now.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Everything a ride record needs besides its seconds.
pub(crate) struct RideMeta {
    pub(crate) id: String,
    pub(crate) workout: Workout,
    pub(crate) compiled: CompiledWorkout,
    pub(crate) trainer_device_id: Option<String>,
    pub(crate) hr_device_id: Option<String>,
    pub(crate) telemetry_source: &'static str,
}

/// [`RideRecorder`] over the store; keeps the seconds to summarize them at the end.
pub(crate) struct StoreRecorder {
    store: Arc<Store>,
    meta: RideMeta,
    rows: Vec<SecondRow>,
}
impl StoreRecorder {
    pub(crate) fn new(store: Arc<Store>, meta: RideMeta) -> Self {
        Self {
            store,
            meta,
            rows: Vec::new(),
        }
    }
}
fn record_error(e: &StoreError) -> RecordError {
    RecordError(e.to_string())
}
impl RideRecorder for StoreRecorder {
    fn start(&mut self, metrics: &[&str]) -> Result<(), RecordError> {
        let m = &self.meta;
        let compiled = with_version(&m.compiled).and_then(|v| CompiledJson::parse(&v));
        self.store
            .start_ride(NewRide {
                id: m.id.clone(),
                started_at: unix_now(),
                workout: RideWorkoutJson::new(&m.workout).map_err(|e| record_error(&e))?,
                compiled: compiled.map_err(|e| record_error(&e))?,
                trainer_device_id: m.trainer_device_id.clone(),
                hr_device_id: m.hr_device_id.clone(),
                telemetry_source: m.telemetry_source.into(),
                metrics: metrics.iter().map(|s| (*s).to_owned()).collect(),
            })
            .map_err(|e| record_error(&e))
    }
    fn append_second(&mut self, session_s: u32, row: &SecondRow) -> Result<(), RecordError> {
        let samples = METRICS
            .iter()
            .zip(row.values())
            .map(|(metric, value)| Sample {
                metric: (*metric).to_owned(),
                value,
                estimated: false,
            })
            .collect();
        self.store
            .append_second(&self.meta.id, session_s, samples)
            .map_err(|e| record_error(&e))?;
        self.rows.push(*row);
        Ok(())
    }
    fn finish(&mut self) -> Result<(), RecordError> {
        let summary = summarize(&self.rows, &self.meta.compiled.blocks);
        let json = with_version(&summary)
            .and_then(|v| RideSummaryJson::parse(&v))
            .map_err(|e| record_error(&e))?;
        self.store
            .finish_ride(&self.meta.id, unix_now(), json)
            .map_err(|e| record_error(&e))
    }
}
/// Serialize `value` as a JSON object with the `v` envelope of the store's columns.
fn with_version<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    let invalid = |reason: String| StoreError::InvalidInput(reason);
    let mut v = serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?;
    v.as_object_mut()
        .ok_or_else(|| invalid("not an object".into()))?
        .insert("v".into(), 1.into());
    Ok(v.to_string())
}

/// A second count as seconds; a ride never has 2^32 of them.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}
fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0_u32), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}
/// The summary of a ride's seconds against its planned blocks.
pub(crate) fn summarize(rows: &[SecondRow], blocks: &[FlatBlock]) -> RideSummary {
    let power: Vec<f64> = rows.iter().filter_map(|r| r.power.map(f64::from)).collect();
    let blocks = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let inside: Vec<&SecondRow> = rows
                .iter()
                .filter(|r| {
                    r.workout_s.is_some_and(|w| {
                        f64::from(w) > b.start_s && f64::from(w) <= b.start_s + b.dur_s
                    })
                })
                .collect();
            BlockSummary {
                index: u32::try_from(i).unwrap_or(u32::MAX),
                planned_s: b.dur_s,
                planned_w: f64::midpoint(b.power_from, b.power_to),
                actual_s: count(inside.len()),
                actual_w: mean(inside.iter().filter_map(|r| r.power.map(f64::from))),
            }
        })
        .collect();
    RideSummary {
        duration_s: count(rows.len()),
        avg_power_w: mean(power.iter().copied()),
        np_w: (!power.is_empty())
            .then(|| normalized_power(&power).ok())
            .flatten(),
        avg_hr_bpm: mean(rows.iter().filter_map(|r| r.hr.map(f64::from))),
        kj: power.iter().sum::<f64>() / 1000.0,
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_domain::workout::Phase;

    fn block(start_s: f64, dur_s: f64, w: f64) -> FlatBlock {
        FlatBlock {
            path: vec![],
            iteration: vec![],
            node_id: String::new(),
            start_s,
            dur_s,
            power_from: w,
            power_to: w,
            phase: Phase::Work,
            dur_range_s: None,
        }
    }
    fn row(power: Option<f32>, hr: Option<f32>, workout_s: Option<f32>) -> SecondRow {
        SecondRow {
            power,
            hr,
            workout_s,
            ..SecondRow::default()
        }
    }

    #[test]
    fn summary_attributes_seconds_to_blocks_and_skips_gaps() {
        let rows = [
            row(Some(100.0), Some(120.0), Some(1.0)),
            row(Some(200.0), None, Some(2.0)),
            row(None, Some(140.0), None), // paused second: no workout position
            row(Some(300.0), Some(160.0), Some(3.0)),
        ];
        let s = summarize(&rows, &[block(0.0, 2.0, 150.0), block(2.0, 2.0, 250.0)]);
        assert!((s.duration_s - 4.0).abs() < f64::EPSILON);
        assert!((s.avg_power_w.unwrap() - 200.0).abs() < 1e-9);
        assert!((s.avg_hr_bpm.unwrap() - 140.0).abs() < 1e-9);
        assert!((s.kj - 0.6).abs() < 1e-9);
        assert_eq!(s.blocks.len(), 2);
        assert!((s.blocks[0].actual_s - 2.0).abs() < f64::EPSILON);
        assert!((s.blocks[0].actual_w.unwrap() - 150.0).abs() < 1e-9);
        assert!((s.blocks[1].actual_s - 1.0).abs() < f64::EPSILON);
        assert!((s.blocks[1].planned_w - 250.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_ride_has_no_averages() {
        let s = summarize(&[], &[]);
        assert!(s.avg_power_w.is_none() && s.np_w.is_none() && s.avg_hr_bpm.is_none());
        assert!(s.kj.abs() < f64::EPSILON && s.blocks.is_empty());
    }
}
