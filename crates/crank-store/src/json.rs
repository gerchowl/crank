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
column!(
    WorkoutTreeJson,
    "workout",
    "tree_json",
    ["id"],
    "Workout tree; owner #76/#68."
);
column!(
    RideWorkoutJson,
    "ride",
    "workout_json",
    ["id"],
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
