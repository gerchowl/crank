//! Pure, allocation-free ANT/FE-C-over-BLE codec (ADR-0002 §4).
//!
//! XOR includes sync, as in the Tacx worked example. HIL #44 Part A: the NEO
//! accepts every checksum rule on writes (it does not verify them) and both
//! message types; XOR including sync stays the encoding.
//! No transport delivery or command confirmation is inferred from decoded pages.

mod types;
pub use types::*;

/// Encode acknowledged data on channel 5, filling unused bytes with `0xFF`.
///
/// Finite values are range checked before rounding to the nearest wire unit
/// (half units round upward). No clamping to field or trainer limits occurs.
/// Page 70 requests acknowledged replies; its count must fit seven bits.
/// A count of 0 is encoded as received (byte 0x80). On the NEO a count of 0
/// was answered five times in the one HIL trial (#44 Part A, ADR-0002 §4.2),
/// consistent with the profile's "until acknowledged"; the cap is unconfirmed.
///
/// # Errors
/// Returns the offending field for non-finite or out-of-domain values.
pub fn encode(page: &ControlPage) -> Result<[u8; 13], EncodeError> {
    let mut p = [0xff; 8];
    match *page {
        ControlPage::BasicResistance { pct } => {
            p[0] = 48;
            p[7] = quantize(pct, 0.0, 100.0, 2.0, 0.0, Field::Pct)?.to_le_bytes()[0];
        }
        ControlPage::TargetPower { watts } => {
            p[0] = 49;
            p[6..8].copy_from_slice(
                &quantize(watts, 0.0, 16_383.75, 4.0, 0.0, Field::Watts)?.to_le_bytes(),
            );
        }
        ControlPage::Wind {
            cw_kg_per_m,
            speed_kmh,
            drafting,
        } => {
            p[0] = 50;
            p[5] = quantize(cw_kg_per_m, 0.0, 1.86, 100.0, 0.0, Field::CwKgPerM)?.to_le_bytes()[0];
            p[6] =
                quantize(speed_kmh, -127.0, 127.0, 1.0, 127.0, Field::SpeedKmh)?.to_le_bytes()[0];
            p[7] = quantize(drafting, 0.0, 1.0, 100.0, 0.0, Field::Drafting)?.to_le_bytes()[0];
        }
        ControlPage::Track { grade_pct, crr } => {
            p[0] = 51;
            p[5..7].copy_from_slice(
                &quantize(grade_pct, -200.0, 200.0, 100.0, 20_000.0, Field::GradePct)?
                    .to_le_bytes(),
            );
            p[7] = quantize(crr, 0.0, 0.01275, 20_000.0, 0.0, Field::Crr)?.to_le_bytes()[0];
        }
        ControlPage::UserConfig {
            rider_kg,
            bike_kg,
            wheel_m,
        } => {
            p[0] = 55;
            p[1..3].copy_from_slice(
                &quantize(rider_kg, 0.0, 655.34, 100.0, 0.0, Field::RiderKg)?.to_le_bytes(),
            );
            let bike = quantize(bike_kg, 0.0, 204.7, 20.0, 0.0, Field::BikeKg)?;
            let mm = quantize(wheel_m, 0.0, 2.54, 1000.0, 0.0, Field::WheelM)?;
            let packed = (bike << 4) | (mm % 10);
            p[4..6].copy_from_slice(&packed.to_le_bytes());
            p[6] = (mm / 10).to_le_bytes()[0];
            p[7] = 0; // Gear ratio unspecified by the API; profile's invalid value.
        }
        ControlPage::Request { page, times } => {
            if times > 127 {
                return Err(EncodeError {
                    field: Field::Times,
                });
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
    field: Field,
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

fn word(p: [u8; 8], i: usize) -> u16 {
    u16::from_le_bytes([p[i], p[i + 1]])
}
fn optional(value: u16, invalid: u16, scale: f32) -> Option<f32> {
    (value != invalid).then_some(f32::from(value) / scale)
}

/// Decode exactly one complete 13-byte ANT data frame, verifying XOR checksum.
///
/// Unspecified pages retain their eight bytes as [`Page::Unknown`];
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
        page: decode_page(p),
    })
}

fn decode_page(p: [u8; 8]) -> Page {
    match p[0] {
        16 => Page::GeneralFe(GeneralFe {
            // Bits 5-7 are reserved in the profile; only the type code is kept.
            equipment_type: p[1] & 0x1f,
            elapsed_s: f32::from(p[2]) * 0.25,
            distance_m: p[3],
            speed_mps: optional(word(p, 4), 0xffff, 1000.0),
            heart_rate_bpm: (p[6] != 0xff).then_some(p[6]),
            state: (p[7] >> 4) & 7,
            lap_toggle: p[7] & 0x80 != 0,
            flags: p[7] & 15,
        }),
        25 => Page::Trainer(TrainerData {
            event_count: p[1],
            cadence_rpm: (p[2] != 0xff).then_some(p[2]),
            accumulated_power_w: word(p, 3),
            power_w: (word(p, 5) & 0xfff != 0xfff).then_some(word(p, 5) & 0xfff),
            status: p[6] >> 4,
            target_limit: p[7] & 3,
            state: (p[7] >> 4) & 7,
            lap_toggle: p[7] & 0x80 != 0,
        }),
        48 => Page::BasicResistance {
            pct: optional(u16::from(p[7]), 0xff, 2.0),
        },
        49 => Page::TargetPower {
            watts: f32::from(word(p, 6)) * 0.25,
        },
        50 => Page::Wind {
            cw_kg_per_m: optional(u16::from(p[5]), 0xff, 100.0),
            speed_kmh: (p[6] != 0xff).then_some(f32::from(p[6]) - 127.0),
            drafting: optional(u16::from(p[7]), 0xff, 100.0),
        },
        51 => Page::Track {
            grade_pct: (word(p, 5) != 0xffff).then(|| (f32::from(word(p, 5)) - 20_000.0) / 100.0),
            crr: f32::from(p[7]) / 20_000.0,
        },
        54 => Page::Capabilities(FeCapabilities {
            max_resistance_n: word(p, 5),
            modes: p[7],
        }),
        55 => Page::UserConfig {
            rider_kg: optional(word(p, 1), 0xffff, 100.0),
            bike_kg: optional(word(p, 4) >> 4, 0xfff, 20.0),
            wheel_m: (p[6] != 0xff && (p[4] & 15 <= 9 || p[4] & 15 == 15)).then(|| {
                let offset = if p[4] & 15 == 15 { 0 } else { p[4] & 15 };
                f32::from(u16::from(p[6]) * 10 + u16::from(offset)) / 1000.0
            }),
            gear_ratio: (p[7] != 0).then_some(f32::from(p[7]) * 0.03),
        },
        70 => Page::Request {
            page: p[6],
            times: p[5] & 127,
            acknowledged: p[5] & 0x80 != 0,
            command_type: p[7],
        },
        71 => Page::CommandStatus(CommandStatus {
            last_page: p[1],
            sequence: p[2],
            status: p[3],
            echo: [p[4], p[5], p[6], p[7]],
        }),
        80 => Page::Manufacturer(ManufacturerInfo {
            hardware_revision: p[3],
            manufacturer_id: word(p, 4),
            model: word(p, 6),
        }),
        81 => Page::Product(ProductInfo {
            software_revision: p[3],
            supplemental_revision: (p[2] != 0xff).then_some(p[2]),
            serial: u32::from_le_bytes([p[4], p[5], p[6], p[7]]),
        }),
        other => Page::Unknown {
            page: other,
            raw: p,
        },
    }
}
