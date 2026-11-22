//! Rider tuning (ADR-0001 §7.2): the values a rider feels, as configuration instead of code.
//!
//! [`Tuning`] is in seconds and rpm, never ticks. [`Tuning::default`] is the decided default,
//! [`Tuning::BOUNDS`] the allowed ranges and [`Tuning::validate`] the check. The engine is
//! given a ride's tuning in [`RideSetup`](crate::RideSetup) and never reads the store, the
//! environment or a file; the core resolves it with [`Tuning::resolve`] from layers over the
//! defaults (the stored setting `"tuning"`; the environment layer is #592, the per-ride record
//! with a source tag per field #591).
//!
//! Groups: [`ErgTuning`], the ERG release (ADR-0002 §7.4, #589), and [`SlewTuning`], the eased
//! ERG target's rates (ADR-0002 §7.10.4, #585; the easer is [`crate::slew`]).
use crate::ticks::TICKS_PER_SECOND;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::RangeInclusive;

/// Rider tuning for one ride (ADR-0001 §7.2). Frozen at `ride_load`: a change takes effect
/// at the next load.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tuning {
    /// Format version of this struct; [`Tuning::VERSION`] is the only one accepted. A change
    /// of meaning or unit of a field bumps it. Serialised as `v`, the store's JSON envelope
    /// (ADR-0001 §6), so the stored setting is the tuning itself.
    #[serde(rename = "v")]
    pub version: u32,
    /// The ERG low-cadence release (ADR-0002 §7.4).
    pub erg: ErgTuning,
    /// The eased ERG target's rates (ADR-0002 §7.10.4).
    pub slew: SlewTuning,
}

/// The ERG low-cadence release and its re-engagement (ADR-0002 §7.4, amended 2026-11-09,
/// #589). Defaults and the reasons for them are in the ADR; [`ErgBounds`] the ranges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErgTuning {
    /// Release ERG at low cadence at all. Default on. Off, ERG is never released for low
    /// cadence: the rider's explicit choice; the rider's pause and the ride end still command
    /// 0 % resistance (ADR-0002 §7.4).
    pub release_enabled: bool,
    /// Cadence below which ERG is released, in rpm (strictly below; no cadence counts as 0).
    /// Default 30.
    pub release_rpm: u8,
    /// How long cadence must stay below [`release_rpm`](Self::release_rpm), with a non-zero ERG
    /// target applied, before ERG is released: seconds, on the 0.25 s tick grid. Default 5 s,
    /// longer than a saddle readjustment (ADR-0002 §7.4).
    pub release_after_s: f64,
    /// Cadence at or above which a release is left (then the §7.5 soft start), in rpm. Must be
    /// above [`release_rpm`](Self::release_rpm). Default 45.
    pub reengage_rpm: u8,
    /// How long cadence must stay at or above [`reengage_rpm`](Self::reengage_rpm) to leave a
    /// release: seconds, on the 0.25 s tick grid, judged on the held cadence. Default 1.5 s,
    /// longer than one held sample, so one stray value never re-engages (ADR-0002 §7.4).
    pub reengage_after_s: f64,
}

/// The eased ERG target (ADR-0002 §7.10.4, #585): the rider's limits on the commanded target,
/// handed to Ruckig by [`crate::slew`] as `max_velocity = rise`, `min_velocity = −rise·factor`,
/// `max_acceleration = 2·rise/τ` and `max_jerk = 4·rise/τ²`, one pair of limits for every move.
/// The defaults are guesses to tune from rides (#624); [`SlewBounds`] the ranges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlewTuning {
    /// The rising rate `rise`, in W/s (an integer): Ruckig's `max_velocity`. Default 20 W/s, so
    /// the 83 W step of a recovery-to-work transition is a 5.5 s S-curve (ADR-0002 §7.10.4).
    pub rise_w_per_s: u16,
    /// The falling rate as a multiple of [`rise_w_per_s`](Self::rise_w_per_s), on a 0.25 grid:
    /// Ruckig's `min_velocity = −rise·factor`. Default 2 (the owner's: a fall twice as fast as a
    /// rise). At least 1, so a fall is never slower than a rise; `rise × factor ≥ 20 W/s` bounds
    /// the time above the plan (ADR-0002 §7.10.6).
    pub fall_factor: f64,
    /// The ease time `τ`, in seconds on the 0.25 s grid: from rest to the full rising rate in
    /// exactly `τ`, as one S-curve. Ruckig's `max_acceleration = 2·rise/τ` and
    /// `max_jerk = 4·rise/τ²`. Default 2 s: each corner spread over a second of rising and a
    /// second of falling acceleration (ADR-0002 §7.10.4).
    pub ease_s: f64,
}

