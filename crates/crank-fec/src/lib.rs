//! Pure, allocation-free ANT/FE-C-over-BLE codec (ADR-0002 §4).
//!
//! XOR includes sync, as in the Tacx worked example. The contradictory sum
//! variant and acceptance of acknowledged writes remain hardware checks in #44.
//! No transport delivery or command confirmation is inferred from decoded pages.
//! Typed telemetry, replay and fuzzing are the separate part B tracked in #129.

mod types;
pub use types::*;

/// Encode acknowledged data on channel 5, filling unused bytes with `0xFF`.
///
/// Finite values are range checked before rounding to the nearest wire unit
/// (half units round upward). No clamping to field or trainer limits occurs.
/// Page 70 requests acknowledged replies; its count must fit seven bits.
///
/// # Errors
/// Returns the offending field for non-finite or out-of-domain values.
pub fn encode(page: &ControlPage) -> Result<[u8; 13], EncodeError> {
    let mut p = [0xff; 8];
    match *page {
        ControlPage::BasicResistance { pct } => {
            p[0] = 48;
            p[7] = quantize(pct, 0.0, 100.0, 2.0, 0.0, "pct")?.to_le_bytes()[0];
        }
        ControlPage::TargetPower { watts } => {
            p[0] = 49;
            p[6..8].copy_from_slice(
                &quantize(watts, 0.0, 16_383.75, 4.0, 0.0, "watts")?.to_le_bytes(),
            );
        }
        ControlPage::Wind {
            cw_kg_per_m,
            speed_kmh,
            drafting,
        } => {
            p[0] = 50;
            p[5] = quantize(cw_kg_per_m, 0.0, 1.86, 100.0, 0.0, "cw_kg_per_m")?.to_le_bytes()[0];
            p[6] = quantize(speed_kmh, -127.0, 127.0, 1.0, 127.0, "speed_kmh")?.to_le_bytes()[0];
            p[7] = quantize(drafting, 0.0, 1.0, 100.0, 0.0, "drafting")?.to_le_bytes()[0];
        }
        ControlPage::Track { grade_pct, crr } => {
            p[0] = 51;
            p[5..7].copy_from_slice(
                &quantize(grade_pct, -200.0, 200.0, 100.0, 20_000.0, "grade_pct")?.to_le_bytes(),
            );
            p[7] = quantize(crr, 0.0, 0.01275, 20_000.0, 0.0, "crr")?.to_le_bytes()[0];
        }
        ControlPage::UserConfig {
            rider_kg,
            bike_kg,
            wheel_m,
        } => {
            p[0] = 55;
            p[1..3].copy_from_slice(
                &quantize(rider_kg, 0.0, 655.34, 100.0, 0.0, "rider_kg")?.to_le_bytes(),
            );
            let bike = quantize(bike_kg, 0.0, 204.7, 20.0, 0.0, "bike_kg")?;
            let mm = quantize(wheel_m, 0.0, 2.54, 1000.0, 0.0, "wheel_m")?;
            let packed = (bike << 4) | (mm % 10);
            p[4..6].copy_from_slice(&packed.to_le_bytes());
            p[6] = (mm / 10).to_le_bytes()[0];
            p[7] = 0; // Gear ratio unspecified by the API; profile's invalid value.
        }
        ControlPage::Request { page, times } => {
            if times > 127 {
                return Err(EncodeError { field: "times" });
            }
            p[0] = 70;
            p[5] = times | 0x80;
            p[6] = page;
            p[7] = 1;
        }
    }
    let mut frame = [0xa4, 9, 0x4f, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    frame[4..12].copy_from_slice(&p);
    frame[12] = checksum(&frame[..12]);
    Ok(frame)
}

// Bounded integer search avoids unchecked float-to-integer casts, allocation,
// and saturation. At most 16 comparisons cover every field in the profile.
fn quantize(
    value: f32,
    min: f32,
    max: f32,
    scale: f64,
    offset: f64,
    field: &'static str,
) -> Result<u16, EncodeError> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(EncodeError { field });
    }
    let rounded = (f64::from(value) * scale + offset).round();
    let (mut lo, mut hi) = (0, u16::MAX);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f64::from(mid) < rounded {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |xor, byte| xor ^ byte)
}
/// Decode exactly one complete 13-byte ANT data frame, verifying XOR checksum.
///
/// All pages retain their eight bytes. Typed page interpretation is #129;
/// interpreting observations and logging unknown pages belong to the caller.
///
/// # Errors
/// Rejects truncated/trailing data, wrong sync/length/type, and bad checksums.
pub fn decode(bytes: &[u8]) -> Result<AntFrame, FrameError> {
    if bytes.len() < 13 {
        return Err(FrameError::Truncated);
    }
    if bytes[0] != 0xa4 {
        return Err(FrameError::Sync);
    }
    if bytes.len() != 13 || bytes[1] != 9 {
        return Err(FrameError::Length);
    }
    if checksum(bytes) != 0 {
        return Err(FrameError::Checksum);
    }
    let msg_type = match bytes[2] {
        0x4e => MsgType::Broadcast,
        0x4f => MsgType::Acknowledged,
        other => return Err(FrameError::MessageType(other)),
    };
    let p = [
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ];
    Ok(AntFrame {
        msg_type,
        channel: bytes[3],
        page: p,
    })
}
