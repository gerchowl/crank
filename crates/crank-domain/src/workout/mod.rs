//! Workout DSL contract and parsing; see ADR-0001 §3–4 and `docs/DSL.md`.
mod dsl;
mod model;
pub use dsl::{MAX_NESTING_DEPTH, format_duration, parse_dsl, parse_duration_word, print_dsl};
pub use model::*;
mod compile;
mod compiled;
pub use compile::{
    LAP_EXPECTED_S, LAP_RANGE_S, TRAINER_MAX_W, WORK_THRESHOLD_PCT, compile_workout, is_work_step,
    normalized_power, target_pct, target_watts,
};
pub use compiled::*;
mod tree;
pub use tree::{TreeError, find_node, re_id};
