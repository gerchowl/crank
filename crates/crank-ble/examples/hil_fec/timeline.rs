//! One validated, time-ordered capture timeline for the Part B analysis (#516, decided design).
//!
//! A live capture has one writer, which stamps and appends every line under one lock
//! (`Capture::record_now` and `record_aux_now`: the rx pump, the tx writes, the `# link`, cue and
//! answer stamps, the `# cps` lines), so the file order is the order in which things happened.
//! [`Timeline::parse`] reads a capture once and holds it to that: the stamps never go back in file
//! order, stamps that tie are ordered by the file, no record appears twice and no line is
//! malformed. Every Part B check reads the capture only through the typed events of a
//! [`Timeline`] (samples, speeds, control writes, requests, command statuses, Cycling Power
//! measurements, stamps): the frames themselves are private to it. Positions (a frame's index
//! among the frames, a note's [`Note::before`]) say what came before what, never µs arithmetic at
//! an edge. A completed run is also held to its check's [`Grammar`] before the check reads it.
use crate::{
    check10::answer_tag,
    frames::{Cps, Note, Record, cps_line, frame_line, note_line, page_of},
    partb::{Control, Request, Sample, Status, Write, requests, samples, speeds, statuses, writes},
};

/// A capture read once, in file order, its stamps validated as non-decreasing.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    /// The frames: read only here, to build the typed views below.
    records: Vec<Record>,
    notes: Vec<Note>,
    cps: Vec<Cps>,
    samples: Vec<Sample>,
    speeds: Vec<(u64, f64)>,
    writes: Vec<Write>,
    requests: Vec<Request>,
    statuses: Vec<Status>,
}

impl Timeline {
    /// Read `.fecap` text. A stamp below the one before it is `Err("contradictory: records out of
    /// time order (line N)")`: no single writer wrote that file, so no check reads it. So is a
    /// record identical to another of the same µs (same kind, same bytes): the writer appends
    /// each record once; and a line that is neither a record nor a comment (a record whose
    /// fields do not read is a record lost, never one to skip).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut tl = Self::default();
        let mut last = 0;
        // The records of the current µs: (line number, record without whitespace variations).
        let mut same_us: Vec<(usize, String)> = Vec::new();
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
            if at != last {
                same_us.clear();
            }
            let record = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if let Some((first, _)) = same_us.iter().find(|(_, r)| *r == record) {
                return Err(format!(
                    "contradictory: line {} repeats line {first}",
                    i + 1
                ));
            }
            same_us.push((i + 1, record));
            last = at;
        }
        tl.samples = samples(&tl.records);
        tl.speeds = speeds(&tl.records);
        tl.writes = writes(&tl.records);
        tl.requests = requests(&tl.records);
        tl.statuses = statuses(&tl.records);
        // A measurement repeating the one before within the same whole second is counted once:
        // check 5 compares per whole second, and a sensor without crank data repeats an unchanged
        // power reading honestly from one second to the next.
        tl.cps
            .dedup_by(|b, a| a.bytes == b.bytes && a.at_us / 1_000_000 == b.at_us / 1_000_000);
        Ok(tl)
    }

    /// Every page-25 sample, repeats of the trainer's last update included (what the live guards
    /// see: the link is alive while page 25 comes).
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The page-25 updates: the samples without repeats, what power and cadence are read from.
    pub fn updates(&self) -> Vec<Sample> {
        self.samples.iter().filter(|s| !s.repeat).copied().collect()
    }

    /// The page-16 speeds, `(at_us, m/s)`, without repeats.
    pub fn speeds(&self) -> &[(u64, f64)] {
        &self.speeds
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

    /// The Cycling Power measurements, without repeats.
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

    /// The records of the timeline in `g`'s vocabulary, one token each ([`Grammar`]); a run of
    /// `f` frames is one `f`.
    fn tokens(&self, g: &Grammar) -> String {
        let mut out = String::new();
        let mut w = self.writes.iter().peekable();
        let mut q = self.requests.iter().peekable();
        let mut notes = self.notes.iter().peekable();
        let mut notes_before = |frame: usize, out: &mut String| {
            while let Some(n) = notes.next_if(|n| n.before <= frame) {
                if let Some(t) = note_token(g, n) {
                    out.push(t);
                }
            }
        };
        for (i, r) in self.records.iter().enumerate() {
            notes_before(i, &mut out);
            let t = if r.rx {
                if page_of(&r.bytes) == Some(71) {
                    'e'
                } else {
                    'f'
                }
            } else if let Some(x) = w.next_if(|x| x.pos == i) {
                match x.control {
                    Control::Resistance(0.0) => 'R',
                    Control::Resistance(_) => 'B',
                    Control::Erg(_) => 'E',
                    Control::Wind(..) | Control::Track(..) | Control::User(..) => 'S',
                    Control::Malformed(_) | Control::NotAFrame => 'X',
                }
            } else if let Some(x) = q.next_if(|x| x.pos == i) {
                match x.page {
                    71 => 'q',
                    81 => 'u',
                    _ => '?',
                }
            } else {
                '?' // a tx frame that is neither a control write nor a request
            };
            if !(t == 'f' && out.ends_with('f')) {
                out.push(t);
            }
        }
        notes_before(usize::MAX, &mut out);
        out
    }

    /// Hold the timeline to `g`: every event of the capture is a token of the check's alphabet,
    /// and the tokens, in timeline order, match the check's shape. `Err("contradictory: …")`
    /// names the sequence otherwise.
    pub fn grammar(&self, g: &Grammar) -> Result<(), String> {
        let ws = &self.writes;
        let fits: Vec<Result<(), usize>> = g.programs.iter().map(|p| follows(ws, p)).collect();
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
        let text = self.tokens(g);
        // The shapes are constants of this crate; one that does not compile matches nothing.
        match g
            .shapes
            .iter()
            .find(|shape| !regex::Regex::new(shape).is_ok_and(|re| re.is_match(&text)))
        {
            None => Ok(()),
            Some(shape) => Err(format!(
                "contradictory: the records do not follow the procedure ({shape}): {text}"
            )),
        }
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

/// The event grammar of a check (decided design #516 point 3): its vocabulary, its shapes and
/// its control programs.
///
/// Vocabulary, one token per record: `R` page 48 at 0 % (a release), `B` page 48 at another
/// level, `E` page 49, `S` pages 50, 51 and 55, `X` a malformed control write, `q` a page-70
/// request for page 71, `u` one for page 81, `e` a page-71 command status (the NEO sends one only
/// in answer to a request, docs/hil/captures), `f` one or more other trainer frames (pages 25 and
/// 16, the broadcast pages 80 and 81, …); a note of a tag in `tags` is the token its code maps
/// to; `# link` stamps are `d` the drop (01), `v` seen (02), `k` connected (03), `n` discovered
/// (04), `p` first page 25 (05), `g` gave up (06), `x` failed (07); the answer to the n-th
/// question of `answers` is `a`, `b`, …; a note of a tag in `records` is its token whatever its
/// payload; a `# cps` line is none outside check 5's `records`. Any other record (another tx frame, an unmapped code, another tag) is `?`, which no
/// shape allows.
#[derive(Clone, Copy, Debug)]
pub struct Grammar {
    /// Stamp tags and the token of each code (the check's own cues, `# link`, check 10's scan
    /// and adverts).
    pub tags: &'static [(&'static str, &'static [(u8, char)])],
    /// The keys of the questions it asks, in token order.
    pub answers: &'static [&'static str],
    /// Note tags of records the check reads whatever their payload (check 5's `cps`), and
    /// their token.
    pub records: &'static [(&'static str, char)],
    /// Anchored regular expressions over the tokens, all of which a completed run matches: the
    /// order and number of its records.
    pub shapes: &'static [&'static str],
    /// The control writes a completed run makes, one program per way the procedure can end.
    pub programs: &'static [&'static [Step]],
}

