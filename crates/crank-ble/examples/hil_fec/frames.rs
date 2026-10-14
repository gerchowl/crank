//! Pure frame construction, redaction and capture analysis for the HIL harness.
use crank_fec::{ControlPage, encode};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Checksum rule applied to an outgoing frame (HIL check 1, ADR-0002 §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checksum {
    /// XOR over all bytes including sync: the codec's rule and the Tacx worked example.
    XorInclSync,
    /// Arithmetic sum excluding sync: the Tacx prose and pycycling.
    SumExclSync,
    /// XOR excluding sync: the remaining reading.
    XorExclSync,
    /// Deliberately wrong (`XorInclSync` ^ 0xFF): shows whether the NEO validates at all.
    Wrong,
}

impl Checksum {
    /// Every variant in the order check 1 cycles them.
    pub const ALL: [Self; 4] = [
        Self::XorInclSync,
        Self::SumExclSync,
        Self::XorExclSync,
        Self::Wrong,
    ];
    /// Stable name for summaries.
    pub fn name(self) -> &'static str {
        match self {
            Self::XorInclSync => "xor_incl_sync",
            Self::SumExclSync => "sum_excl_sync",
            Self::XorExclSync => "xor_excl_sync",
            Self::Wrong => "wrong",
        }
    }
    fn compute(self, frame: &[u8; 13]) -> u8 {
        let body = &frame[..12];
        let xor = |b: &[u8]| b.iter().fold(0, |x, v| x ^ v);
        match self {
            Self::XorInclSync => xor(body),
            Self::SumExclSync => body[1..].iter().fold(0u8, |s, v| s.wrapping_add(*v)),
            Self::XorExclSync => xor(&body[1..]),
            Self::Wrong => xor(body) ^ 0xff,
        }
    }
}

/// Build a frame whose 8 page bytes come from the real codec and whose envelope
/// (message type, checksum rule) is chosen by the check. `None` if the codec rejects `page`.
pub fn build(page: &ControlPage, msg_type: u8, checksum: Checksum) -> Option<[u8; 13]> {
    let mut frame = encode(page).ok()?;
    frame[2] = msg_type;
    frame[12] = checksum.compute(&frame);
    Some(frame)
}

/// Replace page byte `index` (0-7) of an already built frame and recompute its checksum.
/// For values the codec refuses to produce (e.g. a wheel offset nibble of 10-14, #146).
pub fn with_page_byte(
    mut frame: [u8; 13],
    index: usize,
    value: u8,
    checksum: Checksum,
) -> [u8; 13] {
    frame[4 + index] = value;
    frame[12] = checksum.compute(&frame);
    frame
}

/// Page number of a complete 13-byte ANT frame.
pub fn page_of(bytes: &[u8]) -> Option<u8> {
    (bytes.len() == 13 && bytes[0] == 0xa4).then(|| bytes[4])
}

/// Decoded page-71 reply: (last page, sequence, status, echo bytes 4-7).
pub fn command_status(bytes: &[u8]) -> Option<(u8, u8, u8, [u8; 4])> {
    (page_of(bytes)? == 0x47).then(|| {
        (
            bytes[5],
            bytes[6],
            bytes[7],
            [bytes[8], bytes[9], bytes[10], bytes[11]],
        )
    })
}

/// Hex rendering, space separated, as in `.fecap`.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replace the page-81 serial number with the 0xFFFFFFFF "unspecified" value and
/// fix the XOR checksum, so a committed capture carries no device serial.
pub fn redact_frame(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    if page_of(bytes) == Some(0x51) {
        out[8..12].fill(0xff);
        out[12] = out[..12].iter().fold(0, |x, v| x ^ v);
    }
    out
}

