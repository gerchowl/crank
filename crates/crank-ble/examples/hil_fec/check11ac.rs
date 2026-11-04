//! Checks 11a and 11c (#44, #412 part c3), the NEO unplugged. 11a pulls the plug while the rider
//! pedals under ERG: the link is expected to stay up, and page 25 is compared before and after.
//! 11c starts the rider on the cold, unpowered trainer, then connects: how long until it
//! advertises, connects and echoes ERG, and whether ERG targets are honoured at low cadence
//! (if not, the following machine is to report `not_following`, not a fault).
//!
//! The procedures (`rider.rs`) stamp their events into the capture, so `analyse` reproduces the
//! live row: `# check11a <us> 01` (the unplug cue) and `# answer-unplugged`; `# check11c <us>
//! 01|02|03` (the start cue on the cold trainer, the low-cadence and the 90 rpm cue) and the link
//! stamps of check 11b (`# link <us> 01` the drop; `02` seen, `04` connected for 11c).
use crate::{
    check10::{Question, answer, answer_tag},
    check11::{DETECTED, LINK, READY, SEEN, TARGET_W},
    frames::{Note, Record, command_status, page_of, stamp, stamps},
    partb::{Control, SEC, controls, mean, procedure_end, round1, secs, share_within},
};
use crank_fec::{Page, TrainerData, decode};
use serde_json::{Value, json};
use std::time::Duration;

/// Tag of check 11a's stamp.
pub const WINDOW_A: &str = "check11a";
/// The cue to pull the plug, ERG held.
pub const UNPLUG: u8 = 1;
/// After the unplug answer, check 11a rides this long.
pub const AFTER: Duration = Duration::from_secs(30);
/// Check 11a's "before" window: this long up to the unplug cue.
const BEFORE_US: u64 = 15 * SEC;
/// A window's first part, after a cue or the unplug, is left out.
const SETTLE_US: u64 = 5 * SEC;

/// The operator's confirmation that the plug is out; only `unplugged` makes 11a a result.
pub const UNPLUGGED: Question = Question {
    key: "unplugged",
    text: "Is the trainer's mains plug out now (keep pedalling)?",
    options: &["unplugged", "cannot"],
};

/// Tag of check 11c's stamps.
pub const WINDOW_C: &str = "check11c";
/// The start cue, the trainer unplugged and off.
pub const COLD: u8 = 1;
/// The low-cadence cue, ERG engaged.
pub const LOW: u8 = 2;
/// The 90 rpm cue.
pub const NORMAL: u8 = 3;
/// The cold trainer must advertise within this time of the start cue.
pub const ADVERT_WAIT: Duration = Duration::from_mins(1);
/// The connect (discovery and subscription included) must finish within this time of the advert.
pub const CONNECT_WAIT: Duration = Duration::from_secs(30);
/// The stamps are written when the awaited event arrives, at most this much after it.
const STAMP_SLACK_US: u64 = 500_000;
/// Check 11c's low cadence.
pub const LOW_RPM: u8 = 60;
/// A window is judged only from this many page-25 samples, the rider within this many rpm of the
/// cue, and is honoured when this share of its samples is within ±10 % of the target.
const MIN_SAMPLES: usize = 10;
const CADENCE_TOL: f64 = 10.0;
const HONOURED: f64 = 0.8;

