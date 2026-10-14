//! Wire types for ADR-0002 §4; raw codes retain unknown/reserved values.

/// Supported ANT data message types. Acknowledged data does not confirm application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    /// Broadcast data (0x4E).
    Broadcast = 0x4e,
    /// Acknowledged data (0x4F).
    Acknowledged = 0x4f,
}

/// A verified ANT frame, retaining the received channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AntFrame {
    /// ANT message type.
    pub msg_type: MsgType,
    /// ANT channel (normally 5).
    pub channel: u8,
    /// Typed FE-C page; unspecified pages preserve their bytes.
    pub page: Page,
}

/// Decoded ADR-0002 §4 pages, including controls for captures and round trips.
/// Unspecified pages (including 17/19) pass through as [`Page::Unknown`].
/// Adding a page requires an ADR-0002 amendment first; evaluation of 17/19
/// against HIL captures is tracked in <https://github.com/gerchowl/crank/issues/139>.
/// Page 49 and page 51 Crr use the full numeric domains decided in ADR-0002 §4.3,
/// including 0xFFFF target power and 0xFF Crr. Whether those endpoints are the
/// profile's invalid sentinels is **tentative pending HIL #44**; the raw numeric
/// value is kept, with no invented semantics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    /// Page 16, general fitness equipment telemetry.
    GeneralFe(GeneralFe),
    /// Page 25, trainer telemetry.
    Trainer(TrainerData),
    /// Page 48, basic resistance.
    BasicResistance {
        /// Resistance percent, resolution 0.5%; 0xFF is unspecified.
        pct: Option<f32>,
    },
    /// Page 49, target power.
    TargetPower {
        /// Target watts, in 0.25 W increments.
        watts: f32,
    },
    /// Page 50, wind parameters; 0xFF fields are unspecified.
    Wind {
        /// Wind coefficient in kg/m, resolution 0.01 kg/m.
        cw_kg_per_m: Option<f32>,
        /// Wind speed in km/h, resolution 1 km/h; positive is headwind.
        speed_kmh: Option<f32>,
        /// Dimensionless drafting factor, resolution 0.01.
        drafting: Option<f32>,
    },
    /// Page 51, track parameters.
    Track {
        /// Grade percent, resolution 0.01%; 0xFFFF is unspecified.
        grade_pct: Option<f32>,
        /// Dimensionless rolling coefficient, resolution 0.00005; 0xFF is numeric.
        crr: f32,
    },
    /// Page 54, fitness equipment capabilities; 0xFFFF max resistance is
    /// raw and **tentative pending HIL #44**.
    Capabilities(FeCapabilities),
    /// Page 55, user configuration, using the ANT+ FE profile layout (#70).
    UserConfig {
        /// Rider mass in kg, resolution 0.01 kg; 0xFFFF is invalid.
        rider_kg: Option<f32>,
        /// Bicycle mass in kg, resolution 0.05 kg; 0xFFF is invalid.
        bike_kg: Option<f32>,
        /// Wheel diameter in m, resolution 0.001 m; 0xFF coarse byte or
        /// offsets 10–14 give None. Treating 10–14 as reserved (rather than
        /// valid millimetres) is **tentative pending HIL #44**.
        /// Offset 0xF means no offset, so only the coarse diameter is used.
        wheel_m: Option<f32>,
        /// Gear ratio in 0.03 increments; zero is invalid.
        gear_ratio: Option<f32>,
    },
    /// Page 70, page request.
    Request {
        /// Requested page number.
        page: u8,
        /// Seven-bit transmission count (zero retained as received; its
        /// meaning, possibly "until acknowledged", is **tentative pending HIL #44**).
        times: u8,
        /// Whether the reply should use acknowledged data.
        acknowledged: bool,
        /// Request command type (1 means data page).
        command_type: u8,
    },
    /// Page 71, uncorrelated trainer observation, never command confirmation.
    CommandStatus(CommandStatus),
    /// Page 80, manufacturer information; NEO cross-check is #44.
    Manufacturer(ManufacturerInfo),
    /// Page 81, product information; NEO cross-check is #44.
    Product(ProductInfo),
    /// Unrecognized page number and all eight original page bytes.
    Unknown {
        /// Unsupported page number, also present in `raw[0]`.
        page: u8,
        /// All eight page bytes, preserved verbatim.
        raw: [u8; 8],
    },
}

