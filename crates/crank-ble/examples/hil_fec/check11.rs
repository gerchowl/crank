//! Check 11b (#44, #412 part c2): the realistic dropout. The NEO has no battery; unplugged, the
//! rider's pedalling powers it, and it shuts down shortly after pedalling stops. Stopping until
//! the lights go out drops the link; pedalling again wakes the trainer and the harness reconnects
//! (ADR-0002 §6).
//!
//! The procedure (`rider.rs`) stamps its events into the capture, so `analyse` reproduces the live
//! row: `# check11b <us> 01|02` (the stop cue, the trainer already released; the operator's `y`
//! after the reconnect) and the link events `# link <us> 01..07` ([`DETECTED`] … [`FAILED`];
//! [`CONNECTED`] comes from the adapter's connect event, the others from the procedure), plus the
//! operator's `# answer-lights`. [`validate`] holds them to the procedure's rules, live and offline.
use crate::{
    check10::{Question, answer},
    frames::{Note, Stamp, stamp, stamp_of, stamps},
    partb::{Control, SEC, mean, procedure_end, round1, secs},
    timeline::{Grammar, LINK_TOKENS, Step, Timeline},
};
use serde_json::{Value, json};
use std::time::Duration;

/// Tag of the procedure's own stamp.
pub const WINDOW: &str = "check11b";
/// The stop cue, given with the trainer released.
pub const STOPPING: u8 = 1;
/// The operator's `y` after the reconnect, before any control.
pub const CONFIRMED: u8 = 2;
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
/// The reconnect failed (the scan ended, the connect failed): the run fails.
pub const FAILED: u8 = 7;
/// The link must drop within this time of the stop cue.
pub const DROP_WAIT: Duration = Duration::from_secs(90);
/// The procedure polls the link every 100 ms while it waits for the drop, so a drop just before
/// the bound is stamped up to this much after it.
const POLL_SLACK_US: u64 = 200_000;
/// The trainer must be back (first page 25) within this time of the drop.
pub const RECONNECT_WAIT: Duration = Duration::from_mins(1);
/// The procedure stamps "gave up" when its timer for [`RECONNECT_WAIT`] fires, so the stamp lands
/// at the bound or up to this much after it, never later.
const GIVE_UP_SLACK_US: u64 = 1_000_000;
/// The ERG target before the stop and after the reconnect, W.
pub const TARGET_W: u16 = 100;

/// Check 11b's event grammar ([`Timeline::grammar`]): ERG by soft start, the release, the stop
/// cue, the drop with no trainer frame until the reconnect is discovered; then either the first
/// page 25, the release, the lights answer, the `y` and ERG again by soft start, or the bound
/// running out and the lights answer; the closing release last.
pub(crate) const GRAMMAR: Grammar = Grammar {
    tags: &[(WINDOW, &[(STOPPING, '1'), (CONFIRMED, '2')]), LINK_TOKENS],
    answers: &[LIGHTS.key],
    records: &[],
    shapes: &[
        "^f*Rf*u[fe]*(?:E[fe]*)+R[fe]*1[fe]*d(?:vk?n[fe]*p[fe]*R[fe]*u[fe]*a[fe]*2[fe]*(?:E[fe]*)+R[fe]*q[fe]*|(?:v(?:k?n[fe]*)?)?g[fe]*a?[fe]*R[fe]*q[fe]*)$",
    ],
    programs: &[
        &[
            Step::Release,
            Step::SoftStart(TARGET_W),
            Step::Release,
            Step::Release,
            Step::SoftStart(TARGET_W),
            Step::Release,
        ],
        &[
            Step::Release,
            Step::SoftStart(TARGET_W),
            Step::Release,
            Step::Release,
        ],
    ],
};

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

/// The legal link transitions: the drop first; the phases in order, `connected` optional (the
/// adapter's event is not guaranteed); one end at most: first page 25, gave up or failed.
fn follows(prev: Option<u8>, code: u8) -> bool {
    match prev {
        None => code == DETECTED,
        Some(DETECTED) => matches!(code, SEEN | GAVE_UP | FAILED),
        Some(SEEN) => matches!(code, CONNECTED | READY | GAVE_UP | FAILED),
        Some(CONNECTED) => matches!(code, READY | GAVE_UP | FAILED),
        Some(READY) => matches!(code, FIRST_PAGE25 | GAVE_UP | FAILED),
        Some(_) => false,
    }
}

