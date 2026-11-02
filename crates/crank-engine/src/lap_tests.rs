//! Laps and the live cursor (#243 part a: #12, #14), on the manual-clock rig of `tests.rs`.
use crate::engine::{CommandError, EngineCmd, PauseReason, RideState};
use crate::lap::{Lap, LapBlock, LapEnd};
use crate::record::SecondRow;
use crate::tests::{Rig, block, rig};
use crank_domain::workout::Phase;

/// Ride ticks `from..=to`: a sample of `power(k)` W at 90 rpm and 140 bpm on each.
fn pedal(r: &mut Rig, from: u64, to: u64, power: impl Fn(u64) -> u16) {
    for k in from..=to {
        r.sample(k, power(k), 90);
        r.heart(k, 140);
        r.through(k);
    }
}
fn laps(r: &Rig) -> Vec<Lap> {
    r.engine.closed_laps()
}
/// The record rows a lap owns: a second belongs to the lap open at its last tick.
fn rows_of(rows: &[(u32, SecondRow)], lap: &Lap) -> Vec<SecondRow> {
    rows.iter()
        .filter(|(s, _)| {
            let end = f64::from(*s + 1);
            end > lap.start_s && end <= lap.end_s
        })
        .map(|(_, row)| *row)
        .collect()
}

#[test]
fn each_block_is_a_lap_with_its_record_statistics_and_compliance() {
    let mut r = rig();
    let mut blocks = vec![
        block(10.0, 100.0, 100.0, Phase::Work),
        block(10.0, 200.0, 200.0, Phase::Work),
    ];
    for (i, b) in blocks.iter_mut().enumerate() {
        b.node_id = format!("n{i}");
        b.path = vec![0, i];
        b.iteration = vec![i];
    }
    r.load(blocks);
    r.start();
    pedal(&mut r, 0, 79, |k| if k < 40 { 100 } else { 200 });
    assert_eq!(r.state(), RideState::Finished);
    let laps = laps(&r);
    assert_eq!(laps.len(), 2);
    assert_eq!(r.status().laps_closed, 2);
    for (i, (lap, watts)) in laps.iter().zip([100.0, 200.0]).enumerate() {
        let i32 = u32::try_from(i).unwrap();
        assert_eq!(lap.idx, i32);
        assert_eq!(
            lap.block,
            Some(LapBlock {
                index: i32,
                node_id: format!("n{i}"),
                path: vec![0, i32],
                iteration: vec![i32],
            })
        );
        assert_eq!(lap.end, LapEnd::Completed);
        let start = 10.0 * f64::from(i32);
        assert_eq!((lap.start_s, lap.end_s), (start, start + 10.0));
        assert_eq!(
            (lap.workout_start_s, lap.workout_end_s),
            (start, start + 10.0)
        );
        let st = &lap.stats;
        assert_eq!(st.avg_power_w, Some(watts));
        assert_eq!(st.np_w, Some(watts));
        assert_eq!(st.avg_cadence_rpm, Some(90.0));
        assert_eq!(st.avg_hr_bpm, Some(140.0));
        assert_eq!((st.paused_s, st.gap_s.power_s, st.gap_s.hr_s), (0, 0, 0));
        // every second of the lap is classified exactly once
        let c = st.compliance;
        assert_eq!(c.counted_s + c.excluded_s, 10, "lap {i}: {c:?}");
    }
    // the laps' tallies add up to the ride's
    let ride = r.status().compliance;
    let sum = |f: fn(&Lap) -> u32| laps.iter().map(f).sum::<u32>();
    assert_eq!(sum(|l| l.stats.compliance.counted_s), ride.counted_s);
    assert_eq!(sum(|l| l.stats.compliance.within_s), ride.within_s);
    assert_eq!(sum(|l| l.stats.compliance.excluded_s), ride.excluded_s);
    // the second block starts on a fixed 100 → 200 W discontinuity: its first 5 s settle
    assert!(laps[1].stats.compliance.excluded_s >= 5);
    assert!(laps[1].stats.compliance.within_s > 0);
}