/// Redact every record of a `.fecap` text; comments and blank lines are kept.
pub fn redact_capture(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let record = fields.next().zip(fields.next()).filter(|(t, d)| {
            !t.starts_with('#') && t.parse::<u64>().is_ok() && matches!(*d, "rx" | "tx")
        });
        let redacted = record.and_then(|(t, d)| {
            let bytes: Option<Vec<u8>> = fields.map(|h| u8::from_str_radix(h, 16).ok()).collect();
            bytes.map(|b| format!("{t} {d} {}", hex(&redact_frame(&b))))
        });
        out.push_str(redacted.as_deref().unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Mask digit runs of four or more (model/serial numbers embedded in names).
pub fn redact_name(name: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        out.push_str(&if run.len() >= 4 {
            "*".repeat(run.len())
        } else {
            run.clone()
        });
        run.clear();
    };
    for c in name.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// One parsed `.fecap` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Microseconds from capture start.
    pub at_us: u64,
    /// `true` for rx (trainer to host).
    pub rx: bool,
    /// Complete payload.
    pub bytes: Vec<u8>,
}

/// Parse `.fecap` text, skipping comments, blanks and malformed lines.
pub fn parse_capture(text: &str) -> Vec<Record> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let at_us = f.next()?.parse().ok()?;
            let rx = match f.next()? {
                "rx" => true,
                "tx" => false,
                _ => return None,
            };
            let bytes = f
                .map(|h| u8::from_str_radix(h, 16).ok())
                .collect::<Option<_>>()?;
            Some(Record { at_us, rx, bytes })
        })
        .collect()
}

/// Per-page arrival statistics over rx records (idle notification pattern).
pub fn page_rates(records: &[Record]) -> Value {
    let mut by_page: BTreeMap<u8, Vec<u64>> = BTreeMap::new();
    for r in records.iter().filter(|r| r.rx) {
        if let Some(p) = page_of(&r.bytes) {
            by_page.entry(p).or_default().push(r.at_us);
        }
    }
    let mut out = serde_json::Map::new();
    for (page, times) in by_page {
        let gaps: Vec<u64> = times
            .windows(2)
            .map(|w| w[1].saturating_sub(w[0]))
            .collect();
        let span = times.last().zip(times.first()).map_or(0, |(l, f)| l - f);
        #[allow(clippy::cast_precision_loss)]
        let hz = (span > 0).then(|| (times.len() - 1) as f64 / (span as f64 / 1e6));
        out.insert(
            format!("{page}"),
            json!({
                "count": times.len(),
                "rate_hz": hz,
                "gap_us_min": gaps.iter().min(),
                "gap_us_max": gaps.iter().max(),
            }),
        );
    }
    Value::Object(out)
}

/// Request → reply latencies in microseconds: each tx page-70 request for `page` to
/// the first later rx carrying `reply_page`.
pub fn request_latencies(records: &[Record], page: u8, reply_page: u8) -> Vec<u64> {
    let mut out = Vec::new();
    for (i, r) in records.iter().enumerate() {
        let is_request = !r.rx && page_of(&r.bytes) == Some(70) && r.bytes[10] == page;
        if !is_request {
            continue;
        }
        let reply = records[i + 1..]
            .iter()
            .filter(|n| n.rx)
            .find(|n| page_of(&n.bytes) == Some(reply_page) && n.at_us >= r.at_us);
        if let Some(n) = reply {
            out.push(n.at_us - r.at_us);
        }
    }
    out
}

