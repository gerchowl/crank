//! Application facade and Rust↔UI contract (ADR-0001 §4).
//!
//! [`Core`] is the API: async methods over the store, the devices and the ride engine.
//! `crank-desktop` exposes them one-to-one as typed Tauri commands; every error that
//! crosses is an [`ApiError`].
//!
//! Scope (#34, MVP #256): discover and pair a trainer and strap, the workout library,
//! start/pause/resume/skip/end a ride, live frames, the ride history, and the workout
//! editor's DSL parse/print and compile (#111).
//! The athlete model (ADR-0009 §2.1.3, #715): its commands, and the snapshot each ride is
//! loaded with. Deferred, each with its own issue: coach and planner commands (#248), the
//! medical stop (#81), the analysis executor and proposals (#717–#720), the compile's CP, W′
//! and HR-recovery τ (#247), the health cap (#244), live edits (#240), live device readings
//! before a ride (#267).
mod backend;
mod core;
mod dto;
mod error;
mod keeper;
mod library;
mod links;
mod recorder;
mod ui;

pub use backend::DeviceMode;
pub use core::{Core, CoreConfig, EngineHost, FrameSink, RideEnd, Shutdown, SinkClosed};
/// The workout tree and declarations the editor commands (`parse_dsl`, `print_dsl`,
/// `compile_workout`) carry.
pub use crank_domain::workout::{VarDecl, Workout, WorkoutNode};
pub use dto::*;
pub use error::{ApiError, ErrorCode};
pub use keeper::channel::LinkSink;
pub use ui::UiSink;
