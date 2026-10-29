//! Wire types of the facade (ADR-0001 §4.1: camelCase structs, `snake_case` enums,
//! seconds as `f64`, `Option` as `T | null`).
use crate::error::{ApiError, ErrorCode};
use crank_domain::workout::{
    CompiledWorkout, FlatBlock, Phase, Severity, ValidationIssue, ValidationKind, Workout,
};
use crank_engine::{LinkStatus, StreamStatus};
use serde::{Deserialize, Serialize};

/// What the running core is: answered by `core_ready`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    /// Application version.
    pub version: String,
    /// Devices are simulated (`--sim`): no hardware is touched.
    pub sim: bool,
    /// Store schema version.
    pub schema_version: u32,
}

/// The two device roles of a ride.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    /// The smart trainer.
    Trainer,
    /// The heart-rate strap.
    HeartRate,
}
impl DeviceRole {
    /// The `device.kind` column value.
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::HeartRate => "hr",
        }
    }
}

/// A peripheral seen by a scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScannedDevice {
    /// Host-local peripheral id: pass it to `device_pair`.
    pub id: String,
    /// Advertised name.
    pub name: Option<String>,
    /// Offers a trainer protocol.
    pub trainer: bool,
    /// Offers the Heart Rate Service.
    pub heart_rate: bool,
    /// Already remembered in any role.
    pub paired: bool,
}

/// A remembered pairing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    /// Role it is used in.
    pub role: DeviceRole,
    /// Host-local peripheral id.
    pub id: String,
    /// Advertised name.
    pub name: Option<String>,
    /// DIS serial, read when pairing.
    pub serial: Option<String>,
    /// DIS model, read when pairing.
    pub model: Option<String>,
}

/// One role's pairing and live link (`device_links`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkInfo {
    /// The role.
    pub role: DeviceRole,
    /// Who is paired for it.
    pub device: Option<PairedDevice>,
    /// Live link state of the loaded ride; `null` without a ride, and after it ends.
    pub link: Option<LinkStatus>,
}

/// A library workout in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutSummary {
    /// Stable id: pass it to `ride_load`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Compiled duration, s.
    pub total_s: f64,
    /// Estimated training stress.
    pub tss: f64,
    /// Reference FTP, W.
    pub ftp: f64,
}

/// A workout with its compiled form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutDetail {
    /// The workout tree.
    pub workout: Workout,
    /// Its DSL text.
    pub dsl: String,
    /// The compiled timeline.
    pub compiled: CompiledView,
}

/// One compiled block. The boundary owns this type: `crank-domain`'s `FlatBlock` indexes
/// with `usize`, which TypeScript cannot carry without precision loss, so indices are
/// converted with a checked cast (`TryFrom`) and a ride never silently truncates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutBlock {
    /// Child indices into the workout tree.
    pub path: Vec<u32>,
    /// Repeat counter of every enclosing repeat.
    pub iteration: Vec<u32>,
    /// Id of the source node.
    pub node_id: String,
    /// Start on the compiled timeline, s.
    pub start_s: f64,
    /// Planned duration, s.
    pub dur_s: f64,
    /// Target power at the start, W.
    pub power_from: f64,
    /// Target power at the end, W.
    pub power_to: f64,
    /// Display phase.
    pub phase: Phase,
    /// Shortest and longest duration for open-ended blocks, `null` otherwise.
    pub dur_range_s: Option<[f64; 2]>,
}
fn index(i: usize) -> Result<u32, ApiError> {
    u32::try_from(i).map_err(|_| ApiError::new(ErrorCode::Internal, "block index exceeds u32"))
}
impl TryFrom<&FlatBlock> for WorkoutBlock {
    type Error = ApiError;
    fn try_from(b: &FlatBlock) -> Result<Self, ApiError> {
        Ok(Self {
            path: b.path.iter().map(|i| index(*i)).collect::<Result<_, _>>()?,
            iteration: b
                .iteration
                .iter()
                .map(|i| index(*i))
                .collect::<Result<_, _>>()?,
            node_id: b.node_id.clone(),
            start_s: b.start_s,
            dur_s: b.dur_s,
            power_from: b.power_from,
            power_to: b.power_to,
            phase: b.phase,
            dur_range_s: b.dur_range_s,
        })
    }
}

/// A prescription diagnostic of the compiled workout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutIssue {
    /// Machine-readable diagnosis.
    pub kind: ValidationKind,
    /// Error or warning.
    pub severity: Severity,
    /// Source node, `null` for whole-workout diagnostics.
    pub node_id: Option<String>,
    /// Display message.
    pub message: String,
}
impl From<&ValidationIssue> for WorkoutIssue {
    fn from(i: &ValidationIssue) -> Self {
        Self {
            kind: i.kind,
            severity: i.severity,
            node_id: i.node_id.clone(),
            message: i.message.clone(),
        }
    }
}