/// Nearest-rank percentile of an unsorted sample; `None` when empty.
pub fn percentile(sample: &[u64], p: f64) -> Option<u64> {
    let mut s = sample.to_vec();
    s.sort_unstable();
    if s.is_empty() {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let rank = ((p / 100.0) * s.len() as f64).ceil().max(1.0) as usize;
    s.get(rank - 1).copied()
}

/// min / p50 / p99 / max / n of a latency sample.
pub fn distribution(sample: &[u64]) -> Value {
    json!({
        "n": sample.len(),
        "min_us": sample.iter().min(),
        "p50_us": percentile(sample, 50.0),
        "p99_us": percentile(sample, 99.0),
        "max_us": sample.iter().max(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_variants_differ_and_xor_matches_worked_example() {
        // Tacx worked example from ADR-0002 §4.1: checksum 0x30.
        let mut f = [0xa4, 9, 0x4e, 5, 0x10, 0x19, 4, 0, 0, 0, 0xff, 0x24, 0];
        assert_eq!(Checksum::XorInclSync.compute(&f), 0x30);
        assert_eq!(Checksum::SumExclSync.compute(&f), 0xac);
        assert_eq!(Checksum::XorExclSync.compute(&f), 0x94);
        assert_eq!(Checksum::Wrong.compute(&f), 0xcf);
        f[12] = 0x30;
        assert_eq!(f.iter().fold(0, |x, v| x ^ v), 0);
    }

    #[test]
    fn build_sets_type_and_checksum_over_codec_page_bytes() {
        let page = ControlPage::Request { page: 54, times: 1 };
        let f = build(&page, 0x4e, Checksum::SumExclSync).expect("encodes");
        assert_eq!(f[..4], [0xa4, 9, 0x4e, 5]);
        assert_eq!(f[4..12], [70, 0xff, 0xff, 0xff, 0xff, 0x81, 54, 1]);
        assert_eq!(f[12], f[1..12].iter().fold(0u8, |s, v| s.wrapping_add(*v)));
        assert!(
            build(
                &ControlPage::TargetPower { watts: 99_999.0 },
                0x4f,
                Checksum::Wrong
            )
            .is_none()
        );
    }

    #[test]
    fn with_page_byte_keeps_checksum_valid() {
        let base = build(
            &ControlPage::UserConfig {
                rider_kg: 75.0,
                bike_kg: 10.0,
                wheel_m: 0.708,
            },
            0x4f,
            Checksum::XorInclSync,
        )
        .expect("encodes");
        let f = with_page_byte(base, 4, 0x8a, Checksum::XorInclSync);
        assert_eq!(f[8], 0x8a);
        assert_eq!(f.iter().fold(0, |x, v| x ^ v), 0);
    }

    #[test]
    fn page55_diameter_layout_matches_issue_44_bytes() {
        let f = build(
            &ControlPage::UserConfig {
                rider_kg: 75.0,
                bike_kg: 10.0,
                wheel_m: 0.708,
            },
            0x4f,
            Checksum::XorInclSync,
        )
        .expect("encodes");
        assert_eq!(f[4..12], [0x37, 0x4c, 0x1d, 0xff, 0x88, 0x0c, 0x46, 0x00]);
    }

    #[test]
    fn redaction_masks_serial_and_keeps_valid_checksum() {
        let mut f = [
            0xa4, 9, 0x4e, 5, 0x51, 0xff, 0x01, 0x07, 0x78, 0x56, 0x34, 0x12, 0,
        ];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        let r = redact_frame(&f);
        assert_eq!(r[8..12], [0xff; 4]);
        assert_eq!(r.iter().fold(0, |x, v| x ^ v), 0);
        let other = [
            0xa4,
            9,
            0x4e,
            5,
            0x19,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0xa4 ^ 9 ^ 0x4e ^ 5 ^ 0x19,
        ];
        assert_eq!(redact_frame(&other), other);
        assert_eq!(redact_name("Tacx Neo 55624"), "Tacx Neo *****");
        assert_eq!(redact_name("HRM 3 pro"), "HRM 3 pro");
    }

    #[test]
    fn capture_redaction_preserves_comments_and_unparseable_lines() {
        let text = "# header\n\n10 rx A4 09 4E 05 51 FF 01 07 78 56 34 12 00\nbogus line\n";
        let out = redact_capture(text);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines[0], "# header");
        assert_eq!(lines[1], "");
        assert!(lines[2].starts_with("10 rx A4 09 4E 05 51 FF 01 07 FF FF FF FF "));
        assert_eq!(lines[3], "bogus line");
    }

    #[test]
    fn latency_and_rate_analysis() {
        let req = hex(&build(
            &ControlPage::Request { page: 71, times: 1 },
            0x4f,
            Checksum::XorInclSync,
        )
        .expect("e"));
        let text = format!(
            "0 rx A4 09 4E 05 19 00 00 00 00 00 00 00 00\n100 tx {req}\n350 rx A4 09 4E 05 47 31 02 00 00 00 00 00 00\n1000000 rx A4 09 4E 05 19 00 00 00 00 00 00 00 00\n"
        );
        let recs = parse_capture(&text);
        assert_eq!(request_latencies(&recs, 71, 71), vec![250]);
        let rates = page_rates(&recs);
        assert_eq!(rates["25"]["count"], 2);
        assert_eq!(rates["25"]["gap_us_max"], 1_000_000);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 50.0), Some(3));
        assert_eq!(percentile(&[], 99.0), None);
        let status = command_status(&recs[2].bytes).expect("page 71");
        assert_eq!(status.0, 0x31);
    }
}
