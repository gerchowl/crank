//! What a report record may carry (ADR-0003 §2.3, #508): structured fields only.
//!
//! A record is `timestamp`, `level`, `module` (when it passes [`module`]), `event` and the
//! attributes this module names.
//! Free text never reaches a report: the OTLP body (the formatted message) and every attribute
//! not named here (`message`, `error`, `e`, `err`, `?Debug` values) are dropped, not scrubbed,
//! because a pattern scrub of free text cannot be made safe (#482 found a new leak spelling in
//! every review round). An error therefore reaches a report only as its kind or variant name, in
//! the [`ALLOWED`] `kind` field.
//!
//! - [`ALLOWED`] fields are typed: a value of another OTLP type, or a string outside the field's
//!   closed vocabulary, is dropped. A new field is absent from reports until it is added here.
//! - [`IDENTIFIERS`] fields are redacted **by key**: the value is replaced by its class's
//!   placeholder whatever it is. A whole value equal to a [`KnownDevice`]'s name, serial or
//!   platform id picks that device's role (`<trainer>`, `<strap>`) instead. Nothing is ever
//!   searched inside a value.
//! - `event` is a stable event name from [`EVENTS`]: an explicit `event` attribute, else the
//!   body when it is exactly one of crank's literal (uninterpolated) messages, else `log`. vigil
//!   0.2.0 writes the formatted message, not its template, so a body with interpolated values
//!   has no reliable template. **Triage cost**: until call sites log typed fields and an `event`
//!   (#509), such records show `event=log`, and no interpolated value reaches the report.
use super::flatten::{Otlp, Scalar};
use crank_core::{DeviceRole, KnownDevice};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// One report record as flat key → string pairs, the JSON object squelch reads.
pub type Flat = BTreeMap<String, String>;

/// The type an [`ALLOWED`] field's value must have.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// An `intValue`, or a finite `doubleValue`.
    Number,
    /// A `boolValue`.
    Bool,
    /// A `stringValue` that is exactly one of these.
    OneOf(&'static [&'static str]),
    /// A `stringValue` of the form crank-core gives a ride id: `ride-` and lowercase hex digits
    /// (the start time in nanoseconds).
    RideId,
}

/// Connect phases (`crank_core` links): the `from` and `to` of a `link phase` record.
const PHASES: &[&str] = &[
    "resolve",
    "gatt",
    "identity",
    "handshake",
    "ready",
    "not_found",
];

/// The GATT service and characteristic UUIDs crank uses (FE-C, FTMS, Cycling Power, Heart Rate,
/// DIS). They name a protocol, not a device.
pub const GATT_UUIDS: &[&str] = &[
    "6e40fec1-b5a3-f393-e0a9-e50e24dcca9e",
    "6e40fec2-b5a3-f393-e0a9-e50e24dcca9e",
    "6e40fec3-b5a3-f393-e0a9-e50e24dcca9e",
    "00001826-0000-1000-8000-00805f9b34fb",
    "00001818-0000-1000-8000-00805f9b34fb",
    "0000180d-0000-1000-8000-00805f9b34fb",
    "00002a37-0000-1000-8000-00805f9b34fb",
    "0000180a-0000-1000-8000-00805f9b34fb",
    "00002a24-0000-1000-8000-00805f9b34fb",
    "00002a25-0000-1000-8000-00805f9b34fb",
    "00002a26-0000-1000-8000-00805f9b34fb",
];

