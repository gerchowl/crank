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
    /// Eight page bytes, preserved verbatim; typed telemetry decoding is #129.
    pub page: [u8; 8],
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

/// Input rejected before rounding; the field name identifies the domain violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeError {
    /// Invalid field name; includes non-finite input.
    pub field: &'static str,
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
        write!(f, "invalid FE-C field: {}", self.field)
    }
}
impl std::error::Error for EncodeError {}
impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid ANT frame: {self:?}")
    }
}
impl std::error::Error for FrameError {}
