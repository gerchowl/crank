//! The per-field trainer hold (#437): FE-C page 16 (speed) and page 25 (power, cadence)
//! arrive as separate samples, and neither may blank the other's fields. Every test feeds
//! the NEO's interleaving: page 16 instead of page 25 on one tick in five, and page 16 after
//! page 25 inside one tick in five (two samples in the same tick).
use crate::engine::{PauseReason, RideState};
use crate::stream::{MetricId, StreamFrame, StreamId, SubId, SubscribeOpts};
use crate::tests::{Rig, block, rig};
use crank_devices::ControlMode;
use crank_domain::workout::Phase;
use tokio::sync::mpsc;

/// A raw 4 Hz subscription to power, cadence and speed.
struct Watch {
    stream: StreamId,
    subs: [SubId; 3],
    _rx: mpsc::Receiver<StreamFrame>,
}
fn watch(r: &mut Rig) -> Watch {
    let (stream, rx) = r.engine.hub.open(None, 0).expect("open");
    let subs = [MetricId::Power, MetricId::Cadence, MetricId::Speed].map(|m| {
        r.engine
            .hub
            .subscribe(
                stream,
                m,
                SubscribeOpts {
                    rate_hz: 4.0,
                    filters: vec![],
                },
            )
            .expect("subscribe")
    });
    Watch {
        stream,
        subs,
        _rx: rx,
    }
}
/// The stream's power, cadence and speed of the tick processed last.
fn values(r: &Rig, w: &Watch) -> [Option<f64>; 3] {
    let snap = r.engine.hub.snapshot(w.stream).expect("snapshot");
    w.subs.map(|id| {
        snap.samples
            .iter()
            .find(|s| s.sub == id)
            .expect("a sample per tick")
            .v
    })
}
/// The NEO's page order (`docs/hil/captures/spacing.fecap`): page 25 with `w` and `rpm`, but
/// page 16 (8 m/s) alone on ticks `k % 5 == 4`, and page 16 right after page 25 on `k % 5 == 2`.
fn neo(r: &Rig, from: u64, to: u64, w: u16, rpm: u8) {
    for k in from..to {
        if k % 5 != 4 {
            r.page25(k, Some(w), Some(rpm));
        }
        if k % 5 == 4 || k % 5 == 2 {
            r.page16(k, 8.0);
        }
    }
}
fn ride(target_w: f64) -> Rig {
    let mut r = rig();
    r.load(vec![block(600.0, target_w, target_w, Phase::Work)]);
    r.start();
    r
}
fn positive_erg(modes: &[ControlMode]) -> Vec<u16> {
    modes
        .iter()
        .filter_map(|m| match m {
            ControlMode::Erg { target_w } if *target_w > 0 => Some(*target_w),
            _ => None,
        })
        .collect()
}

#[test]
fn page_16_never_blanks_power_or_cadence_in_the_stream() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    neo(&r, 0, 40, 200, 90);
    for k in 0..40 {
        r.through(k);
        let [power, cadence, speed] = values(&r, &w);
        assert_eq!((power, cadence), (Some(200.0), Some(90.0)), "k={k}");
        // the first page 16 arrives in tick 2
        assert_eq!(speed, (k >= 2).then_some(8.0), "k={k}");
    }
}

#[test]
fn erg_re_engages_after_a_low_cadence_release_with_page_16_interleaved() {
    let mut r = ride(200.0);
    neo(&r, 0, 8, 190, 90);
    neo(&r, 8, 16, 120, 39);
    r.through(15);
    assert!(r.status().erg_released, "39 rpm for 2 s releases");
    assert_eq!(
        r.modes().last(),
        Some(&ControlMode::Resistance { pct: 0.0 })
    );
    neo(&r, 16, 25, 100, 59);
    // ADR-0002 §7.4: 60 rpm for 8 ticks (25..=32); the page-16 tick 29 between them holds
    // the 60 rpm of tick 28 and counts
    neo(&r, 25, 32, 100, 60);
    r.through(31);
    assert!(r.status().erg_released, "7 ticks are not 2 s");
    neo(&r, 32, 33, 100, 60);
    r.through(32);
    assert!(!r.status().erg_released, "the eighth tick re-engages");
    neo(&r, 33, 57, 100, 90);
    r.through(56);
    assert_eq!(
        positive_erg(&r.modes()),
        vec![100, 120, 140, 160, 180, 200],
        "the §7.5 soft start from the 3 s power, never restarted"
    );
}