/// A compiled workout as the UI sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompiledView {
    /// Ordered blocks tiling the expected timeline.
    pub blocks: Vec<WorkoutBlock>,
    /// Expected total duration, s.
    pub total_s: f64,
    /// Sum of lower and upper duration bounds, s.
    pub total_range_s: [f64; 2],
    /// Training stress score.
    pub tss: f64,
    /// Intensity factor (NP / FTP).
    #[serde(rename = "if")]
    pub intensity_factor: f64,
    /// Seconds in each of seven power zones.
    pub time_in_zone_s: [u32; 7],
    /// Prescription diagnostics.
    pub issues: Vec<WorkoutIssue>,
}
impl TryFrom<&CompiledWorkout> for CompiledView {
    type Error = ApiError;
    fn try_from(c: &CompiledWorkout) -> Result<Self, ApiError> {
        Ok(Self {
            blocks: c
                .blocks
                .iter()
                .map(WorkoutBlock::try_from)
                .collect::<Result<_, _>>()?,
            total_s: c.total_s,
            total_range_s: c.total_range_s,
            tss: c.tss,
            intensity_factor: c.intensity_factor,
            time_in_zone_s: c.time_in_zone_s,
            issues: c.issues.iter().map(WorkoutIssue::from).collect(),
        })
    }
}

/// Answer of `ride_load`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RideLoaded {
    /// Id the ride record will have once started.
    pub ride_id: String,
    /// Workout, `null` for a free ride.
    pub workout_id: Option<String>,
    /// A strap is connected; without one the ride runs without heart rate.
    pub hr_linked: bool,
}

/// Answer of `ride_end`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RideEnded {
    /// The ride's id; `null` for a ride that never started and was only unloaded.
    pub ride_id: Option<String>,
    /// The engine could not end the ride (stuck or gone) and the core ended it by force: the
    /// ride is recorded as aborted, without a summary, and its data may be incomplete.
    pub forced: bool,
}

/// The ride's state (`ride_status`): the engine's status, and whether a ride is being
/// loaded (devices connecting) right now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RideStatusView {
    /// `ride_load` is connecting devices; the other commands answer as before.
    pub loading: bool,
    /// The loaded ride's status, the last finished ride's, or idle.
    pub status: StreamStatus,
}

/// How a stored ride ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RideOutcome {
    /// Still recording.
    Open,
    /// Ended by the rider or the workout.
    Finished,
    /// Closed by crash recovery.
    Aborted,
}

/// Planned vs actual for one block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockSummary {
    /// Index into the compiled blocks.
    pub index: u32,
    /// Planned duration, s.
    pub planned_s: f64,
    /// Planned mean power, W.
    pub planned_w: f64,
    /// Seconds actually ridden in the block.
    pub actual_s: f64,
    /// Mean power ridden in it, `null` if none was recorded.
    pub actual_w: Option<f64>,
}

/// The minimal summary written when a ride finishes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RideSummary {
    /// Recorded session seconds.
    pub duration_s: f64,
    /// Mean power, W.
    pub avg_power_w: Option<f64>,
    /// Normalized power, W.
    pub np_w: Option<f64>,
    /// Mean heart rate, bpm.
    pub avg_hr_bpm: Option<f64>,
    /// Work done, kJ.
    pub kj: f64,
    /// Planned vs actual per block.
    pub blocks: Vec<BlockSummary>,
}

/// A stored ride (`rides_list`, `ride_summary`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RideRecord {
    /// Ride id.
    pub id: String,
    /// UTC Unix seconds at start.
    pub started_at: f64,
    /// UTC Unix seconds at end.
    pub ended_at: Option<f64>,
    /// How it ended.
    pub outcome: RideOutcome,
    /// Ridden workout's id, `null` for a free ride.
    pub workout_id: Option<String>,
    /// Ridden workout's name.
    pub workout_name: String,
    /// The summary; `null` while open or aborted.
    pub summary: Option<RideSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_domain::workout::Phase;

    #[test]
    fn block_indices_are_checked_not_truncated() {
        let mut b = FlatBlock {
            path: vec![1, 2],
            iteration: vec![0],
            node_id: "n".into(),
            start_s: 0.0,
            dur_s: 1.0,
            power_from: 100.0,
            power_to: 100.0,
            phase: Phase::Work,
            dur_range_s: None,
        };
        assert_eq!(WorkoutBlock::try_from(&b).unwrap().path, [1, 2]);
        if let Ok(big) = usize::try_from(u64::from(u32::MAX) + 1) {
            b.path = vec![big];
            assert_eq!(
                WorkoutBlock::try_from(&b).unwrap_err().code,
                ErrorCode::Internal
            );
            b.path = vec![1];
            b.iteration = vec![big];
            assert_eq!(
                WorkoutBlock::try_from(&b).unwrap_err().code,
                ErrorCode::Internal
            );
        }
    }
}
