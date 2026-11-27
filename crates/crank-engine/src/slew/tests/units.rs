//! The Rust-side checks of #620 and the model's `constants` suite: rsruckig on the bare #269
//! input and the segfault input, the engine constants, the mapping, the twin's limits, the
//! line's tolerances, the closed-form helpers, the input layer and the engagement rule.
#![expect(
    clippy::float_cmp,
    reason = "the constants, the mapping and the twin's limits are pinned exactly"
)]
use super::harness::{
    Faults, Plan, Report, Rng, defaults, grid, ramp, run, run_checked, steady, tu,
};
use crate::slew::line::collinear;
use crate::slew::{
    BRAKE_SLACK, Block, CAP_SLACK, CUT_TOL_A, CUT_TOL_V, CUT_TOL_W, Cap, Cut, DT, EPS, Easer,
    InputLayer, KEY_TOL, Limits, Piece, RAW_JOIN, ROUND_SNAP, SETTLE_AFTER_TICKS, SLOPE_TOL, State,
    TARGET_MARGIN, TWIN, VALUE_TOL, landing_bound, merge_collinear, profile_at, profile_extrema,
    ramp_bound, segments, slope_change,
};
use crate::tuning::SlewTuning;
use rsruckig::prelude::*;

/// rsruckig directly: position 41.25 → 0 from rest, under the given limits.
fn bare_269(vmax: f64, vmin: f64, amax: f64, jmax: f64) -> Result<RuckigResult, RuckigError> {
    let mut otg = Ruckig::<1, ThrowErrorHandler>::new(None, DT);
    let mut i = InputParameter::<1>::new(None);
    i.current_position[0] = 41.25;
    i.target_position[0] = 0.0;
    i.max_velocity[0] = vmax;
    i.min_velocity = Some(DataArrayOrVec::new(None, vmin));
    i.max_acceleration[0] = amax;
    i.min_acceleration = Some(DataArrayOrVec::new(None, -amax));
    i.max_jerk[0] = jmax;
    let mut traj = Trajectory::<1>::new(None);
    otg.calculate(&i, &mut traj)
}

#[test]
fn rsruckig_refuses_the_bare_269_input_and_the_twin_calculates_it() {
    // ADR-0002 §7.10.2 X5: rise 15, ×1.5, ease 2.75; upstream pantor/ruckig#269. If this starts
    // calculating, rsruckig fixed it: update §7.10.2 and this pin.
    let (r, f, e) = (15.0_f64, 1.5, 2.75);
    let (amax, jmax) = (2.0 * r / e, 4.0 * r / (e * e));
    assert!(
        bare_269(r, -r * f, amax, jmax).is_err(),
        "rsruckig no longer refuses the bare #269 input"
    );
    let t = |x: f64| x - (TWIN * x.abs().max(1.0)).copysign(x);
    assert!(matches!(
        bare_269(t(r), t(-r * f), t(amax), t(jmax)),
        Ok(RuckigResult::Working | RuckigResult::Finished)
    ));
    // and through the easer's own calculation (the model's `_one`, first and twin)
    let mut ez = Easer::new(&tu(15, 1.5, 2.75));
    let got =
        [false, true].map(|twin| ez.test_one(State::at_rest(41.25), 41.25, 0.0, 15.0, -22.5, twin));
    assert_eq!(got, [false, true]);
}

#[test]
fn rsruckig_survives_the_segfault_input_with_a_target_velocity_at_its_limit() {
    // Ruckig 0.19.4 segfaults here: current (554.15625, 0.9375, 3.75), target (614, 30, 0),
    // limits 30 / −120, ±15, 7.5. rsruckig must not panic or hang (an answer or an error).
    for tv in [30.0, 30.0 - 30.0 * TARGET_MARGIN] {
        let mut otg = Ruckig::<1, ThrowErrorHandler>::new(None, DT);
        let mut i = InputParameter::<1>::new(None);
        i.current_position[0] = 554.156_25;
        i.current_velocity[0] = 0.9375;
        i.current_acceleration[0] = 3.75;
        i.target_position[0] = 614.0;
        i.target_velocity[0] = tv;
        i.max_velocity[0] = 30.0;
        i.min_velocity = Some(DataArrayOrVec::new(None, -120.0));
        i.max_acceleration[0] = 15.0;
        i.min_acceleration = Some(DataArrayOrVec::new(None, -15.0));
        i.max_jerk[0] = 7.5;
        let mut traj = Trajectory::<1>::new(None);
        let res = otg.calculate(&i, &mut traj);
        assert!(
            matches!(res, Ok(RuckigResult::Working | RuckigResult::Finished)),
            "{res:?}"
        );
        assert!(traj.get_duration().is_finite());
    }
}

