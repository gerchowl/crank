//! Transport-neutral connection factories and events (ADR-0002 §3).
//! See [`ControlMode::grid_key`] for checked wire-grid projection and
//! [`TrainerObservation`] for the uncorrelated observations of ADR-0002 §4.4.
//!
//! Monotonic timestamps use microseconds from a shared runtime epoch (ADR-0001 §5).

mod control;
pub use control::*;
use futures_util::{future::BoxFuture, stream::BoxStream};
use smallvec::SmallVec;
use tokio::sync::mpsc;

/// Monotonic microseconds from the runtime epoch; never UTC. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonoTime(pub u64);

/// Injectable runtime epoch and executor clock (ADR-0001 §5).
pub trait Clock: Send + Sync + 'static {
    /// Current monotonic time, shared with the engine.
    fn now(&self) -> MonoTime;
    /// Suspend until a monotonic deadline.
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()>;
}

/// Trainer protocol selected by discovery. See ADR-0002 §3.
/// Non-exhaustive to support future FTMS additions (#45).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DriverKind {
    /// ANT FE-C over BLE.
    FeC,
    /// Bluetooth Fitness Machine Service.
    Ftms,
    /// Deterministic simulator.
    Sim,
}
/// Opaque platform peripheral identity, stable on the current host. See ADR-0002 §3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRef(pub String);
/// Human-readable device identity, populated from DIS or FE-C pages 80–81. See ADR-0002 §3.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Advertised name.
    pub name: Option<String>,
    /// Model identifier.
    pub model: Option<String>,
    /// Serial number.
    pub serial: Option<String>,
    /// Firmware revision.
    pub firmware: Option<String>,
}
/// Classified discovery result; scanning is implemented by #147. See ADR-0002 §3.
#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    /// Host-local identity.
    pub device: DeviceRef,
    /// Available identity fields.
    pub info: DeviceInfo,
    /// Trainer protocol, or None for a heart-rate strap.
    pub trainer: Option<DriverKind>,
    /// Whether Heart Rate Service is present.
    pub heart_rate: bool,
}
/// Object-safe trainer connection factory (ADR-0002 §3).
pub trait TrainerDriver: Send + Sync + 'static {
    /// Protocol used by this driver.
    fn kind(&self) -> DriverKind;
    /// Connect, subscribe and configure rider/capabilities before returning.
    fn connect(
        &self,
        dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>>;
}
/// A connection owned by the driver's task. See ADR-0002 §3.
pub struct TrainerLink {
    /// Device identity.
    pub info: DeviceInfo,
    /// Advertised control capabilities.
    pub caps: TrainerCaps,
    /// Commands to the single writer task.
    pub control: mpsc::Sender<TrainerCommand>,
    /// Ordered, lossless connection events.
    pub events: mpsc::Receiver<TrainerEvent>,
}
/// Capabilities from FE-C page 54 (ADR-0002 §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainerCaps {
    /// Supports target power.
    pub erg: bool,
    /// Supports track and wind simulation.
    pub sim: bool,
    /// Supports basic resistance.
    pub resistance: bool,
    /// Maximum resistance in newtons, if reported.
    pub max_resistance_n: Option<u16>,
    /// Maximum target watts representable by the driver.
    pub max_target_w: u16,
}
/// Rider/equipment configuration sent on connection. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderConfig {
    /// Rider mass in kilograms.
    pub rider_kg: f32,
    /// Bicycle mass in kilograms.
    pub bike_kg: f32,
    /// Wheel diameter in metres, despite the historical ADR field name.
    pub wheel_m: f32,
}
/// Engine commands; mode selection and following policy belong to the engine. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrainerCommand {
    /// Replace the desired control mode.
    SetMode(ControlMode),
    /// Supply rider configuration (driver-specific connection lifecycle).
    SetRider(RiderConfig),
    /// Release the peripheral.
    Disconnect,
}
/// Driver events; delivery and trainer observation are distinct (ADR-0002 §4.4).
/// Non-exhaustive to support future FTMS additions (#45).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TrainerEvent {
    /// Telemetry at notification arrival time.
    Sample(TrainerSample),
    /// GATT write-with-response succeeded; no claim of trainer application.
    Delivered {
        /// Desired mode whose control page reached the peripheral.
        mode: ControlMode,
    },
    /// Exactly one event per received page 71, and page-25 flags.
    Observed(TrainerObservation),
    /// Transport/configuration readiness changed.
    Link(LinkState),
    /// Transport or protocol failure.
    Fault(DeviceError),
}
/// Link readiness, independent of control mode and trainer following. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Establishing transport and configuration.
    Connecting,
    /// Transport, rider configuration and capabilities are ready.
    Ready,
    /// Transport is lost or released.
    Lost,
}
/// Raw telemetry; the engine unwraps counters and fills gaps (ADR-0002 §5).
#[derive(Clone, Debug, PartialEq)]
pub struct TrainerSample {
    /// Notification arrival time.
    pub at: MonoTime,
    /// Instantaneous watts.
    pub power_w: Option<u16>,
    /// Cadence in revolutions per minute.
    pub cadence_rpm: Option<u8>,
    /// Speed in metres per second.
    pub speed_mps: Option<f32>,
    /// Raw page-16 distance counter, wrapping at 256 metres.
    pub distance_m: Option<f32>,
    /// Raw page-25 counter, wrapping at 65536 W.
    pub accumulated_power_w: Option<u32>,
    /// Raw page-25 event counter, wrapping at 256.
    pub event_count: Option<u8>,
    /// Calibration/configuration flags.
    pub status: TrainerStatus,
    /// Reported target-power limitation.
    pub target_limit: TargetLimit,
}
/// Raw page-25 status nibble; bits 0/1/2 request power/resistance/user setup. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainerStatus(pub u8);
/// Reported power limitation, independent of trainer following. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetLimit {
    /// Target power is attainable.
    AtTarget,
    /// Speed is too low.
    SpeedTooLow,
    /// Speed is too high.
    SpeedTooHigh,
    /// No determination available.
    Undetermined,
}
impl From<u8> for TargetLimit {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::AtTarget,
            1 => Self::SpeedTooLow,
            2 => Self::SpeedTooHigh,
            _ => Self::Undetermined,
        }
    }
}
/// Trainer-reported state; never an acknowledgement of a particular command. See ADR-0002 §3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainerObservation {
    /// Reply arrival time (ADR-0001 §5).
    pub at: MonoTime,
    /// Wire source of this report.
    pub source: ObservationSource,
    /// One control page on its integer grid, if present.
    pub control: Option<ObservedControl>,
    /// Reported command status, if present.
    pub status: Option<CommandStatus>,
    /// Page-25 target flags, if present.
    pub target_limit: Option<TargetLimit>,
}
/// Origin of an uncorrelated trainer observation. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationSource {
    /// Page-71 control echo.
    Page71Echo,
    /// Page-25 target flags.
    Page25Flags,
}
/// Exhaustive for engine routing R1–R5 (ADR-0002 §4.4.1).
/// Page-71 status, retaining every unknown byte. See ADR-0002 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    /// Command reported successful.
    Pass,
    /// Command reported failed.
    Fail,
    /// Command unsupported.
    NotSupported,
    /// Command rejected.
    Rejected,
    /// Command pending.
    Pending,
    /// Unrecognized status.
    Other(u8),
}
impl From<u8> for CommandStatus {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Pass,
            1 => Self::Fail,
            2 => Self::NotSupported,
            3 => Self::Rejected,
            4 => Self::Pending,
            other => Self::Other(other),
        }
    }
}
/// Object-safe heart-rate connection factory (implementation #147). See ADR-0002 §3.
pub trait HeartRateDriver: Send + Sync + 'static {
    /// Connect and subscribe to heart-rate notifications.
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>>;
}
/// Heart-rate connection owned by its driver task. See ADR-0002 §3.
pub struct HeartRateLink {
    /// Device identity.
    pub info: DeviceInfo,
    /// Ordered events.
    pub events: mpsc::Receiver<HrEvent>,
}
/// Heart-rate driver event. See ADR-0002 §3.
#[derive(Clone, Debug, PartialEq)]
pub enum HrEvent {
    /// Sensor sample.
    Sample(HrSample),
    /// Connection state.
    Link(LinkState),
    /// Transport/protocol failure.
    Fault(DeviceError),
}
/// Heart-rate sample from HRS. See ADR-0002 §3.
#[derive(Clone, Debug, PartialEq)]
pub struct HrSample {
    /// Arrival time.
    pub at: MonoTime,
    /// Beats per minute, if valid.
    pub bpm: Option<u8>,
    /// RR intervals in milliseconds.
    pub rr_ms: SmallVec<[u16; 4]>,
    /// Sensor contact, when supported.
    pub contact: Option<bool>,
}
/// Object-safe, post-filtered discovery stream (implementation #147). See ADR-0002 §3.
pub trait Discovery: Send + Sync + 'static {
    /// Scan and classify supported peripherals.
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice>;
}
/// Transport-neutral device failures (ADR-0002 §3).
/// Non-exhaustive to support future FTMS additions (#45).
#[derive(thiserror::Error, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceError {
    /// No adapter is available.
    #[error("adapter unavailable")]
    AdapterUnavailable,
    /// OS Bluetooth permission denied.
    #[error("permission denied")]
    PermissionDenied,
    /// Peripheral was not found.
    #[error("device not found")]
    NotFound,
    /// Connection could not be established.
    #[error("connect failed: {0}")]
    ConnectFailed(String),
    /// GATT operation failed.
    #[error("GATT: {0}")]
    Gatt(String),
    /// Invalid framing, page or control input; no codec dependency.
    #[error("protocol {kind:?}: {detail}")]
    Protocol {
        /// Stable, transport-neutral category.
        kind: ProtocolErrorKind,
        /// Diagnostic detail.
        detail: String,
    },
    /// Raw capture failed; this does not mean the peripheral disconnected (#43).
    #[error("capture: {0}")]
    Capture(String),
    /// Connection closed.
    #[error("disconnected")]
    Disconnected,
}
/// Categories shared by real, simulator and replay drivers. See ADR-0002 §3.
/// Non-exhaustive to support future FTMS additions (#45).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolErrorKind {
    /// Malformed transport frame.
    Framing,
    /// Invalid checksum.
    Checksum,
    /// Unsupported/unexpected page.
    UnexpectedPage,
    /// Control input cannot be encoded.
    Encode,
}
