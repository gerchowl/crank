//! The version-2 record (ADR-0009 §2.3.2, #717): every compliance class produced from engine
//! state, the controlled target, block ends, how the ride finished, the decoder, and the
//! engine/domain lap tie. On the manual-clock rig of `tests.rs`.
use crate::engine::EngineCmd;
use crate::lap::Lap;
use crate::record::{
    BLOCK_METRIC, METRICS, METRICS_V1, RideFinish, SecondControl, SecondRow, decode_record,
};
use crate::tests::{Rig, block, rig};
use crate::ticks::TICK_US;
use crank_devices::{
    CommandStatus, MonoTime, ObservationSource, ObservedControl, TrainerEvent, TrainerObservation,
};
use crank_domain::record::{BlockEnd, ComplianceClass as C, IntervalSecond, interval_stats};
use crank_domain::workout::{FlatBlock, Phase};
use std::collections::BTreeMap;

fn work(dur_s: f64, w: f64) -> FlatBlock {
    block(dur_s, w, w, Phase::Work)
}
/// Ticks `from..=to`: a sample of `w` W at `rpm` and 140 bpm on each, processed one by one.
fn ride(r: &mut Rig, from: u64, to: u64, w: u16, rpm: u8) {
    for k in from..=to {
        r.sample(k, w, rpm);
        r.heart(k, 140);
        r.through(k);
    }
}
fn stop(r: &mut Rig) {
    r.engine.command(EngineCmd::Stop).expect("stop");
}
fn control(row: &SecondRow) -> SecondControl {
    row.control.expect("the engine writes version 2")
}
fn classes(r: &Rig) -> Vec<C> {
    r.rows()
        .iter()
        .map(|(_, row)| control(row).compliance)
        .collect()
}
fn all(classes: &[C], want: C) -> bool {
    !classes.is_empty() && classes.iter().all(|c| *c == want)
}
/// The trainer echoes an ERG target of `w` W (page 71) in tick `k`.
fn echo(r: &Rig, k: u64, w: u16) {
    r.events
        .try_send(TrainerEvent::Observed(TrainerObservation {
            at: MonoTime(k * TICK_US + 2),
            source: ObservationSource::Page71Echo,
            control: Some(ObservedControl::Erg { quarter_w: w * 4 }),
            status: Some(CommandStatus::Pass),
            target_limit: None,
        }))
        .expect("channel");
}

#[test]
fn counted_seconds_are_within_below_or_above_the_controlled_target() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    ride(&mut r, 80, 119, 150, 90);
    ride(&mut r, 120, 159, 260, 90);
    stop(&mut r);
    let c = classes(&r);
    assert_eq!(c.len(), 40);
    // the ride start is an engagement: its soft start and 5 s are settle (ADR-0002 §7.8 c)
    assert!(all(&c[..10], C::Settling), "{c:?}");
    assert!(all(&c[10..20], C::Within), "{c:?}");
    // the 3 s power has left the band one second after the step
    assert!(all(&c[21..30], C::Below), "{c:?}");
    assert!(all(&c[32..40], C::Above), "{c:?}");
    for (_, row) in r.rows() {
        let k = control(&row);
        assert_eq!(k.target_w, Some(200.0), "the target the rider was given");
        assert_eq!((k.adj_rider_pct, k.adj_coach_pct), (0.0, 0.0));
    }
    // the live tally is the sum of the classes
    let t = r.status().compliance;
    let n = |f: fn(&C) -> bool| u32::try_from(c.iter().filter(|x| f(x)).count()).unwrap();
    assert_eq!(t.counted_s, n(|c| c.counted()));
    assert_eq!(t.within_s, n(|c| *c == C::Within));
    assert_eq!(t.excluded_s, n(|c| c.excluded()));
}

