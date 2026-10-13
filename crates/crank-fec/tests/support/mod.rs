//! Shared exact fecap fixture reader; used by codec and driver capture tests.
use crank_fec::{AntFrame, FrameError, decode};

pub(crate) type Record = (u64, bool, Result<AntFrame, FrameError>);

pub(crate) fn replay(capture: &str) -> Vec<Record> {
    let mut records = Vec::new();
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
    records.sort_by_key(|(at, _, _)| *at);
    records
}
