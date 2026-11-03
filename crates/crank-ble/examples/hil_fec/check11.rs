//! Check 11b (#44, #412 part c2): the realistic dropout. The NEO has no battery; unplugged, the
//! rider's pedalling powers it, and it shuts down shortly after pedalling stops. Stopping until
//! the lights go out drops the link; pedalling again wakes the trainer and the harness reconnects
//! (ADR-0002 §6).
//!
//! The procedure (`rider.rs`) stamps its events into the capture, so `analyse` reproduces the live
//! row: `# check11b <us> 01` (the stop cue, the trainer already released) and the link events
//! `# link <us> 01..06` ([`DETECTED`] … [`GAVE_UP`]; [`CONNECTED`] comes from the adapter's
//! connect event, the others from the procedure), plus the operator's `# answer-lights`.
use crate::{
    check10::{Question, answer},
    frames::{Note, Record, stamp, stamps},
    partb::{Control, controls, mean, procedure_end, round1, samples, secs},
};
use crank_fec::{Page, decode};
use serde_json::{Value, json};
use std::time::Duration;

/// Tag of the procedure's own stamp.
pub const WINDOW: &str = "check11b";
/// The stop cue, given with the trainer released.
pub const STOPPING: u8 = 1;
/// Tag of the link events.
pub const LINK: &str = "link";
/// The harness saw the link drop.
pub const DETECTED: u8 = 1;
/// The trainer advertised again (the scan beside the dropped link).
pub const SEEN: u8 = 2;
/// The adapter reported the connection (live: `DeviceConnected`).
pub const CONNECTED: u8 = 3;
/// Services discovered and page 25 subscribed.
pub const READY: u8 = 4;
/// The first page 25 after the reconnect.
pub const FIRST_PAGE25: u8 = 5;
/// The reconnect bound ran out.
pub const GAVE_UP: u8 = 6;
/// The order the link events come in.
pub const LINK_ORDER: [u8; 6] = [DETECTED, SEEN, CONNECTED, READY, FIRST_PAGE25, GAVE_UP];
/// The link must drop within this time of the stop cue.
pub const DROP_WAIT: Duration = Duration::from_secs(90);
/// The trainer must be back (first page 25) within this time of the drop.
pub const RECONNECT_WAIT: Duration = Duration::from_mins(1);
/// The ERG target before the stop and after the reconnect, W.
pub const TARGET_W: u16 = 100;

/// The operator's observation, asked once the link is back (or the bound ran out).
pub const LIGHTS: Question = Question {
    key: "lights",
    text: "When the link dropped, had the trainer's lights gone out?",
    options: &["out", "on", "unsure"],
};

/// The dropout's cause, from the lights answer: only `out` is the power-off the check is about.
fn cause(notes: &[Note]) -> String {
    match answer(notes, &LIGHTS).as_deref() {
        Ok("out") => "power-off (the lights went out)".into(),
        Ok("on") => "unknown: the link dropped with the lights still on, not a power-off".into(),
        Ok(_) => "unknown: the operator was unsure about the lights".into(),
        Err(why) => format!("unknown: {why} to whether the lights went out"),
    }
}

/// The FE state code of page 25 as the ANT+ FE profile names it.
fn fe_state(code: u8) -> String {
    let name = match code {
        1 => "asleep",
        2 => "ready",
        3 => "in use",
        4 => "finished",
        _ => "reserved",
    };
    format!("{code} ({name})")
}

/// The reconnect, from the link stamps: each phase measured from the drop's detection.
fn reconnect(link: &[(u8, u64)], detected: u64) -> String {
    let phase = |code: u8, name: &str| {
        stamp(link, code).map_or(format!("{name} unknown"), |at| {
            format!("{name} +{} s", round1(secs(at.saturating_sub(detected))))
        })
    };
    let phases = [
        phase(SEEN, "seen"),
        phase(CONNECTED, "connected"),
        phase(READY, "discovered"),
    ]
    .join(", ");
    match (stamp(link, FIRST_PAGE25), stamp(link, GAVE_UP)) {
        (Some(at), _) => format!(
            "{} s to the first page 25 ({phases})",
            round1(secs(at - detected))
        ),
        (None, Some(_)) => format!(
            "none within {} s of the drop ({phases})",
            RECONNECT_WAIT.as_secs()
        ),
        (None, None) => format!("unknown: the capture ends before the reconnect did ({phases})"),
    }
}