#[test]
fn a_free_block_is_class_0_without_a_target_and_its_seconds_are_the_self_paced_ones() {
    let mut r = rig();
    r.load(vec![block(5.0, 0.0, 0.0, Phase::Free), work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    stop(&mut r);
    let rows = r.rows();
    let c = classes(&r);
    assert!(all(&c[..5], C::NoTarget), "{c:?}");
    assert!(
        all(&c[5..15], C::Settling),
        "the return to ERG owes a soft start: {c:?}"
    );
    assert!(all(&c[15..], C::Within), "{c:?}");
    // the self-paced rule's input (ADR-0009 §2.3.2): a riding second with no target
    let self_paced: Vec<u32> = rows
        .iter()
        .filter(|(_, row)| row.workout_s.is_some() && control(row).target_w.is_none())
        .map(|(s, _)| *s)
        .collect();
    assert_eq!(self_paced, vec![0, 1, 2, 3, 4]);
    assert!(
        rows[5..]
            .iter()
            .all(|(_, row)| control(row).target_w == Some(200.0))
    );
    // class 0 is in none of the live counts
    let t = r.status().compliance;
    assert_eq!((t.counted_s, t.excluded_s), (5, 10), "{t:?}");
}

#[test]
fn paused_seconds_are_class_6_and_the_resume_settles() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    r.engine.command(EngineCmd::Pause).expect("pause");
    // the trainer keeps sending (a silent one would be a loss, also class 6)
    ride(&mut r, 80, 99, 0, 90);
    r.engine.command(EngineCmd::Resume).expect("resume");
    ride(&mut r, 100, 199, 200, 90);
    stop(&mut r);
    let rows = r.rows();
    let c = classes(&r);
    assert!(all(&c[20..25], C::Paused), "{c:?}");
    assert!(
        rows[20..25]
            .iter()
            .all(|(_, row)| row.workout_s.is_none() && control(row).target_w.is_none())
    );
    assert_eq!(c[25], C::Settling, "{c:?}");
    assert_eq!(c.last(), Some(&C::Within));
}

#[test]
fn a_low_cadence_release_and_its_re_engage_are_released_not_settling_or_not_following() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    // 20 rpm (not a stop: power stays above 0) releases after 5 s (ADR-0002 §7.4 defaults)
    ride(&mut r, 80, 139, 100, 20);
    assert!(r.status().erg_released);
    // 90 rpm re-engages after 1.5 s, then the soft start and its settle
    ride(&mut r, 140, 279, 200, 90);
    assert!(!r.status().erg_released);
    stop(&mut r);
    let c = classes(&r);
    let first = c.iter().position(|c| *c == C::Released).expect("a release");
    let back = first
        + c[first..]
            .iter()
            .position(|c| *c == C::Within)
            .expect("counted again");
    assert!(all(&c[first..back], C::Released), "{c:?}");
    // the release (10 s) and the re-engage: its 1.5 s dwell, the 5 s ramp and 5 s settle
    assert!(back - first >= 20, "{first}..{back}: {c:?}");
    assert!(all(&c[back..], C::Within), "{c:?}");
    // before the release: low-cadence seconds still count (off target)
    assert!(c[20..first].iter().all(|c| c.counted()), "{c:?}");
    assert!(
        !c[10..]
            .iter()
            .any(|c| matches!(c, C::Settling | C::Suspect | C::NotFollowing)),
        "{c:?}"
    );
}

#[test]
fn a_stuck_trainer_is_suspect_then_not_following_and_outranks_the_start_settle() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    // the trainer echoes 100 W against a 200 W target from tick 4: an episode opens (suspect),
    // then confirms (not following)
    for k in 0..=79 {
        r.sample(k, 150, 90);
        if k >= 4 && k % 4 == 0 {
            echo(&r, k, 100);
        }
        r.through(k);
    }
    stop(&mut r);
    let c = classes(&r);
    let suspect = c.iter().position(|c| *c == C::Suspect).expect("suspect");
    let not = c
        .iter()
        .position(|c| *c == C::NotFollowing)
        .expect("not following");
    assert!(
        suspect < not && not < 10,
        "inside the start's settle: {c:?}"
    );
    assert!(all(&c[..suspect], C::Settling), "{c:?}");
    assert!(all(&c[suspect..not], C::Suspect), "{c:?}");
    assert!(all(&c[not..], C::NotFollowing), "{c:?}");
}