/// Inclusive ranges of the [`SlewTuning`] fields (ADR-0002 §7.10.4: design choices).
#[derive(Clone, Debug, PartialEq)]
pub struct SlewBounds {
    /// [`SlewTuning::rise_w_per_s`], W/s: at most 50 keeps every engagement a ramp (its first
    /// second adds at most 25 W); at least 5 keeps a 300 W rise to about a minute.
    pub rise_w_per_s: RangeInclusive<u16>,
    /// [`SlewTuning::fall_factor`], on a 0.25 grid; `rise × factor ≥ 20 W/s` as well.
    pub fall_factor: RangeInclusive<f64>,
    /// [`SlewTuning::ease_s`], seconds on the 0.25 s grid.
    pub ease_s: RangeInclusive<f64>,
}

/// Inclusive ranges of the [`ErgTuning`] fields (ADR-0002 §7.4 amendment, its safety
/// argument).
#[derive(Clone, Debug, PartialEq)]
pub struct ErgBounds {
    /// [`ErgTuning::release_rpm`], rpm.
    pub release_rpm: RangeInclusive<u8>,
    /// [`ErgTuning::release_after_s`], seconds: no tuning holds a stopped rider under ERG
    /// for longer than its upper end.
    pub release_after_s: RangeInclusive<f64>,
    /// [`ErgTuning::reengage_rpm`], rpm; it must also be above `release_rpm`.
    pub reengage_rpm: RangeInclusive<u8>,
    /// [`ErgTuning::reengage_after_s`], seconds; the lower end needs two trainer samples.
    pub reengage_after_s: RangeInclusive<f64>,
}

/// Inclusive ranges of every [`Tuning`] field, one table per group.
#[derive(Clone, Debug, PartialEq)]
pub struct TuningBounds {
    /// The ERG release.
    pub erg: ErgBounds,
    /// The eased ERG target.
    pub slew: SlewBounds,
}

/// Why a tuning is refused. The message names the field by its JSON path.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TuningError {
    /// The `version` (`v`) is not [`Tuning::VERSION`].
    #[error(
        "tuning version v = {found} is not supported (expected {})",
        Tuning::VERSION
    )]
    Version {
        /// The version given.
        found: u32,
    },
    /// A field is outside its range in [`Tuning::BOUNDS`].
    #[error("{field} = {value} is outside {min}..={max}")]
    OutOfBounds {
        /// JSON path of the field.
        field: &'static str,
        /// The value given.
        value: String,
        /// Lower end of the range.
        min: String,
        /// Upper end of the range.
        max: String,
    },
    /// A duration is not a multiple of the 0.25 s tick.
    #[error("{field} = {value} s is not on the 0.25 s grid")]
    OffGrid {
        /// JSON path of the field.
        field: &'static str,
        /// The value given.
        value: String,
    },
    /// The re-engage cadence is not above the release cadence.
    #[error("erg.reengageRpm ({reengage}) must be above erg.releaseRpm ({release})")]
    Hysteresis {
        /// The release cadence.
        release: u8,
        /// The re-engage cadence.
        reengage: u8,
    },
    /// The falling rate `rise × factor` is below 20 W/s (ADR-0002 §7.10.4): the floor that
    /// bounds the time above the plan.
    #[error(
        "slew.riseWPerS × slew.fallFactor ({rise} × {factor}) must be at least {} W/s",
        Tuning::MIN_FALL_W_PER_S
    )]
    FallRate {
        /// [`SlewTuning::rise_w_per_s`].
        rise: u16,
        /// [`SlewTuning::fall_factor`], as written by `f64`'s `Display`.
        factor: String,
    },
    /// The layers do not deserialise into a tuning (unknown field, wrong type).
    #[error("tuning is malformed: {0}")]
    Malformed(String),
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            erg: ErgTuning::default(),
            slew: SlewTuning::default(),
        }
    }
}

impl Default for SlewTuning {
    /// The ADR-0002 §7.10.4 defaults: 20 W/s, ×2, 2 s.
    fn default() -> Self {
        Self {
            rise_w_per_s: 20,
            fall_factor: 2.0,
            ease_s: 2.0,
        }
    }
}

