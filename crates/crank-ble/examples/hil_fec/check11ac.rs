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
    partb::{Control, SEC, controls, mean, released_at, round1, secs, share_within},
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
/// The evidence 11a needs, from check 5's floor of 2 page-25 samples a second. Before: the
/// 15 s window at that rate (30 samples). After, for "no drop": the procedure rides
/// [`AFTER`] (30 s) after the answer, 25 s once [`SETTLE_US`] is left out; at least 20 s of it
/// (5 s of slack for the answer's and the release's own timing), at least 40 samples.
const MIN_BEFORE_SAMPLES: u64 = 30;
const MIN_AFTER_US: u64 = 20 * SEC;
const MIN_AFTER_SAMPLES: u64 = 40;
/// A link whose peer has gone is reported down only when the supervision timeout runs out. The
/// Bluetooth Core Specification bounds `connSupervisionTimeout` at 100 ms to 32 s (Vol 6, Part B,
/// §4.5.2), so a powered-off trainer's link is down at most this long after its last packet;
/// Apple's guideline ranges for the value have differed between releases, the Core bound is the
/// one every link meets.
pub const SUPERVISION_MAX: Duration = Duration::from_secs(32);

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

/// The first control write in `from..to`, the release included, as an error naming it.
fn quiet(ctl: &[(u64, Control)], (from, to): (u64, u64), inside: &str) -> Result<(), String> {
    match ctl.iter().find(|(at, _)| *at >= from && *at < to) {
        Some((at, c)) => Err(format!(
            "{c:?} written at {} s, {inside}",
            round1(secs(*at))
        )),
        None => Ok(()),
    }
}

/// With a drop at `d`, the writes and frames around it must be those of a run: the closing
/// release comes at or after the drop (a write at the drop's µs is after its stamp), and a
/// release before the drop (`end < d`) is the silence guard's, so it follows at least the guard's
/// [`crate::rider::SILENCE_US`] without a page-25 frame and nothing is written from it to the
/// drop. Anything else is no result.
fn drop_evidence(
    records: &[Record],
    ctl: &[(u64, Control)],
    end: u64,
    d: u64,
) -> Result<(), String> {
    use crate::rider::SILENCE_US;
    if released_at(ctl).is_some_and(|closing| d > closing) {
        return Err("contradictory: the drop is stamped after the closing release".into());
    }
    if end < d {
        quiet(
            ctl,
            (end + 1, d),
            "between the silence guard's release and the drop",
        )?;
        if let Some(r) = records
            .iter()
            .rev()
            .find(|r| r.rx && r.at_us <= end && page_of(&r.bytes) == Some(25))
            .filter(|r| r.at_us + SILENCE_US > end)
        {
            return Err(format!(
                "the release before the drop, at {} s, is not the silence guard's: page 25 came {} s before it (the guard waits {} s)",
                round1(secs(end)),
                round1(secs(end - r.at_us)),
                round1(secs(SILENCE_US))
            ));
        }
    }
    Ok(())
}