#[test]
fn the_engine_constants_are_adr_0001_7_2s() {
    // ADR-0001 §7.2, the complete list (round 13)
    assert_eq!(DT, 0.25);
    assert_eq!(EPS, 0.5);
    assert_eq!(SETTLE_AFTER_TICKS, 20);
    assert_eq!(SLOPE_TOL, 1e-9);
    assert_eq!(VALUE_TOL, 1e-6);
    assert_eq!(RAW_JOIN, 1e-9);
    assert_eq!(KEY_TOL, 1e-9);
    assert_eq!(BRAKE_SLACK, EPS / 2.0);
    assert_eq!(CAP_SLACK, 1e-6);
    assert_eq!(ROUND_SNAP, 1e-9);
    assert_eq!((CUT_TOL_W, CUT_TOL_V, CUT_TOL_A), (1e-6, 1e-5, 1e-4));
    assert_eq!(TWIN, 2.0_f64.powi(-20));
    assert_eq!(TARGET_MARGIN, 2.0_f64.powi(-20));
}

#[test]
fn the_mapping_is_rise_fall_and_the_ease_as_adr_0002_7_10_2_states() {
    for t in grid() {
        let rise = f64::from(t.rise_w_per_s);
        let ez = Easer::new(&t);
        let l = ez.limits();
        assert_eq!(l.r_up, rise);
        assert_eq!(l.r_dn, rise * t.fall_factor);
        assert_eq!(l.a_max, 2.0 * rise / t.ease_s);
        assert_eq!(l.j_max, 4.0 * rise / (t.ease_s * t.ease_s));
        // the ease: rest to the full rate in exactly τ, the peak slope of the S-curve the rate
        let prof = slope_change(0.0, 0.0, rise, l);
        assert!((prof.duration() - t.ease_s).abs() < 1e-9);
    }
    let l = Limits::from(&SlewTuning::default());
    assert_eq!((l.r_up, l.r_dn, l.a_max, l.j_max), (20.0, 40.0, 20.0, 20.0));
}

#[test]
fn the_twins_limits_are_each_tightened_by_twin_times_max_1_limit() {
    let mut ez = Easer::new(&tu(15, 1.5, 2.75));
    let got = ez.test_twin_limits(15.0, -22.5);
    let l = *ez.limits();
    let want = (
        15.0 - 15.0 * TWIN,
        -22.5 + 22.5 * TWIN,
        l.a_max * (1.0 - TWIN),
        l.j_max * (1.0 - TWIN),
    );
    assert_eq!(got, want);
}

#[test]
fn a_slow_twin_answer_is_a_refusal() {
    // (20, ×1.25, 1): from a state whose slope will reach the twin's own min_velocity exactly,
    // onto a held line 491.1 W below: the first calculation answers in 20.2 s, the twin in
    // 39.1 s, past landing_bound (24.1 s).
    let t = tu(20, 1.25, 1.0);
    let st = State {
        y: 845.724_082_070_955_2,
        v: -23.935_783_551_800_583,
        a: -13.048_779_813_903_527,
    };
    let rel = 491.102_561_109_772_9;
    let mut ez = Easer::new(&t);
    assert!(
        ez.test_calc(0, st, rel, 0.0) && ez.diagnostics().slow == 0,
        "re-pin: Ruckig changed"
    );
    let mut ez = Easer::new(&t);
    ez.faults = Faults {
        twin_answers: true,
        ..Faults::on([0])
    };
    assert!(
        !ez.test_calc(0, st, rel, 0.0),
        "a slow twin answer is accepted"
    );
    assert_eq!(ez.diagnostics().slow, 1);
}