impl Default for ErgTuning {
    /// The ADR-0002 §7.4 defaults (amendment 2026-11-09, #589).
    fn default() -> Self {
        Self {
            release_enabled: true,
            release_rpm: 30,
            release_after_s: 5.0,
            reengage_rpm: 45,
            reengage_after_s: 1.5,
        }
    }
}

fn in_range<T: PartialOrd + ToString>(
    field: &'static str,
    value: &T,
    range: &RangeInclusive<T>,
) -> Result<(), TuningError> {
    if range.contains(value) {
        Ok(())
    } else {
        Err(TuningError::OutOfBounds {
            field,
            value: value.to_string(),
            min: range.start().to_string(),
            max: range.end().to_string(),
        })
    }
}
#[expect(clippy::cast_precision_loss, reason = "TICKS_PER_SECOND is 4")]
fn on_grid(field: &'static str, value: f64) -> Result<(), TuningError> {
    on_quarter_grid(field, value, TICKS_PER_SECOND as f64)
}
/// `value` is a multiple of `1/per_unit` (the 0.25 grid for `per_unit` = 4).
fn on_quarter_grid(field: &'static str, value: f64, per_unit: f64) -> Result<(), TuningError> {
    let ticks = value * per_unit;
    if (ticks - ticks.round()).abs() < 1e-9 {
        Ok(())
    } else {
        Err(TuningError::OffGrid {
            field,
            value: value.to_string(),
        })
    }
}
fn duration(
    field: &'static str,
    value: f64,
    range: &RangeInclusive<f64>,
) -> Result<(), TuningError> {
    in_range(field, &value, range)?;
    on_grid(field, value)
}

/// Merge `layer` into `base`: an object merges into an object key by key, any other value
/// replaces.
fn merge(base: &mut Value, layer: &Value) {
    match (base, layer) {
        (Value::Object(base), Value::Object(layer)) => {
            for (k, v) in layer {
                match base.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base, layer) => *base = layer.clone(),
    }
}

impl Tuning {
    /// The only accepted [`version`](Self::version).
    pub const VERSION: u32 = 1;
    /// The allowed ranges (ADR-0002 §7.4 amendment).
    pub const BOUNDS: TuningBounds = TuningBounds {
        erg: ErgBounds {
            release_rpm: 20..=50,
            release_after_s: 1.0..=8.0,
            reengage_rpm: 30..=90,
            reengage_after_s: 1.5..=5.0,
        },
        slew: SlewBounds {
            rise_w_per_s: 5..=50,
            fall_factor: 1.0..=4.0,
            ease_s: 1.0..=4.0,
        },
    };
    /// The floor of the falling rate `rise × factor`, W/s (ADR-0002 §7.10.4).
    pub const MIN_FALL_W_PER_S: f64 = 20.0;

    /// Check the version, every range of [`Tuning::BOUNDS`], the tick grid of the durations
    /// and the cross-field rules.
    ///
    /// # Errors
    /// The first [`TuningError`] found.
    pub fn validate(&self) -> Result<(), TuningError> {
        if self.version != Self::VERSION {
            return Err(TuningError::Version {
                found: self.version,
            });
        }
        let (e, b) = (&self.erg, &Self::BOUNDS.erg);
        in_range("erg.releaseRpm", &e.release_rpm, &b.release_rpm)?;
        duration("erg.releaseAfterS", e.release_after_s, &b.release_after_s)?;
        in_range("erg.reengageRpm", &e.reengage_rpm, &b.reengage_rpm)?;
        duration(
            "erg.reengageAfterS",
            e.reengage_after_s,
            &b.reengage_after_s,
        )?;
        if e.reengage_rpm <= e.release_rpm {
            return Err(TuningError::Hysteresis {
                release: e.release_rpm,
                reengage: e.reengage_rpm,
            });
        }
        let (s, b) = (&self.slew, &Self::BOUNDS.slew);
        in_range("slew.riseWPerS", &s.rise_w_per_s, &b.rise_w_per_s)?;
        in_range("slew.fallFactor", &s.fall_factor, &b.fall_factor)?;
        // The factor is on a 0.25 grid like a duration (ADR-0001 §7.2), so the same check.
        on_quarter_grid("slew.fallFactor", s.fall_factor, 4.0)?;
        duration("slew.easeS", s.ease_s, &b.ease_s)?;
        if f64::from(s.rise_w_per_s) * s.fall_factor < Self::MIN_FALL_W_PER_S {
            return Err(TuningError::FallRate {
                rise: s.rise_w_per_s,
                factor: s.fall_factor.to_string(),
            });
        }
        Ok(())
    }

    /// The tuning of `layers` over the defaults, in order (ADR-0001 §7.2): each layer is a
    /// JSON object, possibly partial, merged field by field into the JSON of
    /// [`Tuning::default`]. The result must deserialise (no unknown field) and pass
    /// [`validate`](Self::validate); a layer that fails is never skipped.
    ///
    /// # Errors
    /// [`TuningError::Malformed`] for JSON that is no tuning, else as [`validate`](Self::validate).
    pub fn resolve<'a>(layers: impl IntoIterator<Item = &'a Value>) -> Result<Self, TuningError> {
        let mut merged = serde_json::to_value(Self::default())
            .map_err(|e| TuningError::Malformed(e.to_string()))?;
        for layer in layers {
            merge(&mut merged, layer);
        }
        let tuning: Self =
            serde_json::from_value(merged).map_err(|e| TuningError::Malformed(e.to_string()))?;
        tuning.validate()?;
        Ok(tuning)
    }
}

