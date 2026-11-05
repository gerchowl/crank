//! One validated, time-ordered capture timeline for the Part B analysis (#516, decided design).
//!
//! A live capture has one writer, which stamps and appends every line under one lock
//! (`Capture::record_now` and `record_aux_now`: the rx pump, the tx writes, the `# link`, cue and
//! answer stamps, the `# cps` lines), so the file order is the order in which things happened.
//! [`Timeline::parse`] reads a capture once and holds it to that: the stamps never go back in file
//! order, and stamps that tie are ordered by the file. Every Part B check reads the capture only
//! through a [`Timeline`]: positions (a frame's index in [`Timeline::records`], a note's
//! [`Note::before`]) say what came before what, never µs arithmetic at an edge. A check with
//! stamped events also holds them to its [`Grammar`] before it reads anything else.
use crate::{
    check10::answer_tag,
    frames::{Cps, Note, Record, cps_line, frame_line, note_line},
    partb::{Control, Write, writes},
};

/// A capture read once, in file order, its stamps validated as non-decreasing.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    records: Vec<Record>,
    cps: Vec<Cps>,
    notes: Vec<Note>,
}

impl Timeline {
    /// Read `.fecap` text. A stamp below the one before it is `Err("contradictory: records out of
    /// time order (line N)")`: no single writer wrote that file, so no check reads it. So is a
    /// record identical to another of the same µs (same kind, same bytes): the writer appends
    /// each record once.
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
            } else {
                continue;
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
        Ok(tl)
    }

    /// The frames, both directions, in file (and time) order.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// The Cycling Power notifications, in order.
    pub fn cps(&self) -> &[Cps] {
        &self.cps
    }

    /// The tagged comment lines, in order.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The index of the first frame stamped at or after `us`: where a window that starts a
    /// duration after an event begins.
    pub fn index_at(&self, us: u64) -> usize {
        self.records.partition_point(|r| r.at_us < us)
    }

    /// The run's closing release: the last control write, page 48 at 0 %, after another write.
    /// `None` in a capture cut off before it (every run writes it last, a failed one too).
    pub fn closing_release(&self) -> Option<Write> {
        let w = writes(&self.records);
        w.last()
            .filter(|l| l.control == Control::Resistance(0.0) && w.len() > 1)
            .copied()
    }

    /// The events of the timeline in `g`'s alphabet, one token each ([`Grammar`]); a run of
    /// frames is one `f`.
    fn tokens(&self, g: &Grammar) -> String {
        let mut out = String::new();
        let ws = writes(&self.records);
        let mut w = ws.iter().peekable();
        let mut notes = self.notes.iter().peekable();
        let mut notes_before = |frame: usize, out: &mut String| {
            while let Some(n) = notes.next_if(|n| n.before <= frame) {
                // A Cycling Power notification is a record of the trainer, never a procedure event.
                if n.tag != "cps" && !g.ignored.contains(&n.tag.as_str()) {
                    out.push(note_token(g, n));
                }
            }
        };
        for (i, r) in self.records.iter().enumerate() {
            notes_before(i, &mut out);
            let t = if r.rx {
                'f'
            } else if let Some(x) = w.next_if(|x| x.pos == i) {
                match x.control {
                    Control::Resistance(0.0) => 'R',
                    Control::Resistance(_) => 'B',
                    Control::Erg(_) => 'E',
                    Control::Wind(..) | Control::Track(..) | Control::User(..) => 'S',
                    Control::Malformed(_) | Control::NotAFrame => 'X',
                }
            } else {
                continue; // a request (page 70) writes no control
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
        let ws = writes(&self.records);
        let fits: Vec<Result<(), usize>> = g.programs.iter().map(|p| follows(&ws, p)).collect();
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
        let fits = regex::Regex::new(g.shape).is_ok_and(|re| re.is_match(&text));
        if fits {
            return Ok(());
        }
        Err(format!(
            "contradictory: the events do not follow the procedure ({}): {text}",
            g.shape
        ))
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

/// The event grammar of a check (decided design #516 point 3): its alphabet and its shape.
///
/// Tokens: `R` page 48 at 0 % (a release), `B` page 48 at another level, `E` page 49, `S` pages
/// 50, 51 and 55, `X` a malformed control write, `f` one or more trainer frames; a stamp of the
/// check's own `tag` is its code as a digit (`1` the cue); `# link` stamps are `d` the drop
/// (01), `v` seen (02), `k` connected (03), `n` discovered (04), `p` first page 25 (05), `g`
/// gave up (06), `x` failed (07); the answer to the n-th question of `answers` is `a`, `b`, …
/// `# cps` notes and notes tagged in `ignored` are not events; any other note is `?`, which no
/// shape allows.
/// Requests (page 70) write no control and are not events.
#[derive(Clone, Copy, Debug)]
pub struct Grammar {
    /// The tag of the check's own stamps (its cue), if it has any.
    pub tag: Option<&'static str>,
    /// The keys of the questions it asks, in token order.
    pub answers: &'static [&'static str],
    /// Note tags that are records but no events of the shape (check 10's `scan` and `adv`).
    pub ignored: &'static [&'static str],
    /// The tokens a completed run writes, as an anchored regular expression.
    pub shape: &'static str,
    /// The control writes a completed run makes, one program per way the procedure can end.
    pub programs: &'static [&'static [Step]],
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

/// The token of note `n` in `g`'s alphabet.
fn note_token(g: &Grammar, n: &Note) -> char {
    let code = match n.bytes[..] {
        [c] => Some(c),
        _ => None,
    };
    if g.tag == Some(n.tag.as_str()) {
        return code
            .and_then(|c| char::from_digit(u32::from(c), 10))
            .unwrap_or('?');
    }
    if n.tag == crate::check11::LINK {
        return match code {
            Some(1) => 'd',
            Some(2) => 'v',
            Some(3) => 'k',
            Some(4) => 'n',
            Some(5) => 'p',
            Some(6) => 'g',
            Some(7) => 'x',
            _ => '?',
        };
    }
    g.answers
        .iter()
        .zip('a'..)
        .find(|(key, _)| answer_tag(key) == n.tag)
        .map_or('?', |(_, t)| t)
}

/// The shape of a run that writes ERG targets only (checks 6, 7 and 9): the opening release, the
/// targets, the closing release.
pub const ERG_ONLY: &str = "^Rf*(?:Ef*)+Rf*$";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_order_is_time_order_and_ties_keep_the_file_order() {
        let ok = "# header\n10 tx A4\n# link 10 01\n10 rx A4\n# hil_fec procedure: completed\n";
        let tl = Timeline::parse(ok).expect("in order");
        assert_eq!(tl.records().len(), 2);
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
}
