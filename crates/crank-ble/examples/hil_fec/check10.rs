//! Check 10 (#44, #412 part c1): a second central while `hil_fec` holds the link, and whether the
//! NEO still advertises (ADR-0002 §5, "Single central").
//!
//! The procedure (`rider.rs`) writes its events into the capture as tagged comment lines, so this
//! analysis is a pure function of the `.fecap` and `analyse` reproduces the live row:
//! `# check10 <us> 01|02` (window opened, closed), `# scan <us> 01|00|02` (parallel scan started,
//! ended by itself, stopped by the harness), `# adv <us> 01|00` (an advertisement of this trainer,
//! of another peripheral) and `# answer-<key> <us> <ASCII>` (the operator's typed answers).
use crate::{
    cps::max_gap,
    frames::{Note, Stamp, stamp, stamp_of, stamps},
    partb::{SEC, round1, samples, secs},
    timeline::{Grammar, Step, Timeline},
};
use serde_json::{Value, json};
use std::time::Duration;

/// Tag of the window events.
pub const WINDOW: &str = "check10";
/// Tag of the parallel scan's events.
pub const SCAN: &str = "scan";
/// Tag of one advertisement the parallel scan reported.
pub const ADV: &str = "adv";
/// Window or scan opened.
pub const OPEN: u8 = 1;
/// Window closed, or scan stopped by the harness at the window's end.
pub const CLOSE: u8 = 2;
/// The scan stream ended by itself (it failed to start, or the adapter stopped it).
pub const ENDED: u8 = 0;
/// The window lasts at least this long, answers or not, so the scan sees a connected stretch.
pub const MIN_WINDOW: Duration = Duration::from_mins(1);
/// Both answers must be in within this time from the window's start.
pub const ANSWER_WAIT: Duration = Duration::from_mins(3);
/// Page 25 silent this long inside the window means the link was not held (ADR-0002 §6).
const LINK_GAP_MAX_US: u64 = 2 * SEC;

/// Check 10's event grammar ([`Timeline::grammar`]): the window opens and closes once, the
/// listed answer and the optional connect answer inside it, between the opening and the closing
/// release.
pub(crate) const GRAMMAR: Grammar = Grammar {
    tag: Some(WINDOW),
    answers: &[LISTED.key, CONNECT.key],
    ignored: &[SCAN, ADV],
    shape: "^Rf*1f*af*(?:bf*)?2f*Rf*$",
    programs: &[&[Step::Release, Step::Release]],
};

/// One typed question: capture key (tag `answer-<key>`), text, accepted answers.
pub struct Question {
    /// The capture key.
    pub key: &'static str,
    /// What the operator is asked.
    pub text: &'static str,
    /// The answers it accepts.
    pub options: &'static [&'static str],
}

/// The first question, always asked.
pub const LISTED: Question = Question {
    key: "listed",
    text: "Does the phone list the NEO while hil_fec is connected?",
    options: &["seen", "not-seen", "unsure"],
};

/// The follow-up, asked only when the phone listed the NEO ([`follow_up`]).
pub const CONNECT: Question = Question {
    key: "connect",
    text: "Try to connect the phone to the NEO and give it 30 s. What did the connect do?",
    options: &["connected", "failed", "stuck", "not-tried"],
};

/// The question after `listed`: a phone that did not list the NEO had nothing to connect to.
pub fn follow_up(listed: &str) -> Option<&'static Question> {
    (listed == "seen").then_some(&CONNECT)
}

/// The capture tag of a question's answer.
pub fn answer_tag(key: &str) -> String {
    format!("answer-{key}")
}

/// The recorded answer to `q`, whatever it is.
fn recorded(notes: &[Note], q: &Question) -> Option<String> {
    let tag = answer_tag(q.key);
    notes
        .iter()
        .rev()
        .find(|n| n.tag == tag)
        .map(|n| String::from_utf8_lossy(&n.bytes).into_owned())
}

/// The operator's answer to `q`: `Ok` when it is one of the accepted answers.
pub(crate) fn answer(notes: &[Note], q: &Question) -> Result<String, String> {
    let text = recorded(notes, q).ok_or_else(|| "no answer".to_string())?;
    if q.options.contains(&text.as_str()) {
        Ok(text)
    } else {
        Err(format!("unrecognised answer {text:?}"))
    }
}