/// The `# link` stamps' tokens.
pub const LINK_TOKENS: (&str, &[(u8, char)]) = (
    "link",
    &[
        (1, 'd'),
        (2, 'v'),
        (3, 'k'),
        (4, 'n'),
        (5, 'p'),
        (6, 'g'),
        (7, 'x'),
    ],
);

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
    /// Page 55 with this rider mass, kg, and the 10 kg bike.
    User(u16),
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
            (Step::User(kg), Some(Control::User(r, b))) => near(r, f64::from(kg)) && near(b, 10.0),
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

/// The token of note `n` in `g`'s vocabulary.
fn note_token(g: &Grammar, n: &Note) -> Option<char> {
    if let Some((_, t)) = g.records.iter().find(|(tag, _)| *tag == n.tag) {
        return Some(*t);
    }
    // A Cycling Power measurement is a record of the trainer that only check 5 reads (and has in
    // its `records`); for any other check it is no token.
    if n.tag == "cps" {
        return None;
    }
    if let Some((_, codes)) = g.tags.iter().find(|(tag, _)| *tag == n.tag) {
        let token = match n.bytes[..] {
            [c] => codes.iter().find(|(code, _)| *code == c).map(|(_, t)| *t),
            _ => None,
        };
        return Some(token.unwrap_or('?'));
    }
    Some(
        g.answers
            .iter()
            .zip('a'..)
            .find(|(key, _)| answer_tag(key) == n.tag)
            .map_or('?', |(_, t)| t),
    )
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

/// The shape of a run that writes ERG targets only (checks 6, 7 and 9): frames before the opening
/// release (the link is up before the run writes), the release and the page-81 request that
/// opens every run, the targets, the closing release and its page-71 request; nothing else is
/// written (check 7's silent window included).
pub const ERG_ONLY: &str = "^f*Rf*u[fe]*(?:E[fe]*)+R[fe]*q[fe]*$";

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
        // In partb.rs the checks (`check6` … `check12`, `erg_steps`) take a timeline only.
        let partb = include_str!("partb.rs");
        for check in [
            "pub fn check6(",
            "pub fn check7(",
            "pub fn check8(",
            "pub fn check9(",
            "pub fn check12(",
            "fn erg_steps(",
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

    /// A live capture of the real NEO (Part A check 4, committed) is a timeline: in time order,
    /// no malformed line, no record twice, and its repeated page-25 payloads are repeats, not
    /// contradictions. (The other committed captures were checked the same way, see the PR.)
    #[test]
    fn a_live_neo_capture_is_a_timeline() {
        let tl = Timeline::parse(include_str!("../../../../docs/hil/captures/check4.fecap"))
            .expect("a live capture");
        assert!(!tl.samples().is_empty());
        assert!(tl.updates().len() <= tl.samples().len());
    }
}