/// When the operator answered [`UNPLUGGED`] with `unplugged`, once and after the `cue`; any
/// other answer, none or a second one is no result (decision table).
fn unplugged_at(notes: &[Note], cue: u64) -> Result<u64, String> {
    let answers = notes
        .iter()
        .filter(|n| n.tag == answer_tag(UNPLUGGED.key))
        .count();
    if answers > 1 {
        return Err(format!(
            "contradictory: the unplug question is answered {answers} times"
        ));
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
    Ok(at)
}

/// When the operator answered `q` (the last answer, as [`answer`] reads it).
fn answered_at(notes: &[Note], q: &Question) -> Option<u64> {
    let tag = answer_tag(q.key);
    notes.iter().rev().find(|n| n.tag == tag).map(|n| n.at_us)
}

/// The result of check 11a from the capture's records and notes.
pub fn check11a(records: &[Record], notes: &[Note]) -> Value {
    let method = "stamps validated first (the unplug cue once, at most one drop after it and no trainer frame at or after the drop, the drop at or before the closing release, a release before the drop only as the silence guard's (at least 2 s after the last page 25, nothing written from it to the drop), one answer, ERG 100 W in force before the windows, no control write from 15 s before the cue to the first release, which comes after the cue (none: the capture is cut off, unknown); the after window ends at that release; at least 30 page-25 samples before; without a drop the answer before the release and at least 40 samples over 20 s after), else unknown; unplugged = the operator's answer (decision table); drop = a link 01 stamp after the cue; before = page 25 in the 15 s up to the cue; after = page 25 from 5 s after the answer to the release; each: FE states, target-limit codes, status bits, mean power and share within ±10 % of the ERG target";
    let unknown = |why: String| json!({ "method": method, "verdict": format!("unknown: {why}") });
    let run = || -> Result<Value, String> {
        let cue = stamp(&stamps(notes, WINDOW_A, &[UNPLUG])?, UNPLUG)
            .ok_or("the capture has no unplug cue")?;
        let link = stamps(notes, LINK, &[DETECTED])?;
        let drop = stamp(&link, DETECTED);
        if drop.is_some_and(|d| d < cue) {
            return Err("contradictory: the drop is stamped before the unplug cue".into());
        }
        let at = unplugged_at(notes, cue)?;
        let ctl = controls(records);
        let from = cue.saturating_sub(BEFORE_US);
        // The target in force through both windows: the last write before them.
        let target = ctl.iter().rev().find(|(t, _)| *t < from).map(|(_, c)| *c);
        if target != Some(Control::Erg(f64::from(TARGET_W))) {
            return Err(format!(
                "the target in force was {target:?}, not ERG {TARGET_W} W"
            ));
        }
        // The first release from the window start ends the after window: the run's own, or the
        // silence guard's, both after the cue. One at or before the cue changed the target inside
        // the before window; none at all means the capture was cut off before the run's end
        // (every release is recorded before it is written, a failed one too).
        let end = ctl
            .iter()
            .find(|(t, c)| *t >= from && *c == Control::Resistance(0.0))
            .map(|(t, _)| *t)
            .ok_or("no release write after the unplug cue: the capture is cut off before the run's end")?;
        if end <= cue {
            return Err(format!(
                "Resistance(0.0) written at {} s, at or before the unplug cue",
                round1(secs(end))
            ));
        }
        quiet(&ctl, (from, end), "inside the compared windows")?;
        if let Some(d) = drop {
            drop_evidence(records, &ctl, end, d)?;
        }
        // An rx frame at the drop's own µs counts as after it: the stamp is written when the
        // drop is reported, so the trainer's frames must have ended before it.
        if let Some(d) = drop
            && let Some(r) = records.iter().find(|r| r.rx && r.at_us >= d)
        {
            return Err(format!(
                "contradictory: the trainer still sent a frame {} s after the stamped drop",
                round1(secs(r.at_us - d))
            ));
        }
        let before = window(records, from, cue);
        let after = window(records, at + SETTLE_US, end);
        let count = |w: &Value| w["samples"].as_u64().unwrap_or(0);
        if count(&before) < MIN_BEFORE_SAMPLES {
            return Err(format!(
                "{} page-25 samples in the 15 s before the cue (at least {MIN_BEFORE_SAMPLES})",
                count(&before)
            ));
        }
        if drop.is_none() {
            if at >= end {
                return Err("the unplug is answered at or after the release".into());
            }
            let span = end.saturating_sub(at + SETTLE_US);
            if span < MIN_AFTER_US || count(&after) < MIN_AFTER_SAMPLES {
                return Err(format!(
                    "{} page-25 samples over {} s after the unplug (at least {MIN_AFTER_SAMPLES} over {} s)",
                    count(&after),
                    round1(secs(span)),
                    MIN_AFTER_US / SEC
                ));
            }
        }
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
    let method = "stamps validated first (the start cue, then link 02 seen within 60 s and 04 connected within 30 s of it, the low and 90 rpm cues in order; no frame before the advertisement; the first control write the release, and a release ends the run (none: the capture is cut off)), else unknown; times from the start cue: advertising = 02, connected = 04, first page 25, first ERG echo = the first page 71 naming page 49 after the first page-49 write; low/90 rpm = page 25 from 5 s after each cue to the next cue or the release: honoured when >= 10 samples, the cadence within 10 rpm of the cue and >= 80 % within ±10 % of the ERG target";
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
        let end = released_at(&ctl)
            .ok_or("no release write ends the run: the capture is cut off before the run's end")?;
        let judged = |code: u8, next: Option<u64>, rpm: u8| {
            let Some(at) = stamp(&cues, code) else {
                return (Value::Null, format!("unknown: no {rpm} rpm cue"));
            };
            let to = next.unwrap_or(end).min(end);
            // The procedure writes, then stamps the cue: a write at the cue's own µs came first.
            let target = ctl.iter().rev().find(|(t, _)| *t <= at).map(|(_, c)| *c);
            let w = window(records, at + SETTLE_US, to);
            let verdict = match quiet(&ctl, (at + 1, to), "inside the window") {
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
        r.push(tx(41_000, &RELEASE));
        n.push(note(LINK, 40_000, &[DETECTED]));
        let v = check11a(&r, &n);
        assert!(
            verdict(&v).starts_with("link dropped 10 s after the unplug cue; "),
            "{v}"
        );
        // The release, recorded although the link is down, ends the after window: 36 to 39.5 s.
        assert_eq!(v["after"]["samples"], json!(8));
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

    /// The c3 review's 11a repros: no ERG in force, empty or late evidence, frames after a drop.
    /// Each was a full row before; each is `unknown` with its reason now.
    #[test]
    fn check11a_needs_erg_in_force_enough_evidence_and_a_drop_the_frames_end_at() {
        let (r, n) = ride11a((100, 0, 0));
        let without = |page: u8| -> Vec<Record> {
            r.iter()
                .filter(|x| x.rx || page_of(&x.bytes) != Some(page))
                .cloned()
                .collect()
        };
        let mut resistance = without(49);
        resistance.push(tx(5_000, &ControlPage::BasicResistance { pct: 30.0 }));
        let release_at = |ms: u64| -> Vec<Record> {
            let mut v: Vec<Record> = r.iter().filter(|x| x.at_us < ms * 1000).cloned().collect();
            v.push(tx(ms, &RELEASE));
            v
        };
        let late_answer = [
            n[..1].to_vec(),
            vec![note(&answer_tag("unplugged"), 62_000, b"unplugged")],
        ]
        .concat();
        let with_drop = |at: u64| [n.clone(), vec![note(LINK, at, &[DETECTED])]].concat();
        let silent: Vec<Record> = r.iter().filter(|x| !x.rx).cloned().collect();
        for (records, notes, why) in [
            (
                without(49),
                n.clone(),
                "the target in force was Some(Resistance(0.0)), not ERG 100 W",
            ),
            (
                resistance,
                n.clone(),
                "the target in force was Some(Resistance(30.0)), not ERG 100 W",
            ),
            (
                vec![],
                n.clone(),
                "the target in force was None, not ERG 100 W",
            ),
            (
                silent,
                n.clone(),
                "0 page-25 samples in the 15 s before the cue (at least 30)",
            ),
            (
                r.clone(),
                late_answer,
                "the unplug is answered at or after the release",
            ),
            (
                release_at(45_000),
                n.clone(),
                "18 page-25 samples over 9 s after the unplug (at least 40 over 20 s)",
            ),
            (
                release_at(55_500),
                n.clone(),
                "39 page-25 samples over 19.5 s after the unplug (at least 40 over 20 s)",
            ),
            (
                r.clone(),
                with_drop(40_000),
                "contradictory: the trainer still sent a frame 0 s after the stamped drop",
            ),
        ] {
            let v = check11a(&records, &notes);
            assert_eq!(verdict(&v), format!("unknown: {why}"), "{why}");
        }
        // The floors' edges hold: 30 samples before (the ride's own), 40 samples over 20 s after,
        // and a drop at the cue's own microsecond.
        assert!(
            verdict(&check11a(&release_at(56_000), &n))
                .starts_with("no link drop in 25 s unplugged")
        );
        // (A drop's capture still records its failed release.)
        let mut early: Vec<Record> = r.iter().filter(|x| x.at_us < 30_000_000).cloned().collect();
        early.push(tx(31_000, &RELEASE));
        assert!(
            verdict(&check11a(&early, &with_drop(30_000)))
                .starts_with("link dropped 0 s after the unplug cue")
        );
    }

    /// The r11 repros, each a full row on 32e93ea: with a drop, a write between the first release
    /// and the drop, a release that page 25 does not justify as the silence guard's, and a drop
    /// stamped after the closing release; and the edges of each rule.
    #[test]
    fn check11a_a_drop_needs_the_guards_release_and_no_write_up_to_it() {
        use crate::rider::SILENCE_US;
        let (r, n) = ride11a((100, 0, 0));
        let at_us = |at_us: u64, page: &ControlPage| Record {
            at_us,
            ..tx(0, page)
        };
        // Page 25 until `rx_to` µs, then `extra`, sorted.
        let ride = |rx_to: u64, extra: Vec<Record>| -> Vec<Record> {
            let mut v: Vec<Record> = r
                .iter()
                .filter(|x| x.at_us < rx_to && (x.rx || x.at_us < 30_000_000))
                .cloned()
                .collect();
            v.extend(extra);
            v.sort_by_key(|x| x.at_us);
            v
        };
        let drop_at = |at: u64| [n.clone(), vec![note(LINK, at, &[DETECTED])]].concat();
        let erg300 = ControlPage::TargetPower { watts: 300.0 };
        // The silence guard's shape: page 25 ends at 44.5 s, the release `gap` after it, the
        // drop at 50 s, the closing release at 51 s.
        let guarded = |gap: u64, extra: Vec<Record>| {
            let mut e = vec![at_us(44_500_000 + gap, &RELEASE), tx(51_000, &RELEASE)];
            e.extend(extra);
            ride(45_000_000, e)
        };
        for (records, notes, why) in [
            (
                ride(
                    40_000_000,
                    vec![
                        tx(31_000, &RELEASE),
                        tx(32_000, &erg300),
                        tx(41_000, &RELEASE),
                    ],
                ),
                drop_at(40_000),
                "Erg(300.0) written at 32 s, between the silence guard's release and the drop",
            ),
            (
                ride(
                    40_000_000,
                    vec![at_us(30_000_001, &RELEASE), tx(41_000, &RELEASE)],
                ),
                drop_at(40_000),
                "the release before the drop, at 30 s, is not the silence guard's: page 25 came 0 s before it (the guard waits 2 s)",
            ),
            (
                guarded(SILENCE_US - 1, vec![]),
                drop_at(50_000),
                "the release before the drop, at 46.5 s, is not the silence guard's: page 25 came 2 s before it (the guard waits 2 s)",
            ),
            (
                guarded(SILENCE_US, vec![at_us(49_999_999, &ERG)]),
                drop_at(50_000),
                "Erg(100.0) written at 50 s, between the silence guard's release and the drop",
            ),
            (
                r.clone(),
                drop_at(70_000),
                "contradictory: the drop is stamped after the closing release",
            ),
        ] {
            assert_eq!(
                verdict(&check11a(&records, &notes)),
                format!("unknown: {why}"),
                "{why}"
            );
        }
        // The edges that are valid: the guard's release exactly 2 s after the last page 25, a
        // write at the drop's own µs (after its stamp), a drop at the closing release's µs.
        for (records, notes, link) in [
            (
                guarded(SILENCE_US, vec![]),
                drop_at(50_000),
                "link dropped 20 s",
            ),
            (
                guarded(SILENCE_US, vec![tx(50_000, &ERG)]),
                drop_at(50_000),
                "link dropped 20 s",
            ),
            (r.clone(), drop_at(61_500), "link dropped 31.5 s"),
        ] {
            let v = check11a(&records, &notes);
            assert!(verdict(&v).starts_with(link), "{link}: {v}");
        }
    }

    /// The after window's sample floor is exactly 40 (over a 25.5 s span), and the unplug
    /// question is answered once: two answers, whatever they are, are contradictory.
    #[test]
    fn check11a_the_40_sample_floor_and_one_answer() {
        let (r, n) = ride11a((100, 0, 0));
        let after_to = |to_ms: u64| -> Vec<Record> {
            r.iter()
                .filter(|x| !x.rx || x.at_us < 36_000_000 || x.at_us < to_ms * 1000)
                .cloned()
                .collect()
        };
        assert_eq!(
            verdict(&check11a(&after_to(55_500), &n)),
            "unknown: 39 page-25 samples over 25.5 s after the unplug (at least 40 over 20 s)"
        );
        assert!(
            verdict(&check11a(&after_to(56_000), &n)).starts_with("no link drop in 30.5 s"),
            "40 samples"
        );
        let cue = note(WINDOW_A, 30_000, &[UNPLUG]);
        let answer = |a: &[u8], at| note(&answer_tag("unplugged"), at, a);
        for second in [&b"unplugged"[..], b"cannot"] {
            let notes = vec![
                cue.clone(),
                answer(b"cannot", 31_000),
                answer(second, 32_000),
            ];
            assert_eq!(
                verdict(&check11a(&r, &notes)),
                "unknown: contradictory: the unplug question is answered 2 times"
            );
        }
        let twice = vec![
            cue,
            answer(b"unplugged", 31_000),
            answer(b"unplugged", 31_500),
        ];
        assert_eq!(
            verdict(&check11a(&r, &twice)),
            "unknown: contradictory: the unplug question is answered 2 times"
        );
    }

    /// Each 11a evidence floor on its own, the early release, and a real drop's capture shape.
    #[test]
    fn check11a_each_evidence_floor_holds_on_its_own() {
        let (r, n) = ride11a((100, 0, 0));
        let release_at = |ms: u64| -> Vec<Record> {
            let mut v: Vec<Record> = r.iter().filter(|x| x.at_us < ms * 1000).cloned().collect();
            v.push(tx(ms, &RELEASE));
            v
        };
        let with_drop = |at: u64| [n.clone(), vec![note(LINK, at, &[DETECTED])]].concat();
        // Each floor on its own: 29 samples before; 78 samples but over 19.5 s; 25.5 s but 26
        // samples; a mid-window release (the silence guard's) ends the after window at 50 s.
        let mut few_before = r.clone();
        few_before.retain(|x| !(x.rx && x.at_us == 15_000_000));
        let mut dense = release_at(55_500);
        dense.extend((0..39).map(|k| p25(36_250 + k * 500, 90, 100, 0, 0)));
        let mut sparse = r.clone();
        sparse.retain(|x| !(x.rx && x.at_us >= 36_000_000 && x.at_us % 1_000_000 != 0));
        let mut early_release = r.clone();
        early_release.push(tx(50_000, &RELEASE));
        for (records, why) in [
            (
                few_before,
                "29 page-25 samples in the 15 s before the cue (at least 30)",
            ),
            (
                dense,
                "78 page-25 samples over 19.5 s after the unplug (at least 40 over 20 s)",
            ),
            (
                sparse,
                "26 page-25 samples over 25.5 s after the unplug (at least 40 over 20 s)",
            ),
            (
                early_release,
                "28 page-25 samples over 14 s after the unplug (at least 40 over 20 s)",
            ),
        ] {
            assert_eq!(
                verdict(&check11a(&records, &n)),
                format!("unknown: {why}"),
                "{why}"
            );
        }
        // A real drop's capture still holds the release tx after the drop (tx frames are recorded
        // even when the write fails): only trainer frames count against the drop.
        let mut dropped: Vec<Record> = r.iter().filter(|x| x.at_us < 40_000_000).cloned().collect();
        dropped.push(tx(41_000, &RELEASE));
        assert!(
            verdict(&check11a(&dropped, &with_drop(40_000)))
                .starts_with("link dropped 10 s after the unplug cue")
        );
    }

    /// The r10 repros: a write from the window start (15 s) to the cue (30 s) changes the
    /// target the before window is read under, the release included, cue µs included; a capture
    /// cut off before its release; a trainer frame at the drop's own µs. Each gave a full row.
    #[test]
    fn check11a_no_write_from_the_window_start_to_the_cue_and_a_release_after_it() {
        let (r, n) = ride11a((100, 0, 0));
        let with = |at: u64, page: &ControlPage| -> Vec<Record> {
            let mut v = r.clone();
            v.push(tx(at, page));
            v.sort_by_key(|x| x.at_us);
            v
        };
        let half = ControlPage::TargetPower { watts: 50.0 };
        for (records, why) in [
            (
                with(15_000, &RELEASE),
                "Resistance(0.0) written at 15 s, at or before the unplug cue",
            ),
            (
                with(20_000, &RELEASE),
                "Resistance(0.0) written at 20 s, at or before the unplug cue",
            ),
            (
                with(30_000, &RELEASE),
                "Resistance(0.0) written at 30 s, at or before the unplug cue",
            ),
            (
                with(15_000, &half),
                "Erg(50.0) written at 15 s, inside the compared windows",
            ),
            (
                with(15_000, &ERG),
                "Erg(100.0) written at 15 s, inside the compared windows",
            ),
            (
                with(14_999, &half),
                "the target in force was Some(Erg(50.0)), not ERG 100 W",
            ),
        ] {
            assert_eq!(
                verdict(&check11a(&records, &n)),
                format!("unknown: {why}"),
                "{why}"
            );
        }
        // The unplug answered at the release's own µs leaves no after window.
        let late = vec![
            note(WINDOW_A, 30_000, &[UNPLUG]),
            note(&answer_tag("unplugged"), 61_500, b"unplugged"),
        ];
        assert_eq!(
            verdict(&check11a(&r, &late)),
            "unknown: the unplug is answered at or after the release"
        );
        // Cut off before the release: no end, no result, offline as live.
        let cut: Vec<Record> = r.iter().filter(|x| x.at_us < 61_500_000).cloned().collect();
        assert_eq!(
            verdict(&check11a(&cut, &n)),
            "unknown: no release write after the unplug cue: the capture is cut off before the run's end"
        );
        let none = crate::partb::PowerLag::NotMeasured("test".into());
        let v = crate::partb::analyse(
            "check11a",
            &cut,
            &[],
            &n,
            Some(crate::partb::COMPLETED),
            &none,
        )
        .expect("stem");
        assert_eq!(
            v["invalid"],
            "the capture is cut off: no release write ends it"
        );
        assert_eq!(v["verdict"], Value::Null);
        // A trainer frame at the drop's µs is after it: the stamp follows the frames' end.
        let mut dropped: Vec<Record> = r.iter().filter(|x| x.at_us < 40_000_000).cloned().collect();
        dropped.push(tx(41_000, &RELEASE));
        let drop_at = |at: u64| [n.clone(), vec![note(LINK, at, &[DETECTED])]].concat();
        assert_eq!(
            verdict(&check11a(&dropped, &drop_at(39_500))),
            "unknown: contradictory: the trainer still sent a frame 0 s after the stamped drop"
        );
        assert!(verdict(&check11a(&dropped, &drop_at(39_501))).starts_with("link dropped "));
    }

    /// The 80 % boundary of "honoured" is inclusive.
    #[test]
    fn honoured_from_80_percent_within_10_percent() {
        let w = |share: f64| json!({"samples": 40, "mean_cadence_rpm": 60.0, "within_10pct": share, "mean_power_w": 95.0, "target_limits": [0]});
        assert_eq!(honoured(&w(80.0), 60), "honoured (mean 95.0 W)");
        assert!(honoured(&w(79.9), 60).starts_with("not honoured, not_following"));
    }

    /// An 11c capture cut off before its closing release gives no verdict.
    #[test]
    fn check11c_cut_off_before_its_release_is_unknown() {
        let (mut r, n) = ride11c((60, 100, 0));
        r.retain(|x| x.at_us < 80_000_000);
        assert_eq!(
            verdict(&check11c(&r, &n)),
            "unknown: no release write ends the run: the capture is cut off before the run's end"
        );
    }

    /// A run that did not complete has no 11a or 11c result fields, only `unknown` reasons.
    #[test]
    fn a_stopped_11a_or_11c_run_carries_no_result_fields() {
        let none = crate::partb::PowerLag::NotMeasured("test".into());
        let (r, n) = ride11a((100, 0, 0));
        let v = crate::partb::analyse("check11a", &r, &[], &n, Some("aborted: x"), &none)
            .expect("stem");
        for key in ["verdict", "link", "before", "after"] {
            assert_eq!(v[key], Value::Null, "{key}: {v}");
        }
        let (r, n) = ride11c((60, 100, 0));
        let v = crate::partb::analyse("check11c", &r, &[], &n, Some("aborted: x"), &none)
            .expect("stem");
        for key in ["verdict", "times", "low_cadence", "normal_cadence"] {
            assert_eq!(v[key], Value::Null, "{key}: {v}");
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
        // (With the advert at 60.5 s, the frames are those after it, and the opening release.)
        let mut late: Vec<Record> = r
            .iter()
            .filter(|x| x.at_us >= 60_500_000)
            .cloned()
            .collect();
        late.insert(0, tx(61_200, &RELEASE));
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