/// The scalar fields a report keeps, with their types.
pub const ALLOWED: [(&str, Kind); 17] = [
    ("page", Kind::Number),
    ("attempt", Kind::Number),
    ("ms", Kind::Number),
    ("lost", Kind::Number),
    ("watts", Kind::Number),
    ("rpm", Kind::Number),
    ("force", Kind::Bool),
    ("connected", Kind::Bool),
    ("takeover", Kind::Bool),
    ("taken", Kind::Bool),
    ("device", Kind::OneOf(&["trainer", "strap"])),
    ("from", Kind::OneOf(PHASES)),
    ("to", Kind::OneOf(PHASES)),
    ("via", Kind::OneOf(&["lookup", "scan"])),
    // `crank_devices::ProtocolErrorKind` and `vigil::FlushErrorKind`, as `?Debug` writes them.
    (
        "kind",
        Kind::OneOf(&[
            "Framing",
            "Checksum",
            "UnexpectedPage",
            "Encode",
            "Timeout",
            "Failed",
            "DataLost",
        ]),
    ),
    ("characteristic", Kind::OneOf(GATT_UUIDS)),
    ("ride_id", Kind::RideId),
];

/// What an [`IDENTIFIERS`] field holds, which names its placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identifier {
    /// A device name: `<device-name>`.
    Name,
    /// A serial number (FE-C page 81, DIS `0x2A25`): `<serial>`.
    Serial,
    /// A BLE address: `<ble-addr>`.
    Address,
    /// A platform peripheral id or a key built from one: `<ble-id>`.
    Id,
    /// A file or directory: `<path>`.
    Path,
}

impl Identifier {
    fn placeholder(self) -> &'static str {
        match self {
            Self::Name => "<device-name>",
            Self::Serial => "<serial>",
            Self::Address => "<ble-addr>",
            Self::Id => "<ble-id>",
            Self::Path => "<path>",
        }
    }
}

/// The fields that carry an identifier, kept only as a placeholder. `db_id` is the store's
/// device key, `<kind>:<platform id>`.
pub const IDENTIFIERS: [(&str, Identifier); 17] = [
    ("name", Identifier::Name),
    ("local_name", Identifier::Name),
    ("device_name", Identifier::Name),
    ("serial", Identifier::Serial),
    ("serial_number", Identifier::Serial),
    ("sn", Identifier::Serial),
    ("address", Identifier::Address),
    ("addr", Identifier::Address),
    ("mac", Identifier::Address),
    ("id", Identifier::Id),
    ("peripheral_id", Identifier::Id),
    ("platform_id", Identifier::Id),
    ("device_id", Identifier::Id),
    ("db_id", Identifier::Id),
    ("path", Identifier::Path),
    ("dir", Identifier::Path),
    ("file", Identifier::Path),
];

/// The stable event names: crank's literal log messages today (a survey of the `tracing` calls in
/// `crates/` and `app/src-tauri/`, 2026-11-04; the `event_names_are_crank_messages` test keeps
/// each one in the source). A message with interpolated values is not one. #509's `event` names
/// are added here.
pub const EVENTS: [&str; 40] = [
    "a device did not confirm release",
    "a driver did not close its stream after the engine",
    "BLE scan failed to start",
    "BLE scan failed to stop",
    "cancelled connection cleanup failed",
    "capturing trainer frames",
    "closing the ride's record failed",
    "command discarded while disconnected",
    "disconnect after a failed setup failed",
    "empty trainer capture not removed",
    "link phase",
    "menu: the first default menu item is not the application submenu; keeping the default menu",
    "menu: the last application menu item is not the built-in Quit; keeping the default menu",
    "quit: ending the ride failed",
    "quit: ride span closed with the ride still loaded",
    "quit: the core is busy, the ride span stays open",
    "quit: the ride was not ended within the cap",
    "ready_total",
    "reconnecting heart-rate strap",
    "reconnecting trainer",
    "rejected FE-C notification",
    "ride released",
    "strap connect timed out: riding without heart rate",
    "strap reconnect failed",
    "strap unavailable: riding without heart rate",
    "the core did not open",
    "the core has taken the ride over: an engine command is dropped",
    "the engine did not end: aborting it",
    "the engine did not release the ride: reaping it",
    "the engine did not stop the ride: ending it by force",
    "the engine did not take a window's close",
    "the engine ignores the abort: detaching it",
    "the store did not close the ride's record in time",
    "the trainer did not confirm the end mode of a forced end",
    "the trainer's control channel took no disconnect",
    "trainer capture not created",
    "trainer capture write failed; this capture stops growing",
    "trainer did not confirm release",
    "trainer reconnect failed",
    "unknown FE-C page",
];

