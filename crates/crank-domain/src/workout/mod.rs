//! Workout DSL contract and parsing; see ADR-0001 §3–4 and `docs/DSL.md`.
mod dsl;
mod model;
pub use dsl::{format_duration, parse_dsl, parse_duration_word, print_dsl};
pub use model::*;
