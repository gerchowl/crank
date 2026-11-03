//! A capture the desktop app writes under `CRANK_CAPTURE_DIR` (#411) is read by `analyse`.
//! It is made with `crank_ble::CaptureDir`, the writer the app uses.
use crate::frames::{Checksum, analysis, build, parse_capture};
use crank_ble::CaptureDir;
use crank_devices::MonoTime;
use crank_fec::ControlPage;

#[test]
fn analyse_reads_a_desktop_capture() {
    let dir = tempfile::tempdir().unwrap();
    let (path, capture) = CaptureDir::new(dir.path().join("cap"))
        .create(MonoTime(1_000))
        .unwrap();
    // Two page-71 requests, the first answered after 40 ms, the second never.
    let request = build(
        &ControlPage::Request { page: 71, times: 1 },
        0x4f,
        Checksum::XorInclSync,
    )
    .unwrap();
    let mut reply = [
        0xa4, 9, 0x4f, 5, 0x47, 0x30, 10, 0, 0xff, 0xff, 0xff, 0xff, 0,
    ];
    reply[12] = reply[..12].iter().fold(0, |x, v| x ^ v);
    capture.record(MonoTime(1_000), false, &request).unwrap();
    capture.record(MonoTime(41_000), true, &reply).unwrap();
    capture.record(MonoTime(251_000), false, &request).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let records = parse_capture(&text);
    assert_eq!(records.len(), 3, "{text}");
    let report = analysis(&records);
    assert_eq!(report["requests_page71"].as_u64(), Some(2), "{report}");
    assert_eq!(report["answered"].as_u64(), Some(1), "{report}");
}
