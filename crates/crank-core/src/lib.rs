//! Application facade and Rust↔UI contract (ADR-0001 §4).
//!
//! [`Core`] is the API: async methods over the store, the devices and the ride engine.
//! `crank-desktop` exposes them one-to-one as typed Tauri commands; every error that
//! crosses is an [`ApiError`].
//!
//! Scope of this part (#34 B1, MVP #256): `core_ready`, discover and pair a trainer and
//! strap, and the workout library. Ride lifecycle, streams and history follow in B2.
//! Deferred, each with its own issue: coach and planner commands (#248), the medical
//! stop (#81), athlete inputs (#247), the health cap (#244), live edits (#240), live
//! device readings before a ride (#267).
mod backend;
mod core;
mod dto;
mod error;
mod library;

pub use backend::DeviceMode;
pub use core::{Core, CoreConfig};
pub use dto::*;
pub use error::{ApiError, ErrorCode};

/// Placeholder response used to exercise generated bindings before #34.
#[derive(serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldInfo {
    /// Display name of the application.
    pub app_name: String,
}
