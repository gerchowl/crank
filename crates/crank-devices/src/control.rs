//! Desired and observed controls at the ADR-0002 §4.4.1 wire resolution.
use crate::{DeviceError, ProtocolErrorKind};

/// Wind parameters for page 50 (ADR-0002 §3); defaults follow §7.9.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    /// Wind coefficient in kg/m.
    pub cw_kg_per_m: f32,
    /// Headwind speed in km/h.
    pub speed_kmh: f32,
    /// Drafting multiplier.
    pub drafting: f32,
}
impl Default for Wind {
    fn default() -> Self {
        Self {
            cw_kg_per_m: 0.51,
            speed_kmh: 0.0,
            drafting: 1.0,
        }
    }
}
/// Desired mode (ADR-0002 §3); engine owns capability clamping and ERG policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlMode {
    /// Target power, page 49.
    Erg {
        /// Target watts.
        target_w: u16,
    },
    /// Track and wind simulation, pages 50 and 51.
    Sim {
        /// Grade in percent.
        grade_pct: f32,
        /// Rolling resistance coefficient.
        crr: f32,
        /// Wind parameters.
        wind: Wind,
    },
    /// Basic resistance, page 48.
    Resistance {
        /// Resistance percent, 0..=100.
        pct: f32,
    },
    /// Explicit flat-road SIM with default wind and Crr (ADR-0002 §7.9).
    Free,
}
/// One echoed control page as integers; never a full SIM mode (ADR-0002 §4.4.1).
/// Exhaustive for engine routing R1–R5 (ADR-0002 §4.4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedControl {
    /// Page 49.
    Erg {
        /// Quarter-watt units.
        quarter_w: u16,
    },
    /// Page 51.
    Track {
        /// Signed grade in 0.01 percent, with wire offset removed.
        grade_centi_pct: i32,
        /// Crr in 5×10⁻⁵ units.
        crr_units: u8,
    },
    /// Page 50.
    Wind {
        /// Coefficient in 0.01 kg/m.
        cw_centi: u8,
        /// Signed headwind km/h, with wire offset removed.
        speed_kmh: i16,
        /// Drafting in 0.01 units.
        drafting_centi: u8,
    },
    /// Page 48.
    Resistance {
        /// Resistance in 0.5 percent.
        half_pct: u8,
    },
    /// Non-control echo, neutral to following (ADR-0002 §4.4.1 R1).
    NotControl(u8),
}
impl ControlMode {
    /// Checked integer component projection (ADR-0002 §4.4.1).
    ///
    /// SIM components are ordered by page 50 then 51. Grade and wind speed
    /// are signed physical grid values with their wire offsets removed, matching
    /// [`ObservedControl`]. Rounding matches the codec: nearest wire unit,
    /// with half units rounded upward after adding the wire offset.
    ///
    /// ```
    /// use crank_devices::{ControlMode, ObservedControl};
    /// assert_eq!(ControlMode::Erg { target_w: 201 }.grid_key()?,
    ///     vec![ObservedControl::Erg { quarter_w: 804 }]);
    /// # Ok::<(), crank_devices::DeviceError>(())
    /// ```
    ///
    /// # Errors
    /// Rejects non-finite/out-of-range inputs without clamping (ADR-0002 §4.3).
    pub fn grid_key(self) -> Result<Vec<ObservedControl>, DeviceError> {
        use ObservedControl::{Erg, Resistance, Track};
        Ok(match self {
            Self::Erg { target_w } => vec![Erg {
                quarter_w: target_w.checked_mul(4).ok_or_else(|| invalid("target_w"))?,
            }],
            Self::Resistance { pct } => vec![Resistance {
                half_pct: unit("pct", pct, 0.0, 100.0, 2.0, 0.0)?.to_le_bytes()[0],
            }],
            Self::Free => {
                return Self::Sim {
                    grade_pct: 0.0,
                    crr: 0.004,
                    wind: Wind::default(),
                }
                .grid_key();
            }
            Self::Sim {
                grade_pct,
                crr,
                wind,
            } => vec![
                ObservedControl::Wind {
                    cw_centi: unit("wind.cw_kg_per_m", wind.cw_kg_per_m, 0.0, 1.86, 100.0, 0.0)?
                        .to_le_bytes()[0],
                    // 0..=254 after the +127 wire offset, so it always fits a byte
                    speed_kmh: i16::from(
                        unit("wind.speed_kmh", wind.speed_kmh, -127.0, 127.0, 1.0, 127.0)?
                            .to_le_bytes()[0],
                    ) - 127,
                    drafting_centi: unit("wind.drafting", wind.drafting, 0.0, 1.0, 100.0, 0.0)?
                        .to_le_bytes()[0],
                },
                Track {
                    grade_centi_pct: i32::from(unit(
                        "grade_pct",
                        grade_pct,
                        -200.0,
                        200.0,
                        100.0,
                        20000.0,
                    )?) - 20000,
                    crr_units: unit("crr", crr, 0.0, 0.01275, 20000.0, 0.0)?.to_le_bytes()[0],
                },
            ],
        })
    }
}
fn invalid(field: &str) -> DeviceError {
    DeviceError::Protocol {
        kind: ProtocolErrorKind::Encode,
        detail: format!("invalid control: {field} is not finite or outside its range"),
    }
}
/// Wire value of `value` on a grid: `round(value * scale + offset)`, with ties rounded away
/// from zero (`f64::round`; every wire value here is non-negative, so that is upward).
/// `value` must be finite and inside `min..=max`; anything else is rejected, never clamped
/// (ADR-0002 §4.3), with an error that names `field`. Each call site's range keeps the
/// result inside `0..=40_000`, so the conversion below never actually saturates; the clamp
/// states the rule should a range ever be widened past `u16`.
fn unit(
    field: &str,
    value: f32,
    min: f32,
    max: f32,
    scale: f64,
    offset: f64,
) -> Result<u16, DeviceError> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(invalid(field));
    }
    let rounded = (f64::from(value) * scale + offset).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to u16 just above
    Ok(rounded.clamp(0.0, f64::from(u16::MAX)) as u16)
}
