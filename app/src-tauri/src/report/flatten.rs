//! vigil's OTLP/JSON `logs.jsonl` read as plain records (ADR-0003 §2.2).
//!
//! squelch's `Redactor::records` finds no timestamp and no field at the top level of a vigil line
//! (`{"resourceLogs":[…]}`), so it would keep nothing. [`tail`] takes the last records of a log
//! segment as [`Otlp`] values, which [`super::allowlist::record`] then reduces to what a report may
//! carry. The resource (`host.name`, `process.pid`, `service.instance.id`) is never read.
use serde_json::{Map, Value};
use time::{OffsetDateTime, macros::format_description};

/// One OTLP log record, as read: nothing here is fit for a report yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Otlp {
    /// `timeUnixNano`, else `observedTimeUnixNano`, as RFC 3339 UTC with milliseconds; `unknown`
    /// when neither is a non-zero time, so the record is kept and visibly has no time.
    pub timestamp: String,
    /// `severityText` when it is a `tracing` level name, else the `severityNumber` range's name.
    pub level: Option<&'static str>,
    /// The scope name: the `tracing` target, a compile-time module path.
    pub module: Option<String>,
    /// `body.stringValue`: the formatted message, values interpolated.
    pub body: Option<String>,
    /// The scalar attributes in order; arrays, key-value lists and bytes are dropped.
    pub attributes: Vec<(String, Scalar)>,
}

/// An OTLP `AnyValue` that is a scalar, keeping its type.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    /// `stringValue`.
    Str(String),
    /// `intValue` (a decimal string in OTLP/JSON; a number is accepted too).
    Int(i64),
    /// `doubleValue`, finite.
    Double(f64),
    /// `boolValue`.
    Bool(bool),
}

/// The last `n` records of `jsonl`, oldest first. Lines are parsed from the end and only until
/// `n` records are found, so the rest of an 8 MiB segment is never read as JSON (ADR-0003 §2.2).
/// A line that does not parse is skipped, never guessed at: a quit or a crash can leave
/// `logs.jsonl` ending in a partial line (ADR-0001 §7.1).
#[must_use]
pub fn tail(jsonl: &str, n: usize) -> Vec<Otlp> {
    let mut lines: Vec<Vec<Otlp>> = Vec::new();
    let mut found = 0;
    for text in jsonl.lines().rev() {
        if found >= n {
            break;
        }
        let records = line(text);
        found += records.len();
        lines.push(records);
    }
    let mut out: Vec<Otlp> = lines.into_iter().rev().flatten().collect();
    out.drain(..out.len().saturating_sub(n));
    out
}

/// The records of one `logs.jsonl` line (one OTLP `LogsData`); none when it does not parse.
#[must_use]
pub fn line(line: &str) -> Vec<Otlp> {
    let Ok(data) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for scope in each(&data, "resourceLogs").flat_map(|r| each(r, "scopeLogs")) {
        let module = scope.pointer("/scope/name").and_then(Value::as_str);
        for record in each(scope, "logRecords") {
            out.push(Otlp {
                timestamp: timestamp(record),
                level: level(record),
                module: module.map(Into::into),
                body: record
                    .pointer("/body/stringValue")
                    .and_then(Value::as_str)
                    .map(Into::into),
                attributes: each(record, "attributes")
                    .filter_map(|attr| {
                        let key = attr.get("key")?.as_str()?;
                        Some((key.to_owned(), scalar(attr.get("value")?.as_object()?)?))
                    })
                    .collect(),
            });
        }
    }
    out
}

fn each<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn timestamp(record: &Value) -> String {
    ["timeUnixNano", "observedTimeUnixNano"]
        .into_iter()
        .filter_map(|key| record.get(key)?.as_str()?.parse::<i128>().ok())
        .filter(|&nanos| nanos != 0)
        .find_map(|nanos| {
            let at = OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
            at.format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
            ))
            .ok()
        })
        .unwrap_or_else(|| "unknown".into())
}

const LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

fn level(record: &Value) -> Option<&'static str> {
    let text = record.get("severityText").and_then(Value::as_str);
    if let Some(known) = LEVELS.into_iter().find(|l| text == Some(*l)) {
        return Some(known);
    }
    let index = match record.get("severityNumber").and_then(Value::as_i64)? {
        1..=4 => 0,
        5..=8 => 1,
        9..=12 => 2,
        13..=16 => 3,
        17..=24 => 4,
        _ => return None,
    };
    Some(LEVELS[index])
}

