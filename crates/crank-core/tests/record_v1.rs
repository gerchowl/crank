//! A version-1 ride record (every ride recorded before #717) decodes unchanged with the
//! version-2 decoder (ADR-0009 §2.3.2). The fixture is built outside crank-store, in the
//! chunk format, by `crates/crank-store/tests/fixtures/make_record_v1.py`.
use crank_engine::{SecondRow, decode_record};
use crank_store::Store;

/// The fixture's seconds, as `make_record_v1.py` writes them.
fn expected(s: u16) -> SecondRow {
    let paused = (50..55).contains(&s);
    let value = |v: f32, gap: bool| (!paused && !gap).then_some(v);
    let s32 = f32::from(s);
    SecondRow {
        power: value(100.0 + s32, s.is_multiple_of(13)),
        cadence: value(90.0, false),
        speed: value(8.5, false),
        hr: value(120.0 + f32::from(s % 10), (40..45).contains(&s)),
        workout_s: value(if s < 50 { s32 + 1.0 } else { s32 - 4.0 }, false),
        block: (!paused).then_some(u32::from(s >= 30)),
        control: None,
    }
}

#[test]
fn a_version_1_record_from_the_fixture_decodes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("../../crank-store/tests/fixtures/record-v1.db"),
    )
    .unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    let ride = store.ride("v1-ride").unwrap().expect("the fixture's ride");
    assert_eq!(ride.record_version, 1);
    assert_eq!(ride.end_cause, None, "older rides have no end cause");
    let record = store.record("v1-ride").unwrap().expect("its record");
    assert_eq!(record.record_version, 1);
    let rows = decode_record(record.record_version, &record.metrics).unwrap();
    assert_eq!(rows, (0..75).map(expected).collect::<Vec<_>>());
    // a version-1 record read as version 2 is refused, never filled in
    assert!(decode_record(2, &record.metrics).is_err());
    assert!(store.record("no-such-ride").unwrap().is_none());
}
