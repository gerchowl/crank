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

/// What happened to one page-70 request in a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestOutcome {
    /// Write time, µs from capture start.
    pub at_us: u64,
    /// Requested page.
    pub page: u8,
    /// A distinct page-71 reply carries this request's sequence number.
    pub answered: bool,
    /// Arrival minus write time of that reply.
    pub latency_us: Option<u64>,
    /// Gap to the previous counted write (control page or request), µs.
    pub prev_gap_us: Option<u64>,
    /// The previous counted write was a control page (48–51) rather than a request.
    pub prev_was_control: bool,
}

/// Page-71 reply accounting for one capture (see [`reply_accounting`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReplyAccounting {
    /// Every page-70 request, in write order.
    pub outcomes: Vec<RequestOutcome>,
    /// Page-71 frames received.
    pub frames: usize,
    /// Frames that answered a request (one per answered request).
    pub distinct: usize,
    /// Frames that repeat an earlier reply (same sequence, no new request between, or
    /// the same bytes again later).
    pub copies: usize,
    /// Copies sent as message type `0x4E`.
    pub copies_4e: usize,
    /// Copies arriving within 1 ms of the reply they repeat.
    pub copies_within_1ms: usize,
}

/// Replies older than this cannot answer a request (longest observed: 1.1 s).
const ANSWER_WINDOW_US: u64 = 1_500_000;

/// Attribute page-71 replies to page-70 requests by sequence number.
///
/// The NEO's sequence byte counts the control pages (48–51) and page-70 requests it
/// receives (mod 256, not page 55), so a reply with sequence `s` answers the counted
/// write numbered `s - base`. `base` is re-derived from the most recent unanswered
/// request whenever the arithmetic does not land on a request (the counter holds 0xFF
/// for one or two writes at its wrap). A `0x4E` frame whose sequence and bytes equal an
/// earlier answering `0x4F` reply is a copy of it and is attributed to that original, by
/// sequence and arrival order, however many writes came between; copies are counted
/// separately and never answer a request. Limits: a `0x4E` frame whose `0x4F` original
/// was lost is indistinguishable from an original and counts as the answer; a `0x4F`
/// frame repeating a sequence (the counter holds 0xFF at its wrap) is a new reply.
pub fn reply_accounting(records: &[Record]) -> ReplyAccounting {
    let mut sorted: Vec<&Record> = records.iter().collect();
    sorted.sort_by_key(|r| r.at_us); // stable: ties keep file order
    let mut out = ReplyAccounting::default();
    let mut counted = 0_u64;
    let mut prev: Option<(u64, bool)> = None;
    let mut requests: BTreeMap<u64, usize> = BTreeMap::new(); // counted index -> outcome index
    let mut answered_at: BTreeMap<u64, u64> = BTreeMap::new(); // counted index -> arrival
    let mut answered_when: BTreeMap<u64, u64> = BTreeMap::new(); // counted index -> counted when answered
    let mut base: Option<u64> = None;
    let mut answers: Vec<(u64, Vec<u8>)> = Vec::new(); // arrival, bytes of distinct replies
    for r in sorted {
        let Some(page) = page_of(&r.bytes) else {
            continue;
        };
        if !r.rx && matches!(page, 48..=51 | 70) {
            counted += 1;
            if page == 70 {
                requests.insert(counted, out.outcomes.len());
                out.outcomes.push(RequestOutcome {
                    at_us: r.at_us,
                    page: r.bytes[10],
                    answered: false,
                    latency_us: None,
                    prev_gap_us: prev.map(|(t, _)| r.at_us.saturating_sub(t)),
                    prev_was_control: prev.is_some_and(|(_, control)| control),
                });
            }
            prev = Some((r.at_us, page != 70));
        } else if r.rx && page == 71 {
            out.frames += 1;
            let seq = u64::from(r.bytes[6]);
            let is_copy = r.bytes[2] == 0x4e
                && answers
                    .iter()
                    .any(|(_, b)| b[2] == 0x4f && b[6..12] == r.bytes[6..12] && b[5] == r.bytes[5]);
            if is_copy {
                out.copies += 1;
                out.copies_4e += 1;
                let same = answers
                    .iter()
                    .rev()
                    .find(|(_, b)| b[2] == 0x4f && b[5..12] == r.bytes[5..12])
                    .map(|(t, _)| r.at_us.saturating_sub(*t));
                out.copies_within_1ms += usize::from(same.is_some_and(|d| d <= 1000));
                continue;
            }
            let live = |j: u64| {
                requests.get(&j).is_some_and(|o| {
                    !answered_at.contains_key(&j)
                        && r.at_us >= out.outcomes[*o].at_us
                        && r.at_us - out.outcomes[*o].at_us <= ANSWER_WINDOW_US
                })
            };
            let mut hit = None;
            if let Some(b) = base {
                let c = (seq + 256 - b % 256) % 256;
                // Candidate indices congruent to `c` (mod 256) within the last 256 writes.
                let j = counted - (counted + 256 - c % 256) % 256;
                if j > 0 && answered_at.contains_key(&j) && answered_when.get(&j) == Some(&counted)
                {
                    hit = Some(None); // repeat of a reply, no write since
                } else if j > 0 && live(j) {
                    hit = Some(Some(j));
                }
            }
            let target = match hit {
                Some(t) => t,
                None => requests
                    .keys()
                    .rev()
                    .copied()
                    .find(|j| live(*j))
                    .inspect(|j| {
                        base = Some((seq + 256 - j % 256) % 256);
                    }),
            };
            if let Some(j) = target {
                answered_at.insert(j, r.at_us);
                answered_when.insert(j, counted);
                answers.push((r.at_us, r.bytes.clone()));
                out.distinct += 1;
                if let Some(o) = requests.get(&j) {
                    let o = &mut out.outcomes[*o];
                    o.answered = true;
                    o.latency_us = Some(r.at_us - o.at_us);
                }
            } else {
                out.copies += 1;
                out.copies_4e += usize::from(r.bytes[2] == 0x4e);
                let same = answers
                    .iter()
                    .rev()
                    .find(|(_, b)| b[5..12] == r.bytes[5..12])
                    .map(|(t, _)| r.at_us.saturating_sub(*t));
                out.copies_within_1ms += usize::from(same.is_some_and(|d| d <= 1000));
            }
        }
    }
    out
}