/// The page-81 serials `(at_us, serial)` stamped in `from..to`, deduplicated.
fn serials(all: &[(u64, [u8; 4])], from: u64, to: u64) -> Vec<[u8; 4]> {
    let mut out: Vec<[u8; 4]> = all
        .iter()
        .filter(|(at, _)| *at >= from && *at < to)
        .map(|(_, s)| *s)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The trainer after the reconnect (from `back`) is the one before the drop: one page-81 serial on
/// each side, the same. A redacted capture compares its redacted serials, which `redact` writes
/// alike on both sides; the live run compared the real ones.
/// `all` are the page-81 serials of the capture ([`crate::partb::serials`]).
pub fn same_trainer(all: &[(u64, [u8; 4])], drop: u64, back: u64) -> Result<(), String> {
    match (
        &serials(all, 0, drop)[..],
        &serials(all, back, u64::MAX)[..],
    ) {
        ([a], [b]) if a == b => Ok(()),
        ([_], [_]) => {
            Err("a different trainer answered after the reconnect (page-81 serials differ)".into())
        }
        ([], _) | (_, []) => Err("no page-81 serial on one side of the drop".into()),
        _ => Err("page-81 serials disagree on one side of the drop".into()),
    }
}

/// A run's stamps, held to the procedure's rules.
struct Run {
    stop: Stamp,
    link: Vec<Stamp>,
}

/// Check 11b's stamps against the procedure: the link events as a state machine ([`follows`]),
/// never back in time; the drop within 90 s of the stop cue; the first page 25 within 60 s of
/// the drop, and "gave up" at that bound (within [`GIVE_UP_SLACK_US`]); no control but the release
/// from the stop cue to the operator's `y`; the `y` only after the reconnect and its release; the
/// same trainer on both sides. `Err` names the first breach, which makes the row `unknown`.
fn validate(tl: &Timeline) -> Result<Run, String> {
    let notes = tl.notes();
    let window = stamps(notes, WINDOW, &[STOPPING, CONFIRMED])?;
    let stop = stamp_of(&window, STOPPING).ok_or("the capture has no stop cue")?;
    let mut link: Vec<Stamp> = Vec::new();
    for (seq, n) in notes.iter().enumerate().filter(|(_, n)| n.tag == LINK) {
        match n.bytes[..] {
            [code] if follows(link.last().map(|p| p.code), code) => link.push(Stamp {
                code,
                at_us: n.at_us,
                seq,
                before: n.before,
            }),
            _ => {
                let codes: Vec<u8> = link.iter().map(|l| l.code).collect();
                return Err(format!(
                    "contradictory link stamps: {:02X?} at {} us after {codes:02X?}",
                    n.bytes, n.at_us
                ));
            }
        }
    }
    let drop = stamp_of(&link, DETECTED).ok_or("the capture has no drop")?;
    // The grammar holds a completed run to this; the check reads any capture, so it holds too.
    if drop.seq < stop.seq {
        return Err("contradictory: the drop is stamped before the stop cue".into());
    }
    let detected = drop.at_us;
    let after_s = |at: u64, from: u64| round1(secs(at.saturating_sub(from)));
    if detected > stop.at_us + DROP_WAIT.as_secs() * SEC + POLL_SLACK_US {
        return Err(format!(
            "the drop is stamped {} s after the stop cue, past the {} s bound",
            after_s(detected, stop.at_us),
            DROP_WAIT.as_secs()
        ));
    }
    let bound = detected + RECONNECT_WAIT.as_secs() * SEC;
    let back = stamp_of(&link, FIRST_PAGE25);
    if let Some(at) = back.map(|b| b.at_us).filter(|at| *at > bound) {
        return Err(format!(
            "the first page 25 is stamped {} s after the drop, past the {} s bound",
            after_s(at, detected),
            RECONNECT_WAIT.as_secs()
        ));
    }
    if let Some(at) = stamp(&link, GAVE_UP) {
        if at < bound {
            return Err(format!(
                "the reconnect is stamped given up {} s after the drop, before the {} s bound",
                after_s(at, detected),
                RECONNECT_WAIT.as_secs()
            ));
        }
        if at > bound + GIVE_UP_SLACK_US {
            return Err(format!(
                "the reconnect is stamped given up {} s after the drop, past the {} s bound",
                after_s(at, detected),
                RECONNECT_WAIT.as_secs()
            ));
        }
    }
    let confirmed = stamp_of(&window, CONFIRMED);
    if confirmed.is_some_and(|y| back.is_none_or(|b| y.seq < b.seq)) {
        return Err(
            "contradictory: the operator's y is stamped without a reconnect before it".into(),
        );
    }
    let ctl = tl.writes();
    let early = ctl.iter().find(|w| {
        w.pos >= stop.before
            && confirmed.is_none_or(|y| w.pos < y.before)
            && w.control != Control::Resistance(0.0)
    });
    if let Some(w) = early {
        return Err(format!(
            "{:?} at {} s after the stop cue, before the confirmed resume",
            w.control,
            after_s(w.at_us, stop.at_us)
        ));
    }
    if let Some(back) = back {
        // With only releases before the `y`, a write after the reconnect and before the `y` is the
        // release: that makes it the first write, and the ERG after the `y` cannot be.
        let first = ctl.iter().find(|w| w.pos >= back.before);
        if confirmed.is_some_and(|y| first.is_none_or(|w| w.pos >= y.before)) {
            return Err(
                "contradictory: the operator's y is stamped before the release after the reconnect"
                    .into(),
            );
        }
        same_trainer(&tl.serials(), detected, back.at_us)?;
    }
    Ok(Run { stop, link })
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
fn reconnect(link: &[Stamp], detected: u64) -> String {
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
    if stamp(link, FAILED).is_some() {
        return format!("failed ({phases})");
    }
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

/// What the trainer did after the reconnect (frames from index `first`): its first page 25 (sent
/// before any write) and how the re-engaged ERG target was held ([`validate`] has made sure the
/// release came first).
fn after(tl: &Timeline, first: usize) -> Value {
    let page25 = tl.samples().iter().find(|s| s.pos >= first).copied();
    let ctl = tl.controls();
    let erg = tl
        .writes()
        .iter()
        .rev()
        .find(|w| w.pos >= first && matches!(w.control, Control::Erg(x) if x > 0.0))
        .map(|w| w.at_us);
    let end = procedure_end(tl.end_us(), &ctl);
    // Settled: from 5 s after the last ramp write to the final release.
    let held = erg.and_then(|at| {
        mean(
            tl.updates()
                .iter()
                .filter(|s| s.at_us >= at + 5_000_000 && s.at_us < end)
                .filter_map(|s| s.power.map(f64::from)),
        )
    });
    json!({
        "first_page25_fe_state": page25.map(|t| fe_state(t.state)),
        "first_page25_target_limit": page25.map(|t| t.target_limit),
        "first_page25_power_w": page25.and_then(|t| t.power),
        "erg_mean_power_w": held.map(round1),
    })
}

/// The result of check 11b from the capture's records and notes.
pub fn check11b(tl: &Timeline) -> Value {
    let method = "stamps validated first (link events as a state machine, the 90 s and 60 s bounds, no control but the release from the stop cue to the operator's y, the y after the reconnect's release, the same page-81 serial on both sides), else unknown; drop = the link 01 stamp after the check11b 01 stop cue; trainer ran on = its last page 25 before the drop; reconnect = the link stamps from the drop (seen 02, connected 03, discovered 04, first page 25 05; 06 = the 60 s bound ran out); after = the first page 25 after the reconnect, whether the first write after it was page 48 at 0 %, and the mean power from 5 s after the re-engaged ERG target to the release; cause = the operator's lights answer";
    let unknown = |why: String| json!({ "method": method, "drop": Value::Null, "verdict": format!("unknown: {why}") });
    let Run { stop, link } = match validate(tl) {
        Ok(run) => run,
        Err(why) => return unknown(why),
    };
    let notes = tl.notes();
    let detected = stamp_of(&link, DETECTED).unwrap_or(stop);
    let ran_on = tl
        .samples()
        .iter()
        .rfind(|s| s.pos >= stop.before && s.pos < detected.before)
        .map(|s| round1(secs(s.at_us - stop.at_us)));
    let reconnect = reconnect(&link, detected.at_us);
    let after = stamp_of(&link, FIRST_PAGE25).map(|at| after(tl, at.before));
    let cause = cause(notes);
    let drop = format!(
        "detected {} s after the stop cue (last page 25 {})",
        round1(secs(detected.at_us - stop.at_us)),
        ran_on.map_or("before the stop cue".into(), |s| format!("{s} s after it"))
    );
    let resumed = after.as_ref().map_or_else(
        || "no".to_string(),
        |a| {
            format!(
                "page 25 first at FE state {}; released first; ERG {TARGET_W} W held at {} W",
                a["first_page25_fe_state"].as_str().unwrap_or("unknown"),
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
    use crate::frames::Record;

    fn check11b(r: &[Record], n: &[Note]) -> Value {
        super::check11b(&Timeline::of_parts(r, &[], n))
    }

    fn note(tag: &str, at_ms: u64, bytes: &[u8]) -> Note {
        Note {
            tag: tag.into(),
            at_us: at_ms * 1000,
            bytes: bytes.to_vec(),
            before: 0,
        }
    }

    fn link(stamps: &[(u8, u64)]) -> Vec<Note> {
        let mut n = vec![note(WINDOW, 30_000, &[STOPPING])];
        n.extend(stamps.iter().map(|(c, ms)| note(LINK, *ms, &[*c])));
        n
    }

    /// A page-81 reply at `at_ms` with serial `serial`.
    fn p81(at_ms: u64, serial: u8) -> Record {
        let mut f = [0xa4, 9, 0x4e, 5, 0x51, 0xff, 0xff, 1, serial, 0, 0, 0, 0];
        f[12] = f[..12].iter().fold(0, |x, v| x ^ v);
        Record {
            at_us: at_ms * 1000,
            rx: true,
            bytes: f.to_vec(),
        }
    }

    /// The same trainer's serial before the drop and after a reconnect at `back_ms`.
    fn trainer(back_ms: u64) -> Vec<Record> {
        vec![p81(1_000, 7), p81(back_ms + 100, 7)]
    }

    #[test]
    fn the_reconnect_is_measured_phase_by_phase_from_the_drop() {
        let v = check11b(
            &trainer(46_300),
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
            &trainer(46_300),
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
        let count = u8::try_from(at_ms / 250 % 256).expect("a byte");
        let mut f = [0xa4, 9, 0x4e, 5, 25, count, 90, 0, 0, watts, 0, 0x30, 0];
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
        let notes = [notes, vec![note(WINDOW, 49_000, &[CONFIRMED])]].concat();
        let erg = crank_fec::ControlPage::TargetPower { watts: 100.0 };
        let mut records = vec![p25(45_200, 0), tx(45_300, &RELEASE), tx(50_000, &erg)];
        records.extend(trainer(45_200));
        // Ramping up in the first 5 s, then 100 W until the release at 70 s.
        records.extend((0..10).map(|k| p25(50_000 + k * 500, 20)));
        records.extend((0..30).map(|k| p25(55_000 + k * 500, 100)));
        records.push(tx(70_000, &RELEASE));
        let a = &check11b(&records, &notes)["after_reconnect"];
        assert_eq!(a["erg_mean_power_w"], json!(100.0));
        assert_eq!(a["first_page25_fe_state"], json!("3 (in use)"));
        // Any control but the release from the stop cue to the operator's y breaks the rule: after
        // the reconnect, between it and the first page 25, between the drop and the advert, and
        // between the stop cue and the drop (the review's repros).
        let erg300 = crank_fec::ControlPage::TargetPower { watts: 300.0 };
        for (at, page, why) in [
            (45_300, erg, "Erg(100.0) at 15.3 s"),
            (48_000, erg, "Erg(100.0) at 18 s"),
            (
                48_000,
                crank_fec::ControlPage::BasicResistance { pct: 20.0 },
                "Resistance(20.0) at 18 s",
            ),
            (45_100, erg300, "Erg(300.0) at 15.1 s"),
            (42_000, erg300, "Erg(300.0) at 12 s"),
            (35_000, erg300, "Erg(300.0) at 5 s"),
            (30_000, erg300, "Erg(300.0) at 0 s"),
        ] {
            let mut r = records.clone();
            r.push(tx(at, &page));
            let v = check11b(&r, &notes);
            assert_eq!(
                v["verdict"],
                json!(format!(
                    "unknown: {why} after the stop cue, before the confirmed resume"
                )),
                "{page:?} at {at} ms"
            );
        }
        // Before the stop cue the ride's ERG is the procedure's own.
        let mut r = records.clone();
        r.push(tx(29_999, &erg300));
        assert!(
            check11b(&r, &notes)["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("dropout ")),
            "a write before the stop cue is no breach"
        );
        // No release after the reconnect, the release only after the y (05, y, release, ERG), or
        // no write at all after it: the y came before the release.
        let no_release: Vec<Record> = records
            .iter()
            .filter(|r| r.rx || r.at_us != 45_300_000)
            .cloned()
            .collect();
        let mut late_release = no_release.clone();
        late_release.push(tx(49_500, &RELEASE));
        let no_write = trainer(45_200);
        for r in [no_release, late_release, no_write] {
            assert_eq!(
                check11b(&r, &notes)["verdict"],
                json!(
                    "unknown: contradictory: the operator's y is stamped before the release after the reconnect"
                )
            );
        }
        // And without a y at all, the re-engaged ERG has none before it.
        let v = check11b(&records, &notes[..notes.len() - 1]);
        assert_eq!(
            v["verdict"],
            json!("unknown: Erg(100.0) at 20 s after the stop cue, before the confirmed resume")
        );
    }

    /// The c2 review's last two gaps: an ERG write at exactly the `y` stamp is no release before
    /// it; and a control page whose field does not decode (page 51 grade 0xFFFF, page 55 rider
    /// 0xFFFF) is still a control write, never the release.
    #[test]
    fn a_write_at_the_y_or_a_malformed_control_page_before_it_gives_unknown() {
        let notes = [
            link(&[(1, 40_000), (2, 44_000), (4, 45_000), (5, 45_200)]),
            vec![note(WINDOW, 49_000, &[CONFIRMED])],
        ]
        .concat();
        let erg = crank_fec::ControlPage::TargetPower { watts: 100.0 };
        let mut at_y = trainer(45_200);
        at_y.push(tx(49_000, &erg));
        assert_eq!(
            check11b(&at_y, &notes)["verdict"],
            json!(
                "unknown: contradictory: the operator's y is stamped before the release after the reconnect"
            )
        );
        // An encoded page with one field set to its "invalid" value, checksum fixed.
        let invalid = |at_ms: u64, page: &crank_fec::ControlPage, field: [usize; 2]| {
            let mut f = tx(at_ms, page);
            for i in field {
                f.bytes[i] = 0xff;
            }
            f.bytes[12] = f.bytes[..12].iter().fold(0, |x, v| x ^ v);
            f
        };
        let track = crank_fec::ControlPage::Track {
            grade_pct: 5.0,
            crr: 0.004,
        };
        let user = crank_fec::ControlPage::UserConfig {
            rider_kg: 75.0,
            bike_kg: 10.0,
            wheel_m: 0.708,
        };
        let grade = invalid(42_000, &track, [9, 10]);
        let rider = invalid(46_000, &user, [5, 6]);
        let mut good = vec![tx(45_300, &RELEASE), tx(50_000, &erg)];
        good.extend(trainer(45_200));
        for (bad, why) in [
            (grade, "Malformed(51) at 12 s"),
            (rider, "Malformed(55) at 16 s"),
        ] {
            let mut r = good.clone();
            r.push(bad);
            assert_eq!(
                check11b(&r, &notes)["verdict"],
                json!(format!(
                    "unknown: {why} after the stop cue, before the confirmed resume"
                ))
            );
        }
        // Only page 48 at exactly 0 % is the release: 0.5 % is a control write.
        let mut half = good.clone();
        half.push(tx(
            46_000,
            &crank_fec::ControlPage::BasicResistance { pct: 0.5 },
        ));
        assert_eq!(
            check11b(&half, &notes)["verdict"],
            json!(
                "unknown: Resistance(0.5) at 16 s after the stop cue, before the confirmed resume"
            )
        );
        assert!(
            check11b(&good, &notes)["verdict"]
                .as_str()
                .is_some_and(|v| v.starts_with("dropout "))
        );
    }

    /// The row is `unknown: <why…>`, with no result.
    fn unknown_with(records: &[Record], notes: &[Note], why: &str) {
        let v = check11b(records, notes);
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with(&format!("unknown: {why}"))),
            "{why}: {v}"
        );
        assert_eq!(v["drop"], Value::Null);
    }

    /// The review's repros and the other breaches of the procedure's rules: each is `unknown`
    /// with its reason, never a result.
    #[test]
    fn stamps_that_break_the_procedures_rules_give_unknown_with_the_reason() {
        let ok = link(&[(1, 40_000), (2, 44_000), (4, 45_000), (5, 45_200)]);
        assert!(
            check11b(&trainer(45_200), &ok)["verdict"]
                .as_str()
                .is_some_and(|v| v.starts_with("dropout ")),
            "the valid sequence has a result"
        );
        for (notes, records, why) in [
            (
                link(&[
                    (1, 40_000),
                    (2, 44_000),
                    (3, 44_500),
                    (4, 45_000),
                    (5, 45_200),
                    (6, 100_000),
                ]),
                trainer(45_200),
                "contradictory link stamps",
            ),
            (
                link(&[(1, 40_000), (2, 44_000), (4, 45_000), (5, 200_000)]),
                trainer(200_000),
                "the first page 25 is stamped 160 s after the drop, past the 60 s bound",
            ),
            (
                link(&[(1, 40_000), (6, 41_000)]),
                vec![],
                "the reconnect is stamped given up 1 s after the drop, before the 60 s bound",
            ),
            (
                link(&[(1, 500_000)]),
                vec![],
                "the drop is stamped 470 s after the stop cue, past the 90 s bound",
            ),
            (
                link(&[(1, 40_000), (6, 100_000), (7, 100_100)]),
                vec![],
                "contradictory link stamps",
            ),
            (link(&[(2, 40_000)]), vec![], "contradictory link stamps"),
            (
                [
                    link(&[(1, 40_000), (6, 100_000)]),
                    vec![note(WINDOW, 101_000, &[CONFIRMED])],
                ]
                .concat(),
                vec![],
                "contradictory: the operator's y is stamped without a reconnect before it",
            ),
            (
                ok.clone(),
                vec![p81(1_000, 7), p81(45_300, 8)],
                "a different trainer answered after the reconnect (page-81 serials differ)",
            ),
            (
                ok.clone(),
                vec![p81(1_000, 7)],
                "no page-81 serial on one side of the drop",
            ),
            (
                ok.clone(),
                vec![p81(1_000, 7), p81(2_000, 8), p81(45_300, 7)],
                "page-81 serials disagree on one side of the drop",
            ),
        ] {
            unknown_with(&records, &notes, why);
        }
    }

    /// A skipped phase, and stamps just past each bound, are `unknown`; the bounds' edges are not.
    #[test]
    fn a_skipped_phase_or_a_stamp_past_a_bound_gives_unknown() {
        for (notes, records, why) in [
            // No phase is skipped but the optional connect: seen → first page 25, drop →
            // connected, drop → discovered.
            (
                link(&[(1, 40_000), (2, 44_000), (5, 45_200)]),
                trainer(45_200),
                "contradictory link stamps",
            ),
            (
                link(&[(1, 40_000), (3, 44_500), (4, 45_000), (5, 45_200)]),
                trainer(45_200),
                "contradictory link stamps",
            ),
            (
                link(&[(1, 40_000), (4, 45_000), (5, 45_200)]),
                trainer(45_200),
                "contradictory link stamps",
            ),
            // Just past each bound: the first page 25 at 60.1 s, the drop at 90.3 s (the 0.2 s
            // poll slack included), a give-up 0.1 s early or 1.1 s late (past its 1 s slack),
            // and the review's give-up 500 s after the drop.
            (
                link(&[(1, 40_000), (2, 90_000), (4, 99_000), (5, 100_100)]),
                trainer(100_100),
                "the first page 25 is stamped 60.1 s after the drop, past the 60 s bound",
            ),
            (
                link(&[(1, 120_300)]),
                vec![],
                "the drop is stamped 90.3 s after the stop cue, past the 90 s bound",
            ),
            (
                link(&[(1, 40_000), (6, 99_900)]),
                vec![],
                "the reconnect is stamped given up 59.9 s after the drop, before the 60 s bound",
            ),
            (
                link(&[(1, 40_000), (6, 101_100)]),
                vec![],
                "the reconnect is stamped given up 61.1 s after the drop, past the 60 s bound",
            ),
            (
                link(&[(1, 40_000), (6, 540_000)]),
                vec![],
                "the reconnect is stamped given up 500 s after the drop, past the 60 s bound",
            ),
        ] {
            unknown_with(&records, &notes, why);
        }
        // The bounds' edges: a drop 90 s after the cue (and its poll slack), a reconnect at 60 s, a
        // give-up at 60 s and 1 s after.
        for (notes, records) in [
            (link(&[(1, 120_000)]), vec![]),
            (link(&[(1, 120_200)]), vec![]),
            (link(&[(1, 40_000), (6, 101_000)]), vec![]),
            (
                link(&[(1, 40_000), (2, 90_000), (4, 99_000), (5, 100_000)]),
                trainer(100_000),
            ),
            (link(&[(1, 40_000), (6, 100_000)]), vec![]),
        ] {
            let v = check11b(&records, &notes);
            assert!(
                v["verdict"]
                    .as_str()
                    .is_some_and(|x| x.starts_with("dropout ")),
                "{notes:?}: {v}"
            );
        }
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
        assert_eq!(v["verdict"], json!("unknown: the capture has no drop"));
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
