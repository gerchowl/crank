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
    frames::{Note, Record},
    partb::{SEC, round1, samples, secs},
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

/// The typed questions: capture key (tag `answer-<key>`), text, accepted answers.
pub const QUESTIONS: [(&str, &str, &[&str]); 2] = [
    (
        "listed",
        "Does the phone list the NEO while hil_fec is connected?",
        &["seen", "not-seen", "unsure"],
    ),
    (
        "connect",
        "Try to connect the phone to the NEO and give it 30 s. What did the connect do?",
        &["connected", "failed", "stuck", "not-tried"],
    ),
];

/// The capture tag of a question's answer.
pub fn answer_tag(key: &str) -> String {
    format!("answer-{key}")
}

/// The last event of `tag` with payload `code`.
fn event(notes: &[Note], tag: &str, code: u8) -> Option<u64> {
    notes
        .iter()
        .rev()
        .find(|n| n.tag == tag && n.bytes == [code])
        .map(|n| n.at_us)
}

/// The operator's answer to question `key`: `Ok` when it is one of the accepted answers.
fn answer(notes: &[Note], key: &str) -> Result<String, String> {
    let tag = answer_tag(key);
    let text = notes
        .iter()
        .rev()
        .find(|n| n.tag == tag)
        .map(|n| String::from_utf8_lossy(&n.bytes).into_owned())
        .ok_or_else(|| "no answer".to_string())?;
    let accepted = QUESTIONS
        .iter()
        .find(|q| q.0 == key)
        .map_or(&[][..], |q| q.2);
    if accepted.contains(&text.as_str()) {
        Ok(text)
    } else {
        Err(format!("unrecognised answer {text:?}"))
    }
}

/// The result of check 10 from the capture's records and notes.
pub fn check10(records: &[Record], notes: &[Note]) -> Value {
    let method = "window = the check10 open/close events; advertising = the parallel scan's adverts of this trainer inside the window (largest gap with the window's edges) and the operator's 'listed' answer; second central = the 'connect' answer; link held = no page-25 gap > 2 s inside the window";
    let (Some(from), Some(to)) = (event(notes, WINDOW, OPEN), event(notes, WINDOW, CLOSE)) else {
        return json!({
            "method": method,
            "window_s": Value::Null,
            "advertising": Value::Null,
            "second_central": Value::Null,
            "verdict": "unknown: the capture has no complete check 10 window",
        });
    };
    let inside = |at: u64| at >= from && at < to;
    let adverts = |mine: u8| -> Vec<u64> {
        notes
            .iter()
            .filter(|n| n.tag == ADV && n.bytes == [mine] && inside(n.at_us))
            .map(|n| n.at_us)
            .collect()
    };
    let (trainer, others) = (adverts(1), adverts(0));
    let scan = match (event(notes, SCAN, OPEN), event(notes, SCAN, ENDED)) {
        (None, _) => "was not started".to_string(),
        (Some(_), Some(at)) => format!(
            "ended by itself at {} s (it failed to start or the adapter stopped it)",
            round1(secs(at))
        ),
        (Some(_), None) => "ran over the whole window".to_string(),
    };
    let listed = answer(notes, "listed");
    let advertising = if trainer.is_empty() {
        match listed.as_deref() {
            Ok("seen") => {
                format!("continues: the phone listed it; the scan saw none (the scan {scan})")
            }
            Ok("not-seen") => format!(
                "not seen: the phone did not list it and the scan saw none (the scan {scan})"
            ),
            _ => format!(
                "unknown: the phone's answer is {}, and the scan saw no advertisement of the trainer (the scan {scan}, {} of other peripherals; a host scan may not report a peripheral it is connected to)",
                listed
                    .as_ref()
                    .map_or_else(Clone::clone, |a| format!("{a:?}")),
                others.len()
            ),
        }
    } else {
        format!(
            "continues: the scan saw {} advertisements of the trainer, longest gap {} s; the phone: {}",
            trainer.len(),
            round1(secs(max_gap(trainer.clone(), from, to))),
            listed.as_ref().map_or_else(Clone::clone, Clone::clone)
        )
    };
    let second_central = match answer(notes, "connect").as_deref() {
        Ok("connected") => "accepted: the phone connected while hil_fec held the link".to_string(),
        Ok("failed") => "refused: the phone's connect failed".to_string(),
        Ok("stuck") => "refused: the phone's connect never completed".to_string(),
        Ok(_) => "unknown: the phone did not try".to_string(),
        Err(why) => format!("unknown: {why}"),
    };
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
        "scan": scan,
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

    fn note(tag: &str, at_s: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_s * SEC,
            bytes: bytes.to_vec(),
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
        let mut n = vec![note(WINDOW, 10, &[OPEN]), note(SCAN, 10, &[OPEN])];
        n.extend_from_slice(extra);
        n.extend([note(SCAN, 70, &[CLOSE]), note(WINDOW, 70, &[CLOSE])]);
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
            json!("unknown: unrecognised answer \"maybe\"")
        );
        assert!(
            v["advertising"].as_str().is_some_and(
                |x| x.starts_with("unknown: the phone's answer is unrecognised answer \"yes\"")
            ),
            "{v}"
        );
        let v = check10(&page25(80), &window(&[]));
        assert_eq!(v["second_central"], json!("unknown: no answer"));
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