#[test]
fn a_second_split_by_a_boundary_belongs_to_the_lap_of_its_last_tick() {
    let mut r = rig();
    // 2.5 s blocks: the boundary at tick 10 splits session second 2 (ticks 8..=11).
    r.load(vec![
        block(2.5, 100.0, 100.0, Phase::Work),
        block(2.5, 200.0, 200.0, Phase::Work),
    ]);
    r.start();
    pedal(&mut r, 0, 19, |k| if k < 10 { 100 } else { 200 });
    let laps = laps(&r);
    assert_eq!(laps.len(), 2);
    assert_eq!((laps[0].start_s, laps[0].end_s), (0.0, 2.5));
    assert_eq!((laps[1].start_s, laps[1].end_s), (2.5, 5.0));
    // lap 0 owns seconds 0 and 1; lap 1 owns 2 (100,100,200,200 → 150), 3 and 4
    assert_eq!(laps[0].stats.avg_power_w, Some(100.0));
    let mean = (150.0 + 200.0 + 200.0) / 3.0;
    assert!((laps[1].stats.avg_power_w.unwrap() - mean).abs() < 1e-9);
    let classified = |l: &Lap| l.stats.compliance.counted_s + l.stats.compliance.excluded_s;
    assert_eq!((classified(&laps[0]), classified(&laps[1])), (2, 3));
}

#[test]
fn a_skip_ends_the_lap_at_the_skip_tick_and_moves_the_cursor_there() {
    let mut r = rig();
    r.load(vec![
        block(60.0, 150.0, 150.0, Phase::Work),
        block(60.0, 150.0, 150.0, Phase::Work),
    ]);
    r.start();
    let st = r.status();
    assert_eq!(
        (
            st.block_index,
            st.block_start_session,
            st.block_start_workout
        ),
        (
            Some(0),
            Some(crate::ticks::SessionTicks(0)),
            Some(crate::ticks::WorkoutTicks(0))
        )
    );
    pedal(&mut r, 0, 21, |_| 150);
    r.engine.command(EngineCmd::SkipBlock).unwrap();
    let st = r.status();
    assert_eq!(st.block_index, Some(1));
    assert_eq!(st.block_start_session.map(|t| t.0), Some(22));
    assert_eq!(st.block_start_workout.map(|t| t.0), Some(22));
    assert_eq!(st.block_elapsed.0, 0);
    assert_eq!(st.laps_closed, 1);
    let lap = &laps(&r)[0];
    assert_eq!(lap.end, LapEnd::Skipped);
    assert_eq!((lap.start_s, lap.end_s), (0.0, 5.5));
    assert_eq!((lap.workout_start_s, lap.workout_end_s), (0.0, 5.5));
    pedal(&mut r, 22, 41, |_| 150);
    r.engine.command(EngineCmd::Stop).unwrap();
    let laps = laps(&r);
    assert_eq!(laps.len(), 2);
    assert_eq!(laps[1].end, LapEnd::Stopped);
    assert_eq!((laps[1].start_s, laps[1].end_s), (5.5, 10.5));
    assert_eq!(
        r.status().laps_closed,
        2,
        "a finished ride keeps its lap count"
    );
    // a ride stopped mid-block keeps naming that block, and its start with it
    let st = r.status();
    assert_eq!(st.block_index, Some(1));
    assert_eq!(st.block_start_session.map(|t| t.0), Some(22));
}

