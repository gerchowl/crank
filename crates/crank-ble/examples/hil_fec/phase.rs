//! `phase`: page-16 suppression against the phase of the page-71 request (#610).
//!
//! The NEO (fw 0.7.4) stops broadcasting FE-C page 16 (speed) for 22–25 s when crank's free-running
//! 1 Hz page-70 request for page 71 keeps one relation to the page-16 slot (`latency.fecap`,
//! issue #610). This check locks the request to a chosen offset after each received page 16 and
//! steps that offset across the page-16 period ([`offsets_ms`]), dwelling [`DWELL_US`] on each, so
//! the per-offset page-16 presence gives the band of offsets that is safe for the production
//! scheduler (ADR-0002 §4.4, amendment "phase-locked page-71 request", proposed).
//!
//! The pure parts, [`PhaseLock`] (the scheduler) and [`analyse`] (outages, offset bins, per-step
//! verdicts, the safe band), run offline on any capture; the tests run them on the committed
//! `docs/hil/captures/latency.fecap`. Only [`run`] needs the trainer.
use crate::frames::{Note, Record, frame_line, note_line, page_of};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The page-71 request period the lock falls back to while no page 16 arrives: crank's
/// `T_poll` (ADR-0002 §4.4).
pub const PERIOD_US: u64 = 1_000_000;
/// The least spacing between two requests: half a period, so a page 16 that arrives earlier than
/// the last one cannot pull a second request into the same period.
pub const MIN_GAP_US: u64 = PERIOD_US / 2;
/// Time spent on each offset. In `latency.fecap` the two outages began 20.8–23.8 s after the
/// request took the relation that preceded them (the plateau at offset ≈ 0.2 period from
/// 84.8 s to the outage at 108.6 s; from 216.8–219.8 s to the outage at 240.6 s), and the
/// page-16 slot otherwise moved about once every 12–24 s. 40 s is 1.7 × the longest onset, so an
/// offset that triggers an outage shows it within its own dwell.
pub const DWELL_US: u64 = 40_000_000;
/// The start of each dwell left out of its statistics: a page-16 gap begun under the previous
/// offset belongs to that offset.
pub const SETTLE_US: u64 = 3_000_000;
/// A page-16 gap longer than this inside a step makes the offset unsafe: more than two missing
/// page-16 periods. (Speed is held for 6 s, `SPEED_HOLD_TICKS`; an offset that only thins page
/// 16 out is still not one to build on.)
pub const SAFE_GAP_US: u64 = 2_500_000;
/// A page-16 gap at least this long is an outage (the speed hold, 6 s, no longer covers it).
pub const OUTAGE_US: u64 = 6_000_000;
/// A request counts as followed by page 16 when one arrives within this time after it.
pub const FOLLOW_US: u64 = 1_500_000;
/// Width of an offset bin of the free-running analysis.
pub const BIN_US: u64 = 50_000;
/// Fewest requests in a bin for a verdict.
pub const MIN_BIN_N: usize = 5;
/// Narrowest safe band the sweep must find to PASS: wide enough to place the production offset
/// with at least 75 ms to either edge (BLE delivery is quantised to the ~30 ms connection
/// interval, so the arrival of page 16 itself jitters by that much).
pub const MIN_SAFE_WIDTH_MS: u16 = 150;
/// The tag of the step notes: `# phase <us> <offset ms, u16 LE>`.
pub const STEP_TAG: &str = "phase";

/// The swept offsets after page 16, in ms, ascending: every 50 ms over the period, plus 25 ms
/// steps between 150 and 450 ms, around the edges of the band `latency.fecap` showed
/// (outages began at an offset of 192–193 ms and ended at 393–424 ms).
#[must_use]
pub fn offsets_ms() -> Vec<u16> {
    let mut v: Vec<u16> = (0..1000).step_by(50).collect();
    v.extend((175..450).step_by(50));
    v.sort_unstable();
    v
}

/// Total sweep time, µs.
#[must_use]
pub fn sweep_us() -> u64 {
    DWELL_US * u64::try_from(offsets_ms().len()).unwrap_or(u64::MAX)
}

