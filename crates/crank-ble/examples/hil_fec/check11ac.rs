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
    frames::{Note, Stamp, stamp_of, stamps},
    partb::{Control, SEC, Sample, Write, mean, round1, secs, share_within},
    timeline::Timeline,
};
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

/// The page-25 updates at frame indices `from..to` (none when `from` is past `to`).
fn span(tl: &Timeline, from: usize, to: usize) -> Vec<Sample> {
    tl.updates()
        .into_iter()
        .filter(|s| s.pos >= from && s.pos < to)
        .collect()
}

/// What page 25 reported in `samples`: the FE states, target-limit codes and status bits seen,
/// and the power and cadence against `target`.
fn window(samples: &[Sample]) -> Value {
    let codes = |f: fn(&Sample) -> u8| {
        let mut v: Vec<u8> = samples.iter().map(f).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let power: Vec<f64> = samples
        .iter()
        .filter_map(|d| d.power.map(f64::from))
        .collect();
    json!({
        "samples": samples.len(),
        "fe_states": codes(|d| d.state),
        "target_limits": codes(|d| d.target_limit),
        "status_bits": codes(|d| d.status),
        "mean_power_w": mean(power.iter().copied()).map(round1),
        "within_10pct": share_within(&power, f64::from(TARGET_W), 0.10).map(|x| round1(x * 100.0)),
        "mean_cadence_rpm": mean(samples.iter().filter_map(|d| d.cadence.map(f64::from))).map(round1),
    })
}

/// The first control write at a frame index in `from..to`, the release included, as an error.
fn quiet(ctl: &[Write], (from, to): (usize, usize), inside: &str) -> Result<(), String> {
    match ctl.iter().find(|w| (from..to).contains(&w.pos)) {
        Some(w) => Err(format!(
            "{:?} written at {} s, {inside}",
            w.control,
            round1(secs(w.at_us))
        )),
        None => Ok(()),
    }
}

/// With a drop `d` (after the cue, checked by [`check11a`]), the closing release comes after it,
/// and a release `end` before it in the timeline is the silence guard's: [`crate::rider::silent`] held
/// from the last page 25 before it, and nothing is written from it to the drop. Anything else is
/// no result.
fn drop_evidence(tl: &Timeline, end: &Write, d: &Stamp) -> Result<(), String> {
    if tl.closing_release().is_some_and(|c| c.pos < d.before) {
        return Err("contradictory: the drop is stamped after the closing release".into());
    }
    // The stamp is written when the drop is reported: the trainer's frames ended before it.
    if let Some(at) = tl
        .samples()
        .iter()
        .map(|s| (s.pos, s.at_us))
        .chain(tl.page16().iter().map(|(s, _)| (s.pos, s.at_us)))
        .chain(tl.statuses().iter().map(|s| (s.pos, s.at_us)))
        .find(|(pos, _)| *pos >= d.before)
        .map(|(_, at)| at)
    {
        return Err(format!(
            "contradictory: the trainer still sent a frame {} s after the stamped drop",
            round1(secs(at.saturating_sub(d.at_us)))
        ));
    }
    let ctl = tl.writes();
    if end.pos >= d.before {
        return Ok(());
    }
    quiet(
        ctl,
        (end.pos + 1, d.before),
        "between the silence guard's release and the drop",
    )?;
    // Every page 25 counts, a repeat too: the live guard sees it.
    if let Some(r) = tl
        .samples()
        .iter()
        .rfind(|s| s.pos < end.pos)
        .filter(|s| !crate::rider::silent(s.at_us, end.at_us))
    {
        return Err(format!(
            "the release before the drop, at {} s, is not the silence guard's: page 25 came {} s before it (the guard waits more than {} s)",
            round1(secs(end.at_us)),
            round1(secs(end.at_us - r.at_us)),
            round1(secs(crate::rider::SILENCE_US))
        ));
    }
    Ok(())
}

/// The operator's answer to [`UNPLUGGED`], once and after the `cue`; only `unplugged` makes 11a a
/// result, and any other answer, none or a second one is no result (decision table).
fn unplugged<'a>(notes: &'a [Note], cue: &Stamp) -> Result<&'a Note, String> {
    let tag = answer_tag(UNPLUGGED.key);
    let answers: Vec<usize> = (0..notes.len()).filter(|i| notes[*i].tag == tag).collect();
    if answers.len() > 1 {
        return Err(format!(
            "contradictory: the unplug question is answered {} times",
            answers.len()
        ));
    }
    if answers.first().is_some_and(|i| *i < cue.seq) {
        return Err("contradictory: the unplug is answered before its cue".into());
    }
    match answer(notes, &UNPLUGGED).as_deref() {
        Ok("unplugged") => {}
        Ok(_) => return Err("the operator could not unplug the trainer".into()),
        Err(why) => return Err(format!("{why} to whether the trainer was unplugged")),
    }
    notes
        .iter()
        .find(|n| n.tag == tag)
        .ok_or_else(|| "no answer to whether the trainer was unplugged".into())
}

