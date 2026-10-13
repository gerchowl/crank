//! Validation errors for metric calculations; invalid requests are values, not panics.
use serde::Serialize;
use std::fmt;

/// A metric request cannot be evaluated safely with its supplied parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", content = "parameter", rename_all = "snake_case")]
pub enum MetricError {
    /// A metric parameter cannot be represented safely by the requested algorithm.
    InvalidMetricParameter(&'static str),
    /// Metric intermediate arithmetic is nonfinite or a fitted system is singular.
    NonFiniteCalculation,
    /// A requested recovery grid or prediction horizon exceeds its work bound.
    ComputationLimit,
    /// A named filter parameter is NaN or infinite. Finite negatives are clamped.
    InvalidFilterParameter(&'static str),
    /// A bin width is non-finite, zero, or negative, including on empty input.
    InvalidBinWidth,
    /// The snapped origin or the difference between finite extrema is non-finite.
    NonFiniteRange,
    /// Bin arithmetic overflowed, or the allocated grid would exceed its bound.
    TooManyBins,
}
impl fmt::Display for MetricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetricParameter(name) => write!(f, "invalid metric parameter: {name}"),
            Self::NonFiniteCalculation => {
                f.write_str("metric calculation is non-finite or singular")
            }
            Self::ComputationLimit => f.write_str("metric calculation exceeds its work bound"),
            Self::InvalidFilterParameter(name) => write!(f, "non-finite filter parameter: {name}"),
            Self::InvalidBinWidth => f.write_str("bin width must be finite and positive"),
            Self::NonFiniteRange => f.write_str("distribution range must be finite"),
            Self::TooManyBins => f.write_str("distribution bin count exceeds its bound"),
        }
    }
}
impl std::error::Error for MetricError {}

// Nonfinite configuration is rejected before work, including empty requests.
pub(crate) fn finite_parameters(parameters: &[(&'static str, f64)]) -> Result<(), MetricError> {
    for &(name, value) in parameters {
        if !value.is_finite() {
            return Err(MetricError::InvalidMetricParameter(name));
        }
    }
    Ok(())
}
