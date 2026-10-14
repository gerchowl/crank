//! Wire types of the facade (ADR-0001 §4.1: camelCase structs, `snake_case` enums,
//! seconds as `f64`, `Option` as `T | null`).
use crank_domain::workout::{CompiledWorkout, Workout};
use crank_engine::LinkStatus;
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
    pub compiled: CompiledWorkout,
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