/// Page 16; elapsed time and distance are counters, not unwrapped totals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneralFe {
    /// Equipment type code (25 is trainer), bits 0-4; reserved bits 5-7 are masked.
    pub equipment_type: u8,
    /// Elapsed seconds, wrapping at 64 s, resolution 0.25 s.
    pub elapsed_s: f32,
    /// Distance in metres, wrapping at 256 m.
    pub distance_m: u8,
    /// Speed in m/s, resolution 0.001 m/s (0xFFFF means unavailable).
    pub speed_mps: Option<f32>,
    /// Heart rate in bpm (0xFF means unavailable).
    pub heart_rate_bpm: Option<u8>,
    /// FE state code: 2 ready, 3 in use, 4 finished; other codes retained.
    pub state: u8,
    /// Lap toggle bit, independent of the FE state.
    pub lap_toggle: bool,
    /// Low-nibble capabilities/source flags, retained without interpretation.
    pub flags: u8,
}

/// Page 25; callers unwrap counters using modular arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainerData {
    /// Update event counter, wrapping at 256.
    pub event_count: u8,
    /// Cadence in rpm (0xFF means unavailable).
    pub cadence_rpm: Option<u8>,
    /// Accumulated power counter in W, wrapping at 65536.
    pub accumulated_power_w: u16,
    /// Instantaneous power in W (12-bit 0xFFF means unavailable).
    pub power_w: Option<u16>,
    /// Status nibble: bits 0/1/2 request power/resistance calibration/user config.
    pub status: u8,
    /// Target limit code: 0 at target, 1 speed low, 2 speed high, 3 undetermined.
    pub target_limit: u8,
    /// FE state code, with the lap bit removed.
    pub state: u8,
    /// Lap toggle bit.
    pub lap_toggle: bool,
}

/// Page 54, subject to NEO verification in #44.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeCapabilities {
    /// Maximum resistance in N, raw; 0xFFFF may mean unavailable (**tentative
    /// pending HIL #44**, ADR-0002 §4.2), so callers decide.
    pub max_resistance_n: u16,
    /// Supported modes: bits 0/1/2 basic resistance/target power/simulation.
    pub modes: u8,
}

/// Page 71 as reported; neither sequence nor echo correlates a specific command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandStatus {
    /// Last received command page number.
    pub last_page: u8,
    /// Uninterpreted sequence byte; semantics await #44.
    pub sequence: u8,
    /// Status: 0 pass, 1 fail, 2 unsupported, 3 rejected, 4 pending; others retained.
    pub status: u8,
    /// Last command's bytes 4–7, verbatim; no confirmation claim.
    pub echo: [u8; 4],
}

/// Page 80 common manufacturer fields, subject to NEO verification in #44.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManufacturerInfo {
    /// Hardware revision byte.
    pub hardware_revision: u8,
    /// Little-endian manufacturer identifier.
    pub manufacturer_id: u16,
    /// Little-endian model number.
    pub model: u16,
}

/// Page 81 common product fields, subject to NEO verification in #44.
/// Common-page layout cross-checked against Loghorn/ant-plus's
/// `src/fitness-equipment-sensors.ts` (pages 0x50 and 0x51).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductInfo {
    /// Main software revision byte, retained without combining revisions.
    pub software_revision: u8,
    /// Supplemental software revision byte (0xFF means absent).
    pub supplemental_revision: Option<u8>,
    /// Little-endian serial number; 0xFFFFFFFF is retained as unspecified.
    pub serial: u32,
}