#[test]
fn the_cursor_keeps_the_actual_block_start_across_a_pause_and_a_dropout() {
    let mut r = rig();
    r.load(vec![
        block(10.0, 150.0, 150.0, Phase::Work),
        block(600.0, 150.0, 150.0, Phase::Work),
    ]);
    r.start();
    let check = |r: &Rig| {
        let st = r.status();
        assert_eq!(
            st.block_start_workout.map(|w| w.0 + st.block_elapsed.0),
            Some(st.workout.0),
            "start + elapsed = workout time at session tick {}",
            st.session.0
        );
    };
    pedal(&mut r, 0, 49, |_| 150);
    let st = r.status();
    assert_eq!(st.block_index, Some(1));
    assert_eq!(st.block_start_session.map(|t| t.0), Some(40));
    // the rider pauses for 8 s: session time runs, the block start stays
    r.engine.command(EngineCmd::Pause).unwrap();
    for k in 50..=81 {
        r.sample(k, 150, 90);
        r.through(k);
        check(&r);
    }
    r.engine.command(EngineCmd::Resume).unwrap();
    pedal(&mut r, 82, 89, |_| 150);
    // the trainer goes silent: loss after 8 ticks, paused until samples return
    for k in 90..=109 {
        r.through(k);
        check(&r);
    }
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    for k in 110..=159 {
        r.sample(k, 150, 90);
        r.heart(k, 140);
        r.through(k);
        check(&r);
    }
    assert_eq!(r.state(), RideState::Riding);
    let st = r.status();
    assert_eq!(
        (
            st.block_index,
            st.block_start_session.map(|t| t.0),
            st.block_start_workout.map(|t| t.0)
        ),
        (Some(1), Some(40), Some(40)),
        "neither pause nor dropout moves the actual start"
    );
    assert!(st.workout.0 < st.session.0, "workout time stood still");
    // the lap's pause and gap accounting matches the record seconds it owns
    r.engine.command(EngineCmd::Stop).unwrap();
    let laps = laps(&r);
    let lap = &laps[1];
    let rows = rows_of(&r.rows(), lap);
    let paused = rows.iter().filter(|w| w.workout_s.is_none()).count();
    let riding = |f: fn(&SecondRow) -> bool| {
        rows.iter()
            .filter(|w| w.workout_s.is_some() && f(w))
            .count()
    };
    let st = &lap.stats;
    assert!(paused >= 8, "the rider's pause and the loss: {paused}");
    assert_eq!(st.paused_s as usize, paused);
    assert_eq!(st.gap_s.power_s as usize, riding(|w| w.power.is_none()));
    assert_eq!(st.gap_s.hr_s as usize, riding(|w| w.hr.is_none()));
    assert!(
        st.gap_s.power_s >= 1,
        "the ≤ 2 s before the loss was detected ride with no power"
    );
    assert_eq!(st.avg_power_w, Some(150.0), "gaps and pauses are excluded");
}

#[test]
fn a_free_ride_is_one_lap_and_the_lap_action_opens_the_next() {
    let mut r = rig();
    r.load(vec![]);
    r.start();
    assert_eq!(r.status().block_index, None);
    assert_eq!(r.status().block_start_session, None);
    pedal(&mut r, 0, 19, |_| 120);
    r.engine.command(EngineCmd::SkipBlock).unwrap();
    assert_eq!(r.state(), RideState::Riding, "the free ride goes on");
    assert_eq!(r.status().laps_closed, 1);
    pedal(&mut r, 20, 31, |_| 180);
    r.engine.command(EngineCmd::Stop).unwrap();
    let laps = laps(&r);
    assert_eq!(laps.len(), 2);
    assert_eq!(laps[0].block, None);
    assert_eq!(laps[0].end, LapEnd::Skipped);
    assert_eq!((laps[0].start_s, laps[0].end_s), (0.0, 5.0));
    assert_eq!(laps[0].stats.avg_power_w, Some(120.0));
    assert_eq!(laps[1].end, LapEnd::Stopped);
    assert_eq!((laps[1].start_s, laps[1].end_s), (5.0, 8.0));
    assert_eq!(laps[1].stats.avg_power_w, Some(180.0));
}