/// The second-central result: only a phone that listed the NEO can have been accepted or refused.
fn second_central(notes: &[Note]) -> String {
    let connect = recorded(notes, &CONNECT);
    match (answer(notes, &LISTED).as_deref(), connect) {
        (Ok("seen"), _) => match answer(notes, &CONNECT).as_deref() {
            Ok("connected") => "accepted: the phone connected while hil_fec held the link".into(),
            Ok("failed") => "refused: the phone's connect failed".into(),
            Ok("stuck") => "refused: the phone's connect never completed".into(),
            Ok(_) => "unknown: the phone listed the NEO but did not try to connect".into(),
            Err(why) => format!("unknown: {why} to whether the phone connected"),
        },
        (Ok(listed), Some(c)) => format!(
            "unknown: contradictory answers: the phone's listing was {listed:?}, yet a connect answer {c:?} is recorded"
        ),
        (Ok("not-seen"), None) => {
            "unknown: the phone did not find the NEO, so it could not try to connect".into()
        }
        (Ok(_), None) => {
            "unknown: the operator was unsure the phone listed the NEO, so no connect was tried"
                .into()
        }
        (Err(why), _) => format!("unknown: {why} to whether the phone listed the NEO"),
    }
}

/// How the parallel scan covered the window.
enum Scan {
    Contradictory(String),
    NotStarted,
    Ended(u64),
    Late(u64),
    Stopped(u64),
    Unclosed,
    Whole,
}

impl Scan {
    /// The scan's coverage of the window `from..to` (its stamps): whole only when both scan
    /// stamps are present, it opened before `from`, closed after `to`, and did not end by itself.
    fn of(notes: &[Note], from: &Stamp, to: &Stamp) -> Self {
        let scan = match stamps(notes, SCAN, &[OPEN, ENDED, CLOSE]) {
            Ok(s) => s,
            Err(why) => return Self::Contradictory(why),
        };
        let Some(opened) = stamp_of(&scan, OPEN) else {
            return Self::NotStarted;
        };
        if let Some(at) = stamp(&scan, ENDED) {
            return Self::Ended(at);
        }
        if opened.seq > from.seq {
            return Self::Late(opened.at_us);
        }
        match stamp_of(&scan, CLOSE) {
            None => Self::Unclosed,
            Some(close) if close.seq < to.seq => Self::Stopped(close.at_us),
            Some(_) => Self::Whole,
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Contradictory(why) => format!("is unknown: {why}"),
            Self::NotStarted => "was not started".into(),
            Self::Ended(at) => format!(
                "ended by itself at {} s (it failed to start or the adapter stopped it)",
                round1(secs(*at))
            ),
            Self::Late(at) => format!(
                "started at {} s, after the window opened",
                round1(secs(*at))
            ),
            Self::Stopped(at) => format!(
                "stopped at {} s, before the window closed",
                round1(secs(*at))
            ),
            Self::Unclosed => "has no close line, so its end is unknown".into(),
            Self::Whole => "ran over the whole window".into(),
        }
    }
}

/// The advertising result. "Not seen" needs both observations: the phone's, and a scan that ran
/// over the whole window and saw other peripherals (so it was working) but not the trainer.
fn advertising(
    trainer: &[u64],
    others: usize,
    scan: &Scan,
    listed: &Result<String, String>,
    window: (u64, u64),
) -> String {
    let phone = listed.as_ref().map_or_else(Clone::clone, Clone::clone);
    let scan_text = scan.text();
    if !trainer.is_empty() {
        return format!(
            "continues: the scan saw {} advertisements of the trainer, longest gap {} s; the phone: {phone}",
            trainer.len(),
            round1(secs(max_gap(trainer.to_vec(), window.0, window.1)))
        );
    }
    match (listed.as_deref(), scan) {
        (Ok("seen"), _) => {
            format!("continues: the phone listed it; the scan saw none (the scan {scan_text})")
        }
        (Ok("not-seen"), Scan::Whole) if others > 0 => format!(
            "not seen: the phone did not list it, and the scan, which ran over the whole window and saw {others} advertisements of other peripherals, saw none of the trainer"
        ),
        (Ok("not-seen"), _) => format!(
            "unknown: the phone did not list it, but the scan {scan_text} and saw {others} advertisements of other peripherals, so it cannot confirm that"
        ),
        _ => format!(
            "unknown: the phone's answer is {}, and the scan saw no advertisement of the trainer (the scan {scan_text}, {others} of other peripherals; a host scan may not report a peripheral it is connected to)",
            listed
                .as_ref()
                .map_or_else(Clone::clone, |a| format!("{a:?}")),
        ),
    }
}

