//! Timestamped raw ANT captures replay through the real decoder, without BLE.
use std::fmt::Write;

use crank_fec::{AntFrame, FrameError, MsgType, Page};

mod support;
use support::replay;

/// The crate directory at run time. `env!` would bake in the path of the build that compiled this
/// binary, which no longer exists when the Nix checks reuse it (#371); cargo and nextest set the variable.
fn manifest_dir() -> String {
    std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests")
}

#[test]
fn replay_all_fixtures() {
    let dir = std::path::PathBuf::from(manifest_dir()).join("tests/fixtures");
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