/// The longest `module` kept, in bytes. crank's longest module path is under 40 bytes and a
/// dependency's rarely passes 60; 128 keeps every real one and bounds what one field can carry.
pub const MODULE_MAX: usize = 128;

/// Library names of crank's dependencies that differ from their package name, which is all
/// `Cargo.lock` records: `vigil-telemetry`'s lib is `vigil`.
const LIB_NAMES: [&str; 1] = ["vigil"];

/// The crate names of crank's build, from `Cargo.lock` (`-` written `_`, as in a module path),
/// and [`LIB_NAMES`].
static CRATES: LazyLock<BTreeSet<String>> = LazyLock::new(|| {
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .lines()
        .filter_map(|line| line.strip_prefix("name = \"")?.strip_suffix('"'))
        .map(|name| name.replace('-', "_"))
        .chain(LIB_NAMES.map(String::from))
        .collect()
});

/// Whether a scope name may be kept as `module`: a Rust module path (`[A-Za-z0-9_]+` segments
/// joined by `::`) of at most [`MODULE_MAX`] bytes whose first segment is a crate of crank's build.
/// A `tracing` target is `module_path!()` unless a call site sets one, but a record bridged from
/// the `log` crate (`tracing-log`) carries a runtime `&str` target, so the shape alone is no
/// guarantee: `TacxNeo` or `C47D4F12345B` is a valid path. The crate root is a closed vocabulary
/// that cannot hold a device's or the machine's name. Anything else is dropped, never trimmed.
#[must_use]
pub fn module(target: &str) -> bool {
    let segment =
        |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    target.len() <= MODULE_MAX
        && target.split("::").all(segment)
        && target
            .split("::")
            .next()
            .is_some_and(|root| CRATES.contains(root))
}

/// The keys a report record can have, as squelch's allowlist must name them: without squelch's
/// scrubbed fields (`addr`, `address`), which it always reads, and whose value it masks once more
/// as a host name (`<ble-addr>` renders as `<<host>>`).
pub fn keys() -> impl Iterator<Item = &'static str> {
    ["module", "event"]
        .into_iter()
        .chain(ALLOWED.iter().map(|(key, _)| *key))
        .chain(IDENTIFIERS.iter().map(|(key, _)| *key))
        .filter(|key| !squelch::redact::DEFAULT_SCRUBBED_FIELDS.contains(key))
}

/// `record` reduced to what a report may carry; `known` picks the role of identifier values.
#[must_use]
pub fn record(record: Otlp, known: &[KnownDevice]) -> Flat {
    let mut out = Flat::new();
    out.insert("timestamp".into(), record.timestamp);
    if let Some(level) = record.level {
        out.insert("level".into(), level.into());
    }
    if let Some(module) = record.module.filter(|m| module(m)) {
        out.insert("module".into(), module);
    }
    let named = |s: &Scalar| match s {
        Scalar::Str(s) => EVENTS.iter().find(|e| **e == s.as_str()),
        _ => None,
    };
    let explicit = record.attributes.iter().find(|(k, _)| k == "event");
    let event = explicit
        .and_then(|(_, v)| named(v))
        .or_else(|| EVENTS.iter().find(|e| record.body.as_deref() == Some(**e)))
        .unwrap_or(&"log");
    out.insert("event".into(), (*event).into());
    for (key, value) in record.attributes {
        if let Some((_, class)) = IDENTIFIERS.iter().find(|(k, _)| *k == key) {
            out.insert(key, placeholder(*class, &value, known).into());
        } else if let Some((_, kind)) = ALLOWED.iter().find(|(k, _)| *k == key)
            && let Some(value) = typed(*kind, value)
        {
            out.insert(key, value);
        }
    }
    out
}