/// Requests for page `page` grouped by what preceded them: (preceded by control?, nominal
/// gap in ms) → (requests, answered). Gaps are bucketed to the nearest of the harness's
/// nominal spacings when within 40 %, else 0.
pub fn loss_by_gap(outcomes: &[RequestOutcome], page: u8) -> Vec<(bool, u64, usize, usize)> {
    const NOMINAL: [u64; 6] = [100, 250, 500, 1000, 1500, 2000];
    let mut groups: BTreeMap<(bool, u64), (usize, usize)> = BTreeMap::new();
    for o in outcomes.iter().filter(|o| o.page == page) {
        let Some(gap) = o.prev_gap_us.map(|g| g / 1000) else {
            continue;
        };
        let near = NOMINAL
            .iter()
            .copied()
            .min_by_key(|n| n.abs_diff(gap))
            .filter(|n| n.abs_diff(gap) * 10 <= n * 4 + 150)
            .unwrap_or(0);
        let e = groups.entry((o.prev_was_control, near)).or_default();
        e.0 += 1;
        e.1 += usize::from(o.answered);
    }
    groups
        .into_iter()
        .map(|((c, g), (n, a))| (c, g, n, a))
        .collect()
}

/// One page-71 echo of a control value that had already been superseded by a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleEcho {
    /// Control page the echo is about.
    pub page: u8,
    /// Arrival, µs from capture start.
    pub at_us: u64,
    /// Arrival minus the first write of the value that superseded the echoed one.
    pub age_us: u64,
}

/// Echoes of an old value that arrive after a new value was written (ADR-0002 §4.4.1
/// rule R2 / constant `L`). Per control page it keeps the runs of distinct written
/// values; an echo that matches an earlier run and not the current one is stale, and
/// its age is measured from the first write of the run that followed the echoed one.
/// The desired change precedes that write by the writer delay (≤ 0.75 s, §4.4), which
/// callers add to the age. Echoes of values never written (power-on state) are skipped.
pub fn stale_echoes(records: &[Record]) -> (Vec<StaleEcho>, usize) {
    let mut sorted: Vec<&Record> = records.iter().collect();
    sorted.sort_by_key(|r| r.at_us);
    let mut runs: BTreeMap<u8, Vec<(u64, [u8; 4])>> = BTreeMap::new();
    let (mut stale, mut checked) = (Vec::new(), 0);
    for r in sorted {
        let Some(page) = page_of(&r.bytes) else {
            continue;
        };
        let data = [r.bytes[8], r.bytes[9], r.bytes[10], r.bytes[11]];
        if !r.rx && matches!(page, 48..=51) {
            let entry = runs.entry(page).or_default();
            if entry.last().is_none_or(|(_, v)| *v != data) {
                entry.push((r.at_us, data));
            }
        } else if r.rx && page == 71 {
            let Some(entry) = runs.get(&r.bytes[5]) else {
                continue;
            };
            let Some(last_match) = entry.iter().rposition(|(_, v)| *v == data) else {
                continue;
            };
            checked += 1;
            if let Some((started, _)) = entry.get(last_match + 1) {
                stale.push(StaleEcho {
                    page: r.bytes[5],
                    at_us: r.at_us,
                    age_us: r.at_us.saturating_sub(*started),
                });
            }
        }
    }
    (stale, checked)
}

