//! The in-app bug report (ADR-0003): what the desktop app sends to GitHub's issue form.
//!
//! This module holds the form contract ([`fields`]); the log mapping, scrubbing and the
//! commands follow in #444–#447.
pub mod fields;