/// The request scheduler: one page-71 request `offset` after each page 16, and while page 16 is
/// absent one per [`PERIOD_US`], still `offset` after the last page-16 slot seen.
#[derive(Clone, Debug)]
pub struct PhaseLock {
    offset_us: u64,
    last16: Option<u64>,
    last_sent: Option<u64>,
}

impl PhaseLock {
    /// A lock at `offset_us` after page 16, carrying the page 16 and the request already seen.
    #[must_use]
    pub fn new(offset_us: u64, last16: Option<u64>, last_sent: Option<u64>) -> Self {
        Self {
            offset_us,
            last16,
            last_sent,
        }
    }
    /// A page 16 arrived at `at_us`.
    pub fn page16(&mut self, at_us: u64) {
        if self.last16.is_none_or(|l| at_us > l) {
            self.last16 = Some(at_us);
        }
    }
    /// A request was written at `at_us`.
    pub fn sent(&mut self, at_us: u64) {
        self.last_sent = Some(at_us);
    }
    /// The last page 16 seen.
    #[must_use]
    pub fn last16(&self) -> Option<u64> {
        self.last16
    }
    /// The last request written.
    #[must_use]
    pub fn last_sent(&self) -> Option<u64> {
        self.last_sent
    }
    /// When the next request is due: the first `last16 + offset + n·PERIOD_US` (n ≥ 0) at least
    /// [`MIN_GAP_US`] after the last request; none before the first page 16.
    #[must_use]
    pub fn due(&self) -> Option<u64> {
        let base = self.last16? + self.offset_us;
        let floor = self.last_sent.map_or(0, |s| s + MIN_GAP_US);
        if base >= floor {
            return Some(base);
        }
        let n = (floor - base).div_ceil(PERIOD_US);
        Some(base + n * PERIOD_US)
    }
}

/// A capture's frames (in timestamp order, ties in file order) and its step notes.
#[must_use]
pub fn parse(text: &str) -> (Vec<Record>, Vec<Note>) {
    let mut records = Vec::new();
    let mut notes = Vec::new();
    for line in text.lines() {
        if let Some(r) = frame_line(line) {
            records.push(r);
        } else if let Some(n) = note_line(line, records.len()) {
            notes.push(n);
        }
    }
    records.sort_by_key(|r| r.at_us);
    (records, notes)
}

/// Arrival times of page 16, ascending.
#[must_use]
pub fn page16_times(records: &[Record]) -> Vec<u64> {
    records
        .iter()
        .filter(|r| r.rx && page_of(&r.bytes) == Some(16))
        .map(|r| r.at_us)
        .collect()
}

/// Write times of the page-70 requests for page 71, ascending.
#[must_use]
pub fn request_times(records: &[Record]) -> Vec<u64> {
    records
        .iter()
        .filter(|r| !r.rx && page_of(&r.bytes) == Some(70) && r.bytes.get(10) == Some(&71))
        .map(|r| r.at_us)
        .collect()
}

/// The page-16 period: the median page-16 gap between 800 and 1200 ms; `None` without one.
#[must_use]
pub fn page16_period(t16: &[u64]) -> Option<u64> {
    let gaps: Vec<u64> = t16
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|g| (800_000..=1_200_000).contains(g))
        .collect();
    crate::frames::percentile(&gaps, 50.0)
}

/// One request against the page 16 before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Write time, µs.
    pub at_us: u64,
    /// Time since the last page 16, modulo the page-16 period: the request's offset after the
    /// page-16 slot, extrapolated across missing page 16s.
    pub offset_us: u64,
    /// A page 16 arrived within [`FOLLOW_US`] after it.
    pub followed: bool,
}

/// Every request after the first page 16, with its offset and whether page 16 followed.
#[must_use]
pub fn requests(t16: &[u64], treq: &[u64], period: u64) -> Vec<Request> {
    treq.iter()
        .filter_map(|&at| {
            let i = t16.partition_point(|&t| t <= at);
            let last = t16.get(i.checked_sub(1)?)?;
            Some(Request {
                at_us: at,
                offset_us: (at - last) % period,
                followed: t16.get(i).is_some_and(|&n| n - at <= FOLLOW_US),
            })
        })
        .collect()
}