/// Page 25 in `from..to`.
fn trainer(records: &[Record], from: u64, to: u64) -> Vec<TrainerData> {
    records
        .iter()
        .filter(|r| r.rx && r.at_us >= from && r.at_us < to)
        .filter_map(|r| match decode(&r.bytes).ok()?.page {
            Page::Trainer(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// What page 25 reported in `from..to`: the FE states, target-limit codes and status bits seen,
/// and the power and cadence against `target`.
fn window(records: &[Record], from: u64, to: u64) -> Value {
    let t = trainer(records, from, to);
    let codes = |f: fn(&TrainerData) -> u8| {
        let mut v: Vec<u8> = t.iter().map(f).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let power: Vec<f64> = t.iter().filter_map(|d| d.power_w.map(f64::from)).collect();
    json!({
        "samples": t.len(),
        "fe_states": codes(|d| d.state),
        "target_limits": codes(|d| d.target_limit),
        "status_bits": codes(|d| d.status),
        "mean_power_w": mean(power.iter().copied()).map(round1),
        "within_10pct": share_within(&power, f64::from(TARGET_W), 0.10).map(|x| round1(x * 100.0)),
        "mean_cadence_rpm": mean(t.iter().filter_map(|d| d.cadence_rpm.map(f64::from))).map(round1),
    })
}

/// The first control write in `from..to`, as an error naming it.
fn quiet(ctl: &[(u64, Control)], from: u64, to: u64, inside: &str) -> Result<(), String> {
    match ctl.iter().find(|(at, _)| *at >= from && *at < to) {
        Some((at, c)) => Err(format!(
            "{c:?} written at {} s, {inside}",
            round1(secs(*at))
        )),
        None => Ok(()),
    }
}

/// When the operator answered `q` (the last answer, as [`answer`] reads it).
fn answered_at(notes: &[Note], q: &Question) -> Option<u64> {
    let tag = answer_tag(q.key);
    notes.iter().rev().find(|n| n.tag == tag).map(|n| n.at_us)
}

/// The result of check 11a from the capture's records and notes.
pub fn check11a(records: &[Record], notes: &[Note]) -> Value {
    let method = "stamps validated first (the unplug cue once, at most one drop after it, no control write from 15 s before the cue to the release), else unknown; unplugged = the operator's answer (decision table); drop = a link 01 stamp after the cue; before = page 25 in the 15 s up to the cue; after = page 25 from 5 s after the answer to the release; each: FE states, target-limit codes, status bits, mean power and share within ±10 % of the ERG target";
    let unknown = |why: String| json!({ "method": method, "verdict": format!("unknown: {why}") });
    let run = || -> Result<Value, String> {
        let cue = stamp(&stamps(notes, WINDOW_A, &[UNPLUG])?, UNPLUG)
            .ok_or("the capture has no unplug cue")?;
        let link = stamps(notes, LINK, &[DETECTED])?;
        let drop = stamp(&link, DETECTED);
        if drop.is_some_and(|d| d < cue) {
            return Err("contradictory: the drop is stamped before the unplug cue".into());
        }
        match answer(notes, &UNPLUGGED).as_deref() {
            Ok("unplugged") => {}
            Ok(_) => return Err("the operator could not unplug the trainer".into()),
            Err(why) => return Err(format!("{why} to whether the trainer was unplugged")),
        }
        let at = answered_at(notes, &UNPLUGGED).unwrap_or(cue);
        if at < cue {
            return Err("contradictory: the unplug is answered before its cue".into());
        }
        let ctl = controls(records);
        let end = procedure_end(records, &ctl);
        quiet(
            &ctl,
            cue.saturating_sub(BEFORE_US),
            end,
            "inside the compared windows",
        )?;
        let before = window(records, cue.saturating_sub(BEFORE_US), cue);
        // No frame comes after a drop, so the window ends at the release or the last record.
        let after = window(records, at + SETTLE_US, end);
        let link = drop.map_or_else(
            || {
                format!(
                    "no link drop in {} s unplugged",
                    round1(secs(end.saturating_sub(at)))
                )
            },
            |d| {
                format!(
                    "link dropped {} s after the unplug cue",
                    round1(secs(d - cue))
                )
            },
        );
        let change = |key: &str| format!("{} → {}", before[key], after[key]);
        Ok(json!({
            "method": method,
            "link": link,
            "before": before,
            "after": after,
            "verdict": format!(
                "{link}; page 25 before → after: FE states {}, target limits {}, status bits {}, mean power {} W (ERG {TARGET_W} W)",
                change("fe_states"), change("target_limits"), change("status_bits"), change("mean_power_w")
            ),
        }))
    };
    run().unwrap_or_else(unknown)
}

/// Honoured or not, from one cued window of check 11c (decision table).
fn honoured(w: &Value, rpm: u8) -> String {
    let n = w["samples"].as_u64().unwrap_or(0);
    let cadence = w["mean_cadence_rpm"].as_f64();
    match (w["within_10pct"].as_f64(), cadence) {
        _ if n < MIN_SAMPLES as u64 => format!("unknown: {n} page-25 samples"),
        (_, None) => "unknown: no cadence".into(),
        (_, Some(c)) if (c - f64::from(rpm)).abs() > CADENCE_TOL => {
            format!("unknown: the rider was at {c} rpm, not the cued {rpm}")
        }
        (Some(share), _) if share >= HONOURED * 100.0 => {
            format!("honoured (mean {} W)", w["mean_power_w"])
        }
        (Some(share), _) => format!(
            "not honoured, not_following (mean {} W, {share} % within ±10 %, target limits {})",
            w["mean_power_w"], w["target_limits"]
        ),
        (None, _) => "unknown: no power".into(),
    }
}

/// The result of check 11c from the capture's records and notes.
pub fn check11c(records: &[Record], notes: &[Note]) -> Value {
    let method = "stamps validated first (the start cue, then link 02 seen within 60 s and 04 connected within 30 s of it, the low and 90 rpm cues in order; no frame before the advertisement; the first control write the release), else unknown; times from the start cue: advertising = 02, connected = 04, first page 25, first ERG echo = the first page 71 naming page 49 after the first page-49 write; low/90 rpm = page 25 from 5 s after each cue to the next cue or the release: honoured when >= 10 samples, the cadence within 10 rpm of the cue and >= 80 % within ±10 % of the ERG target";
    let unknown = |why: String| json!({ "method": method, "verdict": format!("unknown: {why}") });
    let run = || -> Result<Value, String> {
        let cues = stamps(notes, WINDOW_C, &[COLD, LOW, NORMAL])?;
        let cold = stamp(&cues, COLD).ok_or("the capture has no start cue")?;
        let link = stamps(notes, LINK, &[SEEN, READY])
            .map_err(|_| "contradictory link stamps: 02 and 04 only, once each, in order")?;
        let seen = stamp(&link, SEEN).ok_or("the capture has no advertisement stamp")?;
        let ready = stamp(&link, READY).ok_or("the capture has no connect stamp")?;
        if seen < cold {
            return Err("contradictory: the advertisement is stamped before the start cue".into());
        }
        let late = |at: u64, from: u64, bound: Duration, what: &str| {
            (at > from + bound.as_secs() * SEC + STAMP_SLACK_US).then(|| {
                format!(
                    "{what} is stamped {} s after, past the {} s bound",
                    round1(secs(at - from)),
                    bound.as_secs()
                )
            })
        };
        if let Some(e) = late(seen, cold, ADVERT_WAIT, "the advertisement")
            .or_else(|| late(ready, seen, CONNECT_WAIT, "the connect"))
        {
            return Err(e);
        }
        if let Some(r) = records.iter().find(|r| r.at_us < seen) {
            return Err(format!(
                "contradictory: a frame is recorded at {} s, before the trainer advertised",
                round1(secs(r.at_us))
            ));
        }
        let ctl = controls(records);
        if let Some((at, c)) = ctl.first().filter(|(_, c)| *c != Control::Resistance(0.0)) {
            return Err(format!(
                "the first write was {c:?} at {} s, not the release",
                round1(secs(*at))
            ));
        }
        let page25 = records
            .iter()
            .find(|r| r.rx && page_of(&r.bytes) == Some(25))
            .map(|r| r.at_us);
        let erg = records
            .iter()
            .find(|r| !r.rx && page_of(&r.bytes) == Some(49))
            .map(|r| r.at_us);
        let echo = erg.and_then(|t| {
            records
                .iter()
                .find(|r| {
                    r.rx && r.at_us > t && command_status(&r.bytes).is_some_and(|c| c.0 == 49)
                })
                .map(|r| r.at_us)
        });
        let end = procedure_end(records, &ctl);
        let judged = |code: u8, next: Option<u64>, rpm: u8| {
            let Some(at) = stamp(&cues, code) else {
                return (Value::Null, format!("unknown: no {rpm} rpm cue"));
            };
            let to = next.unwrap_or(end).min(end);
            // The procedure writes, then stamps the cue: a write at the cue's own µs came first.
            let target = ctl.iter().rev().find(|(t, _)| *t <= at).map(|(_, c)| *c);
            let w = window(records, at + SETTLE_US, to);
            let verdict = match quiet(&ctl, at + 1, to, "inside the window") {
                Err(why) => format!("unknown: {why}"),
                Ok(()) if target != Some(Control::Erg(f64::from(TARGET_W))) => {
                    format!("unknown: the target in force was {target:?}, not ERG {TARGET_W} W")
                }
                Ok(()) => honoured(&w, rpm),
            };
            (w, verdict)
        };
        let (low_w, low) = judged(LOW, stamp(&cues, NORMAL), LOW_RPM);
        let (normal_w, normal) = judged(NORMAL, None, 90);
        let echo = match echo.zip(erg) {
            Some((e, w)) => format!(
                "first ERG echo +{} s ({} s after its page 49)",
                round1(secs(e - cold)),
                round1(secs(e - w))
            ),
            None => "no ERG echo (no page 71 naming page 49)".into(),
        };
        let times = format!(
            "from the start cue: advertising +{} s, connected +{} s, first page 25 +{} s, {echo}",
            round1(secs(seen - cold)),
            round1(secs(ready - cold)),
            page25.map_or("–".into(), |t| round1(secs(t - cold)).to_string()),
        );
        Ok(json!({
            "method": method,
            "times": times,
            "low_cadence": low_w,
            "normal_cadence": normal_w,
            "verdict": format!("{times}; ERG {TARGET_W} W at {LOW_RPM} rpm {low}, at 90 rpm {normal}"),
        }))
    };
    run().unwrap_or_else(unknown)
}

/// Check number, check, result and ADR-0002 section of a check-11 row (11b's from `check11`).
pub fn cells(stem: &str, v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let result = v["verdict"].as_str().unwrap_or("no verdict").to_string();
    match stem {
        "check11a" => ("11a", "Unplug while pedalling", result, "§6"),
        "check11b" => crate::check11::cells(v),
        _ => ("11c", "Unplugged cold start, then connect", result, "§7.5"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_fec::ControlPage;

    fn note(tag: &str, at_ms: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_ms * 1000,
            bytes: bytes.to_vec(),
        }
    }

    fn tx(at_ms: u64, page: &ControlPage) -> Record {
        Record {
            at_us: at_ms * 1000,
            rx: false,
            bytes: crank_fec::encode(page).expect("encodes").to_vec(),
        }
    }

    fn rx(at_ms: u64, mut f: [u8; 13]) -> Record {
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us: at_ms * 1000,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    /// Page 25 at `at_ms`: cadence, power, status bits, target-limit code; FE state 3 (in use).
    fn p25(at_ms: u64, rpm: u8, watts: u16, status: u8, limit: u8) -> Record {
        let w = watts.to_le_bytes();
        rx(
            at_ms,
            [
                0xa4,
                9,
                0x4e,
                5,
                25,
                0,
                rpm,
                0,
                0,
                w[0],
                (w[1] & 15) | status << 4,
                limit | 3 << 4,
                0,
            ],
        )
    }

    const RELEASE: ControlPage = ControlPage::BasicResistance { pct: 0.0 };
    const ERG: ControlPage = ControlPage::TargetPower { watts: 100.0 };

    fn verdict(v: &Value) -> &str {
        v["verdict"].as_str().unwrap_or("")
    }

    /// 11a: ERG 100 W from 5 s, the unplug cue at 30 s, `unplugged` at 31 s, the release at 61.5 s;
    /// page 25 every 0.5 s, from 36 s with `after`'s fields.
    fn ride11a(after: (u16, u8, u8)) -> (Vec<Record>, Vec<Note>) {
        let mut r = vec![tx(0, &RELEASE), tx(5_000, &ERG)];
        r.extend((0..123).map(|k| {
            let at = k * 500;
            let (w, status, limit) = if at >= 36_000 { after } else { (100, 0, 0) };
            p25(at, 90, w, status, limit)
        }));
        r.push(tx(61_500, &RELEASE));
        let n = vec![
            note(WINDOW_A, 30_000, &[UNPLUG]),
            note(&answer_tag("unplugged"), 31_000, b"unplugged"),
        ];
        (r, n)
    }

    #[test]
    fn check11a_compares_page_25_before_and_after_the_unplug() {
        let (r, n) = ride11a((100, 0, 0));
        assert_eq!(
            verdict(&check11a(&r, &n)),
            "no link drop in 30.5 s unplugged; page 25 before → after: FE states [3] → [3], target limits [0] → [0], status bits [0] → [0], mean power 100.0 → 100.0 W (ERG 100 W)"
        );
        // What changed after the unplug shows; the first 5 s after the answer are left out.
        let (r, n) = ride11a((80, 1, 1));
        let v = check11a(&r, &n);
        assert!(
            verdict(&v).contains(
                "target limits [0] → [1], status bits [0] → [1], mean power 100.0 → 80.0 W"
            ),
            "{v}"
        );
        assert_eq!(v["after"]["samples"], json!(51));
        // A drop is the finding: the after window ends at it.
        let (mut r, mut n) = ride11a((100, 0, 0));
        r.retain(|x| x.at_us < 40_000_000);
        n.push(note(LINK, 40_000, &[DETECTED]));
        let v = check11a(&r, &n);
        assert!(
            verdict(&v).starts_with("link dropped 10 s after the unplug cue; "),
            "{v}"
        );
        // The release went with the link: the end is the last record, at 39.5 s.
        assert_eq!(v["after"]["samples"], json!(7));
    }

    #[test]
    fn check11a_without_a_valid_unplug_or_with_a_write_inside_gives_unknown() {
        let base = ride11a((100, 0, 0));
        let with_notes = |n: Vec<Note>| check11a(&base.0, &n);
        let answer = |a: &[u8], at| note(&answer_tag("unplugged"), at, a);
        let cue = note(WINDOW_A, 30_000, &[UNPLUG]);
        for (n, why) in [
            (
                vec![cue.clone(), answer(b"cannot", 31_000)],
                "the operator could not unplug the trainer",
            ),
            (
                vec![cue.clone()],
                "no answer to whether the trainer was unplugged",
            ),
            (
                vec![cue.clone(), answer(b"maybe", 31_000)],
                "unrecognised answer \"maybe\" to whether the trainer was unplugged",
            ),
            (
                vec![answer(b"unplugged", 31_000)],
                "the capture has no unplug cue",
            ),
            (
                vec![cue.clone(), answer(b"unplugged", 29_000)],
                "contradictory: the unplug is answered before its cue",
            ),
            (
                vec![
                    cue.clone(),
                    answer(b"unplugged", 31_000),
                    note(LINK, 20_000, &[DETECTED]),
                ],
                "contradictory: the drop is stamped before the unplug cue",
            ),
            (
                vec![
                    cue.clone(),
                    note(WINDOW_A, 32_000, &[UNPLUG]),
                    answer(b"unplugged", 31_000),
                ],
                "contradictory check11a stamps",
            ),
            (
                vec![
                    cue.clone(),
                    answer(b"unplugged", 31_000),
                    note(LINK, 40_000, &[2]),
                ],
                "contradictory link stamps",
            ),
        ] {
            let v = with_notes(n);
            assert!(
                verdict(&v).starts_with(&format!("unknown: {why}")),
                "{why}: {v}"
            );
        }
        let track = ControlPage::Track {
            grade_pct: 5.0,
            crr: 0.004,
        };
        for (at, page, why) in [
            (
                20_000,
                ControlPage::TargetPower { watts: 120.0 },
                "Erg(120.0) written at 20 s",
            ),
            (
                45_000,
                track,
                "Track(5.0, 0.004000000189989805) written at 45 s",
            ),
        ] {
            let mut r = base.0.clone();
            r.push(tx(at, &page));
            assert_eq!(
                verdict(&check11a(&r, &base.1)),
                format!("unknown: {why}, inside the compared windows")
            );
        }
    }

    /// 11c: the start cue at 0, seen at 8 s, connected at 10 s; the release at 10.5 s, ERG 0 W at 15 s
    /// echoed at 15.4 s, ERG 100 W from 25 s; the 60 rpm cue at 30 s, 90 rpm at 55 s, the release
    /// at 80 s. Page 25 every 0.5 s from 10.2 s: `low` (cadence, power, limit) in the 60 rpm window.
    fn ride11c(low: (u8, u16, u8)) -> (Vec<Record>, Vec<Note>) {
        let mut r = vec![
            tx(10_500, &RELEASE),
            tx(15_000, &ControlPage::TargetPower { watts: 0.0 }),
            tx(15_300, &ControlPage::Request { page: 71, times: 1 }),
            // A page 71 still naming the release comes first; only the one naming page 49 echoes ERG.
            rx(15_200, [0xa4, 9, 0x4e, 5, 0x47, 48, 0, 0, 0, 0, 0, 0, 0]),
            rx(15_400, [0xa4, 9, 0x4e, 5, 0x47, 49, 1, 0, 0, 0, 0, 0, 0]),
            tx(25_000, &ERG),
        ];
        r.extend((0..140).map(|k| {
            let at = 10_200 + k * 500;
            let (rpm, w, limit) = if (30_000..55_000).contains(&at) {
                low
            } else {
                (90, 100, 0)
            };
            p25(at, rpm, w, 0, limit)
        }));
        r.push(tx(80_000, &RELEASE));
        let n = vec![
            note(WINDOW_C, 0, &[COLD]),
            note(LINK, 8_000, &[SEEN]),
            note(LINK, 10_000, &[READY]),
            note(WINDOW_C, 30_000, &[LOW]),
            note(WINDOW_C, 55_000, &[NORMAL]),
        ];
        (r, n)
    }

    #[test]
    fn check11c_times_the_cold_start_and_judges_each_cadence() {
        let (r, n) = ride11c((60, 100, 0));
        assert_eq!(
            verdict(&check11c(&r, &n)),
            "from the start cue: advertising +8 s, connected +10 s, first page 25 +10.2 s, first ERG echo +15.4 s (0.4 s after its page 49); ERG 100 W at 60 rpm honoured (mean 100.0 W), at 90 rpm honoured (mean 100.0 W)"
        );
        let (r, n) = ride11c((60, 60, 1));
        assert!(
            verdict(&check11c(&r, &n)).contains(
                "at 60 rpm not honoured, not_following (mean 60.0 W, 0 % within ±10 %, target limits [1]), at 90 rpm honoured"
            ),
            "{}",
            check11c(&r, &n)
        );
        // Decision table: off the cue, too few samples, no echo.
        let (r, n) = ride11c((80, 100, 0));
        assert!(
            verdict(&check11c(&r, &n))
                .contains("at 60 rpm unknown: the rider was at 80 rpm, not the cued 60")
        );
        let (r, mut n) = ride11c((60, 100, 0));
        n[4] = note(WINDOW_C, 38_000, &[NORMAL]);
        assert!(verdict(&check11c(&r, &n)).contains("at 60 rpm unknown: 6 page-25 samples"));
        let (mut r, n) = ride11c((60, 100, 0));
        r.retain(|x| page_of(&x.bytes) != Some(0x47));
        assert!(
            verdict(&check11c(&r, &n))
                .contains("first page 25 +10.2 s, no ERG echo (no page 71 naming page 49); ")
        );
    }

    #[test]
    fn check11c_stamps_or_writes_that_break_the_procedure_give_unknown() {
        let (r, n) = ride11c((60, 100, 0));
        let with = |i: usize, x: Note| {
            let mut m = n.clone();
            m[i] = x;
            m
        };
        let removed = |i: usize| {
            let mut m = n.clone();
            m.remove(i);
            m
        };
        for (notes, why) in [
            (
                [
                    &n[..1],
                    &[note(LINK, 60_600, &[SEEN]), note(LINK, 61_000, &[READY])],
                    &n[3..],
                ]
                .concat(),
                "the advertisement is stamped 60.6 s after, past the 60 s bound",
            ),
            (
                [
                    &n[..1],
                    &[note(LINK, 10_300, &[SEEN]), note(LINK, 10_400, &[READY])],
                    &n[3..],
                ]
                .concat(),
                "contradictory: a frame is recorded at 10.2 s, before the trainer advertised",
            ),
            (
                with(0, note(WINDOW_C, 9_000, &[COLD])),
                "contradictory: the advertisement is stamped before the start cue",
            ),
            (
                with(2, note(LINK, 38_600, &[READY])),
                "the connect is stamped 30.6 s after, past the 30 s bound",
            ),
            (
                with(2, note(LINK, 7_000, &[READY])),
                "contradictory link stamps",
            ),
            (
                with(2, note(LINK, 10_000, &[6])),
                "contradictory link stamps",
            ),
            (removed(2), "the capture has no connect stamp"),
            (removed(1), "the capture has no advertisement stamp"),
            (removed(0), "the capture has no start cue"),
            (
                with(4, note(WINDOW_C, 25_000, &[NORMAL])),
                "contradictory check11c stamps",
            ),
        ] {
            let v = check11c(&r, &notes);
            assert!(
                verdict(&v).starts_with(&format!("unknown: {why}")),
                "{why}: {v}"
            );
        }
        // The first write must be the release; the target in force must be ERG 100 W; nothing is
        // written inside a judged window.
        let mut first = r.clone();
        first.insert(0, tx(10_400, &ERG));
        assert!(
            verdict(&check11c(&first, &n))
                .starts_with("unknown: the first write was Erg(100.0) at 10.4 s, not the release")
        );
        let mut other = r.clone();
        other.push(tx(26_000, &ControlPage::TargetPower { watts: 80.0 }));
        assert!(
            verdict(&check11c(&other, &n)).contains(
                "at 60 rpm unknown: the target in force was Some(Erg(80.0)), not ERG 100 W"
            )
        );
        let mut inside = r.clone();
        inside.push(tx(40_000, &ControlPage::TargetPower { watts: 120.0 }));
        assert!(
            verdict(&check11c(&inside, &n))
                .contains("at 60 rpm unknown: Erg(120.0) written at 40 s, inside the window")
        );
    }

    #[test]
    fn check11c_the_bounds_edges_with_the_stamping_slack_are_valid() {
        let (r, n) = ride11c((60, 100, 0));
        let with = |i: usize, x: Note| {
            let mut m = n.clone();
            m[i] = x;
            m
        };
        // The bounds' edges, with the stamping slack, are valid.
        // (With the advert at 60.5 s, the frames are those after it.)
        let late: Vec<Record> = r
            .iter()
            .filter(|x| x.at_us >= 60_500_000)
            .cloned()
            .collect();
        for (records, notes) in [
            (
                late,
                [
                    &n[..1],
                    &[note(LINK, 60_500, &[SEEN]), note(LINK, 61_000, &[READY])],
                    &n[3..],
                ]
                .concat(),
            ),
            (r.clone(), with(2, note(LINK, 38_500, &[READY]))),
        ] {
            assert!(
                verdict(&check11c(&records, &notes))
                    .starts_with("from the start cue: advertising "),
                "{notes:?}"
            );
        }
    }
}