#[test]
fn the_soft_start_and_flow_run_through_page_16_without_restarting() {
    let mut r = ride(300.0);
    neo(&r, 0, 40, 150, 90);
    r.through(39);
    // the ride start's soft start: five 1 Hz steps from 150 W; a flow that restarted on a
    // page-16 tick would ramp again from the then-current 150 W
    assert_eq!(positive_erg(&r.modes()), vec![150, 180, 210, 240, 270, 300]);
}

#[test]
fn a_stopped_page_25_stream_trips_the_silence_rule_while_page_16_still_flows() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    neo(&r, 0, 8, 200, 90);
    r.through(7);
    // page 25 stops after tick 7; page 16 goes on every tick
    for k in 8..30 {
        r.page16(k, 8.0);
    }
    for k in 8..15 {
        r.through(k);
        assert_eq!(r.state(), RideState::Riding, "k={k}");
        let [power, cadence, speed] = values(&r, &w);
        // the 1.0 s hold of the last page 25 (tick 7) ends after tick 11; nothing outlives
        // it, not even the fresh page-16 speed
        let held = k <= 11;
        assert_eq!(power, held.then_some(200.0), "k={k}");
        assert_eq!(cadence, held.then_some(90.0), "k={k}");
        assert_eq!(speed, held.then_some(8.0), "k={k}");
    }
    // ADR-0002 §6: 8 ticks without page 25 lose the trainer
    r.through(15);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    assert!(r.status().trainer_lost_since.is_some());
}

#[test]
fn a_silence_clears_the_held_speed() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    neo(&r, 0, 8, 200, 90);
    // 8 silent ticks (8..=15), then page 25 alone: the page-16 speed of tick 7 is inside its
    // 6 s bound but was sampled before the silence epoch
    for k in 16..20 {
        r.page25(k, Some(150), Some(90));
    }
    r.through(15);
    assert_eq!(r.state(), RideState::Paused(PauseReason::TrainerLost));
    // the link stayed ready: the mode is re-issued at once and tick 16's cadence resumes
    r.through(17);
    assert_eq!(r.state(), RideState::Riding);
    let [power, _, speed] = values(&r, &w);
    assert_eq!((power, speed), (Some(150.0), None));
}

#[test]
fn cadence_zero_is_a_value_and_a_missing_cadence_is_not() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    neo(&r, 0, 8, 200, 90);
    r.through(7);
    // the rider stops: page 25 reports 0 W and 0 rpm, then page 16 alone (no cadence in it)
    r.page25(8, Some(0), Some(0));
    r.page16(9, 0.0);
    for k in 8..=9 {
        r.through(k);
        let [power, cadence, _] = values(&r, &w);
        assert_eq!((power, cadence), (Some(0.0), Some(0.0)), "k={k}");
    }
    // stopped pedalling (ADR-0001 §5): 12 ticks of held 0 W and 0 rpm from tick 8, page-16
    // ticks included
    for k in 10..24 {
        if k % 5 == 4 {
            r.page16(k, 0.0);
        } else {
            r.page25(k, Some(0), Some(0));
        }
    }
    r.through(18);
    assert_eq!(r.state(), RideState::Riding);
    r.through(19);
    assert_eq!(r.state(), RideState::Paused(PauseReason::StoppedPedalling));
}