/// Run lengths of consecutive unanswered requests for `page`, in write order, as
/// `(run length, number of runs)`.
pub fn lost_runs(outcomes: &[RequestOutcome], page: u8) -> Vec<(usize, usize)> {
    let mut runs: BTreeMap<usize, usize> = BTreeMap::new();
    let mut current = 0;
    for o in outcomes.iter().filter(|o| o.page == page) {
        if o.answered {
            if current > 0 {
                *runs.entry(current).or_default() += 1;
            }
            current = 0;
        } else {
            current += 1;
        }
    }
    if current > 0 {
        *runs.entry(current).or_default() += 1;
    }
    runs.into_iter().collect()
}

/// Whether the losses look independent: observed adjacent lost pairs against the
/// `(n - 1)·p²` expected for independent losses, with the Poisson probability of seeing
/// at most that many (`p` = lost / requests).
pub fn independence(requests: usize, lost: usize, runs: &[(usize, usize)]) -> Value {
    if requests < 2 {
        return Value::Null;
    }
    #[allow(clippy::cast_precision_loss)]
    let p = lost as f64 / requests as f64;
    #[allow(clippy::cast_precision_loss)]
    let expected = (requests - 1) as f64 * p * p;
    let observed: usize = runs.iter().map(|(len, n)| (len - 1) * n).sum();
    // Poisson CDF at `observed`.
    let mut term = (-expected).exp();
    let mut cdf = term;
    for i in 1..=observed {
        #[allow(clippy::cast_precision_loss)]
        let i = i as f64;
        term *= expected / i;
        cdf += term;
    }
    json!({
        "p_lost": p,
        "expected_adjacent_lost_pairs": expected,
        "observed_adjacent_lost_pairs": observed,
        "poisson_p_at_most_observed": cdf,
    })
}