/// One page-16 outage: a gap of at least [`OUTAGE_US`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outage {
    /// The last page 16 before it, µs.
    pub start_us: u64,
    /// Its length, µs.
    pub len_us: u64,
    /// The offset of the first request inside it, µs.
    pub first_offset: Option<u64>,
    /// The offset of the last request inside it, µs.
    pub last_offset: Option<u64>,
}

/// The page-16 outages, with the request offsets at their start and end.
#[must_use]
pub fn outages(t16: &[u64], reqs: &[Request]) -> Vec<Outage> {
    t16.windows(2)
        .filter(|w| w[1] - w[0] >= OUTAGE_US)
        .map(|w| {
            let inside: Vec<&Request> = reqs
                .iter()
                .filter(|r| r.at_us > w[0] && r.at_us < w[1])
                .collect();
            Outage {
                start_us: w[0],
                len_us: w[1] - w[0],
                first_offset: inside.first().map(|r| r.offset_us),
                last_offset: inside.last().map(|r| r.offset_us),
            }
        })
        .collect()
}

/// A bin's or step's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Page 16 kept flowing.
    Safe,
    /// Page 16 was suppressed.
    Unsafe,
    /// Neither (a bin with mixed outcomes).
    Mixed,
    /// Too few requests.
    Unmeasured,
}

impl Verdict {
    fn name(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Unsafe => "unsafe",
            Self::Mixed => "mixed",
            Self::Unmeasured => "unmeasured",
        }
    }
}

/// One [`BIN_US`] bin of request offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bin {
    /// Lower edge, ms.
    pub from_ms: u64,
    /// Requests in it.
    pub n: usize,
    /// Of those, followed by page 16.
    pub followed: usize,
    /// Safe: ≥ 90 % followed; unsafe: < 50 %; both with at least [`MIN_BIN_N`] requests.
    pub verdict: Verdict,
}

/// The offset bins of free-running requests (a capture with no step notes).
#[must_use]
pub fn bins(reqs: &[Request]) -> Vec<Bin> {
    let mut by: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
    for r in reqs {
        let e = by.entry(r.offset_us / BIN_US).or_default();
        e.0 += 1;
        e.1 += usize::from(r.followed);
    }
    by.into_iter()
        .map(|(k, (n, followed))| Bin {
            from_ms: k * BIN_US / 1000,
            n,
            followed,
            verdict: if n < MIN_BIN_N {
                Verdict::Unmeasured
            } else if followed * 10 >= n * 9 {
                Verdict::Safe
            } else if followed * 2 < n {
                Verdict::Unsafe
            } else {
                Verdict::Mixed
            },
        })
        .collect()
}

/// One swept offset, from its step note to the next (or the end of the capture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Offset after page 16, ms.
    pub offset_ms: u16,
    /// Step start, µs.
    pub from_us: u64,
    /// Step end, µs.
    pub to_us: u64,
    /// Requests written after the settle time.
    pub requests: usize,
    /// Page-71 replies received after the settle time.
    pub replies: usize,
    /// Page 16s received after the settle time.
    pub page16: usize,
    /// Longest page-16 gap that ends after the settle time (counting the step's edges as gap
    /// ends), µs.
    pub max_gap_us: u64,
    /// Safe: no gap over [`SAFE_GAP_US`] and at least one page 16; else unsafe.
    pub verdict: Verdict,
}

