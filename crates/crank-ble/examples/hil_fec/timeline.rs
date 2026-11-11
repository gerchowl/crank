//! One validated, time-ordered capture timeline for the Part B analysis (#516, decided design).
//!
//! A live capture has one writer, which stamps and appends every line under one lock
//! (`Capture::record_now` and `record_aux_now`: the rx pump, the tx writes, the `# link`, cue and
//! answer stamps, the `# cps` lines), so the file order is the order in which things happened.
//! [`Timeline::parse`] reads a capture once and holds it to that: the stamps never go back in file
//! order, stamps that tie are ordered by the file, and no line is malformed. Every Part B check
//! reads the capture only through the typed events of a [`Timeline`] (samples, speeds, control
//! writes, requests, command statuses, Cycling Power measurements, stamps): the frames
//! themselves are private to it. Positions (a frame's index
//! among the frames, a note's [`Note::before`]) say what came before what, never µs arithmetic at
//! an edge. A completed run of checks 6, 9 and 12 is also held to its exact control program
//! ([`Step`], [`Timeline::program`]) before the check reads it (ADR-0007 §4).
use crate::{
    frames::{Cps, Note, Record, cps_line, frame_line, note_line},
    partb::{
        Control, Request, Sample, Speed, Status, Write, requests, samples, speeds, statuses, writes,
    },
};

/// A capture read once, in file order, its stamps validated as non-decreasing.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    /// The frames: read only here, to build the typed views below.
    records: Vec<Record>,
    notes: Vec<Note>,
    cps: Vec<Cps>,
    samples: Vec<Sample>,
    /// Every page 16 ([`Timeline::page16`]); the fresh speeds are those not marked repeat.
    page16: Vec<(Speed, bool)>,
    writes: Vec<Write>,
    requests: Vec<Request>,
    statuses: Vec<Status>,
}

impl Timeline {
    /// Read `.fecap` text. A stamp below the one before it is `Err("contradictory: records out of
    /// time order (line N)")`: no single writer wrote that file, so no check reads it. So is a
    /// line that is neither a record nor a comment (a record whose fields do not read is a
    /// record lost, never one to skip).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut tl = Self::default();
        let mut last = 0;
        for (i, line) in text.lines().enumerate() {
            let at = if let Some(r) = frame_line(line) {
                let at = r.at_us;
                tl.records.push(r);
                at
            } else if let Some(n) = note_line(line, tl.records.len()) {
                if let Some(c) = cps_line(line) {
                    tl.cps.push(c);
                }
                let at = n.at_us;
                tl.notes.push(n);
                at
            } else if is_comment(line) {
                continue;
            } else {
                return Err(format!("contradictory: line {} is no record", i + 1));
            };
            if at < last {
                return Err(format!(
                    "contradictory: records out of time order (line {})",
                    i + 1
                ));
            }
            last = at;
        }
        tl.samples = samples(&tl.records);
        let page16 = speeds(&tl.records);
        tl.writes = writes(&tl.records);
        tl.requests = requests(&tl.records);
        tl.statuses = statuses(&tl.records);
        for st in &mut tl.statuses {
            st.request = tl
                .requests
                .iter()
                .rfind(|q| q.page == 71 && q.pos < st.pos)
                .map(|q| q.pos);
        }
        // Page 16 with the elapsed time (page byte 2) of the page 16 before it repeats it.
        let mut last = None;
        for s in page16 {
            tl.page16.push((s, last == Some(s.elapsed)));
            last = Some(s.elapsed);
        }
        Ok(tl)
    }

    /// Every page-25 sample, repeats of the trainer's last update included (what the live guards
    /// see: the link is alive while page 25 comes).
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The page-25 updates: the samples without repeats (the update event count changed), what
    /// power and cadence are read from (ADR-0007 §2, §4).
    pub fn updates(&self) -> Vec<Sample> {
        self.samples.iter().filter(|s| !s.repeat).copied().collect()
    }

    /// The fresh page-16 speeds, `(at_us, m/s)`: no repeats (the elapsed time held).
    pub fn speeds(&self) -> Vec<(u64, f64)> {
        self.page16
            .iter()
            .filter(|(_, repeat)| !repeat)
            .filter_map(|(s, _)| s.mps.map(|v| (s.at_us, v)))
            .collect()
    }

    /// Every page-16 frame, each with whether it repeats the one before (its elapsed time, page
    /// byte 2, did not change).
    pub fn page16(&self) -> &[(Speed, bool)] {
        &self.page16
    }

    /// The control writes.
    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    /// The control writes as `(at_us, control)`.
    pub fn controls(&self) -> Vec<(u64, Control)> {
        self.writes.iter().map(|w| (w.at_us, w.control)).collect()
    }

    /// The page-70 requests.
    pub fn requests(&self) -> &[Request] {
        &self.requests
    }

    /// The page-71 command statuses.
    pub fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    /// The page-81 serials, `(at_us, serial)`.
    pub fn serials(&self) -> Vec<(u64, [u8; 4])> {
        crate::partb::serials(&self.records)
    }

    /// The Cycling Power measurements, as they came.
    pub fn cps(&self) -> &[Cps] {
        &self.cps
    }

    /// The tagged comment lines (stamps, answers, adverts, scan events), in order.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The stamp of the last frame, µs (0 without frames).
    pub fn end_us(&self) -> u64 {
        self.records.last().map_or(0, |r| r.at_us)
    }

    /// The index of the first frame stamped at or after `us`: where a window that starts a
    /// duration after an event begins.
    pub fn index_at(&self, us: u64) -> usize {
        self.records.partition_point(|r| r.at_us < us)
    }

    /// The run's closing release: the last control write, page 48 at 0 %, after another write.
    /// `None` in a capture cut off before it (every run writes it last, a failed one too).
    pub fn closing_release(&self) -> Option<Write> {
        let w = &self.writes;
        w.last()
            .filter(|l| l.control == Control::Resistance(0.0) && w.len() > 1)
            .copied()
    }

    /// Hold the control writes to `programs`, the check's exact control program, one per way the
    /// procedure can end (ADR-0007 §4): compared item by item, one must fit.
    /// `Err("contradictory: …")` names the write where the program that fits furthest breaks.
    pub fn program(&self, programs: &[&[Step]]) -> Result<(), String> {
        let ws = &self.writes;
        let fits: Vec<Result<(), usize>> = programs.iter().map(|p| follows(ws, p)).collect();
        if !fits.iter().any(Result::is_ok) {
            // Named at the write where the program that fits furthest breaks.
            let i = fits.iter().filter_map(|f| f.err()).max().unwrap_or(0);
            return Err(match ws.get(i) {
                Some(w) => format!(
                    "contradictory: control write {} ({:?} at {} s) is not the procedure's",
                    i + 1,
                    w.control,
                    crate::partb::round1(crate::partb::secs(w.at_us))
                ),
                None => "contradictory: a control write of the procedure is missing".into(),
            });
        }
        Ok(())
    }

    /// A timeline of separately built frames and notes (tests), merged by [`text_of`].
    #[cfg(test)]
    pub fn of_parts(records: &[Record], cps: &[Cps], notes: &[Note]) -> Self {
        Self::parse(&text_of(records, cps, notes)).expect("merged by stamp")
    }
}