fn scalar(value: &Map<String, Value>) -> Option<Scalar> {
    let (kind, value) = value.iter().next()?;
    match (kind.as_str(), value) {
        ("stringValue", Value::String(s)) => Some(Scalar::Str(s.clone())),
        // OTLP/JSON writes a 64-bit `intValue` as a decimal string; accept a number too.
        ("intValue", Value::String(s)) => s.parse().ok().map(Scalar::Int),
        ("intValue", Value::Number(n)) => n.as_i64().map(Scalar::Int),
        ("doubleValue", Value::Number(n)) => {
            n.as_f64().filter(|f| f.is_finite()).map(Scalar::Double)
        }
        ("boolValue", Value::Bool(b)) => Some(Scalar::Bool(*b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(record: &str) -> Otlp {
        let line = format!(
            r#"{{"resourceLogs":[{{"resource":{{"attributes":[{{"key":"host.name","value":{{"stringValue":"riders-mbp"}}}}]}},"scopeLogs":[{{"scope":{{"name":"crank_ble"}},"logRecords":[{record}]}}]}}]}}"#
        );
        let mut records = super::line(&line);
        assert_eq!(records.len(), 1, "{line}");
        records.remove(0)
    }

    #[test]
    fn maps_a_vigil_record() {
        let r = one(
            r#"{"timeUnixNano":"1793650453584412000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"trainer connected"},"attributes":[{"key":"page","value":{"intValue":"25"}},{"key":"connected","value":{"boolValue":true}},{"key":"ratio","value":{"doubleValue":0.5}},{"key":"id","value":{"stringValue":"x"}}],"eventName":"event crates/x.rs:1","traceId":"ab","spanId":"cd","flags":1}"#,
        );
        assert_eq!(
            r,
            Otlp {
                timestamp: "2026-11-02T20:14:13.584Z".into(),
                level: Some("INFO"),
                module: Some("crank_ble".into()),
                body: Some("trainer connected".into()),
                attributes: vec![
                    ("page".into(), Scalar::Int(25)),
                    ("connected".into(), Scalar::Bool(true)),
                    ("ratio".into(), Scalar::Double(0.5)),
                    ("id".into(), Scalar::Str("x".into())),
                ],
            },
            "resource, eventName, ids and flags are not read"
        );
    }

    #[test]
    fn severity_number_stands_in_for_a_missing_or_unknown_text() {
        let level = |rec: &str| one(rec).level;
        assert_eq!(level(r#"{"severityNumber":3}"#), Some("TRACE"));
        assert_eq!(level(r#"{"severityNumber":5}"#), Some("DEBUG"));
        assert_eq!(level(r#"{"severityNumber":12}"#), Some("INFO"));
        assert_eq!(
            level(r#"{"severityNumber":13,"severityText":"Tacx Neo"}"#),
            Some("WARN")
        );
        assert_eq!(level(r#"{"severityNumber":24}"#), Some("ERROR"));
        assert_eq!(level(r#"{"severityNumber":0,"severityText":"x"}"#), None);
        assert_eq!(
            level(r#"{"severityText":"WARN","severityNumber":17}"#),
            Some("WARN")
        );
    }

    #[test]
    fn a_zero_missing_or_bad_time_is_unknown_and_observed_time_is_the_fallback() {
        let time = |rec: &str| one(rec).timestamp;
        assert_eq!(time(r#"{"timeUnixNano":"0"}"#), "unknown");
        assert_eq!(time("{}"), "unknown");
        assert_eq!(time(r#"{"timeUnixNano":"soon"}"#), "unknown");
        assert_eq!(
            time(r#"{"timeUnixNano":"0","observedTimeUnixNano":"1000000000"}"#),
            "1970-01-01T00:00:01.000Z"
        );
    }

    #[test]
    fn non_scalar_values_and_bodies_are_dropped() {
        let r = one(
            r#"{"body":{"kvlistValue":{"values":[]}},"attributes":[{"key":"a","value":{"arrayValue":{"values":[]}}},{"key":"b","value":{"bytesValue":"AA=="}},{"key":"c","value":{"kvlistValue":{"values":[]}}},{"key":"d","value":{"intValue":"x"}}]}"#,
        );
        assert_eq!((r.body, r.attributes), (None, vec![]));
    }

    #[test]
    fn the_tail_is_the_last_records_and_a_partial_last_line_is_skipped() {
        let full = r#"{"resourceLogs":[{"scopeLogs":[{"scope":{"name":"m"},"logRecords":[{"body":{"stringValue":"one"}},{"body":{"stringValue":"two"}}]}]}]}"#;
        let jsonl = format!("{full}\n{full}\n{}", &full[..full.len() / 2]);
        let bodies =
            |n| -> Vec<String> { tail(&jsonl, n).into_iter().filter_map(|r| r.body).collect() };
        assert_eq!(bodies(10), ["one", "two", "one", "two"]);
        assert_eq!(bodies(3), ["two", "one", "two"]);
        assert_eq!(bodies(0), Vec::<String>::new());
    }
}