/// What the trainer did after the reconnect: its first page 25 (sent before any write), whether
/// the first write was the release, and how the re-engaged ERG target was held.
fn after(records: &[Record], first: u64) -> Value {
    let page25 = records
        .iter()
        .filter(|r| r.rx && r.at_us >= first)
        .find_map(|r| match decode(&r.bytes).ok()?.page {
            Page::Trainer(t) => Some(t),
            _ => None,
        });
    let ctl = controls(records);
    let later: Vec<&(u64, Control)> = ctl.iter().filter(|(at, _)| *at >= first).collect();
    let released_first = later.first().map(|(_, c)| *c == Control::Resistance(0.0));
    let erg = later
        .iter()
        .rev()
        .find(|(_, c)| matches!(c, Control::Erg(w) if *w > 0.0))
        .map(|(at, _)| *at);
    let end = procedure_end(records, &ctl);
    // Settled: from 5 s after the last ramp write to the final release.
    let held = erg.and_then(|at| {
        mean(
            samples(records)
                .iter()
                .filter(|s| s.at_us >= at + 5_000_000 && s.at_us < end)
                .filter_map(|s| s.power.map(f64::from)),
        )
    });
    json!({
        "first_page25_fe_state": page25.map(|t| fe_state(t.state)),
        "first_page25_target_limit": page25.map(|t| t.target_limit),
        "first_page25_power_w": page25.and_then(|t| t.power_w),
        "first_write_was_release": released_first,
        "erg_mean_power_w": held.map(round1),
    })
}

/// The result of check 11b from the capture's records and notes.
pub fn check11b(records: &[Record], notes: &[Note]) -> Value {
    let method = "drop = the link 01 stamp after the check11b 01 stop cue; trainer ran on = its last page 25 before the drop; reconnect = the link stamps from the drop (seen 02, connected 03, discovered 04, first page 25 05; 06 = the 60 s bound ran out); after = the first page 25 after the reconnect, whether the first write after it was page 48 at 0 %, and the mean power from 5 s after the re-engaged ERG target to the release; cause = the operator's lights answer";
    let unknown = |why: String| json!({ "method": method, "drop": Value::Null, "verdict": format!("unknown: {why}") });
    let (stop, link) = match (
        stamps(notes, WINDOW, &[STOPPING]),
        stamps(notes, LINK, &LINK_ORDER),
    ) {
        (Ok(s), Ok(l)) => (stamp(&s, STOPPING), l),
        (Err(why), _) | (_, Err(why)) => return unknown(why),
    };
    let (Some(stop), Some(detected)) = (stop, stamp(&link, DETECTED)) else {
        return unknown("the capture has no stop cue and drop".into());
    };
    if detected < stop {
        return unknown("contradictory: the drop is stamped before the stop cue".into());
    }
    let ran_on = samples(records)
        .iter()
        .rev()
        .find(|s| s.at_us >= stop && s.at_us < detected)
        .map(|s| round1(secs(s.at_us - stop)));
    let reconnect = reconnect(&link, detected);
    let after = stamp(&link, FIRST_PAGE25).map(|at| after(records, at));
    let cause = cause(notes);
    let drop = format!(
        "detected {} s after the stop cue (last page 25 {})",
        round1(secs(detected - stop)),
        ran_on.map_or("before the stop cue".into(), |s| format!("{s} s after it"))
    );
    let resumed = after.as_ref().map_or_else(
        || "no".to_string(),
        |a| {
            format!(
                "page 25 first at FE state {}; first write the release: {}; ERG {TARGET_W} W held at {} W",
                a["first_page25_fe_state"].as_str().unwrap_or("unknown"),
                a["first_write_was_release"],
                a["erg_mean_power_w"]
            )
        },
    );
    json!({
        "method": method,
        "drop": drop,
        "cause": cause,
        "reconnect": reconnect,
        "after_reconnect": after,
        "verdict": format!("dropout {cause}; {drop}; reconnect {reconnect}; after it {resumed}"),
    })
}

