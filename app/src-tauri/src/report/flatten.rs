//! vigil's OTLP/JSON `logs.jsonl` mapped to the flat records squelch reads (ADR-0003 §2.2).
//!
//! squelch's `Redactor::records` finds no timestamp and no field at the top level of a vigil line
//! (`{"resourceLogs":[…]}`), so it would keep nothing. Each OTLP log record becomes one flat
//! record here; the resource (`host.name`, `process.pid`, `service.instance.id`) is dropped.
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use time::{OffsetDateTime, macros::format_description};

/// One log record as flat key → string pairs: `timestamp`, `level` (when known), `module`,
/// `message` and the event's attributes.
pub type Flat = BTreeMap<String, String>;

/// The keys a record owns: an attribute with one of these names becomes `attr.<key>`.
const OWN_KEYS: [&str; 4] = ["timestamp", "level", "message", "module"];

/// Every record of every line of `jsonl`, in order. A line that does not parse is skipped, never
/// guessed at: a quit or a crash can leave `logs.jsonl` ending in a partial line (ADR-0001 §7.1).
#[must_use]
pub fn records(jsonl: &str) -> Vec<Flat> {
    jsonl.lines().flat_map(line).collect()
}

/// The records of one `logs.jsonl` line (one OTLP `LogsData`); none when it does not parse.
#[must_use]
pub fn line(line: &str) -> Vec<Flat> {
    let Ok(data) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for scope in each(&data, "resourceLogs").flat_map(|r| each(r, "scopeLogs")) {
        let module = scope.pointer("/scope/name").and_then(Value::as_str);
        for record in each(scope, "logRecords") {
            out.push(flat(record, module));
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

fn flat(record: &Value, module: Option<&str>) -> Flat {
    let mut out = Flat::new();
    out.insert("timestamp".into(), timestamp(record));
    if let Some(level) = level(record) {
        out.insert("level".into(), level);
    }
    if let Some(module) = module {
        out.insert("module".into(), module.into());
    }
    if let Some(message) = record.pointer("/body/stringValue").and_then(Value::as_str) {
        out.insert("message".into(), message.into());
    }
    for attr in each(record, "attributes") {
        let (Some(key), Some(value)) = (
            attr.get("key").and_then(Value::as_str),
            attr.get("value")
                .and_then(Value::as_object)
                .and_then(scalar),
        ) else {
            continue;
        };
        let key = if OWN_KEYS.contains(&key) {
            format!("attr.{key}")
        } else {
            key.to_owned()
        };
        out.insert(key, value);
    }
    out
}

/// `timeUnixNano`, else `observedTimeUnixNano`, as RFC 3339 UTC with milliseconds; `unknown` when
/// neither is a non-zero time, so the record is kept and visibly has no time.
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

/// `severityText`, else the `severityNumber` range's name.
fn level(record: &Value) -> Option<String> {
    let text = record.get("severityText").and_then(Value::as_str);
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        return Some(text.into());
    }
    let name = match record.get("severityNumber").and_then(Value::as_i64)? {
        1..=4 => "TRACE",
        5..=8 => "DEBUG",
        9..=12 => "INFO",
        13..=16 => "WARN",
        17..=24 => "ERROR",
        _ => return None,
    };
    Some(name.into())
}

/// An OTLP `AnyValue` that is a string, an integer, a double or a bool, as a string. Arrays,
/// key-value lists and bytes are dropped.
fn scalar(value: &Map<String, Value>) -> Option<String> {
    let (kind, value) = value.iter().next()?;
    match (kind.as_str(), value) {
        ("stringValue", Value::String(s)) => Some(s.clone()),
        // OTLP/JSON writes a 64-bit `intValue` as a decimal string; accept a number too.
        ("intValue", Value::String(s)) if s.parse::<i64>().is_ok() => Some(s.clone()),
        ("intValue" | "doubleValue", Value::Number(n)) => Some(n.to_string()),
        ("boolValue", Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(record: &str) -> Flat {
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
            r#"{"timeUnixNano":"1793650453584412000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"trainer connected"},"attributes":[{"key":"page","value":{"intValue":"25"}},{"key":"connected","value":{"boolValue":true}},{"key":"ratio","value":{"doubleValue":0.5}}],"eventName":"event crates/x.rs:1","traceId":"ab","spanId":"cd","flags":1}"#,
        );
        let expected: Flat = [
            ("timestamp", "2026-11-02T20:14:13.584Z"),
            ("level", "INFO"),
            ("module", "crank_ble"),
            ("message", "trainer connected"),
            ("page", "25"),
            ("connected", "true"),
            ("ratio", "0.5"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        assert_eq!(
            r, expected,
            "resource, eventName, ids and flags are dropped"
        );
    }

    #[test]
    fn severity_number_stands_in_for_a_missing_text() {
        let level = |rec: &str| one(rec).get("level").cloned();
        assert_eq!(level(r#"{"severityNumber":3}"#).as_deref(), Some("TRACE"));
        assert_eq!(level(r#"{"severityNumber":5}"#).as_deref(), Some("DEBUG"));
        assert_eq!(level(r#"{"severityNumber":12}"#).as_deref(), Some("INFO"));
        assert_eq!(
            level(r#"{"severityNumber":13,"severityText":""}"#).as_deref(),
            Some("WARN")
        );
        assert_eq!(level(r#"{"severityNumber":24}"#).as_deref(), Some("ERROR"));
        assert_eq!(level(r#"{"severityNumber":0}"#), None);
        assert_eq!(
            level(r#"{"severityText":"WARN","severityNumber":17}"#).as_deref(),
            Some("WARN")
        );
    }

    #[test]
    fn a_zero_missing_or_bad_time_is_unknown_and_observed_time_is_the_fallback() {
        let time = |rec: &str| one(rec)["timestamp"].clone();
        assert_eq!(time(r#"{"timeUnixNano":"0"}"#), "unknown");
        assert_eq!(time("{}"), "unknown");
        assert_eq!(time(r#"{"timeUnixNano":"soon"}"#), "unknown");
        assert_eq!(
            time(r#"{"timeUnixNano":"0","observedTimeUnixNano":"1000000000"}"#),
            "1970-01-01T00:00:01.000Z"
        );
    }

    #[test]
    fn an_attribute_never_overwrites_the_records_own_keys() {
        let r = one(
            r#"{"severityText":"WARN","body":{"stringValue":"real"},"attributes":[{"key":"level","value":{"stringValue":"fake"}},{"key":"message","value":{"stringValue":"fake"}},{"key":"timestamp","value":{"stringValue":"fake"}},{"key":"module","value":{"stringValue":"fake"}}]}"#,
        );
        assert_eq!(
            (r["level"].as_str(), r["message"].as_str()),
            ("WARN", "real")
        );
        assert_eq!(r["module"], "crank_ble");
        for key in [
            "attr.level",
            "attr.message",
            "attr.timestamp",
            "attr.module",
        ] {
            assert_eq!(r[key], "fake", "{key}");
        }
    }

    #[test]
    fn non_scalar_values_and_bodies_are_dropped() {
        let r = one(
            r#"{"body":{"kvlistValue":{"values":[]}},"attributes":[{"key":"a","value":{"arrayValue":{"values":[]}}},{"key":"b","value":{"bytesValue":"AA=="}},{"key":"c","value":{"kvlistValue":{"values":[]}}}]}"#,
        );
        assert_eq!(r.keys().collect::<Vec<_>>(), ["module", "timestamp"]);
    }

    #[test]
    fn a_partial_last_line_is_skipped_and_the_complete_ones_kept() {
        let full = r#"{"resourceLogs":[{"scopeLogs":[{"scope":{"name":"m"},"logRecords":[{"body":{"stringValue":"one"}},{"body":{"stringValue":"two"}}]}]}]}"#;
        let jsonl = format!("{full}\n{full}\n{}", &full[..full.len() / 2]);
        let messages: Vec<_> = records(&jsonl)
            .into_iter()
            .map(|r| r["message"].clone())
            .collect();
        assert_eq!(messages, ["one", "two", "one", "two"]);
    }
}
