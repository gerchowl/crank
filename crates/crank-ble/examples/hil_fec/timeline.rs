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
    /// time order (line N)")`: no single writer wrote that file, so no check reads it.
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
            } else {
                continue;
            };
            if at < last {
                return Err(format!(
                    "contradictory: records out of time order (line {})",
                    i + 1
                ));
            }
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

    /// Hold the stamps to `g`: at most one cue; each question answered at most once, after the
    /// cue and before the closing release; each drop after the cue and not after the closing
    /// release. A breach is `Err("contradictory: …")`. No cue at all is the check's own
    /// "no cue" (no result), so a full row always has exactly one.
    pub fn grammar(&self, g: &Grammar) -> Result<(), String> {
        let (tag, code) = g.cue;
        let cues: Vec<(usize, &Note)> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.tag == tag && n.bytes == [code])
            .collect();
        if cues.len() > 1 {
            return Err(format!(
                "contradictory: the {tag} {code:02X} cue is stamped {} times",
                cues.len()
            ));
        }
        let cue = cues.first().map(|(i, _)| *i);
        let closing = self.closing_release();
        let place = |what: &str, i: usize, n: &Note| {
            if cue.is_some_and(|c| i < c) {
                return Err(format!("contradictory: {what} before the {tag} cue"));
            }
            if closing.is_some_and(|c| n.before > c.pos) {
                return Err(format!("contradictory: {what} after the closing release"));
            }
            Ok(())
        };
        for key in g.answers {
            let atag = answer_tag(key);
            let answers: Vec<(usize, &Note)> = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.tag == atag)
                .collect();
            if answers.len() > 1 {
                return Err(format!(
                    "contradictory: the {key} question is answered {} times",
                    answers.len()
                ));
            }
            for (i, n) in answers {
                place(&format!("the {key} question is answered"), i, n)?;
            }
        }
        if g.drop {
            for (i, n) in self.notes.iter().enumerate() {
                if n.tag == crate::check11::LINK && n.bytes == [crate::check11::DETECTED] {
                    place("the drop is stamped", i, n)?;
                }
            }
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

/// The stamped events of a check (decided design #516 point 3): its cue (tag, code), the
/// questions the operator answers, and whether a `# link 01` drop may be stamped.
#[derive(Clone, Copy, Debug)]
pub struct Grammar {
    /// The cue the check's events follow.
    pub cue: (&'static str, u8),
    /// The keys of the questions it asks.
    pub answers: &'static [&'static str],
    /// A drop may be stamped.
    pub drop: bool,
}

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
