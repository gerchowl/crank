//! The privacy pin of the bug report's log tail (ADR-0003 §2.3, #508).
//!
//! `fixtures/report_privacy/logs.jsonl` is vigil 0.2.0 output of crank's own logging pipeline
//! (`LogPlan::install` in a capture directory, events at crank's targets), hand-salted afterwards:
//! `host.name`, two lines written by hand (edge cases, identifier and typed fields, `event`
//! attributes) and a partial last line. No hardware session log is committed, so the events
//! stand in for one. Then every leak spelling #482's review rounds found is salted, one at a
//! time, into the body, the free-text fields and every field a record can keep; none may reach
//! the preview, while what a triager needs must.
use crank_core::{DeviceRole, KnownDevice};
use crank_desktop::report::{self, allowlist};
use squelch::{Destination, Redactor, Report, Transport};

const LOG: &str = include_str!("fixtures/report_privacy/logs.jsonl");

/// What the core would report: the paired trainer and strap.
fn known() -> Vec<KnownDevice> {
    vec![
        KnownDevice {
            role: DeviceRole::Trainer,
            platform_id: "3F2504E0-4F89-41D3-9A0C-0305E82C3301".into(),
            name: Some("Tacx Neo 55624".into()),
            serial: Some("2655624".into()),
        },
        KnownDevice {
            role: DeviceRole::HeartRate,
            platform_id: "7A1B2C3D-5E6F-4A0B-9C1D-2E3F4A5B6C7D".into(),
            name: Some("HRM-Pro:9871".into()),
            serial: None,
        },
    ]
}

/// What the reporter would see, built as the preview command will build it (#446).
fn preview(jsonl: &str) -> String {
    let redactor = report::allowlisted(Redactor::with_identity(
        Some("/home/rider".into()),
        Some("rider".into()),
    ));
    let records = report::diagnostics(jsonl, &known(), &redactor).expect("diagnostics");
    Report::to(Destination::parse("gerchowl/crank").expect("destination"))
        .via(Transport::Browser)
        .diagnostics(records)
        .preview()
        .expect("preview")
}

/// The leak spellings of #482's review rounds (PR #482; the crank-444 round briefs r1–r3).
const LEAKS: &[&str] = &[
    // Known names, in every spelling.
    "Tacx Neo 55624",
    "tacx neo 55624",
    "TacxNeo",
    "Tacx%20Neo%2055624",
    "q=Tacx+Neo+55624",
    "Tacx_Neo_55624",
    "Tacx-Neo-55624",
    "Tacx\u{a0}Neo 55624",
    "Tacx\u{200b}Neo 55624",
    "\u{ff34}\u{ff41}\u{ff43}\u{ff58} Neo 55624",
    "HRM-Pro:9871",
    // Known serials: decimal, hex, zero-padded, byte orders, byte arrays, grouped digits.
    "2655624",
    "0x288588",
    "0x00288588",
    "00288588",
    "288588",
    "88 85 28 00",
    "00 28 85 88",
    "[88, 85, 28, 00]",
    "0x88 0x85 0x28 0x00",
    "[136, 133, 40, 0]",
    "[0, 40, 133, 136]",
    "2655 624",
    // Serial-label variants with unknown serials.
    "serial no. 7788991",
    "serial is 7788992",
    "Serial Number String: 7788993",
    "S/N: 7788994",
    "SN 7788995",
    "serial: Some(\"7788996\")",
    "serial (7788997)",
    "serial, 7788998",
    "serial -> 7788999",
    "serial: [7788981]",
    "serial `7788982`",
    "serial of 7788983",
    "serial7788984",
    "serial read: 7788985",
    "serial \u{2014} 7788986",
    "serial; 7788987",
    "serial \u{2192} 7788988",
    // BLE addresses: separators, mixed, Cisco, bare, bluez, URL-encoded, byte arrays.
    "C4:7D:4F:12:34:56",
    "c4-7d-4f-12-34-57",
    "C4_7D_4F_12_34_58",
    "C4:7D-4F:12-34:59",
    "c47d.4f12.345a",
    "C47D4F12345B",
    "device-C4-7D-4F-12-34-5C",
    "node_C4_7D_4F_12_34_5D",
    "/org/bluez/hci0/dev_C4_7D_4F_12_34_5E",
    "C4%3A7D%3A4F%3A12%3A34%3A5F",
    "[0xC4, 0x7D, 0x4F, 0x12, 0x34, 0x60]",
    "[196, 125, 79, 18, 52, 97]",
    "C4:7D:4F:12:3\u{200b}4:62",
    // Platform ids: known and unknown, dashed, dashless, next to `_`, lower case.
    "3F2504E0-4F89-41D3-9A0C-0305E82C3301",
    "3f2504e04f8941d39a0c0305e82c3301",
    "id_9D3B6A1E-2C4F-4B8A-9E7D-1F2A3B4C5D6E_x",
    "7A1B2C3D-5E6F-4A0B-9C1D-2E3F4A5B6C7D",
    // Unknown devices' names.
    "KICKR CORE 1A2B",
    "Wahoo TICKR 77AB",
    // The machine.
    "riders-mbp.local",
    "/home/rider/captures/trainer.fecap",
];

/// Each spelling's distinctive part: its runs of four or more letters and digits (`Tacx`,
/// `55624`, `7788991`, `C47D4F12345B`, `riders`), lower-cased, and the spelling itself.
fn needles(leak: &str) -> Vec<String> {
    let mut out: Vec<String> = leak
        .split(|c: char| !c.is_alphanumeric())
        .filter(|run| run.chars().count() >= 4)
        .map(str::to_lowercase)
        .collect();
    out.push(leak.to_lowercase());
    out
}