/// Check number, check, result and ADR-0002 section of check 11b's row.
pub fn cells(v: &Value) -> (&'static str, &'static str, String, &'static str) {
    let result = v["verdict"].as_str().unwrap_or("no verdict").to_string();
    ("11b", "Dropout: unplugged stop, reconnect", result, "§6")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check10::answer_tag;

    fn note(tag: &str, at_ms: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_ms * 1000,
            bytes: bytes.to_vec(),
        }
    }

    fn link(stamps: &[(u8, u64)]) -> Vec<Note> {
        let mut n = vec![note(WINDOW, 30_000, &[STOPPING])];
        n.extend(stamps.iter().map(|(c, ms)| note(LINK, *ms, &[*c])));
        n
    }

    #[test]
    fn the_reconnect_is_measured_phase_by_phase_from_the_drop() {
        let v = check11b(
            &[],
            &link(&[
                (1, 40_000),
                (2, 44_200),
                (3, 44_900),
                (4, 46_100),
                (5, 46_300),
            ]),
        );
        assert_eq!(
            v["reconnect"],
            json!("6.3 s to the first page 25 (seen +4.2 s, connected +4.9 s, discovered +6.1 s)")
        );
        assert_eq!(
            v["drop"],
            json!("detected 10 s after the stop cue (last page 25 before the stop cue)")
        );
        // A phase without its stamp is unknown, not guessed.
        let v = check11b(
            &[],
            &link(&[(1, 40_000), (2, 44_200), (4, 46_100), (5, 46_300)]),
        );
        assert_eq!(
            v["reconnect"],
            json!("6.3 s to the first page 25 (seen +4.2 s, connected unknown, discovered +6.1 s)")
        );
        let v = check11b(&[], &link(&[(1, 40_000), (2, 44_200)]));
        assert!(
            v["reconnect"].as_str().is_some_and(
                |r| r.starts_with("unknown: the capture ends before the reconnect did")
            ),
            "{v}"
        );
    }

    /// A page 25 at `at_ms` reporting `watts`.
    fn p25(at_ms: u64, watts: u8) -> Record {
        let mut f = [0xa4, 9, 0x4e, 5, 25, 0, 90, 0, 0, watts, 0, 0x30, 0];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us: at_ms * 1000,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    fn tx(at_ms: u64, page: &crank_fec::ControlPage) -> Record {
        Record {
            at_us: at_ms * 1000,
            rx: false,
            bytes: crank_fec::encode(page).expect("encodes").to_vec(),
        }
    }

    const RELEASE: crank_fec::ControlPage = crank_fec::ControlPage::BasicResistance { pct: 0.0 };

    #[test]
    fn the_trainer_ran_on_only_counts_page_25_after_the_stop_cue() {
        let notes = link(&[(1, 40_000)]);
        let records: Vec<Record> = (0..60).map(|k| p25(k * 500, 90)).collect();
        assert_eq!(
            check11b(&records, &notes)["drop"],
            json!("detected 10 s after the stop cue (last page 25 before the stop cue)")
        );
        let records: Vec<Record> = (0..70).map(|k| p25(k * 500, 90)).collect();
        assert_eq!(
            check11b(&records, &notes)["drop"],
            json!("detected 10 s after the stop cue (last page 25 4.5 s after it)")
        );
    }

    /// After the reconnect: the first write must be the release, and the ERG target is judged from
    /// 5 s after its last ramp write to the final release.
    #[test]
    fn after_the_reconnect_the_first_write_and_the_settled_power_are_read_from_the_capture() {
        let notes = link(&[
            (1, 40_000),
            (2, 44_000),
            (3, 44_500),
            (4, 45_000),
            (5, 45_200),
        ]);
        let erg = crank_fec::ControlPage::TargetPower { watts: 100.0 };
        let mut records = vec![p25(45_200, 0), tx(45_300, &RELEASE), tx(50_000, &erg)];
        // Ramping up in the first 5 s, then 100 W until the release at 70 s.
        records.extend((0..10).map(|k| p25(50_000 + k * 500, 20)));
        records.extend((0..30).map(|k| p25(55_000 + k * 500, 100)));
        records.push(tx(70_000, &RELEASE));
        let a = &check11b(&records, &notes)["after_reconnect"];
        assert_eq!(a["first_write_was_release"], json!(true));
        assert_eq!(a["erg_mean_power_w"], json!(100.0));
        assert_eq!(a["first_page25_fe_state"], json!("3 (in use)"));
        records[1] = tx(45_300, &erg);
        let a = &check11b(&records, &notes)["after_reconnect"];
        assert_eq!(a["first_write_was_release"], json!(false));
    }

    #[test]
    fn duplicate_or_out_of_order_link_stamps_are_contradictory() {
        for bad in [
            link(&[(1, 40_000), (3, 44_000), (3, 44_500)]),
            link(&[(1, 40_000), (3, 44_000), (2, 44_500)]),
            link(&[(1, 40_000), (2, 39_000)]),
            link(&[(1, 20_000)]),
            [
                link(&[(1, 40_000)]),
                vec![note(WINDOW, 31_000, &[STOPPING])],
            ]
            .concat(),
        ] {
            let v = check11b(&[], &bad);
            assert!(
                v["verdict"]
                    .as_str()
                    .is_some_and(|x| x.starts_with("unknown: contradictory")),
                "{bad:?}: {v}"
            );
            assert_eq!(v["drop"], Value::Null);
        }
        let v = check11b(&[], &[note(WINDOW, 30_000, &[STOPPING])]);
        assert_eq!(
            v["verdict"],
            json!("unknown: the capture has no stop cue and drop")
        );
    }

    #[test]
    fn the_cause_comes_only_from_a_lights_answer() {
        let with = |answer: Option<&str>| {
            let mut n = link(&[(1, 40_000), (6, 100_000)]);
            n.extend(answer.map(|a| note(&answer_tag("lights"), 101_000, a.as_bytes())));
            check11b(&[], &n)["cause"].clone()
        };
        assert_eq!(with(Some("out")), json!("power-off (the lights went out)"));
        assert_eq!(
            with(Some("on")),
            json!("unknown: the link dropped with the lights still on, not a power-off")
        );
        assert_eq!(
            with(Some("unsure")),
            json!("unknown: the operator was unsure about the lights")
        );
        assert_eq!(
            with(Some("dark")),
            json!("unknown: unrecognised answer \"dark\" to whether the lights went out")
        );
        assert_eq!(
            with(None),
            json!("unknown: no answer to whether the lights went out")
        );
    }
}