/// Outgoing controls, independent of transport and engine policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlPage {
    /// Page 48: resistance percent, 0..=100.
    BasicResistance {
        /// Resistance in percent, rounded to 0.5%.
        pct: f32,
    },
    /// Page 49: target power, 0..=16383.75 W.
    TargetPower {
        /// Watts, rounded to 0.25 W.
        watts: f32,
    },
    /// Page 50: wind resistance parameters.
    Wind {
        /// Coefficient in kg/m, 0..=1.86, rounded to 0.01 kg/m.
        cw_kg_per_m: f32,
        /// Headwind in km/h, -127..=127, rounded to 1 km/h.
        speed_kmh: f32,
        /// Drafting factor, 0..=1, rounded to 0.01.
        drafting: f32,
    },
    /// Page 51: simulation track parameters.
    Track {
        /// Grade percent, -200..=200, rounded to 0.01%.
        grade_pct: f32,
        /// Rolling coefficient, 0..=0.01275, rounded to 0.00005.
        crr: f32,
    },
    /// Page 55: ANT+ FE profile User Configuration layout, cross-checked
    /// against public implementations, with the corrected packing decided in #70.
    /// #44 must confirm NEO acceptance. Diameter is rounded to integer mm, then
    /// coarse byte = mm / 10, offset nibble = mm % 10: 708 mm gives 70 + 8.
    /// Unlike pycycling's rounded-centimetre residual, the offset is never negative
    /// and cannot borrow into bicycle mass. Gear ratio is unspecified (byte 7 = 0).
    UserConfig {
        /// Rider kg, 0..=655.34, rounded to 0.01 kg (0xFFFF excluded).
        rider_kg: f32,
        /// Bicycle kg, 0..=204.7, rounded to 0.05 kg (0xFFF excluded).
        bike_kg: f32,
        /// Wheel diameter in m, 0..=2.54, rounded to 1 mm.
        wheel_m: f32,
    },
    /// Page 70: request acknowledged replies, with unused bytes 0xFF.
    Request {
        /// Requested page number.
        page: u8,
        /// Seven-bit transmission count, 0..=127.
        times: u8,
    },
}

/// A [`ControlPage`] field that failed [`encode`](crate::encode) validation.
///
/// Callers match on the variant, not on a string. Variants are named after the
/// `ControlPage` field they describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Field {
    /// `BasicResistance::pct`.
    Pct,
    /// `TargetPower::watts`.
    Watts,
    /// `Wind::cw_kg_per_m`.
    CwKgPerM,
    /// `Wind::speed_kmh`.
    SpeedKmh,
    /// `Wind::drafting`.
    Drafting,
    /// `Track::grade_pct`.
    GradePct,
    /// `Track::crr`.
    Crr,
    /// `UserConfig::rider_kg`.
    RiderKg,
    /// `UserConfig::bike_kg`.
    BikeKg,
    /// `UserConfig::wheel_m`.
    WheelM,
    /// `Request::times`.
    Times,
}

impl Field {
    /// The `ControlPage` field name this variant stands for.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pct => "pct",
            Self::Watts => "watts",
            Self::CwKgPerM => "cw_kg_per_m",
            Self::SpeedKmh => "speed_kmh",
            Self::Drafting => "drafting",
            Self::GradePct => "grade_pct",
            Self::Crr => "crr",
            Self::RiderKg => "rider_kg",
            Self::BikeKg => "bike_kg",
            Self::WheelM => "wheel_m",
            Self::Times => "times",
        }
    }
}

/// Input rejected before rounding; `field` identifies the domain violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeError {
    /// The invalid field; non-finite input is reported against its field too.
    pub field: Field,
}

/// Rejected ANT framing; checksum failures are never passed to page decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer than 13 bytes.
    Truncated,
    /// Sync byte is not 0xA4.
    Sync,
    /// Payload length is not 9, or frame has trailing bytes.
    Length,
    /// XOR of all bytes including sync and checksum is nonzero.
    Checksum,
    /// Message type is neither broadcast nor acknowledged data.
    MessageType(u8),
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid FE-C field: {}", self.field.name())
    }
}
impl std::error::Error for EncodeError {}
impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid ANT frame: {self:?}")
    }
}
impl std::error::Error for FrameError {}
