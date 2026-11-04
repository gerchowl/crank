//! The in-app bug report (ADR-0003): what the desktop app sends to GitHub's issue form.
//!
//! [`fields`] is the form contract; [`diagnostics`] turns vigil's `logs.jsonl` into the log tail:
//! [`flatten`] reads the last OTLP records (§2.2), [`allowlist`] reduces each to its structured
//! fields, identifiers replaced by key (§2.3, #508), and squelch's [`Redactor`] reads them with
//! [`allowlisted`]. No free text (a message, an error's text) is ever part of it. The commands
//! and the dialog follow in #446.
pub mod allowlist;
pub mod fields;
pub mod flatten;

use crank_core::KnownDevice;
use squelch::{Record, Redactor};

/// How many records the log tail holds (ADR-0003 §2.2).
pub const TAIL: usize = 200;

/// `redactor` (`Redactor::new()` in the app) with the keys of [`allowlist::record`] in place of
/// squelch's allowlist.
#[must_use]
pub fn allowlisted(redactor: Redactor) -> Redactor {
    redactor.allow_fields(allowlist::keys())
}

/// The report could not be built: the caller sends none (fail closed).
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// squelch kept fewer records than it was given: its reading of a record is not the one
    /// [`allowlist`] wrote it for (a squelch change, or a redactor without [`allowlisted`]).
    Dropped {
        /// Records given.
        given: usize,
        /// Records squelch returned.
        kept: usize,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dropped { given, kept } => {
                write!(f, "squelch kept {kept} of {given} report records")
            }
        }
    }
}

impl std::error::Error for Error {}

/// The last [`TAIL`] records of `jsonl` (one segment of vigil's log), selected before anything
/// else is done to them, reduced by [`allowlist::record`] with `known` (`Core::known_devices`; a
/// caller that cannot read them sends no report), then read by `redactor` as `logs.jsonl`.
///
/// # Errors
/// [`Error::Dropped`] when squelch does not keep every record.
pub fn diagnostics(
    jsonl: &str,
    known: &[KnownDevice],
    redactor: &Redactor,
) -> Result<Vec<Record>, Error> {
    let records = flatten::tail(jsonl, TAIL);
    let given = records.len();
    let flat: String = records
        .into_iter()
        .map(|record| {
            let object = allowlist::record(record, known)
                .into_iter()
                .map(|(k, v)| (k, v.into()));
            serde_json::Value::Object(object.collect()).to_string() + "\n"
        })
        .collect();
    let kept = redactor.records(&flat, Some("logs.jsonl"));
    if kept.len() == given {
        Ok(kept)
    } else {
        Err(Error::Dropped {
            given,
            kept: kept.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(i: usize) -> String {
        format!(
            r#"{{"resourceLogs":[{{"scopeLogs":[{{"scope":{{"name":"m"}},"logRecords":[{{"timeUnixNano":"1000000000","attributes":[{{"key":"attempt","value":{{"intValue":"{i}"}}}}]}}]}}]}}]}}"#
        )
    }

    #[test]
    fn only_the_last_tail_records_are_kept() {
        let jsonl: Vec<String> = (0..TAIL + 50).map(line).collect();
        let records = diagnostics(&jsonl.join("\n"), &[], &allowlisted(Redactor::new()))
            .expect("diagnostics");
        assert_eq!(records.len(), TAIL);
        assert!(
            records[0].render().contains("attempt=50"),
            "{}",
            records[0].render()
        );
    }

    #[test]
    fn a_record_squelch_drops_means_no_report() {
        let none = Redactor::new().allow_fields(Vec::<String>::new());
        assert_eq!(
            diagnostics(&line(1), &[], &none),
            Err(Error::Dropped { given: 1, kept: 0 })
        );
    }
}