/// The tick ERG is released on when, from tick 20, page 25 reports `cadence` (page 16 still
/// interleaved).
fn release_tick(cadence: Option<u8>) -> u64 {
    let mut r = ride(200.0);
    neo(&r, 0, 20, 200, 90);
    for k in 20..40 {
        if k % 5 != 4 {
            r.page25(k, Some(150), cadence);
        }
        if k % 5 == 4 || k % 5 == 2 {
            r.page16(k, 8.0);
        }
    }
    (20..40)
        .find(|&k| {
            r.through(k);
            r.status().erg_released
        })
        .expect("released")
}

#[test]
fn an_invalid_cadence_on_page_25_releases_like_cadence_zero() {
    // page 25 always carries cadence: `None` there is the trainer's 0xFF (invalid), unknown at
    // once and 0 for the release (ADR-0002 §7.4), never a held 90 rpm
    assert_eq!(release_tick(Some(0)), 27, "8 ticks below 40 rpm: 20..=27");
    assert_eq!(release_tick(None), 27);
}

#[test]
fn an_invalid_field_is_unknown_at_once_and_a_field_not_carried_is_held() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    r.page25(0, Some(200), Some(90));
    r.page16(0, 8.0);
    // tick 1: page 16 alone carries no power or cadence: both held
    r.page16(1, 8.0);
    // tick 2: page 25 reports power and cadence invalid, then page 16 its speed invalid
    r.page25(2, None, None);
    r.page16_invalid(2);
    r.through(1);
    assert_eq!(values(&r, &w), [Some(200.0), Some(90.0), Some(8.0)]);
    r.through(2);
    assert_eq!(values(&r, &w), [None, None, None]);
}

#[test]
fn speed_is_held_six_seconds_while_page_25_flows() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    r.page16(0, 8.0);
    for k in 0..30 {
        r.page25(k, Some(200), Some(90));
    }
    for k in 0..30 {
        r.through(k);
        let [_, _, speed] = values(&r, &w);
        // 6.0 s: ticks 0..=24
        assert_eq!(speed, (k <= 24).then_some(8.0), "k={k}");
    }
}

#[test]
fn within_a_tick_the_last_value_of_each_field_wins() {
    let mut r = ride(300.0);
    let w = watch(&mut r);
    // tick 0 holds three samples: page 25 (150 W, 80 rpm), page 16, then page 25 with 210 W
    // and cadence invalid: the last word on cadence is "unknown"
    r.page25(0, Some(150), Some(80));
    r.page16(0, 8.0);
    r.send_sample(3, Some(210), None, None, Some(1));
    r.through(0);
    assert_eq!(values(&r, &w), [Some(210.0), None, Some(8.0)]);
    // the 3 s power window takes the tick's power once: the soft start begins at 210 W
    r.page25(1, Some(210), Some(80));
    r.through(1);
    assert_eq!(positive_erg(&r.modes()).first(), Some(&210));
}

#[test]
fn a_real_zero_after_ninety_in_the_same_tick_wins() {
    let mut r = ride(200.0);
    let w = watch(&mut r);
    r.page25(0, Some(200), Some(90));
    r.send_sample(3, Some(0), Some(0), None, Some(1));
    r.through(0);
    assert_eq!(values(&r, &w)[..2], [Some(0.0), Some(0.0)]);
}

#[test]
fn an_invalid_power_after_a_value_leaves_the_tick_without_power() {
    let mut r = ride(300.0);
    // tick 0: 150 W, then the same tick's page 25 reports power (and cadence) invalid: the 3 s
    // power window gets nothing from tick 0, and the flow starts on tick 1
    r.page25(0, Some(150), Some(80));
    r.send_sample(3, None, None, None, Some(1));
    r.page25(1, Some(210), Some(80));
    r.through(1);
    // the soft start's origin is the 3 s mean of 210 W alone, not of 150 and 210
    assert_eq!(positive_erg(&r.modes()).first(), Some(&210));
}