#[test]
fn ramp_bound_closes_at_the_real_rates_and_stays_finite_as_the_slope_goes_to_zero() {
    let t = defaults();
    let l = Limits::from(&t);
    let over = |s: f64, r: f64| {
        s.abs() * (s.abs() / l.a_max + l.a_max / l.j_max) / 2.0 / r
            + s.abs() / l.a_max
            + l.a_max / l.j_max
    };
    let ease_dn = t.ease_s * (1.0 + t.fall_factor) / 2.0;
    for s in [1e-9, -1e-9, 0.17, -2.0] {
        let b = ramp_bound(&l, -100.0, s, 0.0, s);
        let want =
            100.0 / (l.r_dn + s) + if s > 0.0 { over(s, l.r_up - s) } else { 0.0 } + ease_dn + 0.75;
        assert!(
            b.is_finite() && (b - want).abs() <= 1e-9,
            "from above at {s}: {b} vs {want}"
        );
        let b = ramp_bound(&l, 100.0, s, 0.0, s);
        let want =
            100.0 / (l.r_up - s) + if s < 0.0 { over(s, l.r_dn + s) } else { 0.0 } + ease_dn + 0.75;
        assert!(
            b.is_finite() && (b - want).abs() <= 1e-9,
            "from below at {s}: {b} vs {want}"
        );
    }
    // the direction not known: 2 W below a line falling at 15 W/s, at rest: the slower rate
    let (s, delta) = (-15.0_f64, 2.0);
    let v = -s;
    let stop_t = v / l.a_max + l.a_max / l.j_max;
    let stop_d = v * stop_t / 2.0;
    let t_s = s.abs() / l.a_max + l.a_max / l.j_max;
    let want = (delta + stop_d) / (l.r_up - s).min(l.r_dn + s)
        + s.abs() * t_s / 2.0 / (l.r_dn + s)
        + t_s
        + ease_dn
        + stop_t
        + 0.75;
    assert!((ramp_bound(&l, delta, 0.0, 0.0, s) - want).abs() <= 1e-9);
    // landing_bound from rest is Δ/r + T_e + 0.75 s
    assert_eq!(
        landing_bound(&l, 300.0, 0.0, 0.0),
        300.0 / 20.0 + 2.0 + 0.75
    );
    assert_eq!(
        landing_bound(&l, -300.0, 0.0, 0.0),
        300.0 / 40.0 + 3.0 + 0.75
    );
}

#[test]
fn the_cut_bounds_are_the_adr_table() {
    use super::harness::bound_of;
    for t in std::iter::once(defaults()).chain(super::harness::corners()) {
        let l = Limits::from(&t);
        assert_eq!(bound_of(&l, Cut::X2), l.r_up + l.a_max * DT);
        assert_eq!(bound_of(&l, Cut::X3), l.r_up.max(l.r_dn) + l.a_max * DT);
        assert_eq!(bound_of(&l, Cut::X4), l.r_dn + l.a_max * DT);
    }
}

fn blk(ticks: u32, a: f64, b: f64) -> Block {
    Block {
        ticks,
        from_w: a,
        to_w: b,
    }
}