/// The result of check 10 from the capture's timeline.
pub fn check10(tl: &Timeline) -> Value {
    let method = "window = the check10 open/close events; advertising = the parallel scan's adverts of this trainer inside the window (largest gap with the window's edges) and the operator's 'listed' answer, 'not seen' only when both agree; second central = the 'connect' answer, asked only when the phone listed the NEO; link held = no page-25 gap > 2 s inside the window";
    let (records, notes) = (tl.records(), tl.notes());
    let window = stamps(notes, WINDOW, &[OPEN, CLOSE]).and_then(|w| {
        match (stamp_of(&w, OPEN), stamp_of(&w, CLOSE)) {
            (Some(from), Some(to)) => Ok((from, to)),
            _ => Err("the capture has no complete check 10 window".into()),
        }
    });
    // The procedure asks the connect question exactly when the phone listed the NEO; a completed
    // run with the one and not the other did not run it.
    let window =
        window.and_then(
            |w| match (answer(notes, &LISTED).as_deref(), recorded(notes, &CONNECT)) {
                (Ok("seen"), None) => Err(
                    "contradictory: the phone listed the NEO, yet no connect answer is recorded"
                        .into(),
                ),
                _ => Ok(w),
            },
        );
    let (from, to) = match window {
        Ok(w) => w,
        Err(why) => {
            return json!({
                "method": method,
                "window_s": Value::Null,
                "advertising": Value::Null,
                "second_central": Value::Null,
                "verdict": format!("unknown: {why}"),
            });
        }
    };
    // Inside: between the window's stamps in the timeline.
    let adverts = |mine: u8| -> Vec<u64> {
        notes[from.seq + 1..to.seq]
            .iter()
            .filter(|n| n.tag == ADV && n.bytes == [mine])
            .map(|n| n.at_us)
            .collect()
    };
    let (trainer, others) = (adverts(1), adverts(0));
    let scan = Scan::of(notes, &from, &to);
    let (from, to) = (from.at_us, to.at_us);
    let listed = answer(notes, &LISTED);
    let advertising = advertising(&trainer, others.len(), &scan, &listed, (from, to));
    let second_central = second_central(notes);
    let page25: Vec<u64> = samples(records).iter().map(|s| s.at_us).collect();
    let link_gap = max_gap(page25, from, to);
    let link = if link_gap > LINK_GAP_MAX_US {
        format!("lost (page 25 silent for {} s)", round1(secs(link_gap)))
    } else {
        "held".to_string()
    };
    json!({
        "method": method,
        "window_s": [round1(secs(from)), round1(secs(to))],
        "scan": scan.text(),
        "trainer_adverts": trainer.len(),
        "other_adverts": others.len(),
        "phone_listed": listed.unwrap_or_else(|e| e),
        "page25_max_gap_s": round1(secs(link_gap)),
        "advertising": advertising,
        "second_central": second_central,
        "link": link,
        "verdict": format!("second central {second_central}; advertising while connected {advertising}; hil_fec link {link}"),
    })
}

