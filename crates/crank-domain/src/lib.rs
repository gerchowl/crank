//! Pure domain logic; see ADR-0001 §2–4.
pub mod distributions;
pub mod filters;
pub mod history;
pub mod metric_error;
pub mod scales;
/// Workout trees and text DSL.
pub mod workout;

// Math.round rounds ties towards +infinity and preserves negative zero. Adding
// 0.5 before floor loses the representable value immediately below a half.
fn js_round(value: f64) -> f64 {
    if !value.is_finite() || value == 0. {
        return value;
    }
    let floor = value.floor();
    let rounded = if value - floor < 0.5 {
        floor
    } else {
        floor + 1.
    };
    if rounded == 0. {
        0_f64.copysign(value)
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn js_round_preserves_half_boundary_and_signed_zero() {
        let below_half = f64::from_bits(0.5_f64.to_bits() - 1);
        assert_eq!(super::js_round(below_half).to_bits(), 0_f64.to_bits());
        assert_eq!(super::js_round(0.5).to_bits(), 1_f64.to_bits());
        assert_eq!(super::js_round(-0.5).to_bits(), (-0_f64).to_bits());
        assert_eq!(super::js_round(-1.5).to_bits(), (-1_f64).to_bits());
        assert!(super::js_round(f64::NAN).is_nan());
    }
}