#[test]
fn the_lines_tolerances_hold_at_their_boundaries() {
    // SLOPE_TOL: collinear lines merge at 0.5e-9 relative, not at 2e-9
    let base = 7.0;
    for (rel, want) in [(0.5e-9, 1), (2e-9, 2)] {
        let s2 = base * (1.0 + rel);
        let segs = segments(
            &[
                ramp(100.0, 100.0 + base * 10.0, 10.0),
                blk(40, 100.0 + base * 10.0, 100.0 + base * 10.0 + s2 * 10.0),
            ],
            0,
            1.0,
            1e9,
        );
        assert_eq!(segs.len(), want, "slopes {rel} apart");
    }
    // VALUE_TOL: flat pieces 1e-7 W apart merge, 1e-5 W apart do not
    let p = |k0, r0, joined| Piece {
        k0,
        r0,
        s: 0.0,
        n: 4,
        joined,
    };
    for (gap, want) in [(1e-7, 1), (1e-5, 2)] {
        assert_eq!(
            merge_collinear(&[p(0, 100.0, false), p(4, 100.0 + gap, true)]).len(),
            want
        );
    }
    // a near-zero merged slope snaps to 0 (1e-10 W/s), 1e-8 W/s does not
    for (slope, flat) in [(1e-10, true), (1e-8, false)] {
        let sg = segments(
            &[
                steady(100.0, 5.0),
                blk(400, 200.0, 200.0 + slope * 100.0),
                steady(300.0, 5.0),
            ],
            0,
            1.0,
            1e9,
        );
        assert_eq!(sg.len(), 3);
        assert_eq!(sg[1].s == 0.0, flat, "slope {slope}");
    }
    // the raw join: a block 1e-4 W off the previous end is a discontinuity
    let sg = segments(
        &[steady(100.0, 5.0), steady(100.0 + 1e-4, 5.0)],
        0,
        1.0,
        1e9,
    );
    assert!(sg.len() == 2 && !sg[1].joined);
    // the post-clamp join keeps the raw join: a ramp starting 1e-7 W off a held block
    let sg = segments(
        &[
            steady(100.0, 5.0),
            ramp(100.0 + 1e-7, 150.0, 10.0),
            steady(150.0, 5.0),
        ],
        0,
        1.0,
        1e9,
    );
    assert!(sg.len() == 3 && !sg[1].joined);
    // the merge after the clamp: a ramp into the cap and a steeper one beyond it, one piece at M
    let sg = segments(
        &[
            steady(100.0, 5.0),
            ramp(100.0, 400.0, 10.0),
            ramp(400.0, 800.0, 5.0),
            steady(800.0, 5.0),
        ],
        0,
        1.0,
        250.0,
    );
    assert_eq!(sg.len(), 3, "{sg:?}");
    assert_eq!((sg[2].r0, sg[2].s), (250.0, 0.0));
    // round 6: a flat piece at M never swallows a lower flat block (cap 250)
    let sg = segments(
        &[
            steady(100.0, 10.0),
            steady(300.0, 4.0),
            ramp(300.0, 150.0, 0.25),
            steady(150.0, 30.0),
        ],
        0,
        1.0,
        250.0,
    );
    assert!(
        sg.iter().any(|q| q.r0 == 150.0 && q.s == 0.0 && !q.joined),
        "{sg:?}"
    );
    assert!(collinear(7.0, 7.0 * (1.0 + 0.5e-9)) && !collinear(7.0, 7.0 * (1.0 + 2e-9)));
}

#[test]
fn profile_extrema_and_profile_at_agree_with_the_profile_sampled_densely() {
    let g = grid();
    let mut rng = Rng(1212);
    for _ in 0..3000 {
        let t = rng.pick(&g);
        let l = Limits::from(&t);
        let (y, v, a) = (
            rng.uniform(0.0, 500.0),
            rng.uniform(-l.r_dn, l.r_up),
            rng.uniform(-l.a_max, l.a_max),
        );
        let target = if rng.uniform(0.0, 1.0) < 0.5 {
            0.0
        } else {
            rng.uniform(-l.r_dn, l.r_up)
        };
        let prof = slope_change(v, a, target, &l);
        let dur = prof.duration();
        let ys: Vec<f64> = (0..=4000)
            .map(|i| profile_at(y, v, a, &prof, dur * f64::from(i) / 4000.0).0)
            .collect();
        let (smin, smax) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        let (lo, hi) = profile_extrema(y, v, a, &prof);
        assert!(
            lo <= smin + 1e-6 && hi >= smax - 1e-6 && lo >= smin - 1e-3 && hi <= smax + 1e-3,
            "{t:?} {y} {v} {a}"
        );
        let end = profile_at(y, v, a, &prof, dur);
        assert!((end.0 - ys[4000]).abs() <= 1e-9);
        assert!(
            (profile_at(y, v, a, &prof, dur + 1.0).0 - ys[4000] - end.1).abs() <= 1e-6,
            "constant slope after"
        );
        // the profile ends at the target slope with no curvature, within a_max and j_max
        assert!(
            (end.1 - target).abs() < 1e-6 && end.2.abs() < 1e-6,
            "{end:?} → {target}"
        );
        assert!(
            prof.segments()
                .iter()
                .all(|(d, j)| *d >= 0.0 && (j.abs() - l.j_max).abs() < 1e-9 || *j == 0.0)
        );
    }
}