/// One OTLP line with `value` in the body, in the free-text fields, in `event`, in every field a
/// record can keep (as a string, and as an integer where it is one), and in `severityText`.
fn salted(value: &str) -> String {
    let attr =
        |key: &str, value: serde_json::Value| serde_json::json!({"key": key, "value": value});
    let string = serde_json::json!({"stringValue": value});
    let record = |body: &str, attributes: Vec<serde_json::Value>| {
        serde_json::json!({"timeUnixNano": "1793728308690000000", "severityText": value,
            "severityNumber": 13, "body": {"stringValue": body}, "attributes": attributes})
    };
    let mut records = vec![
        record(value, vec![]),
        record(
            "trainer reconnect failed",
            ["error", "e", "err", "message", "msg", "event"]
                .into_iter()
                .map(|key| attr(key, string.clone()))
                .collect(),
        ),
    ];
    for (key, _) in allowlist::ALLOWED {
        records.push(record("ride released", vec![attr(key, string.clone())]));
    }
    // An identifier can be a number; a number in a typed field is kept as one (ADR-0003 §5).
    for (key, _) in allowlist::IDENTIFIERS {
        records.push(record("ride released", vec![attr(key, string.clone())]));
        if let Ok(n) = value.parse::<i64>() {
            let int = serde_json::json!({"intValue": n.to_string()});
            records.push(record("ride released", vec![attr(key, int)]));
        }
    }
    serde_json::json!({"resourceLogs": [{"scopeLogs": [{"scope": {"name": "crank_ble"},
        "logRecords": records}]}]})
    .to_string()
}

/// The identities salted into the fixture.
const FIXTURE_IDENTITIES: &[&str] = &[
    "Tacx",
    "55624",
    "\u{ff34}\u{ff41}\u{ff43}\u{ff58}",
    "2655624",
    "288588",
    "0x28",
    "136, 133",
    "2655 624",
    "HRM-Pro",
    "9871",
    "KICKR",
    "1A2B",
    "Wahoo",
    "TICKR",
    "77AB",
    "7788991",
    "7788992",
    "7788993",
    "7788994",
    "7788995",
    "C4:7D:4F:12",
    "c4-7d-4f",
    "C4_7D_4F",
    "c4%3A7d",
    "3F2504E0",
    "7A1B2C3D",
    "9D3B6A1E",
    "F0:1A:2B",
    "f01a.2b3c",
    "F01A2B3C4D5E",
    "riders-mbp",
    "/home/rider",
    "\u{200b}",
    "\u{a0}",
];

fn assert_private(preview: &str, leaks: &[&str], baseline: &str) {
    let (preview, baseline) = (preview.to_lowercase(), baseline.to_lowercase());
    for leak in leaks {
        for needle in needles(leak) {
            if !baseline.contains(&needle) {
                assert!(
                    !preview.contains(&needle),
                    "{needle:?} of {leak:?} leaked:\n{preview}"
                );
            }
        }
    }
}

#[test]
fn no_leak_spelling_survives_in_any_field() {
    // The same records with a harmless value, so a needle that is a word of every report
    // (`serial` as a key, `trainer` in an event name) is not counted as a leak.
    let (word, number) = (preview(&salted("x")), preview(&salted("1")));
    for leak in LEAKS {
        let preview = preview(&salted(leak));
        let baseline = if leak.parse::<i64>().is_ok() {
            &number
        } else {
            &word
        };
        assert_private(&preview, &[leak], baseline);
        assert_eq!(
            preview.lines().count(),
            baseline.lines().count(),
            "every salted record is reported, as structure only:\n{preview}"
        );
    }
}

#[test]
fn the_fixture_log_keeps_what_triage_needs_and_no_identity() {
    let preview = preview(LOG);
    let lower = preview.to_lowercase();
    for identity in FIXTURE_IDENTITIES {
        let identity = identity.to_lowercase();
        assert!(
            !lower.contains(&identity),
            "{identity:?} leaked:\n{preview}"
        );
    }
    for kept in [
        "crank_ble::gatt",
        "crank_core::links",
        " WARN ",
        " DEBUG ",
        "event=\"trainer reconnect failed\"",
        "event=\"link phase\"",
        "event=\"unknown FE-C page\"",
        "event=\"rejected FE-C notification\"",
        "event=log",
        "page=25",
        "page=81",
        "watts=212",
        "rpm=88.5",
        "attempt=2",
        "from=resolve",
        "to=gatt",
        "via=scan",
        "device=trainer",
        "kind=Checksum",
        "force=false",
        "ride_id=ride-18f3a2b4c5d6e7f8",
        "characteristic=6e40fec2-b5a3-f393-e0a9-e50e24dcca9e",
        "name=<trainer>",
        "serial=<trainer>",
        "local_name=<device-name>",
        "name=<device-name>",
        "sn=<trainer>",
        "sn=<serial>",
        "serial=<serial>",
        "serial_number=<trainer>",
        "address=<<host>>",
        "mac=<ble-addr>",
        "peripheral_id=<strap>",
        "db_id=<ble-id>",
        "path=<path>",
        "dir=<path>",
    ] {
        assert!(preview.contains(kept), "{kept:?} is missing:\n{preview}");
    }
    for dropped in ["raw=", "error=", "message=", " e="] {
        assert!(
            !preview.contains(dropped),
            "{dropped:?} is kept:\n{preview}"
        );
    }
}