/// The sweep steps of a capture, from its `phase` notes.
#[must_use]
pub fn steps(records: &[Record], notes: &[Note]) -> Vec<Step> {
    let marks: Vec<(u16, u64)> = notes
        .iter()
        .filter(|n| n.tag == STEP_TAG)
        .filter_map(|n| match n.bytes[..] {
            [lo, hi] => Some((u16::from_le_bytes([lo, hi]), n.at_us)),
            _ => None,
        })
        .collect();
    let end = records.last().map_or(0, |r| r.at_us);
    let t16 = page16_times(records);
    marks
        .iter()
        .enumerate()
        .map(|(i, &(offset_ms, from_us))| {
            let to_us = marks.get(i + 1).map_or(end, |m| m.1).max(from_us);
            let counted = from_us + SETTLE_US;
            let within = |t: u64| t >= counted && t < to_us;
            let count = |rx: bool, page: u8| {
                records
                    .iter()
                    .filter(|r| r.rx == rx && within(r.at_us) && page_of(&r.bytes) == Some(page))
                    .count()
            };
            // gaps between page 16s, with the counted window's edges as the outer ends
            let mut edges: Vec<u64> = vec![counted.min(to_us)];
            edges.extend(t16.iter().copied().filter(|&t| within(t)));
            edges.push(to_us);
            let max_gap_us = edges.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
            let page16 = edges.len() - 2;
            Step {
                offset_ms,
                from_us,
                to_us,
                requests: count(false, 70),
                replies: count(true, 71),
                page16,
                max_gap_us,
                verdict: if page16 > 0 && max_gap_us <= SAFE_GAP_US {
                    Verdict::Safe
                } else {
                    Verdict::Unsafe
                },
            }
        })
        .collect()
}

/// The contiguous runs of safe steps, as `(first, last)` offsets in ms, widest first. A run spans
/// adjacent swept offsets; a step that ended early (shorter than its dwell) breaks it.
#[must_use]
pub fn safe_bands(steps: &[Step]) -> Vec<(u16, u16)> {
    let mut runs: Vec<(u16, u16)> = Vec::new();
    let mut open: Option<(u16, u16)> = None;
    for s in steps {
        let full = s.to_us - s.from_us + 1_000_000 >= DWELL_US;
        open = match (open, s.verdict == Verdict::Safe && full) {
            (Some((a, _)), true) => Some((a, s.offset_ms)),
            (None, true) => Some((s.offset_ms, s.offset_ms)),
            (Some(r), false) => {
                runs.push(r);
                None
            }
            (None, false) => None,
        };
    }
    runs.extend(open);
    runs.sort_by_key(|&(a, b)| std::cmp::Reverse(b - a));
    runs
}

/// The widest safe band and whether it passes ([`MIN_SAFE_WIDTH_MS`]), as the operator's line.
#[must_use]
pub fn verdict_line(bands: &[(u16, u16)]) -> (bool, String) {
    match bands.first() {
        Some(&(a, b)) if b - a >= MIN_SAFE_WIDTH_MS => (
            true,
            format!(
                "PASS: safe band {a}–{b} ms after page 16 (centre {} ms); all safe bands: {bands:?}",
                a + (b - a) / 2
            ),
        ),
        Some(&(a, b)) => (
            false,
            format!(
                "FAIL: widest safe band {a}–{b} ms is under {MIN_SAFE_WIDTH_MS} ms; all safe bands: {bands:?}"
            ),
        ),
        None => (false, "FAIL: no offset kept page 16 flowing".into()),
    }
}

/// Milliseconds of `us` (a capture is far shorter than the `u32` range of µs, 71 min, beyond
/// which this saturates).
fn ms(us: u64) -> f64 {
    f64::from(u32::try_from(us).unwrap_or(u32::MAX)) / 1000.0
}