#[test]
fn safe_brakings_slack_is_half_of_eps_strictly_inside_x2() {
    let ez = Easer::new(&defaults());
    let segs = segments(&[steady(100.0, 60.0)], 0, 1.0, 2000.0);
    let v = 5.0;
    let peak = profile_extrema(0.0, v, 0.0, &slope_change(v, 0.0, 0.0, ez.limits())).1;
    for (over, want) in [(0.25 * EPS, true), (0.75 * EPS, false)] {
        let st = State {
            y: 100.0 + over - peak,
            v,
            a: 0.0,
        };
        assert_eq!(
            ez.test_brake_ok(st, 1, &segs, 2000.0),
            want,
            "a stop {over} W past the line"
        );
    }
}

#[test]
fn the_hard_clamp_is_absolute() {
    for t in [defaults(), tu(50, 4.0, 1.0)] {
        let mut ez = Easer::new(&t);
        for (line_w, st0, nst, want, cut) in [
            (
                200.0,
                State::at_rest(1.0),
                State {
                    y: -3.0,
                    v: -12.0,
                    a: -5.0,
                },
                State::at_rest(0.0),
                Cut::X4,
            ),
            (
                400.0,
                State::at_rest(249.9),
                State {
                    y: 250.3,
                    v: 1.0,
                    a: 0.0,
                },
                State::at_rest(250.0),
                Cut::X3,
            ),
        ] {
            let segs = segments(&[steady(line_w, 20.0)], 0, 1.0, 250.0);
            let (out, exc) = ez.test_after(st0, nst, 4, &segs, 250.0, f64::min(line_w, 250.0));
            assert_eq!((out, exc), (want, Some(cut)));
        }
    }
}

#[test]
fn the_meets_end_holds_at_rest_where_it_met_the_line() {
    for (t, s) in [
        (defaults(), 0.17),
        (defaults(), 8.0),
        (tu(50, 4.0, 1.0), 40.0),
    ] {
        let mut ez = Easer::new(&t);
        let ((p_end, v_end, _), got) = ez
            .test_meet_end(State::at_rest(500.0), 120.0, s, 0.3 * DT)
            .unwrap_or_else(|| panic!("Ruckig refused the meet onto a {s} W/s line"));
        assert!(
            (v_end + s).abs() <= 1e-6,
            "it ended with relative slope {v_end}"
        );
        let want = (p_end - s * 0.3 * DT, -s, 0.0);
        assert!(
            (got.0 - want.0).abs() <= 1e-9 && (got.1 - want.1).abs() <= 1e-9 && got.2 == 0.0,
            "{got:?} vs {want:?}"
        );
    }
}

#[test]
fn the_cap_and_the_input_layer_are_the_round_16_rules() {
    assert_eq!(Cap::new(f64::NAN), None);
    assert_eq!(Cap::new(f64::NEG_INFINITY), None);
    assert_eq!(Cap::new(-5.0).map(Cap::get), Some(0.0));
    assert_eq!(Cap::new(f64::INFINITY), Some(Cap::NONE));
    let mut l = InputLayer::new(f64::NAN);
    assert!(
        l.fault() && l.cap_in_force() == Cap::NONE,
        "an invalid initial cap: a fault, no cap known"
    );
    l.begin_tick();
    l.cap(100.0);
    assert!(!l.fault() && l.cap_in_force().get() == 100.0);
    l.begin_tick();
    l.cap(f64::NAN);
    assert!(
        l.fault() && l.cap_in_force().get() == 100.0,
        "the last valid cap stays in force"
    );
    l.begin_tick();
    assert!(
        l.fault(),
        "the fault stays while the cap's latest value is invalid"
    );
    l.cap(50.0);
    assert!(!l.fault() && l.cap_in_force().get() == 50.0);
    assert!(!l.offset(f64::INFINITY) && l.fault() && l.multiplier() == 1.0);
    l.begin_tick();
    assert!(l.fault());
    assert!(l.offset(0.5) && !l.fault() && l.multiplier() == 0.5);
    assert!(!l.engage(f64::NAN) && l.fault());
    l.begin_tick();
    assert!(!l.fault(), "an invalid engagement faults only its tick");
    assert!(l.engage(120.0));
}

