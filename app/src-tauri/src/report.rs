//! The in-app bug report (ADR-0003): what the desktop app sends to GitHub's issue form.
//!
//! [`fields`] is the form contract; [`diagnostics`] turns vigil's `logs.jsonl` into the redacted
//! log tail: [`flatten`] maps each OTLP record to a flat one (§2.2), [`scrub`] replaces device
//! identities in every value (§2.3), and squelch's [`Redactor`] keeps only the scrubbed defaults
//! and [`scrub::ALLOWED_FIELDS`]. The commands and the dialog follow in #446.
pub mod fields;
pub mod flatten;
pub mod scrub;

use squelch::{Record, Redactor};

/// How many records the log tail holds (ADR-0003 §2.2).
pub const TAIL: usize = 200;

/// `redactor` (`Redactor::new()` in the app) with crank's field allowlist in place of squelch's.
#[must_use]
pub fn allowlisted(redactor: Redactor) -> Redactor {
    redactor.allow_fields(scrub::ALLOWED_FIELDS)
}

/// The last [`TAIL`] records of `jsonl` (one segment of vigil's log), redacted: flattened, the
/// tail selected first so that only it is scrubbed, pre-scrubbed with `scrub`, then read by
/// `redactor` as `logs.jsonl`. Lines that do not parse are skipped, a partial last one included.
#[must_use]
pub fn diagnostics(jsonl: &str, scrub: &scrub::Scrub, redactor: &Redactor) -> Vec<Record> {
    let mut records = flatten::records(jsonl);
    records.drain(..records.len().saturating_sub(TAIL));
    let flat: String = records
        .into_iter()
        .map(|record| {
            let object = scrub.record(record).into_iter().map(|(k, v)| (k, v.into()));
            serde_json::Value::Object(object.collect()).to_string() + "\n"
        })
        .collect();
    redactor.records(&flat, Some("logs.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_tail_records_are_kept() {
        let line = |i: usize| {
            format!(
                r#"{{"resourceLogs":[{{"scopeLogs":[{{"scope":{{"name":"m"}},"logRecords":[{{"timeUnixNano":"1000000000","body":{{"stringValue":"record {i}"}}}}]}}]}}]}}"#
            )
        };
        let jsonl: Vec<String> = (0..TAIL + 50).map(line).collect();
        let scrub = scrub::Scrub::new(&[]).expect("table");
        let records = diagnostics(&jsonl.join("\n"), &scrub, &allowlisted(Redactor::new()));
        assert_eq!(records.len(), TAIL);
        assert!(
            records[0].render().contains("record 50"),
            "{}",
            records[0].render()
        );
    }
}