/// The result of check 11a from the capture's timeline.
pub fn check11a(tl: &Timeline) -> Value {
    let method = "stamps validated first, in timeline order (the unplug cue once, at most one drop after it and no trainer frame after the drop, the drop before the closing release, a release before the drop only as the silence guard's (more than 2 s after the last page 25 before it, nothing written from it to the drop), one answer after the cue and before the closing release, ERG 100 W in force before the windows, no control write from 15 s before the cue to the first release, which comes after the cue (none: the capture is cut off, unknown); the after window ends at that release; at least 30 page-25 samples before; without a drop the answer before the release and at least 40 samples over 20 s after), else unknown; unplugged = the operator's answer (decision table); drop = a link 01 stamp after the cue; before = page 25 in the 15 s up to the cue; after = page 25 from 5 s after the answer to the release; each: FE states, target-limit codes, status bits, mean power and share within ±10 % of the ERG target";
    let unknown = |why: String| json!({ "method": method, "verdict": format!("unknown: {why}") });
    let run = || -> Result<Value, String> {
        let notes = tl.notes();
        let cue = stamp_of(&stamps(notes, WINDOW_A, &[UNPLUG])?, UNPLUG)
            .ok_or("the capture has no unplug cue")?;
        let drop = stamp_of(&stamps(notes, LINK, &[DETECTED])?, DETECTED);
        if drop.is_some_and(|d| d.seq < cue.seq) {
            return Err("contradictory: the drop is stamped before the unplug cue".into());
        }
        let at = unplugged(notes, &cue)?;
        let ctl = tl.writes();
        let from = tl.index_at(cue.at_us.saturating_sub(BEFORE_US));
        // The target in force through both windows: the last write before them.
        let target = ctl.iter().rev().find(|w| w.pos < from).map(|w| w.control);
        if target != Some(Control::Erg(f64::from(TARGET_W))) {
            return Err(format!(
                "the target in force was {target:?}, not ERG {TARGET_W} W"
            ));
        }
        // The first release from the window start ends the after window: the run's own, or the
        // silence guard's, both after the cue. One before the cue changed the target inside the
        // before window; none at all means the capture was cut off before the run's end (every
        // release is recorded before it is written, a failed one too).
        let end = ctl
            .iter()
            .find(|w| w.pos >= from && w.control == Control::Resistance(0.0))
            .copied()
            .ok_or("no release write after the unplug cue: the capture is cut off before the run's end")?;
        if end.pos < cue.before {
            return Err(format!(
                "Resistance(0.0) written at {} s, before the unplug cue",
                round1(secs(end.at_us))
            ));
        }
        quiet(ctl, (from, end.pos), "inside the compared windows")?;
        if let Some(d) = drop {
            drop_evidence(tl, &end, &d)?;
            // The answer before the closing release (ADR-0007 §4: each in order); without a drop
            // the after window ends at that release, and the answer must come before it (below).
            if tl.closing_release().is_some_and(|c| at.before > c.pos) {
                return Err(
                    "contradictory: the unplug is answered after the closing release".into(),
                );
            }
        }
        let before = window(&span(tl, from, cue.before));
        let after = window(&span(tl, tl.index_at(at.at_us + SETTLE_US), end.pos));
        let count = |w: &Value| w["samples"].as_u64().unwrap_or(0);
        if count(&before) < MIN_BEFORE_SAMPLES {
            return Err(format!(
                "{} page-25 samples in the 15 s before the cue (at least {MIN_BEFORE_SAMPLES})",
                count(&before)
            ));
        }
        if drop.is_none() {
            if at.before > end.pos {
                return Err("the unplug is answered after the release".into());
            }
            let span = end.at_us.saturating_sub(at.at_us + SETTLE_US);
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
                    round1(secs(end.at_us.saturating_sub(at.at_us)))
                )
            },
            |d| {
                format!(
                    "link dropped {} s after the unplug cue",
                    round1(secs(d.at_us - cue.at_us))
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

/// Check 11c's stamps before the link: the start cue, then the advertisement within
/// [`ADVERT_WAIT`] and the connect within [`CONNECT_WAIT`] of it, and no frame before the
/// advertisement. `(cues, cold, seen, ready)`.
fn cold_stamps(tl: &Timeline) -> Result<(Vec<Stamp>, Stamp, Stamp, Stamp), String> {
    let notes = tl.notes();
    let cues = stamps(notes, WINDOW_C, &[COLD, LOW, NORMAL])?;
    let cold = stamp_of(&cues, COLD).ok_or("the capture has no start cue")?;
    let link = stamps(notes, LINK, &[SEEN, READY])
        .map_err(|_| "contradictory link stamps: 02 and 04 only, once each, in order")?;
    let seen = stamp_of(&link, SEEN).ok_or("the capture has no advertisement stamp")?;
    let ready = stamp_of(&link, READY).ok_or("the capture has no connect stamp")?;
    if seen.seq < cold.seq {
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
    if let Some(e) = late(seen.at_us, cold.at_us, ADVERT_WAIT, "the advertisement")
        .or_else(|| late(ready.at_us, seen.at_us, CONNECT_WAIT, "the connect"))
    {
        return Err(e);
    }
    // Nothing was recorded before the advertisement: the cold trainer has no link (the times
    // below are read from the start cue on).
    if seen.before > 0 {
        return Err("contradictory: a frame is recorded before the trainer advertised".into());
    }
    Ok((cues, cold, seen, ready))
}

/// The result of check 11c from the capture's timeline.
pub fn check11c(tl: &Timeline) -> Value {
    let method = "stamps validated first (the start cue, then link 02 seen within 60 s and 04 connected within 30 s of it, the low and 90 rpm cues in order; no frame before the advertisement; the first control write the release, and a release ends the run (none: the capture is cut off)), else unknown; times from the start cue: advertising = 02, connected = 04, first page 25, first ERG echo = the first page 71 naming page 49 after the first page-49 write; low/90 rpm = page 25 from 5 s after each cue to the next cue or the release: honoured when >= 10 samples, the cadence within 10 rpm of the cue and >= 80 % within ±10 % of the ERG target";
    let unknown = |why: String| json!({ "method": method, "verdict": format!("unknown: {why}") });
    let run = || -> Result<Value, String> {
        let (cues, cold, seen, ready) = cold_stamps(tl)?;
        let ctl = tl.writes();
        if let Some(w) = ctl
            .first()
            .filter(|w| w.control != Control::Resistance(0.0))
        {
            return Err(format!(
                "the first write was {:?} at {} s, not the release",
                w.control,
                round1(secs(w.at_us))
            ));
        }
        let page25 = tl.samples().first().map(|s| s.at_us);
        let erg = ctl
            .iter()
            .find(|w| matches!(w.control, Control::Erg(_)))
            .copied();
        let echo = erg.and_then(|w| {
            tl.statuses()
                .iter()
                .find(|s| s.pos > w.pos && s.page == 49)
                .map(|s| s.at_us)
        });
        let end = tl
            .closing_release()
            .ok_or("no release write ends the run: the capture is cut off before the run's end")?;
        let judged = |code: u8, next: Option<Stamp>, rpm: u8| {
            let Some(at) = stamp_of(&cues, code) else {
                return (Value::Null, format!("unknown: no {rpm} rpm cue"));
            };
            let to = next.map_or(end.pos, |n| n.before.min(end.pos));
            // The procedure writes, then stamps the cue: a write before the cue came first.
            let target = ctl
                .iter()
                .rev()
                .find(|w| w.pos < at.before)
                .map(|w| w.control);
            let w = window(&span(tl, tl.index_at(at.at_us + SETTLE_US), to));
            let verdict = match quiet(ctl, (at.before, to), "inside the window") {
                Err(why) => format!("unknown: {why}"),
                Ok(()) if target != Some(Control::Erg(f64::from(TARGET_W))) => {
                    format!("unknown: the target in force was {target:?}, not ERG {TARGET_W} W")
                }
                Ok(()) => honoured(&w, rpm),
            };
            (w, verdict)
        };
        let (low_w, low) = judged(LOW, stamp_of(&cues, NORMAL), LOW_RPM);
        let (normal_w, normal) = judged(NORMAL, None, 90);
        let echo = match echo.zip(erg) {
            Some((e, w)) => format!(
                "first ERG echo +{} s ({} s after its page 49)",
                round1(secs(e - cold.at_us)),
                round1(secs(e - w.at_us))
            ),
            None => "no ERG echo (no page 71 naming page 49)".into(),
        };
        let times = format!(
            "from the start cue: advertising +{} s, connected +{} s, first page 25 +{} s, {echo}",
            round1(secs(seen.at_us - cold.at_us)),
            round1(secs(ready.at_us - cold.at_us)),
            page25.map_or("–".into(), |t| round1(secs(t - cold.at_us)).to_string()),
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
    use crate::frames::{Record, page_of};
    use crank_fec::ControlPage;

    fn check11a(r: &[Record], n: &[Note]) -> Value {
        super::check11a(&Timeline::of_parts(r, &[], n))
    }

    /// Check 11a's verdict.
    fn verdict_of(r: &[Record], n: &[Note]) -> String {
        verdict(&check11a(r, n)).to_string()
    }

    /// The start of a verdict that is no result.
    const NO_RESULT: &str = "unknown: ";

    fn check11c(r: &[Record], n: &[Note]) -> Value {
        super::check11c(&Timeline::of_parts(r, &[], n))
    }

    fn analyse(stem: &str, r: &[Record], n: &[Note], outcome: &str) -> Value {
        let none = crate::partb::PowerLag::NotMeasured("test".into());
        let text = crate::timeline::text_of(r, &[], n);
        crate::partb::analyse(stem, &text, Some(outcome), &none).expect("stem")
    }

    fn note(tag: &str, at_ms: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_ms * 1000,
            bytes: bytes.to_vec(),
            before: 0,
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
        // The update event count, which moves with each update the trainer sends.
        let count = u8::try_from(at_ms / 250 % 256).expect("a byte");
        rx(
            at_ms,
            [
                0xa4,
                9,
                0x4e,
                5,
                25,
                count,
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

    /// 11a: ERG 100 W from 5 s (the soft start from 0.5 s), the unplug cue at 30 s, `unplugged` at 31 s, the release at 61.5 s;
    /// page 25 every 0.5 s, from 36 s with `after`'s fields.
    /// `Rig::engage` from 0 W to `target` W as the soft start writes it, its last write at `end_ms`.
    fn soft_start(end_ms: u64, target: f32) -> Vec<Record> {
        let erg = |watts| ControlPage::TargetPower { watts };
        let mut v = vec![tx(end_ms - 4_500, &erg(0.0))];
        v.extend((0..=5_u8).map(|k| {
            tx(
                end_ms - u64::from(5 - k) * 800,
                &erg(target * f32::from(k) / 5.0),
            )
        }));
        v
    }

    /// The page-70 requests every run writes: for page 81 after its opening release, for page 71
    /// after its closing one.
    const SERIAL: ControlPage = ControlPage::Request { page: 81, times: 1 };
    const REQUEST: ControlPage = ControlPage::Request { page: 71, times: 1 };

    fn ride11a(after: (u16, u8, u8)) -> (Vec<Record>, Vec<Note>) {
        let mut r = vec![tx(0, &RELEASE), tx(300, &SERIAL)];
        r.extend(soft_start(5_000, 100.0));
        r.extend((0..123).map(|k| {
            let at = k * 500;
            let (w, status, limit) = if at >= 36_000 { after } else { (100, 0, 0) };
            p25(at, 90, w, status, limit)
        }));
        r.push(tx(61_500, &RELEASE));
        r.push(tx(61_800, &REQUEST));
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
        // #516 round 4 M25: no answer, no drop: no result (the procedure answers before it rides
        // on).
        assert!(verdict_of(&base.0, std::slice::from_ref(&cue)).starts_with(NO_RESULT));
        // Answered before the cue: no result (ADR-0007 §4: each found once and in order).
        assert!(
            verdict_of(&base.0, &[answer(b"unplugged", 29_000), cue.clone()])
                .starts_with(NO_RESULT)
        );
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
                "the unplug is answered after the release",
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
        ] {
            let v = check11a(&records, &notes);
            assert_eq!(verdict(&v), format!("unknown: {why}"), "{why}");
        }
        // Trainer frames after the stamped drop: no result.
        assert!(verdict_of(&r, &with_drop(40_000)).starts_with(NO_RESULT));
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
                "the release before the drop, at 30 s, is not the silence guard's: page 25 came 0 s before it (the guard waits more than 2 s)",
            ),
            (
                guarded(SILENCE_US - 1, vec![]),
                drop_at(50_000),
                "the release before the drop, at 46.5 s, is not the silence guard's: page 25 came 2 s before it (the guard waits more than 2 s)",
            ),
            // #516 N1: exactly SILENCE_US is not yet silent for the live guard either.
            (
                guarded(SILENCE_US, vec![]),
                drop_at(50_000),
                "the release before the drop, at 46.5 s, is not the silence guard's: page 25 came 2 s before it (the guard waits more than 2 s)",
            ),
            (
                guarded(SILENCE_US + 1, vec![at_us(49_999_999, &ERG)]),
                drop_at(50_000),
                "Erg(100.0) written at 50 s, between the silence guard's release and the drop",
            ),
            (
                guarded(SILENCE_US + 1, vec![p25(46_500, 90, 100, 0, 0)]),
                drop_at(50_000),
                "the release before the drop, at 46.5 s, is not the silence guard's: page 25 came 0 s before it (the guard waits more than 2 s)",
            ),
        ] {
            assert_eq!(
                verdict(&check11a(&records, &notes)),
                format!("unknown: {why}"),
                "{why}"
            );
        }
        // A drop after the closing release: no result.
        assert!(verdict_of(&r, &drop_at(70_000)).starts_with(NO_RESULT));
        // The edges that are valid: the guard's release 1 µs past the silence bound after the last
        // page 25 (other pages may come), a write after the drop's stamp at its own µs, a drop
        // stamped before the closing release at its µs.
        let page71 = rx(46_000, [0xa4, 9, 0x4e, 5, 0x47, 48, 0, 0, 0, 0, 0, 0, 0]);
        for (records, notes, link) in [
            (
                guarded(SILENCE_US + 1, vec![page71]),
                drop_at(50_000),
                "link dropped 20 s",
            ),
            (
                guarded(SILENCE_US + 1, vec![]),
                drop_at(50_000),
                "link dropped 20 s",
            ),
            (
                guarded(SILENCE_US + 1, vec![tx(50_000, &ERG)]),
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
            assert!(verdict_of(&r, &notes).starts_with(NO_RESULT));
        }
        let twice = vec![
            cue,
            answer(b"unplugged", 31_000),
            answer(b"unplugged", 31_500),
        ];
        assert!(verdict_of(&r, &twice).starts_with(NO_RESULT));
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
                // The page 25 recorded before the release at its µs is inside the window.
                "29 page-25 samples over 14 s after the unplug (at least 40 over 20 s)",
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
                "Resistance(0.0) written at 15 s, before the unplug cue",
            ),
            (
                with(20_000, &RELEASE),
                "Resistance(0.0) written at 20 s, before the unplug cue",
            ),
            // After the cue's stamp at its µs (the merge puts the stamp first): the run's end.
            (
                with(30_000, &RELEASE),
                "the unplug is answered after the release",
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
        // A release written before the cue's stamp at its µs is before the cue.
        let text = crate::timeline::text_of(&with(30_000, &RELEASE), &[], &n);
        let cue = "# check11a 30000000 01\n";
        let release = format!(
            "30000000 tx {}\n",
            crate::frames::hex(&tx(0, &RELEASE).bytes)
        );
        let text = text
            .replacen(&release, "", 1)
            .replacen(cue, &format!("{release}{cue}"), 1);
        assert_eq!(
            verdict(&super::check11a(&Timeline::parse(&text).expect("in order"))),
            "unknown: Resistance(0.0) written at 30 s, before the unplug cue"
        );
        // The unplug answered at the release's own µs (before it) leaves no after window.
        let late = vec![
            note(WINDOW_A, 30_000, &[UNPLUG]),
            note(&answer_tag("unplugged"), 61_500, b"unplugged"),
        ];
        assert_eq!(
            verdict(&check11a(&r, &late)),
            "unknown: 0 page-25 samples over 0 s after the unplug (at least 40 over 20 s)"
        );
        // Cut off before the release: no end, no result, offline as live.
        let cut: Vec<Record> = r.iter().filter(|x| x.at_us < 61_500_000).cloned().collect();
        assert_eq!(
            verdict(&check11a(&cut, &n)),
            "unknown: no release write after the unplug cue: the capture is cut off before the run's end"
        );
        let v = analyse("check11a", &cut, &n, crate::partb::COMPLETED);
        assert_eq!(
            v["invalid"],
            "the capture is cut off: no release write ends it"
        );
        assert_eq!(v["verdict"], Value::Null);
        // A trainer frame at the drop's µs is after it: the stamp follows the frames' end.
        // (No frame after `d`; the merge puts the stamp before a frame of its µs.)
        let mut dropped: Vec<Record> = r.iter().filter(|x| x.at_us < 40_000_000).cloned().collect();
        dropped.extend([tx(41_000, &RELEASE), tx(41_300, &REQUEST)]);
        let drop_at = |at: u64| [n.clone(), vec![note(LINK, at, &[DETECTED])]].concat();
        assert!(verdict_of(&dropped, &drop_at(39_500)).starts_with(NO_RESULT));
        assert!(verdict_of(&dropped, &drop_at(39_501)).starts_with("link dropped "));
    }

    /// The capture text of `lines`, in the given (file) order, with a completed outcome line.
    fn file(lines: &[String]) -> String {
        lines.iter().fold(String::new(), |mut t, l| {
            t.push_str(l);
            t.push('\n');
            t
        }) + "# hil_fec procedure: completed\n"
    }

    fn line(r: &Record) -> String {
        let d = if r.rx { "rx" } else { "tx" };
        format!("{} {d} {}", r.at_us, crate::frames::hex(&r.bytes))
    }

    fn stamped(tag: &str, at_us: u64, bytes: &[u8]) -> String {
        format!("# {tag} {at_us} {}", crate::frames::hex(bytes))
    }

    /// The silence guard's 11a shape as file lines: ERG 100 W from 5 s, page 25 every 0.5 s to
    /// 44.5 s, the unplug cue at 30 s (the answer at 31 s when `answered`), the guard's release
    /// `gap` µs after the last page 25.
    fn guarded_file(gap: u64, answered: bool) -> Vec<String> {
        // (µs, notes before frames at a tie, line), in time order.
        let mut lines: Vec<(u64, u8, String)> = vec![
            (0, 1, line(&tx(0, &RELEASE))),
            (300_000, 1, line(&tx(300, &SERIAL))),
        ];
        lines.extend(
            soft_start(5_000, 100.0)
                .iter()
                .map(|r| (r.at_us, 1, line(r))),
        );
        lines.extend((0..90).map(|k| (k * 500_000, 1, line(&p25(k * 500, 90, 100, 0, 0)))));
        lines.push((30_000_000, 0, stamped(WINDOW_A, 30_000_000, &[UNPLUG])));
        if answered {
            lines.push((
                31_000_000,
                0,
                stamped("answer-unplugged", 31_000_000, b"unplugged"),
            ));
        }
        lines.sort_by_key(|l| (l.0, l.1));
        let mut lines: Vec<String> = lines.into_iter().map(|l| l.2).collect();
        lines.push(line(&Record {
            at_us: 44_500_000 + gap,
            ..tx(0, &RELEASE)
        }));
        lines
    }

    fn row(text: &str) -> Value {
        let none = crate::partb::PowerLag::NotMeasured("test".into());
        crate::partb::analyse("check11a", text, Some(crate::partb::COMPLETED), &none).expect("stem")
    }

    /// #516: the falsifier's captures, each a full 11a row (`completed`, no `invalid`, "link
    /// dropped 20 s") before the timeline; each is no result now, and the honest shape is still
    /// a full row.
    #[test]
    fn forged_captures_b1_b2_b3_n1_give_no_full_row() {
        use crate::rider::SILENCE_US;
        let drop = stamped(LINK, 50_000_000, &[DETECTED]);
        let closing = [line(&tx(51_000, &RELEASE)), line(&tx(51_300, &REQUEST))].join("\n");
        let honest = file(
            &[
                guarded_file(SILENCE_US + 1, true),
                vec![drop.clone(), closing.clone()],
            ]
            .concat(),
        );
        assert!(
            verdict(&row(&honest)).starts_with("link dropped 20 s after the unplug cue"),
            "{}",
            row(&honest)
        );
        // B1: a write at the guard's release µs, after it in the file, is between it and the drop.
        let erg300 = line(&Record {
            at_us: 44_500_000 + SILENCE_US + 1,
            ..tx(0, &ControlPage::TargetPower { watts: 300.0 })
        });
        let b1 = file(
            &[
                guarded_file(SILENCE_US + 1, true),
                vec![erg300, drop.clone(), closing.clone()],
            ]
            .concat(),
        );
        let not_the_procedure =
            |v: &Value| v["completed"] == json!(false) || verdict(v).starts_with("unknown: ");
        assert!(not_the_procedure(&row(&b1)), "{}", row(&b1));
        // B2: a page 25 at 46.4 s written first: the file is not in time order.
        let b2 = file(
            &[
                vec![line(&p25(46_400, 90, 100, 0, 0))],
                guarded_file(SILENCE_US + 1, true),
                vec![drop.clone(), closing.clone()],
            ]
            .concat(),
        );
        let v = row(&b2);
        assert_eq!(
            v["invalid"],
            json!("contradictory: records out of time order (line 2)")
        );
        assert_eq!(
            (&v["completed"], &v["verdict"]),
            (&json!(false), &Value::Null)
        );
        // B3: the answer after the closing release.
        let late = stamped("answer-unplugged", 80_000_000, b"unplugged");
        let b3 = file(
            &[
                guarded_file(SILENCE_US + 1, false),
                vec![drop.clone(), closing.clone(), late],
            ]
            .concat(),
        );
        assert!(not_the_procedure(&row(&b3)), "{}", row(&b3));
        // N1: a release exactly SILENCE_US after the last page 25 is not the live guard's.
        let n1 = file(&[guarded_file(SILENCE_US, true), vec![drop, closing]].concat());
        assert_eq!(
            verdict(&row(&n1)),
            "unknown: the release before the drop, at 46.5 s, is not the silence guard's: page 25 came 2 s before it (the guard waits more than 2 s)"
        );
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
        let (r, n) = ride11a((100, 0, 0));
        let v = analyse("check11a", &r, &n, "aborted: x");
        for key in ["verdict", "link", "before", "after"] {
            assert_eq!(v[key], Value::Null, "{key}: {v}");
        }
        let (r, n) = ride11c((60, 100, 0));
        let v = analyse("check11c", &r, &n, "aborted: x");
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
                "contradictory: a frame is recorded before the trainer advertised",
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