/// The role placeholder of the known device `value` wholly is, else `class`'s.
fn placeholder(class: Identifier, value: &Scalar, known: &[KnownDevice]) -> &'static str {
    let fold = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let value = match value {
        Scalar::Str(s) => fold(s),
        Scalar::Int(n) => n.to_string(),
        Scalar::Double(_) | Scalar::Bool(_) => return class.placeholder(),
    };
    let is = |s: &Option<String>| s.as_deref().is_some_and(|s| fold(s) == value);
    let role = (class != Identifier::Path)
        .then(|| {
            known
                .iter()
                .find(|d| is(&d.name) || is(&d.serial) || fold(&d.platform_id) == value)
        })
        .flatten();
    match role.map(|d| d.role) {
        Some(DeviceRole::Trainer) => "<trainer>",
        Some(DeviceRole::HeartRate) => "<strap>",
        None => class.placeholder(),
    }
}

/// `value` as a string when it has `kind`'s type.
fn typed(kind: Kind, value: Scalar) -> Option<String> {
    match (kind, value) {
        (Kind::Number, Scalar::Int(n)) => Some(n.to_string()),
        (Kind::Number, Scalar::Double(n)) => Some(n.to_string()),
        (Kind::Bool, Scalar::Bool(b)) => Some(b.to_string()),
        (Kind::OneOf(words), Scalar::Str(s)) => words.contains(&s.as_str()).then_some(s),
        (Kind::RideId, Scalar::Str(s)) => s
            .strip_prefix("ride-")
            .is_some_and(|hex| {
                (1..=32).contains(&hex.len())
                    && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            })
            .then_some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn otlp(body: Option<&str>, attributes: Vec<(&str, Scalar)>) -> Otlp {
        Otlp {
            timestamp: "unknown".into(),
            level: Some("WARN"),
            module: Some("crank_ble".into()),
            body: body.map(Into::into),
            attributes: attributes.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }
    fn s(v: &str) -> Scalar {
        Scalar::Str(v.into())
    }

    #[test]
    fn the_body_and_free_text_attributes_are_dropped_and_types_are_enforced() {
        let r = record(
            otlp(
                Some("trainer reconnect failed"),
                vec![
                    ("error", s("connect to Tacx Neo 55624 failed")),
                    ("message", s("x")),
                    ("page", Scalar::Int(25)),
                    ("watts", Scalar::Double(212.5)),
                    ("attempt", s("3")),
                    ("connected", Scalar::Bool(true)),
                    ("force", Scalar::Int(1)),
                    ("kind", s("Checksum")),
                    ("to", s("gatt C4:7D")),
                    ("ride_id", s("ride-18f3a")),
                    ("level", s("fake")),
                ],
            ),
            &[],
        );
        let expected: Flat = [
            ("timestamp", "unknown"),
            ("level", "WARN"),
            ("module", "crank_ble"),
            ("event", "trainer reconnect failed"),
            ("page", "25"),
            ("watts", "212.5"),
            ("connected", "true"),
            ("kind", "Checksum"),
            ("ride_id", "ride-18f3a"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        assert_eq!(r, expected);
    }

    #[test]
    fn the_event_is_a_known_name_else_the_literal_body_else_log() {
        let event = |body: Option<&str>, explicit: Option<Scalar>| {
            let attrs = explicit.into_iter().map(|e| ("event", e)).collect();
            record(otlp(body, attrs), &[])["event"].clone()
        };
        assert_eq!(event(Some("ride released"), None), "ride released");
        assert_eq!(event(Some("seen Tacx Neo 55624"), None), "log");
        assert_eq!(event(None, None), "log");
        assert_eq!(
            event(Some("seen Tacx Neo 55624"), Some(s("link phase"))),
            "link phase"
        );
        assert_eq!(
            event(Some("ride released"), Some(s("tacx neo 55624"))),
            "ride released",
            "an unknown explicit name falls back"
        );
        assert_eq!(event(None, Some(Scalar::Int(7))), "log");
    }

    #[test]
    fn identifier_fields_become_placeholders_and_known_devices_their_roles() {
        let known = [
            KnownDevice {
                role: DeviceRole::Trainer,
                platform_id: "3F2504E0-4F89-41D3-9A0C-0305E82C3301".into(),
                name: Some("Tacx Neo 55624".into()),
                serial: Some("2655624".into()),
            },
            KnownDevice {
                role: DeviceRole::HeartRate,
                platform_id: "hrm".into(),
                name: None,
                serial: None,
            },
        ];
        let field =
            |key: &str, value: Scalar| record(otlp(None, vec![(key, value)]), &known)[key].clone();
        assert_eq!(field("name", s("tacx\u{a0}neo  55624")), "<trainer>");
        assert_eq!(
            field("name", s("Tacx Neo")),
            "<device-name>",
            "whole value only"
        );
        assert_eq!(field("local_name", s("KICKR 1A2B")), "<device-name>");
        assert_eq!(field("serial", s("2655624")), "<trainer>");
        assert_eq!(field("sn", Scalar::Int(2_655_624)), "<trainer>");
        assert_eq!(field("sn", Scalar::Int(7_788_991)), "<serial>");
        assert_eq!(field("addr", s("C4:7D:4F:12:34:56")), "<ble-addr>");
        assert_eq!(
            field("id", s("3f2504e0-4f89-41d3-9a0c-0305e82c3301")),
            "<trainer>"
        );
        assert_eq!(field("peripheral_id", s("hrm")), "<strap>");
        assert_eq!(field("db_id", s("trainer:3F2504E0")), "<ble-id>");
        assert_eq!(field("path", s("hrm")), "<path>", "a path has no role");
        assert_eq!(field("dir", Scalar::Bool(true)), "<path>");
    }

    #[test]
    fn module_is_kept_only_as_a_path_of_a_crate_of_the_build() {
        for kept in ["crank_ble", "crank_core::links", "vigil", "tauri::app"] {
            assert!(module(kept), "{kept}");
        }
        let long = format!("crank_ble::{}", "a".repeat(MODULE_MAX));
        for dropped in [
            "",
            "Tacx Neo 55624",
            "crank_ble C4:7D:4F:12:34:56",
            "crank_ble::C4:7D:4F:12:34:56",
            "rider@example.com",
            "crank_ble::r\u{e9}seau",
            "crank_ble::",
            "::crank_ble",
            "TacxNeo",
            "C47D4F12345B",
            "serial7788984",
            &long,
        ] {
            assert!(!module(dropped), "{dropped:?}");
        }
        let with = |name: &str| {
            let mut r = otlp(None, vec![]);
            r.module = Some(name.into());
            record(r, &[]).get("module").cloned()
        };
        assert_eq!(
            with("crank_core::core").as_deref(),
            Some("crank_core::core")
        );
        assert_eq!(with("Tacx Neo 55624"), None, "dropped, not scrubbed");
    }

    #[test]
    fn event_names_are_crank_messages() {
        // Each name but `log` is a literal message of a `tracing` call in crank's sources, so a
        // reworded message does not silently turn its records into `event=log`.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut sources = String::new();
        let crates = std::fs::read_dir(root.join("crates")).expect("crates");
        let mut dirs: Vec<_> = crates
            .map(|c| c.expect("crate").path().join("src"))
            .filter(|src| src.is_dir())
            .collect();
        dirs.push(root.join("app/src-tauri/src"));
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("source dir") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("src/report/allowlist.rs")
                {
                    sources += &std::fs::read_to_string(&path).expect("source");
                }
            }
        }
        for event in EVENTS {
            assert!(!event.contains('{'), "{event:?} is a template with values");
            assert!(
                sources.contains(&format!("\"{event}\"")),
                "{event:?} is no longer a crank log message"
            );
        }
    }
}