#[test]
fn an_erg_second_without_3_s_power_is_class_9() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    // page 25 keeps arriving (no loss, no pause) with an invalid power field
    for k in 80..=119 {
        r.sample_opt(k, None, Some(90));
        r.through(k);
    }
    stop(&mut r);
    let rows = r.rows();
    let c = classes(&r);
    assert!(rows[20..].iter().all(|(_, row)| row.power.is_none()));
    // the 3 s window still holds power for three seconds
    assert!(all(&c[23..], C::NoPower), "{c:?}");
    assert!(
        rows[23..].iter().all(|(_, row)| row.workout_s.is_some()),
        "riding"
    );
    assert!(r.status().compliance.excluded_s >= 17);
}

fn ends(r: &Rig) -> Vec<(u32, BlockEnd)> {
    r.rows()
        .iter()
        .filter_map(|(s, row)| control(row).block_end.map(|e| (*s, e)))
        .collect()
}

#[test]
fn each_block_ends_on_its_last_riding_second_and_the_ride_end_says_how() {
    let mut r = rig();
    r.load(vec![work(5.0, 200.0), work(5.0, 200.0), work(600.0, 200.0)]);
    r.start();
    // block 0 runs out at 5 s; 2 s of block 1, then the rider skips it
    ride(&mut r, 0, 27, 200, 90);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    // 3 s of block 2, a pause (its seconds are no block's), then End
    ride(&mut r, 28, 39, 200, 90);
    r.engine.command(EngineCmd::Pause).expect("pause");
    r.through(47);
    stop(&mut r);
    assert_eq!(r.rows().len(), 12, "the paused tail is recorded too");
    assert_eq!(
        ends(&r),
        vec![
            (4, BlockEnd::Completed),
            (6, BlockEnd::Skipped),
            (9, BlockEnd::Stopped)
        ]
    );
    assert_eq!(r.finished_how(), Some(RideFinish::Stopped));

    // the workout runs out: its last block completes, and so does the ride
    let mut r = rig();
    r.load(vec![work(5.0, 200.0), work(5.0, 250.0)]);
    r.start();
    ride(&mut r, 0, 39, 200, 90);
    assert_eq!(
        ends(&r),
        vec![(4, BlockEnd::Completed), (9, BlockEnd::Completed)]
    );
    assert_eq!(r.finished_how(), Some(RideFinish::Completed));

    // skipping the last block also runs the workout out
    let mut r = rig();
    r.load(vec![work(5.0, 200.0), work(600.0, 250.0)]);
    r.start();
    ride(&mut r, 0, 31, 200, 90);
    r.engine.command(EngineCmd::SkipBlock).expect("skip");
    assert_eq!(
        ends(&r),
        vec![(4, BlockEnd::Completed), (7, BlockEnd::Skipped)]
    );
    assert_eq!(r.finished_how(), Some(RideFinish::Completed));

    // a ride without blocks has no block ends
    let mut r = rig();
    r.load(vec![]);
    r.start();
    ride(&mut r, 0, 19, 150, 90);
    r.engine.command(EngineCmd::SkipBlock).expect("lap");
    ride(&mut r, 20, 39, 150, 90);
    stop(&mut r);
    assert!(ends(&r).is_empty());
    assert!(classes(&r).iter().all(|c| *c == C::NoTarget));
}

/// The record rows a lap owns: a second belongs to the lap open at its last tick.
fn rows_of(rows: &[(u32, SecondRow)], lap: &Lap) -> Vec<IntervalSecond> {
    rows.iter()
        .filter(|(s, _)| {
            let end = f64::from(*s + 1);
            end > lap.start_s && end <= lap.end_s
        })
        .map(|(_, row)| row.interval_second())
        .collect()
}

