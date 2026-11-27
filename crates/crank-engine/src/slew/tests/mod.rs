//! Tests of the easer (#620): ports of the model's suites (`docs/adr/models/erg_slew.py`), its
//! properties P1–P7 and D1–D5, golden vectors, and the Rust-side checks of the issue.
#![expect(
    clippy::many_single_char_names,
    reason = "the model's notation: t (tuning), k (tick), y, v, a, r, m"
)]
#![expect(
    clippy::too_many_lines,
    reason = "the model's suites, transcribed case by case"
)]
pub(crate) mod harness;

mod golden;
mod model;
mod props;
mod units;
