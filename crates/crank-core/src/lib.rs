//! Application facade and Rust↔UI contract (ADR-0001 §4).
//!
//! [`Core`] is the API: async methods over the store, the devices and the ride engine.
//! `crank-desktop` exposes them one-to-one as typed Tauri commands; every error that
//! crosses is an [`ApiError`].
//!
//! Scope (#34, MVP #256): discover and pair a trainer and strap, the workout library,
//! start/pause/resume/skip/end a ride, live frames, and the ride history.
//! Deferred, each with its own issue: coach and planner commands (#248), the medical
//! stop (#81), athlete inputs (#247), the health cap (#244), live edits (#240), live
//! device readings before a ride (#267).
mod backend;
mod core;
mod dto;
mod error;
mod library;
mod recorder;

pub use backend::DeviceMode;
pub use core::{Core, CoreConfig, FrameSink, RideEnd, Shutdown, SinkClosed};
pub use dto::*;
pub use error::{ApiError, ErrorCode};