#[test]
fn skip_outside_a_running_ride_is_still_refused() {
    let mut r = rig();
    assert_eq!(
        r.engine.command(EngineCmd::SkipBlock),
        Err(CommandError::NotLoaded)
    );
    r.load(vec![]);
    assert_eq!(
        r.engine.command(EngineCmd::SkipBlock),
        Err(CommandError::WrongState)
    );
    assert!(laps(&r).is_empty());
}

/// Normalized power written out from its definition, window by window (no running sums): the
/// fourth-power mean of every 30 s mean, to the fourth root.
fn np_by_windows(power: &[f64]) -> f64 {
    let windows: Vec<f64> = power
        .windows(30)
        .map(|w| (w.iter().sum::<f64>() / 30.0).powi(4))
        .collect();
    let n = f64::from(u32::try_from(windows.len()).unwrap());
    (windows.iter().sum::<f64>() / n).powf(0.25)
}
fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}
/// One long block; ride whole seconds `from..to` at `power(s)` W (`None`: no power), 90 rpm.
fn seconds(r: &mut Rig, from: u64, to: u64, power: impl Fn(u64) -> Option<u16>) {
    for s in from..to {
        for k in 4 * s..4 * s + 4 {
            r.sample_opt(k, power(s), Some(90));
            r.through(k);
        }
    }
}

#[test]
fn np_of_a_long_lap_is_the_rolling_fourth_power_mean_not_the_mean() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    // 30 s at 100 W then 30 s at 300 W: the window ending j seconds into the 300 W half holds
    // j seconds of 300 W, so its mean is 100 + 200·j/30 for j = 0..=30.
    seconds(&mut r, 0, 60, |s| Some(if s < 30 { 100 } else { 300 }));
    r.engine.command(EngineCmd::Stop).unwrap();
    let st = &laps(&r)[0].stats;
    let fourth: f64 = (0..=30)
        .map(|j| (100.0 + 200.0 * f64::from(j) / 30.0).powi(4))
        .sum();
    let np = (fourth / 31.0).powf(0.25);
    assert_eq!(st.avg_power_w, Some(200.0));
    assert!(np > 220.0, "the hard half weighs more: {np}");
    assert!(close(st.np_w, np), "NP {:?}, expected {np}", st.np_w);
}

#[test]
fn np_of_a_lap_under_thirty_seconds_is_its_mean() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    seconds(&mut r, 0, 20, |s| Some(if s < 10 { 100 } else { 300 }));
    r.engine.command(EngineCmd::Stop).unwrap();
    let st = &laps(&r)[0].stats;
    assert_eq!(st.avg_power_w, Some(200.0));
    assert!(close(st.np_w, 200.0), "{:?}", st.np_w);
}

#[test]
fn np_joins_the_seconds_with_power_end_to_end_across_pauses_and_gaps() {
    let mut r = rig();
    r.load(vec![block(600.0, 200.0, 200.0, Phase::Work)]);
    r.start();
    // 20 s at 100 W, an 8 s rider pause, 5 s without power, 20 s at 300 W
    seconds(&mut r, 0, 20, |_| Some(100));
    r.engine.command(EngineCmd::Pause).unwrap();
    seconds(&mut r, 20, 28, |_| Some(100));
    r.engine.command(EngineCmd::Resume).unwrap();
    seconds(&mut r, 28, 33, |_| None);
    seconds(&mut r, 33, 53, |_| Some(300));
    r.engine.command(EngineCmd::Stop).unwrap();
    let st = &laps(&r)[0].stats;
    assert_eq!(st.paused_s, 8);
    assert_eq!(st.gap_s.power_s, 5);
    let joined: Vec<f64> = (0..40)
        .map(|s| if s < 20 { 100.0 } else { 300.0 })
        .collect();
    assert_eq!(st.avg_power_w, Some(200.0));
    let np = np_by_windows(&joined);
    assert!(np > 203.0, "not the plain mean: {np}");
    assert!(close(st.np_w, np), "NP {:?}, expected {np}", st.np_w);
}
