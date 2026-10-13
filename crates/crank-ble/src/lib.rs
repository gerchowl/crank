//! FE-C driver over a transport seam (ADR-0002 §§3–6).
//! Engine owns mode policy, counter rollover, gap filling and trainer following.
mod capture;
mod driver;
mod gatt;
mod pacing;
#[cfg(feature = "replay")]
mod replay;
mod telemetry;
pub use capture::*;
pub use crank_devices::Clock;
use crank_devices::{DeviceError, ProtocolErrorKind};
pub use driver::*;
pub use gatt::BtleplugTransport;
#[cfg(feature = "replay")]
pub use replay::ReplayTrainer;
fn protocol(error: impl std::fmt::Display, kind: ProtocolErrorKind) -> DeviceError {
    DeviceError::Protocol {
        kind,
        detail: error.to_string(),
    }
}
static CHECKSUM_ERRORS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Process-wide rejected checksum counter for instrumentation (ADR-0002 §4.1).
#[must_use]
pub fn checksum_error_count() -> u64 {
    CHECKSUM_ERRORS.load(std::sync::atomic::Ordering::Relaxed)
}
fn frame_error(error: crank_fec::FrameError) -> DeviceError {
    let kind = if error == crank_fec::FrameError::Checksum {
        CHECKSUM_ERRORS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ProtocolErrorKind::Checksum
    } else {
        ProtocolErrorKind::Framing
    };
    tracing::warn!(?error, ?kind, "rejected FE-C notification");
    protocol(error, kind)
}

#[cfg(test)]
mod tests;