/// Check number, check, result and ADR-0002 section of check 10's row.
pub fn cells(v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let result = v["verdict"].as_str().unwrap_or("no verdict").to_string();
    ("10", "Second central (phone) while connected", result, "§5")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::Record;

    fn check10(r: &[Record], n: &[Note]) -> Value {
        super::check10(&Timeline::of_parts(r, &[], n))
    }

    fn note(tag: &str, at_s: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_s * SEC,
            bytes: bytes.to_vec(),
            before: 0,
        }
    }

    /// Page 25 every 0.5 s from 0 to `to_s`.
    fn page25(to_s: u64) -> Vec<Record> {
        (0..to_s * 2)
            .map(|k| {
                let mut f = [0xa4, 9, 0x4e, 5, 25, 0, 0, 0, 0, 0, 0, 0x30, 0];
                f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
                Record {
                    at_us: k * SEC / 2,
                    rx: true,
                    bytes: f.to_vec(),
                }
            })
            .collect()
    }

    fn window(extra: &[Note]) -> Vec<Note> {
        // As the procedure stamps them: the scan opens before the window and closes after it.
        let mut n = vec![note(SCAN, 10, &[OPEN]), note(WINDOW, 10, &[OPEN])];
        n.extend_from_slice(extra);
        n.extend([note(WINDOW, 70, &[CLOSE]), note(SCAN, 70, &[CLOSE])]);
        n
    }

    #[test]
    fn the_advert_gap_counts_the_window_edges() {
        // Adverts 10-40 s only: the gap runs from the last one to the window's end. One before
        // the window does not count.
        let mut ads: Vec<Note> = (10..=40).map(|s| note(ADV, s, &[1])).collect();
        ads.push(note(ADV, 5, &[1]));
        let v = check10(&page25(80), &window(&ads));
        assert_eq!(v["trainer_adverts"], json!(31), "{v}");
        assert!(
            v["advertising"]
                .as_str()
                .is_some_and(|a| a.contains("longest gap 30 s")),
            "{v}"
        );
    }

    #[test]
    fn a_page_25_gap_over_2_s_inside_the_window_is_a_lost_link() {
        let mut records = page25(80);
        records.retain(|r| !(30 * SEC < r.at_us && r.at_us < 32 * SEC + SEC / 10));
        let v = check10(&records, &window(&[]));
        assert_eq!(v["link"], json!("lost (page 25 silent for 2.5 s)"), "{v}");
        let mut records = page25(80);
        records.retain(|r| !(30 * SEC < r.at_us && r.at_us < 32 * SEC));
        assert_eq!(check10(&records, &window(&[]))["link"], json!("held"));
        // Silent from 69 s to the window's end at 70 s: within; from 67.5 s: not.
        let mut records = page25(80);
        records.retain(|r| r.at_us < 67 * SEC);
        assert!(
            check10(&records, &window(&[]))["link"]
                .as_str()
                .is_some_and(|l| l.starts_with("lost"))
        );
    }

    #[test]
    fn answers_outside_the_accepted_set_are_unknown_not_guessed() {
        let a = |key: &str, text: &str| note(&answer_tag(key), 20, text.as_bytes());
        let v = check10(
            &page25(80),
            &window(&[a("listed", "yes"), a("connect", "maybe")]),
        );
        assert_eq!(
            v["second_central"],
            json!("unknown: unrecognised answer \"yes\" to whether the phone listed the NEO")
        );
        assert!(
            v["advertising"].as_str().is_some_and(
                |x| x.starts_with("unknown: the phone's answer is unrecognised answer \"yes\"")
            ),
            "{v}"
        );
        let v = check10(
            &page25(80),
            &window(&[a("listed", "seen"), a("connect", "maybe")]),
        );
        assert_eq!(
            v["second_central"],
            json!("unknown: unrecognised answer \"maybe\" to whether the phone connected")
        );
        let v = check10(&page25(80), &window(&[]));
        assert_eq!(
            v["second_central"],
            json!("unknown: no answer to whether the phone listed the NEO")
        );
    }

    /// Every combination of the two answers: only `seen` with a connect outcome is accepted or
    /// refused; a connect answer after anything but `seen` is a contradiction, not a result.
    #[test]
    fn the_second_central_follows_the_answer_table() {
        const UNSURE: &str =
            "unknown: the operator was unsure the phone listed the NEO, so no connect was tried";
        const NOT_FOUND: &str =
            "unknown: the phone did not find the NEO, so it could not try to connect";
        let listed = [
            Some("seen"),
            Some("not-seen"),
            Some("unsure"),
            Some("yes"),
            None,
        ];
        let connect = [
            Some("connected"),
            Some("failed"),
            Some("stuck"),
            Some("not-tried"),
            Some("maybe"),
            None,
        ];
        for l in listed {
            for c in connect {
                let mut n = Vec::new();
                n.extend(l.map(|t| note(&answer_tag("listed"), 20, t.as_bytes())));
                n.extend(c.map(|t| note(&answer_tag("connect"), 25, t.as_bytes())));
                let got = check10(&page25(80), &window(&n))["second_central"].clone();
                let want = match (l, c) {
                    (Some("seen"), Some("connected")) => {
                        "accepted: the phone connected while hil_fec held the link".to_string()
                    }
                    (Some("seen"), Some("failed")) => "refused: the phone's connect failed".into(),
                    (Some("seen"), Some("stuck")) => {
                        "refused: the phone's connect never completed".into()
                    }
                    (Some("seen"), Some("not-tried")) => {
                        "unknown: the phone listed the NEO but did not try to connect".into()
                    }
                    (Some("seen"), Some("maybe")) => {
                        "unknown: unrecognised answer \"maybe\" to whether the phone connected"
                            .into()
                    }
                    // The procedure asks the connect question after `seen`: none is no run of it.
                    (Some("seen"), None) => {
                        let v = check10(&page25(80), &window(&n));
                        assert_eq!(
                            (&v["second_central"], &v["verdict"]),
                            (
                                &Value::Null,
                                &json!(
                                    "unknown: contradictory: the phone listed the NEO, yet no connect answer is recorded"
                                )
                            )
                        );
                        continue;
                    }
                    (Some(l @ ("not-seen" | "unsure")), Some(c)) => format!(
                        "unknown: contradictory answers: the phone's listing was {l:?}, yet a connect answer {c:?} is recorded"
                    ),
                    (Some("not-seen"), None) => NOT_FOUND.into(),
                    (Some("unsure"), None) => UNSURE.into(),
                    (Some(l), _) => format!(
                        "unknown: unrecognised answer {l:?} to whether the phone listed the NEO"
                    ),
                    (None, _) => "unknown: no answer to whether the phone listed the NEO".into(),
                };
                assert_eq!(got, json!(want), "{l:?} {c:?}");
            }
        }
    }

    /// "Not seen" needs the phone's answer and a scan that ran over the whole window and saw other
    /// peripherals; a missing, short or blind scan leaves it unknown.
    #[test]
    fn advertising_not_seen_needs_both_observations() {
        let not_seen = note(&answer_tag("listed"), 20, b"not-seen");
        let others: Vec<Note> = (11..15).map(|s| note(ADV, s, &[0])).collect();
        let mut whole = vec![not_seen.clone()];
        whole.extend_from_slice(&others);
        let v = check10(&page25(80), &window(&whole));
        assert_eq!(
            v["advertising"],
            json!(
                "not seen: the phone did not list it, and the scan, which ran over the whole window and saw 4 advertisements of other peripherals, saw none of the trainer"
            )
        );
        // Blind: the scan saw nobody at all.
        let v = check10(&page25(80), &window(std::slice::from_ref(&not_seen)));
        assert_eq!(
            v["advertising"],
            json!(
                "unknown: the phone did not list it, but the scan ran over the whole window and saw 0 advertisements of other peripherals, so it cannot confirm that"
            )
        );
        // Short: the scan ended at 30 s.
        let mut short = whole.clone();
        short.push(note(SCAN, 30, &[ENDED]));
        assert!(
            check10(&page25(80), &window(&short))["advertising"]
                .as_str()
                .is_some_and(|a| a.starts_with(
                    "unknown: the phone did not list it, but the scan ended by itself at 30 s"
                )),
        );
        // Missing: no scan was started.
        let mut missing = vec![note(WINDOW, 10, &[OPEN]), not_seen];
        missing.extend_from_slice(&others);
        missing.push(note(WINDOW, 70, &[CLOSE]));
        let v = check10(&page25(80), &missing);
        assert!(
            v["advertising"].as_str().is_some_and(|a| a
                .starts_with("unknown: the phone did not list it, but the scan was not started")),
            "{v}"
        );
        // A phone that listed it is evidence on its own, scan or not.
        let seen = [
            note(WINDOW, 10, &[OPEN]),
            note(&answer_tag("listed"), 20, b"seen"),
            note(&answer_tag("connect"), 25, b"failed"),
            note(WINDOW, 70, &[CLOSE]),
        ];
        assert_eq!(
            check10(&page25(80), &seen)["advertising"],
            json!("continues: the phone listed it; the scan saw none (the scan was not started)")
        );
    }

    /// The scan's stamps must cover the window: a late start, an early stop or a missing close
    /// leave "not seen" unknown, with the reason.
    #[test]
    fn a_scan_counts_as_whole_only_when_its_stamps_cover_the_window() {
        let not_seen = note(&answer_tag("listed"), 20, b"not-seen");
        let others = (11..15).map(|s| note(ADV, s, &[0]));
        // The scan's open goes before the window's, its close after the window's: at the same µs
        // that is the procedure's file order.
        let with = |scan: &[Note]| {
            let (open, close): (Vec<Note>, Vec<Note>) =
                scan.iter().cloned().partition(|n| n.bytes == [OPEN]);
            let mut n = open;
            n.extend([note(WINDOW, 10, &[OPEN]), not_seen.clone()]);
            n.extend(others.clone());
            n.push(note(WINDOW, 70, &[CLOSE]));
            n.extend(close);
            check10(&page25(80), &n)["advertising"].clone()
        };
        let unknown = |why: &str| {
            json!(format!(
                "unknown: the phone did not list it, but the scan {why} and saw 4 advertisements of other peripherals, so it cannot confirm that"
            ))
        };
        assert_eq!(
            with(&[note(SCAN, 69, &[OPEN]), note(SCAN, 70, &[CLOSE])]),
            unknown("started at 69 s, after the window opened")
        );
        assert_eq!(
            with(&[note(SCAN, 10, &[OPEN]), note(SCAN, 12, &[CLOSE])]),
            unknown("stopped at 12 s, before the window closed")
        );
        assert_eq!(
            with(&[note(SCAN, 10, &[OPEN])]),
            unknown("has no close line, so its end is unknown")
        );
        // At the window's own µs, the file order decides: opened after the window's stamp is late.
        let late = [
            note(WINDOW, 10, &[OPEN]),
            note(SCAN, 10, &[OPEN]),
            not_seen.clone(),
            note(WINDOW, 70, &[CLOSE]),
            note(SCAN, 70, &[CLOSE]),
        ];
        assert_eq!(
            check10(&page25(80), &late)["advertising"],
            json!(
                "unknown: the phone did not list it, but the scan started at 10 s, after the window opened and saw 0 advertisements of other peripherals, so it cannot confirm that"
            )
        );
        // Stamps at the window's edges, or beyond them, cover it.
        for (open, close) in [(10, 70), (9, 71)] {
            assert!(
                with(&[note(SCAN, open, &[OPEN]), note(SCAN, close, &[CLOSE])])
                    .as_str()
                    .is_some_and(|a| a.starts_with("not seen:")),
                "{open} {close}"
            );
        }
    }

    /// Duplicate or out-of-order stamps are a contradiction, whichever comes last in the file.
    #[test]
    fn duplicate_or_out_of_order_stamps_are_contradictory() {
        let not_seen = note(&answer_tag("listed"), 20, b"not-seen");
        let others = (11..15).map(|s| note(ADV, s, &[0]));
        let with = |scan: &[Note]| {
            let mut n = vec![note(WINDOW, 10, &[OPEN]), not_seen.clone()];
            n.extend(others.clone());
            n.extend_from_slice(scan);
            n.push(note(WINDOW, 70, &[CLOSE]));
            check10(&page25(80), &n)["advertising"].clone()
        };
        for scan in [
            vec![
                note(SCAN, 5, &[OPEN]),
                note(SCAN, 30, &[CLOSE]),
                note(SCAN, 75, &[CLOSE]),
            ],
            vec![
                note(SCAN, 5, &[OPEN]),
                note(SCAN, 6, &[OPEN]),
                note(SCAN, 75, &[CLOSE]),
            ],
            vec![note(SCAN, 5, &[OPEN]), note(SCAN, 4, &[CLOSE])],
        ] {
            let a = with(&scan);
            assert!(
                a.as_str().is_some_and(|a| a.starts_with(
                    "unknown: the phone did not list it, but the scan is unknown: contradictory scan stamps"
                )),
                "{scan:?}: {a}"
            );
        }
        // The same stamp twice at its µs is no timeline at all, nor is one out of time order.
        assert_eq!(
            Timeline::parse("# scan 5000000 01\n# scan 5000000 01\n").err(),
            Some("contradictory: line 2 repeats line 1".into())
        );
        // Out of time order in the file, the capture is no timeline at all.
        assert_eq!(
            Timeline::parse("# scan 75000000 02\n# scan 5000000 01\n").err(),
            Some("contradictory: records out of time order (line 2)".into())
        );
        let v = check10(
            &page25(80),
            &[
                note(WINDOW, 10, &[OPEN]),
                note(WINDOW, 70, &[CLOSE]),
                note(WINDOW, 72, &[CLOSE]),
            ],
        );
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("unknown: contradictory check10 stamps")),
            "{v}"
        );
        assert_eq!(v["advertising"], Value::Null);
    }

    #[test]
    fn without_a_window_there_is_no_verdict_of_either_kind() {
        let v = check10(&page25(80), &[note(WINDOW, 10, &[OPEN])]);
        assert_eq!(
            (&v["advertising"], &v["second_central"]),
            (&Value::Null, &Value::Null)
        );
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("unknown"))
        );
    }
}
