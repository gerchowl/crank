//! Rider tuning (ADR-0001 §7.2): the values a rider feels, as configuration instead of code.
//!
//! [`Tuning`] is in seconds and rpm, never ticks. [`Tuning::default`] is the decided default,
//! [`Tuning::BOUNDS`] the allowed ranges and [`Tuning::validate`] the check. The engine is
//! given a ride's tuning in [`RideSetup`](crate::RideSetup) and never reads the store, the
//! environment or a file; the core resolves it with [`Tuning::resolve`] from layers over the
//! defaults (the stored setting `"tuning"`; the environment layer is #592, the per-ride record
//! with a source tag per field #591).
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
    /// The layers do not deserialise into a tuning (unknown field, wrong type).
    #[error("tuning is malformed: {0}")]
    Malformed(String),
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            erg: ErgTuning::default(),
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
    let ticks = value * TICKS_PER_SECOND as f64;
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
    };

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
        assert_eq!(t.validate(), Ok(()));
        assert_eq!(
            serde_json::to_value(t).expect("json"),
            json!({"v": 1, "erg": {"releaseEnabled": true, "releaseRpm": 30,
                "releaseAfterS": 5.0, "reengageRpm": 45, "reengageAfterS": 1.5}})
        );
    }
    #[test]
    fn the_bounds_are_the_adr_0002_7_4_ranges() {
        let b = Tuning::BOUNDS.erg;
        assert_eq!(b.release_rpm, 20..=50);
        assert_eq!(b.release_after_s, 1.0..=8.0);
        assert_eq!(b.reengage_rpm, 30..=90);
        assert_eq!(b.reengage_after_s, 1.5..=5.0);
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
            prop_assert_eq!(Tuning { version: 1, erg }.validate(), Ok(()));
        }
    }
}