/// Whole ticks in `s` seconds of a validated duration (on the grid, so exact).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a validated duration is a small non-negative multiple of 0.25 s"
)]
pub(crate) fn ticks(s: f64) -> u64 {
    (s * TICKS_PER_SECOND as f64).round().max(0.0) as u64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn the_default_is_the_adr_0002_7_4_values() {
        // ADR-0002 §7.4, amendment 2026-11-09 (#589)
        let t = Tuning::default();
        assert_eq!(t.version, 1);
        assert_eq!(
            t.erg,
            ErgTuning {
                release_enabled: true,
                release_rpm: 30,
                release_after_s: 5.0,
                reengage_rpm: 45,
                reengage_after_s: 1.5,
            }
        );
        // ADR-0002 §7.10.4 (#585): 20 W/s, ×2, 2 s
        assert_eq!(
            t.slew,
            SlewTuning {
                rise_w_per_s: 20,
                fall_factor: 2.0,
                ease_s: 2.0,
            }
        );
        assert_eq!(t.validate(), Ok(()));
        assert_eq!(
            serde_json::to_value(t).expect("json"),
            json!({"v": 1, "erg": {"releaseEnabled": true, "releaseRpm": 30,
                "releaseAfterS": 5.0, "reengageRpm": 45, "reengageAfterS": 1.5},
                "slew": {"riseWPerS": 20, "fallFactor": 2.0, "easeS": 2.0}})
        );
    }
    #[test]
    fn the_bounds_are_the_adr_0002_7_4_ranges() {
        let b = Tuning::BOUNDS.erg;
        assert_eq!(b.release_rpm, 20..=50);
        assert_eq!(b.release_after_s, 1.0..=8.0);
        assert_eq!(b.reengage_rpm, 30..=90);
        assert_eq!(b.reengage_after_s, 1.5..=5.0);
        let s = Tuning::BOUNDS.slew;
        assert_eq!(s.rise_w_per_s, 5..=50);
        assert_eq!(s.fall_factor, 1.0..=4.0);
        assert_eq!(s.ease_s, 1.0..=4.0);
        assert!((Tuning::MIN_FALL_W_PER_S - 20.0).abs() < f64::EPSILON);
    }
    fn with_slew(f: impl FnOnce(&mut SlewTuning)) -> Result<(), TuningError> {
        let mut t = Tuning::default();
        f(&mut t.slew);
        t.validate()
    }
    #[test]
    fn validate_refuses_slew_values_out_of_bounds_off_grid_or_below_the_fall_floor() {
        let field = |r: Result<(), TuningError>| match r {
            Err(TuningError::OutOfBounds { field, .. } | TuningError::OffGrid { field, .. }) => {
                field
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(field(with_slew(|s| s.rise_w_per_s = 4)), "slew.riseWPerS");
        assert_eq!(field(with_slew(|s| s.rise_w_per_s = 51)), "slew.riseWPerS");
        assert_eq!(
            field(with_slew(|s| s.fall_factor = 0.75)),
            "slew.fallFactor"
        );
        assert_eq!(
            field(with_slew(|s| s.fall_factor = 4.25)),
            "slew.fallFactor"
        );
        assert_eq!(field(with_slew(|s| s.fall_factor = 2.1)), "slew.fallFactor");
        assert_eq!(
            field(with_slew(|s| s.fall_factor = f64::NAN)),
            "slew.fallFactor"
        );
        assert_eq!(field(with_slew(|s| s.ease_s = 0.75)), "slew.easeS");
        assert_eq!(field(with_slew(|s| s.ease_s = 4.25)), "slew.easeS");
        assert_eq!(field(with_slew(|s| s.ease_s = 2.1)), "slew.easeS");
        assert_eq!(field(with_slew(|s| s.ease_s = f64::INFINITY)), "slew.easeS");
        // the falling rate's floor: rise × factor ≥ 20 W/s, both fields named
        assert_eq!(
            with_slew(|s| {
                s.rise_w_per_s = 19;
                s.fall_factor = 1.0;
            }),
            Err(TuningError::FallRate {
                rise: 19,
                factor: "1".into()
            })
        );
        let msg = with_slew(|s| {
            s.rise_w_per_s = 5;
            s.fall_factor = 3.75;
        })
        .expect_err("18.75 W/s")
        .to_string();
        assert!(
            msg.contains("slew.riseWPerS") && msg.contains("slew.fallFactor"),
            "{msg}"
        );
        for (rise, factor) in [(20, 1.0), (5, 4.0), (16, 1.25), (10, 2.0)] {
            assert_eq!(
                with_slew(|s| {
                    s.rise_w_per_s = rise;
                    s.fall_factor = factor;
                }),
                Ok(()),
                "{rise} × {factor}"
            );
        }
        // the corners of the bounds that pass the floor
        for (rise, factor, ease) in [
            (5, 4.0, 4.0),
            (50, 1.0, 1.0),
            (50, 4.0, 4.0),
            (20, 1.0, 4.0),
        ] {
            assert_eq!(
                with_slew(|s| *s = SlewTuning {
                    rise_w_per_s: rise,
                    fall_factor: factor,
                    ease_s: ease
                }),
                Ok(())
            );
        }
    }
    #[test]
    fn a_stored_layer_without_slew_resolves_to_the_slew_defaults() {
        // ADR-0001 §7.2: Tuning::VERSION stays 1; a layer written before #620 resolves unchanged.
        assert_eq!(Tuning::VERSION, 1);
        let t = Tuning::resolve([&json!({"v": 1, "erg": {"releaseRpm": 25}})]).expect("valid");
        assert_eq!(t.slew, SlewTuning::default());
        let t =
            Tuning::resolve([&json!({"slew": {"riseWPerS": 50, "easeS": 1.0}})]).expect("valid");
        assert_eq!(
            (t.slew.rise_w_per_s, t.slew.fall_factor, t.slew.ease_s),
            (50, 2.0, 1.0)
        );
        assert!(matches!(
            Tuning::resolve([&json!({"slew": {"riseWPerSec": 5}})]),
            Err(TuningError::Malformed(_))
        ));
    }
    fn with(f: impl FnOnce(&mut ErgTuning)) -> Result<(), TuningError> {
        let mut t = Tuning::default();
        f(&mut t.erg);
        t.validate()
    }
    #[test]
    fn validate_refuses_out_of_bounds_off_grid_and_inconsistent_values() {
        let field = |r: Result<(), TuningError>| match r {
            Err(TuningError::OutOfBounds { field, .. } | TuningError::OffGrid { field, .. }) => {
                field
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(field(with(|e| e.release_rpm = 19)), "erg.releaseRpm");
        assert_eq!(field(with(|e| e.release_rpm = 51)), "erg.releaseRpm");
        assert_eq!(
            field(with(|e| e.release_after_s = 0.75)),
            "erg.releaseAfterS"
        );
        assert_eq!(
            field(with(|e| e.release_after_s = 8.25)),
            "erg.releaseAfterS"
        );
        assert_eq!(
            field(with(|e| e.release_after_s = 5.1)),
            "erg.releaseAfterS"
        );
        assert_eq!(
            field(with(|e| e.release_after_s = f64::NAN)),
            "erg.releaseAfterS"
        );
        assert_eq!(field(with(|e| e.reengage_rpm = 29)), "erg.reengageRpm");
        assert_eq!(field(with(|e| e.reengage_rpm = 91)), "erg.reengageRpm");
        assert_eq!(
            field(with(|e| e.reengage_after_s = 1.25)),
            "erg.reengageAfterS"
        );
        assert_eq!(
            field(with(|e| e.reengage_after_s = 5.25)),
            "erg.reengageAfterS"
        );
        assert_eq!(
            field(with(|e| e.reengage_after_s = 2.1)),
            "erg.reengageAfterS"
        );
        // re-engage at or below release: no hysteresis
        for (release, reengage) in [(40, 40), (45, 40)] {
            assert_eq!(
                with(|e| {
                    e.release_rpm = release;
                    e.reengage_rpm = reengage;
                }),
                Err(TuningError::Hysteresis { release, reengage })
            );
        }
        assert_eq!(
            with(|e| {
                e.release_rpm = 40;
                e.reengage_rpm = 41;
            }),
            Ok(())
        );
        let t = Tuning {
            version: 2,
            ..Tuning::default()
        };
        assert_eq!(t.validate(), Err(TuningError::Version { found: 2 }));
    }
    #[test]
    fn resolve_merges_partial_layers_over_the_defaults_and_refuses_bad_ones() {
        assert_eq!(Tuning::resolve([]), Ok(Tuning::default()));
        let t = Tuning::resolve([&json!({"erg": {"releaseRpm": 25}})]).expect("valid");
        assert_eq!(t.erg.release_rpm, 25);
        assert_eq!(t.erg.reengage_rpm, 45, "untouched fields keep the default");
        // a later layer wins over an earlier one (the env layer of #592 after the stored one)
        let t = Tuning::resolve([
            &json!({"erg": {"releaseRpm": 25, "releaseEnabled": false}}),
            &json!({"erg": {"releaseRpm": 35}}),
        ])
        .expect("valid");
        assert_eq!((t.erg.release_rpm, t.erg.release_enabled), (35, false));
        let bad = |v: Value| Tuning::resolve([&v]).expect_err("refused");
        assert!(matches!(
            bad(json!({"erg": {"releaseRmp": 25}})),
            TuningError::Malformed(_)
        ));
        assert!(matches!(
            bad(json!({"autoPause": {}})),
            TuningError::Malformed(_)
        ));
        assert!(matches!(
            bad(json!({"erg": {"releaseRpm": "25"}})),
            TuningError::Malformed(_)
        ));
        assert!(matches!(bad(json!([1])), TuningError::Malformed(_)));
        assert_eq!(bad(json!({"v": 2})), TuningError::Version { found: 2 });
        assert!(matches!(
            bad(json!({"erg": {"releaseAfterS": 20}})),
            TuningError::OutOfBounds {
                field: "erg.releaseAfterS",
                ..
            }
        ));
        assert_eq!(
            bad(json!({"erg": {"reengageRpm": 30}})),
            TuningError::Hysteresis {
                release: 30,
                reengage: 30
            }
        );
    }
    #[test]
    fn ticks_are_exact_on_the_grid() {
        assert_eq!(ticks(5.0), 20);
        assert_eq!(ticks(1.5), 6);
        assert_eq!(ticks(0.25), 1);
    }

    /// Every in-bounds tuning, as the strategy draws them.
    pub(crate) fn in_bounds() -> impl Strategy<Value = ErgTuning> {
        let b = Tuning::BOUNDS.erg;
        let grid = |r: RangeInclusive<f64>| {
            (ticks(*r.start())..=ticks(*r.end())).prop_map(|k| {
                #[expect(clippy::cast_precision_loss, reason = "small tick counts")]
                let s = k as f64 / 4.0;
                s
            })
        };
        (
            b.release_rpm.clone(),
            grid(b.release_after_s.clone()),
            b.reengage_rpm.clone(),
            grid(b.reengage_after_s.clone()),
        )
            .prop_filter("re-engage above release", |(r, _, e, _)| e > r)
            .prop_map(
                |(release_rpm, release_after_s, reengage_rpm, reengage_after_s)| ErgTuning {
                    release_enabled: true,
                    release_rpm,
                    release_after_s,
                    reengage_rpm,
                    reengage_after_s,
                },
            )
    }
    proptest! {
        #[test]
        fn every_drawn_tuning_validates(erg in in_bounds()) {
            prop_assert_eq!(Tuning { version: 1, erg, slew: SlewTuning::default() }.validate(), Ok(()));
        }
    }
}