/// The analysis of a capture: page-16 period, outages, free-running offset bins and, for a sweep
/// capture, the steps, the safe bands and the verdict.
#[must_use]
pub fn analyse(records: &[Record], notes: &[Note]) -> Value {
    let t16 = page16_times(records);
    let treq = request_times(records);
    let period = page16_period(&t16);
    let reqs = period.map_or_else(Vec::new, |p| requests(&t16, &treq, p));
    let outage_rows: Vec<Value> = outages(&t16, &reqs)
        .iter()
        .map(|o| {
            json!({
                "from_s": ms(o.start_us) / 1000.0,
                "len_s": ms(o.len_us) / 1000.0,
                "first_request_offset_ms": o.first_offset.map(ms),
                "last_request_offset_ms": o.last_offset.map(ms),
            })
        })
        .collect();
    let bin_rows: Vec<Value> = bins(&reqs)
        .iter()
        .map(|b| {
            json!({"from_ms": b.from_ms, "n": b.n, "followed": b.followed, "verdict": b.verdict.name()})
        })
        .collect();
    let steps = steps(records, notes);
    let step_rows: Vec<Value> = steps
        .iter()
        .map(|s| {
            json!({
                "offset_ms": s.offset_ms,
                "from_s": ms(s.from_us) / 1000.0,
                "dwell_s": ms(s.to_us - s.from_us) / 1000.0,
                "requests": s.requests,
                "replies": s.replies,
                "page16": s.page16,
                "max_page16_gap_ms": ms(s.max_gap_us),
                "verdict": s.verdict.name(),
            })
        })
        .collect();
    let bands = safe_bands(&steps);
    let (pass, line) = verdict_line(&bands);
    json!({
        "page16_period_ms": period.map(ms),
        "page16_frames": t16.len(),
        "requests": treq.len(),
        "outages": outage_rows,
        "free_running_bins": bin_rows,
        "steps": step_rows,
        "safe_bands_ms": bands,
        "pass": !steps.is_empty() && pass,
        "verdict": if steps.is_empty() { "no sweep steps in this capture".to_string() } else { line },
    })
}