/// ADR-0009 §2.3.2: the engine's live lap statistics equal the domain's interval function on
/// the same record seconds.
#[test]
fn the_live_lap_stats_equal_the_domain_interval_function() {
    let mut r = rig();
    r.load(vec![
        work(20.0, 150.0),
        work(30.0, 250.0),
        block(20.0, 0.0, 0.0, Phase::Free),
        work(600.0, 200.0),
    ]);
    r.start();
    for k in 0..=399 {
        // a power gap, an HR gap, varying power
        let power = (!(150..170).contains(&k)).then(|| 140 + u16::try_from(k % 37).unwrap() * 3);
        r.sample_opt(k, power, Some(85));
        if !(200..240).contains(&k) {
            r.heart(k, 130 + u8::try_from(k % 20).unwrap());
        }
        r.through(k);
        if k == 247 {
            r.engine.command(EngineCmd::Pause).expect("pause");
        }
        if k == 263 {
            r.engine.command(EngineCmd::Resume).expect("resume");
        }
        if k == 271 {
            r.engine
                .command(EngineCmd::SkipBlock)
                .expect("skip the free block");
        }
    }
    stop(&mut r);
    let rows = r.rows();
    let laps = r.engine.closed_laps();
    assert_eq!(laps.len(), 4);
    for lap in &laps {
        let d = interval_stats(&rows_of(&rows, lap));
        let s = &lap.stats;
        assert_eq!(
            (d.avg_power_w, d.np_w, d.avg_hr_bpm, d.avg_cadence_rpm),
            (s.avg_power_w, s.np_w, s.avg_hr_bpm, s.avg_cadence_rpm),
            "lap {}",
            lap.idx
        );
        assert_eq!(
            (d.paused_s, d.power_gap_s, d.cadence_gap_s, d.hr_gap_s),
            (s.paused_s, s.gap_s.power_s, s.gap_s.cadence_s, s.gap_s.hr_s),
            "lap {}",
            lap.idx
        );
        assert_eq!(
            (d.counted_s, d.within_s, d.excluded_s),
            (
                s.compliance.counted_s,
                s.compliance.within_s,
                s.compliance.excluded_s
            ),
            "lap {}",
            lap.idx
        );
    }
    // the ride exercises every part of the tie
    let all = interval_stats(
        &rows
            .iter()
            .map(|(_, r)| r.interval_second())
            .collect::<Vec<_>>(),
    );
    assert!(
        all.paused_s > 0 && all.power_gap_s > 0 && all.hr_gap_s > 0,
        "{all:?}"
    );
    assert!(all.counted_s > 0 && all.excluded_s > 0, "{all:?}");
}

fn columns(rows: &[(u32, SecondRow)], names: &[&str]) -> BTreeMap<String, Vec<Option<f32>>> {
    let mut cols: BTreeMap<String, Vec<Option<f32>>> = BTreeMap::new();
    for (_, row) in rows {
        let values = row.values();
        for name in names {
            let i = METRICS.iter().position(|m| m == name).expect("a metric");
            cols.entry((*name).to_owned()).or_default().push(values[i]);
        }
        #[expect(clippy::cast_precision_loss, reason = "block indices are small")]
        let block = row.block.map(|b| b as f32);
        cols.entry(BLOCK_METRIC.to_owned()).or_default().push(block);
    }
    cols
}

#[test]
fn the_decoder_reads_both_versions() {
    let mut r = rig();
    r.load(vec![block(5.0, 0.0, 0.0, Phase::Free), work(10.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 39, 200, 90);
    r.engine.command(EngineCmd::Pause).expect("pause");
    r.through(47);
    stop(&mut r);
    let rows = r.rows();
    let want: Vec<SecondRow> = rows.iter().map(|(_, row)| *row).collect();
    assert_eq!(
        decode_record(2, &columns(&rows, &METRICS)),
        Ok(want.clone())
    );
    // a version-1 record: the same seconds without the control metrics
    let v1 = decode_record(1, &columns(&rows, &METRICS_V1)).expect("v1");
    let base: Vec<SecondRow> = want
        .iter()
        .map(|row| SecondRow {
            control: None,
            ..*row
        })
        .collect();
    assert_eq!(v1, base);
    // what a version cannot be read without
    assert!(decode_record(2, &columns(&rows, &METRICS_V1)).is_err());
    assert!(decode_record(3, &columns(&rows, &METRICS)).is_err());
    let mut bad = columns(&rows, &METRICS);
    bad.get_mut("compliance").expect("present")[0] = Some(10.0);
    assert!(decode_record(2, &bad).is_err());
    let mut bad = columns(&rows, &METRICS);
    bad.get_mut("block_end").expect("present")[0] = Some(1.5);
    assert!(decode_record(2, &bad).is_err());
    let mut bad = columns(&rows, &METRICS);
    bad.get_mut("power").expect("present").pop();
    assert!(decode_record(2, &bad).is_err());
}

/// Review of #763: a ride started while the previous ride's end mode is still owed sends no
/// control for up to 2 s (#279). Those ERG seconds keep the block's target and are settling, so
/// they are never read as free (self-paced) seconds.
#[test]
fn a_start_gated_by_the_previous_end_mode_keeps_its_target_and_settles() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 39, 200, 90);
    stop(&mut r); // the end mode is sent and not confirmed: the next ride is gated
    let first = r.rows().len();
    r.modes();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 40, 119, 200, 90);
    assert!(
        r.modes().contains(&crate::tests::erg(200)),
        "ERG once the gate has passed"
    );
    stop(&mut r);
    let rows = r.rows();
    let second: Vec<_> = rows[first..].iter().map(|(_, row)| control(row)).collect();
    assert_eq!(second.len(), 20);
    assert!(
        second.iter().all(|c| c.target_w == Some(200.0)),
        "{second:?}"
    );
    assert!(
        second[..10].iter().all(|c| c.compliance == C::Settling),
        "{second:?}"
    );
    assert!(second.iter().all(|c| c.compliance != C::NoTarget));
}