#[test]
fn the_engagement_starts_at_rest_at_its_origin_and_completes_on_the_line_or_at_the_full_rate() {
    let cap = Cap::new(300.0).expect("valid");
    assert_eq!(Easer::engagement(137.0, cap), State::at_rest(137.0));
    assert_eq!(Easer::engagement(400.0, cap), State::at_rest(300.0));
    assert_eq!(Easer::engagement(-3.0, cap), State::at_rest(0.0));
    assert!(Easer::engagement(f64::NAN, cap).y.is_nan());
    let ez = Easer::new(&defaults());
    assert!(ez.engagement_complete(&State::at_rest(136.6), 137.0, 0.0));
    assert!(!ez.engagement_complete(&State::at_rest(130.0), 137.0, 0.0));
    // behind a line steeper than the rate (30 W/s > r↑ = 20): complete once chasing at r↑
    let chase = State {
        y: 100.0,
        v: 20.0 * (1.0 - TARGET_MARGIN),
        a: 0.0,
    };
    assert!(ez.engagement_complete(&chase, 150.0, 30.0));
    assert!(!ez.engagement_complete(&State { v: 15.0, ..chase }, 150.0, 30.0));
    assert!(
        !ez.engagement_complete(&chase, 150.0, 10.0),
        "a trackable line completes on the line only"
    );
    let fall = State {
        y: 300.0,
        v: -40.0 * (1.0 - TARGET_MARGIN),
        a: 0.0,
    };
    assert!(ez.engagement_complete(&fall, 200.0, -50.0));
    // an engagement from 0 W onto 137 W completes when it is on the line (8.25 s)
    let tr = run(&Plan::new(defaults(), vec![steady(137.0, 30.0)]).origin(0.0));
    let k = (0..tr.y.len())
        .find(|&k| {
            ez.engagement_complete(
                &State {
                    y: tr.y[k],
                    v: tr.v[k],
                    a: tr.a[k],
                },
                tr.r[k],
                0.0,
            )
        })
        .expect("completes");
    assert!(tr.on[k] && (k..tr.on.len()).all(|j| tr.on[j]));
}

#[test]
fn a_held_line_re_plans_only_on_a_change() {
    // Step 7: no calculation on an unchanged input. A jump onto a held line plans once (twice
    // at most with a brake), then samples for the rest of the block.
    let tr = run(&Plan::new(
        defaults(),
        vec![steady(100.0, 5.0), steady(400.0, 120.0)],
    ));
    assert!(tr.recalcs <= 3, "{} calculations", tr.recalcs);
    let tr = run(&Plan::new(
        defaults(),
        vec![
            steady(100.0, 5.0),
            ramp(100.0, 400.0, 120.0),
            steady(400.0, 30.0),
        ],
    ));
    assert!(
        tr.recalcs <= 6,
        "{} calculations on a ramp with two vertices",
        tr.recalcs
    );
}

#[test]
fn a_ramp_split_into_blocks_of_one_slope_is_the_single_block_bitwise() {
    let g = grid();
    let mut rng = Rng(613);
    let mut rep = Report::default();
    for _ in 0..300 {
        let t = rng.pick(&g);
        let l = Limits::from(&t);
        let s = rng.uniform(-1.5 * l.r_dn, 1.5 * l.r_up);
        let w0 = rng.uniform(300.0, 800.0);
        let d = rng.pick(&[1.0, 2.0, 5.0, 10.0, 20.0]);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "small"
        )]
        let nt = (d / DT) as u32;
        let mut cuts: Vec<u32> = (0..3)
            .map(|_| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "small"
                )]
                let c = rng.uniform(1.0, f64::from(nt)) as u32;
                c.clamp(1, nt - 1)
            })
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let pts: Vec<u32> = std::iter::once(0)
            .chain(cuts)
            .chain(std::iter::once(nt))
            .collect();
        let mut split = vec![steady(w0, 5.0)];
        let mut prev_to = w0;
        for w in pts.windows(2) {
            let to = w0 + s * f64::from(w[1]) * DT;
            split.push(blk(w[1] - w[0], prev_to, to));
            prev_to = to;
        }
        split.push(steady(w0 + s * d, 15.0));
        let single = vec![
            steady(w0, 5.0),
            blk(nt, w0, w0 + s * d),
            steady(w0 + s * d, 15.0),
        ];
        let a = run_checked(&Plan::new(t, split).cap(4000.0), &mut rep, "split");
        let b = run(&Plan::new(t, single).cap(4000.0));
        let dev =
            a.y.iter()
                .zip(&b.y)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
        assert!(
            dev == 0.0,
            "split ramp {s} W/s {t:?} {pts:?}: differs by {dev}"
        );
    }
    rep.assert_clean();
}