/// The `.fecap` text of separately built frames, cps lines and notes (tests): merged by stamp,
/// a note before a frame of the same µs, each kind in its given order.
#[cfg(test)]
pub fn text_of(records: &[Record], cps: &[Cps], notes: &[Note]) -> String {
    use crate::frames::{CPS_TAG, hex};
    let mut lines: Vec<(u64, u8, String)> = Vec::new();
    for n in notes {
        lines.push((
            n.at_us,
            0,
            format!("# {} {} {}", n.tag, n.at_us, hex(&n.bytes)),
        ));
    }
    for c in cps {
        lines.push((
            c.at_us,
            0,
            format!("{CPS_TAG}{} {}", c.at_us, hex(&c.bytes)),
        ));
    }
    for r in records {
        let d = if r.rx { "rx" } else { "tx" };
        lines.push((r.at_us, 1, format!("{} {d} {}", r.at_us, hex(&r.bytes))));
    }
    lines.sort_by_key(|l| (l.0, l.1));
    lines.into_iter().map(|l| l.2 + "\n").collect()
}

/// One step of a check's control program: what its procedure in `rider.rs` writes, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Page 48 at 0 %.
    Release,
    /// `Rig::engage` to this target, W: ERG 0 W, then the soft start's
    /// [`crate::rider::SOFT_START_STEPS`] + 1 writes on a line from the 3 s power to the target.
    SoftStart(u16),
    /// ERG at this target, W.
    Erg(u16),
    /// Page 50 at its defaults (0.51 kg/m, no wind, drafting 1).
    Wind,
    /// Page 51 at this grade, %, Crr 0.004.
    Track(u16),
}