#[test]
fn a_target_jump_at_a_block_boundary_settles_for_five_seconds() {
    let mut r = rig();
    r.load(vec![work(20.0, 150.0), work(600.0, 250.0)]);
    r.start();
    ride(&mut r, 0, 79, 150, 90);
    ride(&mut r, 80, 159, 250, 90);
    stop(&mut r);
    let rows = r.rows();
    let c = classes(&r);
    assert!(all(&c[10..20], C::Within), "{c:?}");
    assert!(all(&c[20..25], C::Settling), "{c:?}");
    assert!(all(&c[25..], C::Within), "{c:?}");
    assert_eq!(control(&rows[19].1).target_w, Some(150.0));
    assert_eq!(control(&rows[20].1).target_w, Some(250.0));
}

/// A release re-engaged during a pause: the soft start after the resume is owed for the
/// pause, a settle of its own (class 4), not the release's (class 5).
#[test]
fn a_re_engage_during_a_pause_leaves_the_resume_settling_not_released() {
    let mut r = rig();
    r.load(vec![work(600.0, 200.0)]);
    r.start();
    ride(&mut r, 0, 79, 200, 90);
    ride(&mut r, 80, 139, 100, 20);
    assert!(r.status().erg_released);
    r.engine.command(EngineCmd::Pause).expect("pause");
    ride(&mut r, 140, 179, 100, 90);
    assert!(!r.status().erg_released, "re-engaged while paused");
    r.engine.command(EngineCmd::Resume).expect("resume");
    ride(&mut r, 180, 279, 200, 90);
    stop(&mut r);
    let c = classes(&r);
    assert!(all(&c[35..45], C::Paused), "{c:?}");
    assert!(all(&c[45..55], C::Settling), "{c:?}");
    assert!(all(&c[55..], C::Within), "{c:?}");
}

/// A trainer dropout that recovers changes nothing of how the ride ends.
#[test]
fn a_ride_with_a_recovered_dropout_ends_as_it_would_without_one() {
    for stopped in [true, false] {
        let mut r = rig();
        r.load(vec![work(60.0, 200.0)]);
        r.start();
        ride(&mut r, 0, 79, 200, 90);
        // 20 silent ticks: the trainer is lost and the ride paused
        r.through(99);
        assert_eq!(
            r.state(),
            crate::RideState::Paused(crate::PauseReason::TrainerLost)
        );
        ride(&mut r, 100, 139, 200, 90);
        assert_eq!(r.state(), crate::RideState::Riding, "recovered");
        if stopped {
            stop(&mut r);
        } else {
            ride(&mut r, 140, 299, 200, 90);
            assert_eq!(r.state(), crate::RideState::Finished);
        }
        let c = classes(&r);
        assert!(c.contains(&C::Paused), "{c:?}");
        let (how, end) = if stopped {
            (RideFinish::Stopped, BlockEnd::Stopped)
        } else {
            (RideFinish::Completed, BlockEnd::Completed)
        };
        assert_eq!(r.finished_how(), Some(how));
        assert_eq!(
            ends(&r).iter().map(|(_, e)| *e).collect::<Vec<_>>(),
            vec![end]
        );
    }
}
