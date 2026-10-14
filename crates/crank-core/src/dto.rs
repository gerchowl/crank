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
    /// Always `null` until ride lifecycle lands (#34 part B2).
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