/// Whether the control writes `ws` are exactly `program`: `Err` with the index of the first write
/// that is not (`ws.len()` when one is missing).
fn follows(ws: &[Write], program: &[Step]) -> Result<(), usize> {
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-3 * b.abs().max(1.0);
    let at = |i: usize| ws.get(i).map(|w| w.control);
    let mut i = 0;
    for step in program {
        if let Step::SoftStart(target) = *step {
            if at(i) != Some(Control::Erg(0.0)) {
                return Err(i);
            }
            let steps = crate::rider::SOFT_START_STEPS;
            let Some(Control::Erg(from)) = at(i + 1) else {
                return Err(i + 1);
            };
            let to = f64::from(target);
            for k in 0..=steps {
                #[allow(clippy::cast_precision_loss)]
                let line = from + (to - from) * (k as f64 / steps as f64);
                let j = i + 1 + usize::try_from(k).unwrap_or(usize::MAX);
                // Each write is the line rounded to 1 W from the unrounded 3 s power.
                match at(j) {
                    Some(Control::Erg(w))
                        if (w - line).abs() <= 1.0 && (k < steps || near(w, to)) => {}
                    _ => return Err(j),
                }
            }
            i += 2 + usize::try_from(steps).unwrap_or(usize::MAX);
            continue;
        }
        let fits = match (*step, at(i)) {
            (Step::Release, Some(Control::Resistance(p))) => p == 0.0,
            (Step::Erg(t), Some(Control::Erg(w))) => near(w, f64::from(t)),
            (Step::Wind, Some(Control::Wind(cw, v, d))) => {
                near(cw, 0.51) && v == 0.0 && near(d, 1.0)
            }
            (Step::Track(g), Some(Control::Track(grade, crr))) => {
                near(grade, f64::from(g)) && near(crr, 0.004)
            }
            _ => false,
        };
        if !fits {
            return Err(i);
        }
        i += 1;
    }
    if i == ws.len() { Ok(()) } else { Err(i) }
}

