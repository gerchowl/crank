//! The privacy pin of the bug report's log tail (ADR-0003 §2.3).
//!
//! `fixtures/report_privacy/logs.jsonl` is vigil 0.2.0 output of crank's own logging pipeline
//! (`LogPlan::install` in a capture directory, events at crank's targets), hand-salted afterwards:
//! `host.name`, a second line written by hand (a zero time, a `severityNumber`-only level, a
//! colliding `message` attribute, and the `path`/`dir` record, whose target the capture filter
//! keeps only at WARN), and a partial last line. No hardware session log is committed, so the
//! events stand in for one. Every device identity below is salted into `message`, `error` and the
//! allowlisted `path` and `dir`, in the spellings of ADR-0003 §2.3 (no-break and zero-width
//! spaces, URL encoding, a dashless id, page 81's byte order); none may reach the preview, while
//! what a triager needs must.
use crank_core::{DeviceRole, KnownDevice};
use crank_desktop::report::{self, scrub::Scrub};
use squelch::{Destination, Redactor, Report, Transport};

const LOG: &str = include_str!("fixtures/report_privacy/logs.jsonl");

/// What the core would report: a sighting of another NEO first (a prefix of the paired one's
/// name, so the table's longest-first order matters), the paired trainer and strap, a strap
/// seen with a BLE-address id, and one named like a placeholder word.
fn known() -> Vec<KnownDevice> {
    vec![
        KnownDevice {
            role: DeviceRole::Trainer,
            platform_id: "5B6C7D8E-1A2B-4C3D-8E9F-0A1B2C3D4E5F".into(),
            name: Some("Tacx Neo".into()),
            serial: None,
        },
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
        KnownDevice {
            role: DeviceRole::HeartRate,
            platform_id: "F0:1A:2B:3C:4D:5E".into(),
            name: Some("TICKR X 2C1D".into()),
            // Spans `4F:12` of an unknown address: the address rule must run before the table.
            serial: Some("4F12".into()),
        },
        // An odd advertised name that is also a word of a placeholder (`<ble-addr>`).
        KnownDevice {
            role: DeviceRole::HeartRate,
            platform_id: "6A7B8C9D-0E1F-4A2B-9C3D-4E5F6A7B8C9D".into(),
            name: Some("Addr".into()),
            serial: None,
        },
    ]
}

/// What the reporter would see, built as the preview command will build it (#446), with crank's
/// allowlist plus `also`.
fn preview(scrub: &Scrub, also: &[&str]) -> String {
    let redactor = report::allowlisted(Redactor::with_identity(
        Some("/home/rider".into()),
        Some("rider".into()),
    ))
    .also_allow(also.iter().copied());
    let records = report::diagnostics(LOG, scrub, &redactor);
    Report::to(Destination::parse("gerchowl/crank").expect("destination"))
        .via(Transport::Browser)
        .diagnostics(records)
        .preview()
        .expect("preview")
}

/// Salted identities. Each scrub rule has at least one that squelch does not hide by itself, so
/// removing any rule fails this test (with [`EVERY_FIELD`] for the field rules).
const LEAKS: [&str; 39] = [
    // Known devices, in every spelling.
    "Tacx Neo 55624",
    "tacx neo 55624",
    "TacxNeo",
    "Tacx%20Neo",
    "55624",
    "2655624",
    "88 85 28 00",
    "0x288588",
    "0x00288588",
    "00 28 85 88",
    "288588",
    "2655 624",
    "0x28 0x85",
    "133, 40",
    "40, 133",
    "HRM-Pro:9871",
    "9871",
    "3F2504E0",
    "3f2504e0",
    "7A1B2C3D",
    // Unknown devices: the name field, the serial fields and free text.
    "KICKR CORE 1A2B",
    "Wahoo TICKR 77AB",
    "7788991",
    "7788992",
    "7788993",
    "7788994",
    "7788995",
    // A strap known by its BLE address, in Cisco and bare form.
    "f01a.2b3c",
    "F01A2B3C4D5E",
    // BLE addresses and platform ids nobody remembered.
    "C4:7D:4F:12:34",
    "c4-7d-4f-12-34",
    "C4_7D_4F_12_34",
    "12:34:5A",
    "9D3B6A1E",
    "1F2A3B4C5D6E",
    // The machine.
    "riders-mbp",
    "/home/rider",
    "rider/",
    "\u{200b}",
];

/// Every field the fixture carries, as if each were allowlisted: the scrub must hold on its own.
const EVERY_FIELD: [&str; 5] = ["name", "local_name", "serial", "sn", "raw"];

fn assert_private(preview: &str) {
    for leak in LEAKS {
        assert!(!preview.contains(leak), "{leak:?} leaked:\n{preview}");
    }
    assert!(
        !preview.contains('\u{200b}') && !preview.contains('\u{a0}'),
        "{preview}"
    );
}

#[test]
fn no_device_identity_survives_into_the_preview() {
    let preview = preview(&Scrub::new(&known()).expect("table"), &[]);
    assert_private(&preview);
    for kept in [
        "crank_ble",
        "trainer reconnect failed",
        "page=25",
        "6e40fec2-b5a3-f393-e0a9-e50e24dcca9e",
        "<trainer>",
        "<trainer-id>",
        "<trainer-serial>",
        "<strap>",
        "<ble-addr>",
        "<ble-id>",
        "serial no. <serial>",
        "path=",
        "capture opened",
        "addresses resolved",
        "strap <strap> seen",
    ] {
        assert!(preview.contains(kept), "{kept:?} is missing:\n{preview}");
    }
}

#[test]
fn no_device_identity_survives_even_if_every_field_were_allowlisted() {
    let preview = preview(&Scrub::new(&known()).expect("table"), &EVERY_FIELD);
    assert_private(&preview);
    assert_eq!(
        preview.matches(" name=<trainer>").count(),
        2,
        "both NEO name fields, the one with a no-break space too:\n{preview}"
    );
    for kept in [
        "name=<device-name>",
        "local_name=<device-name>",
        "sn=<serial>",
        "serial=<serial>",
    ] {
        assert!(preview.contains(kept), "{kept:?} is missing:\n{preview}");
    }
}
