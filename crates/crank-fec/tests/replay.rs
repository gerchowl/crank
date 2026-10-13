//! Timestamped raw ANT captures replay through the real decoder, without BLE.
use std::fmt::Write;

use crank_fec::{AntFrame, FrameError, MsgType, Page, decode};

type Record = (u64, bool, Result<AntFrame, FrameError>);

fn replay(capture: &str) -> Vec<Record> {
    let mut records = Vec::new();
    let mut previous = 0;
    for line in capture.lines() {
        let line = line.split('#').next().expect("line").trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let at: u64 = fields
            .next()
            .expect("timestamp")
            .parse()
            .expect("microseconds");
        assert!(at >= previous, "capture must be ordered");
        previous = at;
        let received = match fields.next().expect("direction") {
            "rx" => true,
            "tx" => false,
            other => panic!("invalid direction: {other}"),
        };
        let bytes: Vec<_> = fields
            .map(|s| u8::from_str_radix(s, 16).expect("hex byte"))
            .collect();
        // Keep errors as records: a bad notification cannot consume its successor.
        records.push((at, received, decode(&bytes)));
    }
    records
}

#[test]
fn replay_all_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = 0;
    for entry in std::fs::read_dir(dir).expect("fixtures") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "fecap") {
            continue;
        }
        let records = replay(&std::fs::read_to_string(&path).expect("capture"));
        assert!(!records.is_empty(), "empty capture: {}", path.display());
        // Hand-computed typed snapshots accompany every capture, including errors.
        let mut actual = String::new();
        for record in &records {
            writeln!(actual, "{record:?}").expect("snapshot string");
        }
        let expected = std::fs::read_to_string(path.with_extension("expected")).expect("snapshot");
        assert_eq!(actual, expected, "{}", path.display());
        files += 1;
    }
    assert!(files >= 2, "worked example and synthetic stream required");
}

#[test]
fn errors_and_unknown_pages_preserve_stream_order() {
    let records = replay(
        "0 rx 00\n1 rx A4 09 4E 05 11 01 02 03 04 05 06 07 F7\n2 rx A4 09 4E 05 13 01 02 03 04 05 06 07 F5\n",
    );
    assert_eq!(records[0], (0, true, Err(FrameError::Truncated)));
    for (index, page) in [(1, 17), (2, 19)] {
        assert_eq!(
            records[index],
            (
                u64::try_from(index).expect("index"),
                true,
                Ok(AntFrame {
                    msg_type: MsgType::Broadcast,
                    channel: 5,
                    page: Page::Unknown {
                        page,
                        raw: [page, 1, 2, 3, 4, 5, 6, 7]
                    },
                })
            )
        );
    }
}