/// A comment line: `#` and no stamp after the tag (the header, the outcome and finished lines, a
/// `redact` header), or a blank line.
fn is_comment(line: &str) -> bool {
    let mut f = line.split_whitespace();
    match f.next() {
        None => true,
        Some(h) if h.starts_with('#') => {
            // `# tag <number> …` is a note; `#tag …` or `# tag word …` a comment.
            h != "#" || f.nth(1).is_none_or(|n| n.parse::<u64>().is_err())
        }
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_order_is_time_order_and_ties_keep_the_file_order() {
        let ok = "# header\n10 tx A4\n# link 10 01\n10 rx A4\n# hil_fec procedure: completed\n";
        let tl = Timeline::parse(ok).expect("in order");
        assert_eq!(
            tl.writes().len(),
            1,
            "the tx record, no ANT frame, is a write"
        );
        assert_eq!(
            tl.notes()[0].before,
            1,
            "the stamp sits between the two frames"
        );
        assert_eq!(
            Timeline::parse("10 tx A4\n# link 11 01\n10 rx A4\n").err(),
            Some("contradictory: records out of time order (line 3)".into())
        );
        assert_eq!(
            Timeline::parse("10 tx A4\n9 rx A4\n").err(),
            Some("contradictory: records out of time order (line 2)".into())
        );
        assert_eq!(
            Timeline::parse("5 rx A4\n# cps 4 00\n").err(),
            Some("contradictory: records out of time order (line 2)".into())
        );
        assert_eq!(tl.index_at(10), 0);
        assert_eq!(tl.index_at(11), 2);
    }

    /// The soft start is ERG 0 W, then six writes on the line from the 3 s power to the target
    /// (each rounded to 1 W), the last one the target itself.
    #[test]
    fn the_soft_start_is_erg_0_then_six_steps_on_the_line_to_its_target() {
        let ws = |c: &[f64]| {
            c.iter()
                .enumerate()
                .map(|(pos, w)| Write {
                    data: [0; 4],
                    pos,
                    at_us: u64::try_from(pos).expect("small"),
                    control: Control::Erg(*w),
                })
                .collect::<Vec<_>>()
        };
        let program = [Step::SoftStart(100)];
        let ramp = |v: [f64; 6]| ws(&[&[0.0][..], &v[..]].concat());
        assert_eq!(
            follows(&ramp([0.0, 20.0, 40.0, 60.0, 80.0, 100.0]), &program),
            Ok(())
        );
        assert_eq!(
            follows(&ramp([30.0, 44.0, 58.0, 72.0, 86.0, 100.0]), &program),
            Ok(())
        );
        assert_eq!(
            follows(&ramp([0.0, 20.0, 40.0, 60.0, 81.0, 100.0]), &program),
            Ok(())
        );
        // Off the line, short of the target, without the ERG 0 W, one write too many.
        assert_eq!(
            follows(&ramp([0.0, 20.0, 60.0, 60.0, 80.0, 100.0]), &program),
            Err(3)
        );
        assert_eq!(
            follows(&ramp([0.0, 20.0, 40.0, 60.0, 80.0, 99.0]), &program),
            Err(6)
        );
        // (ERG 0 W missing: the ramp is read from 20 W, and 40 W is off that line.)
        assert_eq!(
            follows(&ws(&[0.0, 20.0, 40.0, 60.0, 80.0, 100.0]), &program),
            Err(2)
        );
        assert_eq!(
            follows(
                &ws(&[0.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0, 100.0]),
                &program
            ),
            Err(7)
        );
    }

    /// #516 round 4 N3: a line that is neither a record nor a comment is a lost record, so the
    /// capture is contradictory; comments (header, outcome, finished, redact header) are not.
    #[test]
    fn a_malformed_line_is_contradictory() {
        let ok = "# elapsed_us direction complete_frame_hex\n# Real Tacx NEO capture (HIL #44); page-81 serial redacted to FF FF FF FF.\n10 rx A4 09\n# check10 20 01\n\n# hil_fec procedure: completed\n# hil_fec finished: 1700000000\n";
        assert!(Timeline::parse(ok).is_ok());
        for (bad, line) in [
            ("10 rx A4 09\n20 rx ZZ\n", 2),
            ("10 rx A4 09\nbogus line\n", 2),
            ("10 rx A4 09\n20 xx A4\n", 2),
            ("# adv 50 zz\n", 1),
            ("# cps 5 2Z\n", 1),
        ] {
            assert_eq!(
                Timeline::parse(bad).err(),
                Some(format!("contradictory: line {line} is no record")),
                "{bad:?}"
            );
        }
    }

    /// #516 round 4: a check reads the capture only through the timeline's typed events. The
    /// frames are private to `Timeline` (no accessor returns them) and no check module reads raw
    /// records or parses capture text.
    #[test]
    fn checks_read_typed_events_only() {
        let timeline = include_str!("timeline.rs");
        let api = &timeline[..timeline.find("#[cfg(test)]").unwrap_or(timeline.len())];
        assert!(
            !api.contains("-> &[Record]") && !api.contains("pub records"),
            "Timeline hands out its frames"
        );
        for (name, src) in [
            ("check10.rs", include_str!("check10.rs")),
            ("check11.rs", include_str!("check11.rs")),
            ("check11ac.rs", include_str!("check11ac.rs")),
            ("cps.rs", include_str!("cps.rs")),
        ] {
            let code = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
            for raw in [
                "Record",
                "parse_capture",
                "frame_line",
                ".records()",
                "crank_fec",
            ] {
                assert!(!code.contains(raw), "{name} reads raw records ({raw})");
            }
        }
        // In partb.rs the checks (`check6` … `check12`, `erg_steps`) and `analyse` with what it
        // calls (`result_of`, the release, check 6's power lag and its session) take a timeline
        // only.
        let partb = include_str!("partb.rs");
        for check in [
            "pub fn check6(",
            "pub fn check7(",
            "pub fn check8(",
            "pub fn check9(",
            "pub fn check12(",
            "fn erg_steps(",
            "pub fn analyse(",
            "fn result_of(",
            "pub fn release(",
            "pub fn serial_in(",
            "impl PowerLag {",
        ] {
            let at = partb.find(check).expect(check);
            let body = &partb[at..at + partb[at..].find("\n}\n").expect("end")];
            for raw in [
                "Record",
                "parse_capture",
                ".records()",
                "decode(",
                "page_of(",
            ] {
                assert!(!body.contains(raw), "{check} reads raw records ({raw})");
            }
        }
    }

    /// Every committed live capture of the real NEO, read in stamp order, is a timeline in which
    /// the NEO repeats updates (a held count, ADR-0007 §1). Part A's captures predate the one
    /// stamping writer (#516), and ADR-0007 §3 says they show reordering: check 1's file is not
    /// in time order as committed, so it is no timeline as it is; check 4's is.
    #[test]
    fn the_live_neo_captures_are_timelines() {
        for (name, text) in [
            (
                "check1",
                include_str!("../../../../docs/hil/captures/check1.fecap"),
            ),
            (
                "check2",
                include_str!("../../../../docs/hil/captures/check2.fecap"),
            ),
            (
                "check3",
                include_str!("../../../../docs/hil/captures/check3.fecap"),
            ),
            (
                "check4",
                include_str!("../../../../docs/hil/captures/check4.fecap"),
            ),
            (
                "idle",
                include_str!("../../../../docs/hil/captures/idle.fecap"),
            ),
            (
                "latency",
                include_str!("../../../../docs/hil/captures/latency.fecap"),
            ),
            (
                "spacing",
                include_str!("../../../../docs/hil/captures/spacing.fecap"),
            ),
        ] {
            let mut lines: Vec<(u64, &str)> = text
                .lines()
                .filter_map(|l| Some((l.split_whitespace().next()?.parse().ok()?, l)))
                .collect();
            lines.sort_by_key(|(at, _)| *at);
            let sorted: String = lines.iter().flat_map(|(_, l)| [*l, "\n"]).collect();
            let tl = Timeline::parse(&sorted).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!tl.samples().is_empty(), "{name}");
            assert!(
                tl.updates().len() < tl.samples().len(),
                "{name}: repeats count once"
            );
        }
        Timeline::parse(include_str!("../../../../docs/hil/captures/check4.fecap"))
            .expect("check 4");
        assert!(
            Timeline::parse(include_str!("../../../../docs/hil/captures/check1.fecap"))
                .is_err_and(|e| e.contains("records out of time order")),
            "check 1 as committed is out of time order"
        );
    }

    /// A trainer frame line at `at` µs: `A4 09 4E 05` and the 8 page bytes, its checksum fixed.
    fn frame(at: u64, page: [u8; 8]) -> String {
        let mut f = vec![0xa4, 9, 0x4e, 5];
        f.extend(page);
        f.push(f.iter().fold(0, |x, v| x ^ v));
        let hex: Vec<String> = f.iter().map(|b| format!("{b:02X}")).collect();
        format!("{at} rx {}\n", hex.join(" "))
    }

    /// The one freshness notion (ADR-0007 §2, §4): a page 25 whose update event count (page
    /// byte 1) equals the one before is a repeat, whatever else it carries; so is a page 16 whose
    /// elapsed time (page byte 2) does. Any other step, back included, is an update.
    #[test]
    fn a_held_count_or_elapsed_time_is_a_repeat() {
        let p25 = |at: u64, count: u8, w: u8| frame(at, [0x19, count, 90, 0, 0, w, 0, 0x30]);
        let tl = Timeline::parse(
            &[
                p25(0, 10, 150),
                p25(250, 10, 150),
                p25(500, 12, 160),
                p25(750, 9, 170),
            ]
            .concat(),
        )
        .expect("a timeline");
        let powers: Vec<_> = tl.updates().iter().map(|s| s.power).collect();
        assert_eq!(powers, [Some(150), Some(160), Some(170)]);
        let p16 =
            |at: u64, elapsed: u8| frame(at, [0x10, 0x19, elapsed, 0, 0xe8, 0x03, 0xff, 0x30]);
        let tl = Timeline::parse(&[p16(0, 4), p16(250, 4), p16(500, 5), p16(750, 3)].concat())
            .expect("a timeline");
        assert_eq!(tl.speeds().len(), 3, "the second is a repeat");
    }

    /// Cycling Power measurements are read as they came, one byte-identical to the one before
    /// included (ADR-0007 §4: check 5 reads them as they come).
    #[test]
    fn cps_measurements_are_read_as_they_came() {
        let cps = |at: u64| format!("# cps {at} 20 00 96 00 0A 00 E8 03\n");
        let tl = Timeline::parse(&[cps(0), cps(300), cps(600)].concat()).expect("a timeline");
        assert_eq!(tl.cps().len(), 3);
    }

    /// An echo is a page-71 status after the write's request, naming its page and bytes
    /// (ADR-0007, PR #587's table): `release` counts only a status whose page-71 request came
    /// after the closing release, so the opening release's echo is none.
    #[test]
    fn an_echo_answers_a_request_after_its_write() {
        use crate::partb::{Control, Write, release};
        use serde_json::json;
        let w = |pos: usize, control: Control, d: u8| Write {
            pos,
            at_us: 0,
            control,
            data: [0xff, 0xff, 0xff, d],
        };
        let st = |pos: usize, request: Option<usize>| Status {
            pos,
            at_us: 0,
            page: 48,
            echo: [0xff, 0xff, 0xff, 0],
            request,
        };
        let ws = [
            w(0, Control::Resistance(0.0), 0),
            w(3, Control::Erg(150.0), 0x10),
            w(6, Control::Resistance(0.0), 0),
        ];
        // Answering a request before the closing release (the opening one's echo): not echoed.
        for (status, echoed) in [
            (st(2, Some(1)), false),
            (st(8, Some(7)), true),
            (st(8, None), false),
        ] {
            assert_eq!(
                release(&ws, &[status])["page71_echoed"],
                json!(echoed),
                "{status:?}"
            );
        }
    }
}
