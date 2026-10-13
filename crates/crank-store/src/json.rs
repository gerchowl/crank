//! Versioned column contracts (ADR-0001 §6; architect decision #33 round 2).
use crate::StoreError;
mod sealed {
    pub trait Sealed {}
}
/// Sealed per-column schema; owner-type migration is tracked in #156.
pub trait JsonColumn: sealed::Sealed + Sized + std::fmt::Debug {
    /// Owning table.
    const TABLE: &'static str;
    /// Owning column.
    const COLUMN: &'static str;
    /// Ordered primary-key columns.
    const KEYS: &'static [&'static str];
    /// Validate an object with an integer `v`.
    /// # Errors
    /// Returns `InvalidJson` naming the table and column.
    fn parse(text: &str) -> Result<Self, StoreError>;
    /// Serialized validated envelope, including `v`.
    fn as_str(&self) -> &str;
}
fn validate(text: &str, table: &'static str, column: &'static str) -> Result<(), StoreError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| StoreError::InvalidJson {
            table,
            column,
            reason: e.to_string(),
        })?;
    if !value.is_object() || !value.get("v").is_some_and(|v| v.is_i64() || v.is_u64()) {
        return Err(StoreError::InvalidJson {
            table,
            column,
            reason: "expected object with integer v".into(),
        });
    }
    Ok(())
}
/// Envelope version written by the workout column constructors.
const WORKOUT_SCHEMA_VERSION: u32 = 1;
macro_rules! column {
    ($name:ident, $table:literal, $column:literal, [$($key:literal),+], $doc:literal) => {
        #[doc = $doc]
        #[doc = "Versioned object contract (ADR-0001 §6). Owner-type migration: #156."]
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name(String);
        impl sealed::Sealed for $name {}
        impl JsonColumn for $name {
            const TABLE: &'static str = $table;
            const COLUMN: &'static str = $column;
            const KEYS: &'static [&'static str] = &[$($key),+];
            fn parse(text: &str) -> Result<Self, StoreError> {
                validate(text, Self::TABLE, Self::COLUMN)?; Ok(Self(text.into()))
            }
            fn as_str(&self) -> &str { &self.0 }
        }
    };
}
// #76 landed before schema v1's first release. Keep the version envelope while
// validating the producer's actual Workout schema, including nested nodes.
macro_rules! workout_column {
    ($name:ident, $table:literal, $column:literal, $doc:literal) => {
        #[doc = $doc]
        #[doc = "Validated through crank-domain's Workout (#76/#68) with integer v."]
        #[doc = "ADR-0001 §6; owner-type tracking: #156."]
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            text: String,
            workout: crank_domain::workout::Workout,
        }
        impl $name {
            /// Serialize a domain workout, adding the `v` envelope itself so
            /// producers never write it by hand. The result is re-parsed, so it
            /// satisfies exactly the contract the read path enforces.
            /// # Errors
            /// Returns `InvalidJson` if the workout cannot round-trip (for
            /// example a non-finite number).
            pub fn new(workout: &crank_domain::workout::Workout) -> Result<Self, StoreError> {
                let invalid = |reason: String| StoreError::InvalidJson {
                    table: $table,
                    column: $column,
                    reason,
                };
                let mut value =
                    serde_json::to_value(workout).map_err(|e| invalid(e.to_string()))?;
                let object = value
                    .as_object_mut()
                    .ok_or_else(|| invalid("workout did not serialize to an object".into()))?;
                object.insert("v".into(), WORKOUT_SCHEMA_VERSION.into());
                <Self as JsonColumn>::parse(&value.to_string())
            }
            /// The producer's validated workout value.
            #[must_use]
            pub fn workout(&self) -> &crank_domain::workout::Workout {
                &self.workout
            }
        }
        impl sealed::Sealed for $name {}
        impl JsonColumn for $name {
            const TABLE: &'static str = $table;
            const COLUMN: &'static str = $column;
            const KEYS: &'static [&'static str] = &["id"];
            fn parse(text: &str) -> Result<Self, StoreError> {
                validate(text, Self::TABLE, Self::COLUMN)?;
                let workout = serde_json::from_str(text).map_err(|e| StoreError::InvalidJson {
                    table: Self::TABLE,
                    column: Self::COLUMN,
                    reason: e.to_string(),
                })?;
                Ok(Self {
                    text: text.into(),
                    workout,
                })
            }
            fn as_str(&self) -> &str {
                &self.text
            }
        }
    };
}
workout_column!(
    WorkoutTreeJson,
    "workout",
    "tree_json",
    "Workout tree; owner #76/#68."
);
workout_column!(
    RideWorkoutJson,
    "ride",
    "workout_json",
    "Ridden workout; owner #76/#68."
);
column!(
    CompiledJson,
    "ride",
    "compiled_json",
    ["id"],
    "Compiled snapshot; owner #69."
);
column!(
    SnippetJson,
    "snippet",
    "json",
    ["id"],
    "Snippet; owner #34/UI."
);
column!(
    LayoutJson,
    "layout",
    "json",
    ["id"],
    "UI layout; owner #34/UI."
);
column!(
    SettingJson,
    "setting",
    "json",
    ["key"],
    "Setting; owner #34."
);
column!(
    RideSummaryJson,
    "ride",
    "summary_json",
    ["id"],
    "Ride summary; owner #32."
);
column!(
    LapStatsJson,
    "lap",
    "stats_json",
    ["ride_id", "idx"],
    "Lap statistics; owner #32."
);
column!(
    RecoveryFitJson,
    "recovery",
    "fit_json",
    ["ride_id", "node_id", "iteration"],
    "HR recovery fit; owner #136."
);
column!(
    CoachFeaturesJson,
    "coach_decision",
    "features_json",
    ["call_id"],
    "Coach features; owner #59."
);