/// Reproducible reply accounting and stale-echo summary of one capture.
pub fn analysis(records: &[Record]) -> Value {
    let acc = reply_accounting(records);
    let r71: Vec<&RequestOutcome> = acc.outcomes.iter().filter(|o| o.page == 71).collect();
    let answered = r71.iter().filter(|o| o.answered).count();
    let latencies: Vec<u64> = r71.iter().filter_map(|o| o.latency_us).collect();
    let (stale, checked) = stale_echoes(records);
    let ages: Vec<u64> = stale.iter().map(|s| s.age_us).collect();
    json!({
        "method": "replies matched to requests by the page-71 sequence byte (counts control pages 48-51 and page-70 requests, mod 256); repeats of an answered reply are copies; a request is lost when no reply carries its sequence",
        "requests_page71": r71.len(),
        "answered": answered,
        "lost": r71.len() - answered,
        "answered_over_500ms": latencies.iter().filter(|l| **l > 500_000).count(),
        "reply_latency_us": distribution(&latencies),
        "page71_frames": acc.frames,
        "distinct_replies": acc.distinct,
        "copies": acc.copies,
        "copies_msg_4e": acc.copies_4e,
        "copies_within_1ms": acc.copies_within_1ms,
        "lost_run_lengths": lost_runs(&acc.outcomes, 71).iter().map(|(len, n)| json!({"run": len, "count": n})).collect::<Vec<_>>(),
        "independence": independence(r71.len(), r71.len() - answered, &lost_runs(&acc.outcomes, 71)),
        "loss_by_preceding_gap": loss_by_gap(&acc.outcomes, 71).iter().map(|(control, gap, n, a)| json!({
            "preceded_by": if *control { "control_write" } else { "request" },
            "gap_ms_nominal": gap, "requests": n, "answered": a,
        })).collect::<Vec<_>>(),
        "stale_echoes": {
            "method": "echo of a control value after a different value of the same page was written; age from the first write of the superseding value (add the writer delay, at most 0.75 s, for age from the desired change)",
            "echoes_checked": checked,
            "stale": ages.len(),
            "max_age_us": ages.iter().max(),
            "ages_us": ages,
        },
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
        assert_eq!(
            redact_name("Example Trainer 12345"),
            "Example Trainer *****"
        );
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

    fn rec(at_us: u64, rx: bool, bytes: Vec<u8>) -> Record {
        Record { at_us, rx, bytes }
    }
    fn req71() -> Vec<u8> {
        build(
            &ControlPage::Request { page: 71, times: 1 },
            0x4f,
            Checksum::XorInclSync,
        )
        .expect("encodes")
        .to_vec()
    }
    fn ctl49(w: f32) -> Vec<u8> {
        build(
            &ControlPage::TargetPower { watts: w },
            0x4f,
            Checksum::XorInclSync,
        )
        .expect("encodes")
        .to_vec()
    }
    fn reply71(msg: u8, last: u8, seq: u8, echo: [u8; 4]) -> Vec<u8> {
        let mut f = vec![
            0xa4, 9, msg, 5, 0x47, last, seq, 0, echo[0], echo[1], echo[2], echo[3], 0,
        ];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        f
    }

    #[test]
    fn duplicate_replies_cannot_hide_lost_requests() {
        // Five requests 250 ms apart (sequence 10..14); request 12 and 14 are never answered,
        // while 10 and 13 each arrive twice (0x4F then a 0x4E copy). Counting frames gives 5
        // replies for 5 requests; counting sequences gives 3 answered, 2 lost.
        let mut r = Vec::new();
        for i in 0..5_u64 {
            r.push(rec(i * 250_000, false, req71()));
        }
        let echo = [0xff; 4];
        r.push(rec(40_000, true, reply71(0x4f, 0x30, 10, echo)));
        r.push(rec(40_300, true, reply71(0x4e, 0x30, 10, echo)));
        r.push(rec(290_000, true, reply71(0x4f, 0x30, 11, echo)));
        r.push(rec(790_000, true, reply71(0x4f, 0x30, 13, echo)));
        r.push(rec(790_200, true, reply71(0x4e, 0x30, 13, echo)));
        let acc = reply_accounting(&r);
        assert_eq!(acc.frames, 5);
        assert_eq!(acc.distinct, 3);
        assert_eq!(
            (acc.copies, acc.copies_4e, acc.copies_within_1ms),
            (2, 2, 2)
        );
        let answered: Vec<bool> = acc.outcomes.iter().map(|o| o.answered).collect();
        assert_eq!(answered, [true, true, false, true, false]);
        assert_eq!(acc.outcomes[1].prev_gap_us, Some(250_000));
        // The first request has no predecessor, so it is not in a gap group.
        assert_eq!(loss_by_gap(&acc.outcomes, 71), vec![(false, 250, 4, 2)]);
        let j = analysis(&r);
        assert_eq!(
            (j["requests_page71"].as_u64(), j["lost"].as_u64()),
            (Some(5), Some(2))
        );
    }

    #[test]
    fn sequence_wrap_and_held_counter_are_followed() {
        // 300 requests; the counter wraps at 256 and holds 0xFF for one extra write.
        let seqs = (250_u8..=255)
            .chain([255])
            .chain((0_u8..=255).cycle())
            .take(300);
        let mut r = Vec::new();
        for (i, seq) in (0_u64..).zip(seqs) {
            r.push(rec(i * 1_000_000, false, req71()));
            r.push(rec(
                i * 1_000_000 + 50_000,
                true,
                reply71(0x4f, 0x30, seq, [0xff; 4]),
            ));
        }
        let acc = reply_accounting(&r);
        assert_eq!((acc.distinct, acc.copies), (300, 0));
        assert!(acc.outcomes.iter().all(|o| o.answered));
    }

    #[test]
    fn stale_echo_is_an_old_value_after_the_new_write() {
        let old = [0xff, 0xff, 0x90, 0x01];
        let new = [0xff, 0xff, 0xb8, 0x01];
        let mut r = vec![rec(0, false, ctl49(100.0))];
        r.push(rec(100_000, true, reply71(0x4f, 0x31, 1, old)));
        r.push(rec(1_000_000, false, ctl49(110.0)));
        r.push(rec(1_022_000, true, reply71(0x4f, 0x31, 2, old))); // stale, 22 ms after the write
        r.push(rec(2_000_000, true, reply71(0x4f, 0x31, 3, new))); // current: not stale
        r.push(rec(
            2_100_000,
            true,
            reply71(0x4f, 0x30, 4, [0xff, 0xff, 0xff, 0x00]),
        )); // never written
        let (stale, checked) = stale_echoes(&r);
        assert_eq!(checked, 3);
        assert_eq!(
            stale,
            vec![StaleEcho {
                page: 0x31,
                at_us: 1_022_000,
                age_us: 22_000
            }]
        );
    }

    #[test]
    fn probe_late_copy_after_control_write() {
        // A lost, B answered, a control write, then the 0x4E copy of B. The copy must not be
        // taken for A's answer.
        let echo = [0xff; 4];
        let r = vec![
            rec(0, false, req71()),       // A: sequence 10
            rec(250_000, false, req71()), // B: sequence 11
            rec(290_000, true, reply71(0x4f, 0x30, 11, echo)),
            rec(300_000, false, ctl49(100.0)), // sequence 12
            rec(340_000, true, reply71(0x4e, 0x30, 11, echo)), // late copy of B
        ];
        let acc = reply_accounting(&r);
        let answered: Vec<bool> = acc.outcomes.iter().map(|o| o.answered).collect();
        assert_eq!(answered, [false, true]);
        assert_eq!((acc.distinct, acc.copies, acc.copies_4e), (1, 1, 1));
    }
}