/// The sweep on a live session: every offset of [`offsets_ms`] for [`DWELL_US`], then the
/// analysis of what was captured. Prints the verdict line.
pub async fn run(s: &crate::session::Session) -> Value {
    use std::time::Duration;
    use tokio::time::sleep;
    let mut seen = s.mark();
    let mut lock = PhaseLock::new(0, None, None);
    let mut write_errors = 0_usize;
    println!(
        "phase: {} offsets, {} s each, about {} min; leave the trainer alone",
        offsets_ms().len(),
        DWELL_US / 1_000_000,
        sweep_us().div_ceil(60_000_000)
    );
    for offset_ms in offsets_ms() {
        lock = PhaseLock::new(u64::from(offset_ms) * 1000, lock.last16(), lock.last_sent());
        let start = s
            .capture
            .record_aux_now(&*s.clock, STEP_TAG, &offset_ms.to_le_bytes())
            .map_or_else(|_| s.elapsed_us(), |at| s.capture_us(at));
        println!(
            "phase: offset {offset_ms} ms for {} s",
            DWELL_US / 1_000_000
        );
        while s.elapsed_us() < start + DWELL_US {
            let fresh = s.since(seen);
            seen += fresh.len();
            for rx in fresh {
                if page_of(&rx.bytes) == Some(16) {
                    lock.page16(s.capture_us(rx.at));
                }
            }
            if lock.due().is_some_and(|d| s.elapsed_us() >= d) {
                let (at, err) = s
                    .send(
                        crate::checks::request(71, 1, crate::checks::ACK, crate::checks::XOR),
                        true,
                    )
                    .await;
                write_errors += usize::from(err.is_some());
                lock.sent(s.capture_us(at));
            }
            sleep(Duration::from_millis(5)).await;
        }
    }
    let text = std::fs::read_to_string(&s.capture_path).unwrap_or_default();
    let (records, notes) = parse(&text);
    let mut summary = analyse(&records, &notes);
    if let Some(line) = summary["verdict"].as_str() {
        println!("{line}");
    }
    summary["check"] = json!("phase");
    summary["what_was_sent"] = json!(format!(
        "page-70 requests for page 71 locked to an offset after each page 16 (fallback {PERIOD_US} us), offsets {:?} ms, {} s each",
        offsets_ms(),
        DWELL_US / 1_000_000
    ));
    summary["write_errors"] = json!(write_errors);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATENCY: &str = include_str!("../../../../docs/hil/captures/latency.fecap");

    fn latency() -> (Vec<Record>, Vec<Note>) {
        parse(LATENCY)
    }

    #[test]
    fn the_sweep_covers_the_period_finer_near_the_band_edges() {
        let o = offsets_ms();
        assert_eq!(o.len(), 26);
        assert_eq!((o[0], o[o.len() - 1]), (0, 950));
        assert!(o.windows(2).all(|w| w[0] < w[1]));
        assert!([175, 225, 275, 325, 375, 425].iter().all(|x| o.contains(x)));
        assert_eq!(sweep_us(), 26 * 40_000_000);
    }

    #[test]
    fn the_lock_waits_for_page_16_then_follows_it() {
        let mut l = PhaseLock::new(300_000, None, None);
        assert_eq!(l.due(), None);
        l.page16(1_000_000);
        assert_eq!(l.due(), Some(1_300_000));
        l.sent(1_300_000);
        // no new page 16: one period later, same offset after the old slot
        assert_eq!(l.due(), Some(2_300_000));
        // page 16 moved 250 ms later: offset after the new one
        l.page16(2_240_000);
        assert_eq!(l.due(), Some(2_540_000));
        // an older page 16 never moves the lock back
        l.page16(2_000_000);
        assert_eq!(l.last16(), Some(2_240_000));
        // the next page 16 only 460 ms after it: due 300 ms later is still half a period past
        // the last request
        l.sent(2_540_000);
        l.page16(2_800_000);
        assert_eq!(l.due(), Some(3_100_000));
        // a page 16 that would put a request 460 ms after the last one: never two requests
        // within half a period, so a period later, at the same offset after that page 16
        l.page16(2_700_000);
        assert_eq!(l.due(), Some(3_100_000), "an older page 16 is ignored");
        let mut m = PhaseLock::new(300_000, Some(2_240_000), Some(2_540_000));
        m.page16(2_700_000);
        assert_eq!(m.due(), Some(4_000_000));
        assert!(m.due().unwrap() >= m.last_sent().unwrap() + MIN_GAP_US);
    }

    /// Driven by the page 16s of `latency.fecap` (as if their arrivals had not depended on the
    /// requests), the lock requests exactly `offset` after each page 16, at least half a period
    /// apart, and inside the two outages once per period at the same offset after the last
    /// page-16 slot.
    #[test]
    fn the_lock_on_the_latency_capture_keeps_its_offset_through_the_outages() {
        let (records, _) = latency();
        let t16 = page16_times(&records);
        let offset = 350_000;
        let mut lock = PhaseLock::new(offset, None, None);
        let mut sent = Vec::new();
        let mut i = 0;
        let end = *t16.last().unwrap();
        let mut now = 0;
        while now < end {
            while t16.get(i).is_some_and(|&t| t <= now) {
                lock.page16(t16[i]);
                i += 1;
            }
            if lock.due().is_some_and(|d| now >= d) {
                lock.sent(now);
                sent.push((now, lock.last16().unwrap()));
            }
            now += 1_000;
        }
        assert!(sent.windows(2).all(|w| w[1].0 - w[0].0 >= MIN_GAP_US));
        for &(at, last16) in &sent {
            let since = at - last16;
            assert!(
                since % PERIOD_US >= offset && since % PERIOD_US < offset + 1_000,
                "request at {at} is {since} us after page 16"
            );
        }
        let inside = |from: u64, len: u64| {
            sent.iter()
                .filter(|(at, _)| *at > from && *at < from + len)
                .count()
        };
        // 25.2 s and 22.2 s outages: one request per period
        assert_eq!(inside(108_622_130, 25_232_387), 25);
        assert_eq!(inside(240_624_932, 22_233_290), 22);
    }

    #[test]
    fn the_latency_capture_has_the_two_outages_in_the_band() {
        let (records, notes) = latency();
        let t16 = page16_times(&records);
        let p = page16_period(&t16).unwrap();
        assert!((985_000..=995_000).contains(&p), "{p}");
        let reqs = requests(&t16, &request_times(&records), p);
        assert_eq!(reqs.len(), 344);
        let o = outages(&t16, &reqs);
        assert_eq!(o.len(), 2, "{o:?}");
        assert_eq!((o[0].start_us, o[0].len_us), (108_622_130, 25_232_387));
        assert_eq!((o[1].start_us, o[1].len_us), (240_624_932, 22_233_290));
        for x in &o {
            let first = x.first_offset.unwrap();
            let last = x.last_offset.unwrap();
            assert!((185_000..=200_000).contains(&first), "{x:?}");
            assert!((390_000..=430_000).contains(&last), "{x:?}");
        }
        assert!(steps(&records, &notes).is_empty());
    }

    /// The free-running bins: the outage band (250–400 ms) and the reply-on-page-16 band
    /// (900–950 ms, page 16 thinned to one per ~2 s) are unsafe; the plateaus the request sat on
    /// with page 16 flowing are safe.
    #[test]
    fn the_latency_capture_bins_find_the_unsafe_offsets() {
        let (records, _) = latency();
        let t16 = page16_times(&records);
        let reqs = requests(&t16, &request_times(&records), page16_period(&t16).unwrap());
        let b = bins(&reqs);
        let unsafe_ms: Vec<u64> = b
            .iter()
            .filter(|b| b.verdict == Verdict::Unsafe)
            .map(|b| b.from_ms)
            .collect();
        assert_eq!(unsafe_ms, vec![250, 300, 350, 900]);
        let safe_ms: Vec<u64> = b
            .iter()
            .filter(|b| b.verdict == Verdict::Safe)
            .map(|b| b.from_ms)
            .collect();
        // 950 ms: 60 of 67 followed, just under the 90 % of a safe bin
        assert_eq!(safe_ms, vec![650, 700]);
        let v = analyse(&records, &[]);
        assert_eq!(v["pass"], json!(false));
        assert_eq!(v["outages"].as_array().map(Vec::len), Some(2));
    }

    fn rx16(at: u64) -> String {
        format!("{at} rx A4 09 4E 05 10 19 00 00 00 00 FF 24 00\n")
    }
    fn tx70(at: u64) -> String {
        format!("{at} tx A4 09 4F 05 46 FF FF FF FF 01 47 01 00\n")
    }
    fn step(at: u64, offset_ms: u16) -> String {
        let [lo, hi] = offset_ms.to_le_bytes();
        format!("# phase {at} {lo:02X} {hi:02X}\n")
    }

    /// A synthetic sweep: page 16 every second except a 10 s outage in the 300 ms step and a
    /// 3 s gap in the 350 ms step; the safe band is the run of full, gap-free steps.
    #[test]
    fn steps_give_verdicts_and_the_widest_safe_band() {
        let mut text = String::new();
        let offsets = [200_u16, 250, 300, 350, 400, 450];
        for (i, &o) in offsets.iter().enumerate() {
            let from = u64::try_from(i).unwrap() * DWELL_US;
            text += &step(from, o);
            for k in 0..40 {
                let t = from + k * 1_000_000 + 10_000;
                let gap = match o {
                    300 => (10..20).contains(&k),
                    350 => (20..23).contains(&k),
                    _ => false,
                };
                if !gap {
                    text += &rx16(t);
                }
                text += &tx70(t + u64::from(o) * 1000);
            }
        }
        text += &rx16(u64::try_from(offsets.len()).unwrap() * DWELL_US);
        let (records, notes) = parse(&text);
        let s = steps(&records, &notes);
        let v: Vec<(u16, Verdict)> = s.iter().map(|s| (s.offset_ms, s.verdict)).collect();
        assert_eq!(
            v,
            vec![
                (200, Verdict::Safe),
                (250, Verdict::Safe),
                (300, Verdict::Unsafe),
                (350, Verdict::Unsafe),
                (400, Verdict::Safe),
                (450, Verdict::Safe),
            ]
        );
        assert_eq!(s[2].max_gap_us, 11_000_000);
        assert_eq!(s[0].requests, 37); // the first 3 s settle
        assert_eq!(safe_bands(&s), vec![(200, 250), (400, 450)]);
        let (pass, line) = verdict_line(&safe_bands(&s));
        assert!(!pass, "{line}");
        assert!(verdict_line(&[(400, 600)]).0);
        let a = analyse(&records, &notes);
        assert_eq!(a["pass"], json!(false));
        assert_eq!(a["steps"].as_array().map(Vec::len), Some(6));
    }
}
