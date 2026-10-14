//! The only error type that crosses the boundary (ADR-0001 §4.4).
use crank_devices::DeviceError;
use crank_domain::workout::CompileError;
use crank_engine::{CommandError, StreamError};
use crank_store::StoreError;
use serde::{Deserialize, Serialize};

/// Machine-readable failure class; the UI branches on this, never on `message`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// A request value is not acceptable.
    InvalidInput,
    /// The named entity does not exist.
    NotFound,
    /// The request conflicts with current state: a ride already running, no ride
    /// loaded, `ride.lock` held by another process, a stream limit.
    Conflict,
    /// A trainer or strap is missing, unreachable or failed.
    DeviceUnavailable,
    /// The OS denied Bluetooth.
    PermissionDenied,
    /// A remote service failed (coach, intervals.icu; not used by the ride core).
    Upstream,
    /// The store failed.
    Storage,
    /// A bug or an unexpected internal state.
    Internal,
}

/// A failed command: a code plus a human-readable message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct ApiError {
    /// Failure class.
    pub code: ErrorCode,
    /// Diagnostic text for the user or the log.
    pub message: String,
}
impl ApiError {
    /// Build an error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        let code = match e {
            StoreError::LockBusy => ErrorCode::Conflict,
            StoreError::InvalidInput(_) => ErrorCode::InvalidInput,
            _ => ErrorCode::Storage,
        };
        Self::new(code, e.to_string())
    }
}
impl From<CommandError> for ApiError {
    fn from(e: CommandError) -> Self {
        let code = match e {
            CommandError::NotLoaded | CommandError::WrongState => ErrorCode::Conflict,
            CommandError::NoTrainer | CommandError::TrainerNotReady => ErrorCode::DeviceUnavailable,
            CommandError::Record(_) => ErrorCode::Storage,
            CommandError::EngineStopped => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}
impl From<StreamError> for ApiError {
    fn from(e: StreamError) -> Self {
        let code = match e {
            StreamError::UnknownStream | StreamError::UnknownSub => ErrorCode::NotFound,
            StreamError::TooManySubscriptions | StreamError::TooManyStreams => ErrorCode::Conflict,
            StreamError::InvalidRate | StreamError::InvalidFilter(_) => ErrorCode::InvalidInput,
            StreamError::EngineStopped => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}
impl From<DeviceError> for ApiError {
    fn from(e: DeviceError) -> Self {
        let code = match e {
            DeviceError::PermissionDenied => ErrorCode::PermissionDenied,
            DeviceError::Protocol { .. } | DeviceError::Capture(_) => ErrorCode::Internal,
            _ => ErrorCode::DeviceUnavailable,
        };
        Self::new(code, e.to_string())
    }
}
impl From<CompileError> for ApiError {
    fn from(e: CompileError) -> Self {
        Self::new(
            ErrorCode::InvalidInput,
            format!("workout does not compile: {e}"),
        )
    }
}
