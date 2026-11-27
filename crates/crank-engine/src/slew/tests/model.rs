//! The model's literal suites (`docs/adr/models/erg_slew.py`: `tables`, `corners`, `pinned`,
//! `boundary`, `persistent`, `recovery`, `repros`, `above`, `ramp_pairs`), on the port. The plans
//! the model draws from Python's random generators are in the golden fixture (`golden.rs`).
use super::harness::{
    Ev, Faults, Kind, Plan, Report, Trace, check, check_above, corners, defaults, ev, grid, ramp,
    run, run_checked, steady, tu,
};
use crate::slew::{Block, Cut, DT, EPS, Limits, landing_bound, ramp_bound, tick_f};
use crate::tuning::SlewTuning;

/// A pinned case: (name, tuning, blocks, events, engagement origin, initial cap).
type PinnedCase = (
    &'static str,
    SlewTuning,
    Vec<Block>,
    Vec<Ev>,
    Option<f64>,
    f64,
);
/// A falsifier's repro: (name, tuning, blocks, initial cap, faults, events).
type Repro = (&'static str, SlewTuning, Vec<Block>, f64, Faults, Vec<Ev>);
/// A not-a-number case: (name, blocks, events, first and last-but-one degraded tick).
type NanCase = (&'static str, Vec<Block>, Vec<Ev>, usize, usize);

fn land_time(tr: &Trace, after: i64) -> Option<f64> {
    let n = tr.on.len();
    for k in (0..n).rev() {
        if !tr.on[k] {
            let k1 = i64::try_from(k + 1).expect("small");
            return (k + 1 < n).then(|| tick_f(k1 - after) * DT);
        }
    }
    Some(0.0)
}

fn excluded_seconds(tr: &Trace) -> usize {
    tr.settle.chunks(4).filter(|s| s.iter().any(|x| *x)).count()
}

fn plain(blocks: Vec<Block>) -> Plan {
    Plan::new(defaults(), blocks)
}

fn assert_figure(name: &str, got: Option<f64>, want: f64) {
    let got = got.unwrap_or_else(|| panic!("{name}: never landed"));
    assert!((got - want).abs() <= 1e-9, "{name}: port {got}, ADR {want}");
}

#[test]
fn the_figures_adr_0002_7_10_quotes_hold() {
    let mut rep = Report::default();
    let mut lands = |a: f64, b: f64| {
        let tr = run_checked(
            &plain(vec![steady(a, 5.0), steady(b, 60.0)]),
            &mut rep,
            "jump",
        );
        land_time(&tr, 20)
    };
    assert_figure("rise 300 W", lands(100.0, 400.0), 16.25);
    assert_figure("fall 300 W", lands(400.0, 100.0), 9.75);
    assert_figure("rise 83 W", lands(137.0, 220.0), 5.5);
    assert_figure("fall 83 W", lands(220.0, 137.0), 4.5);
    assert_figure("rise 20 W", lands(150.0, 170.0), 2.5);
    assert_figure("fall 150 W", lands(250.0, 100.0), 6.0);
    let eng = |w: f64, rep: &mut Report| {
        let tr = run_checked(&plain(vec![steady(w, 60.0)]).origin(0.0), rep, "engagement");
        land_time(&tr, 0)
    };
    assert_figure("engage 137", eng(137.0, &mut rep), 8.25);
    assert_figure("engage 250", eng(250.0, &mut rep), 13.75);
    let tr = run_checked(
        &plain(vec![
            steady(100.0, 10.0),
            ramp(100.0, 180.0, 60.0),
            steady(220.0, 60.0),
        ]),
        &mut rep,
        "owner",
    );
    assert_figure("owner's case", land_time(&tr, 280), 3.25);
    let tr = run_checked(
        &plain(vec![
            steady(100.0, 5.0),
            ramp(100.0, 400.0, 10.0),
            steady(400.0, 30.0),
        ]),
        &mut rep,
        "steep up",
    );
    assert_figure("steep up", land_time(&tr, 20), 16.5);
    let tr = run_checked(
        &plain(vec![
            steady(400.0, 5.0),
            ramp(400.0, 100.0, 5.0),
            steady(100.0, 30.0),
        ]),
        &mut rep,
        "steep down",
    );
    assert_figure("steep down", land_time(&tr, 20), 10.0);
    // ramps within the rates: off-line ticks and excluded seconds
    let ramp_tr = |a: f64, b: f64, secs: f64, rep: &mut Report| {
        run_checked(
            &plain(vec![steady(a, 10.0), ramp(a, b, secs), steady(b, 15.0)]),
            rep,
            "ramp",
        )
    };
    let tr = ramp_tr(100.0, 400.0, 60.0, &mut rep);
    assert_eq!(tr.on.iter().filter(|o| !**o).count(), 6, "5 W/s off-line");
    assert_eq!(excluded_seconds(&tr), 2, "5 W/s excluded");
    let tr = ramp_tr(300.0, 100.0, 60.0, &mut rep);
    assert_eq!(tr.on.iter().filter(|o| !**o).count(), 0, "fall off-line");
    let tr = ramp_tr(100.0, 400.0, 120.0, &mut rep);
    assert_eq!(excluded_seconds(&tr), 1, "2.5 W/s excluded");
    let tr = ramp_tr(50.0, 150.0, 30.0, &mut rep);
    assert_eq!(tr.on.iter().filter(|o| !**o).count(), 4, "50->150 off-line");
    // skip 300 -> 250 W lands
    let tr = run_checked(
        &plain(vec![
            steady(300.0, 10.0),
            steady(250.0, 10.0),
            steady(250.0, 20.0),
        ])
        .events(vec![ev(44, Kind::Skip, 0.0)]),
        &mut rep,
        "skip",
    );
    assert_figure("skip 300->250 lands", land_time(&tr, 0), 13.75);
    // short intervals at the defaults: the 5th rep's hard block, peak and mean
    for (on_s, peak, mean) in [(15.0, 360.0, 232.0), (30.0, 400.0, 316.0)] {
        let mut blocks = Vec::new();
        for _ in 0..6 {
            blocks.extend([steady(100.0, on_s), steady(400.0, on_s)]);
        }
        blocks.push(steady(100.0, 30.0));
        let tr = run_checked(&plain(blocks), &mut rep, "short");
        let n = (on_s / DT).round();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "small"
        )]
        let n = n as usize;
        let seg = &tr.y[9 * n..10 * n];
        let max = seg.iter().copied().fold(f64::MIN, f64::max);
        #[expect(clippy::cast_precision_loss, reason = "small")]
        let avg = seg.iter().sum::<f64>() / seg.len() as f64;
        assert_eq!(
            (max.round(), avg.round()),
            (peak, mean),
            "{on_s}/{on_s} peak, mean"
        );
    }
    // the first second of an engagement: j·t³/6
    for (t, want) in [(defaults(), 3.333), (tu(50, 1.0, 1.0), 25.0)] {
        let tr = run(&Plan::new(t, vec![steady(400.0, 30.0)]).origin(0.0));
        assert!((tr.y[3] - want).abs() < 5e-4, "first second {}", tr.y[3]);
    }
    rep.assert_clean();
}

#[test]
fn the_corners_of_the_bounds_keep_every_property() {
    let plans = [
        vec![steady(100.0, 5.0), steady(400.0, 60.0), steady(100.0, 60.0)],
        vec![
            steady(100.0, 5.0),
            ramp(100.0, 400.0, 30.0),
            steady(400.0, 10.0),
            ramp(400.0, 100.0, 15.0),
            steady(100.0, 30.0),
        ],
        vec![
            steady(100.0, 5.0),
            ramp(100.0, 400.0, 5.0),
            steady(400.0, 30.0),
            ramp(400.0, 100.0, 2.0),
            steady(100.0, 30.0),
        ],
        (0..5)
            .flat_map(|_| [steady(100.0, 15.0), steady(400.0, 15.0)])
            .collect(),
        vec![
            steady(300.0, 5.0),
            ramp(300.0, 100.0, 10.0),
            steady(100.0, 20.0),
        ],
    ];
    let mut rep = Report::default();
    for t in corners() {
        for (i, p) in plans.iter().enumerate() {
            for origin in [None, Some(0.0)] {
                let mut plan = Plan::new(t, p.clone());
                plan.origin = origin;
                run_checked(
                    &plan,
                    &mut rep,
                    &format!("corner {t:?} plan {i} origin {origin:?}"),
                );
            }
        }
    }
    rep.assert_clean();
}

#[test]
fn the_pinned_regressions_of_the_falsifications_keep_every_property() {
    let d = defaults();
    let cases: Vec<PinnedCase> = vec![
        (
            "stuck step 233 -> 200 W",
            tu(11, 2.0, 3.0),
            vec![steady(233.0, 10.0), steady(200.0, 60.0)],
            vec![],
            None,
            2000.0,
        ),
        (
            "stuck step 200 -> 233 W",
            tu(11, 2.0, 3.0),
            vec![steady(200.0, 10.0), steady(233.0, 60.0)],
            vec![],
            None,
            2000.0,
        ),
        (
            "stuck step 275 -> 200 W",
            tu(20, 1.25, 3.75),
            vec![steady(275.0, 10.0), steady(200.0, 60.0)],
            vec![],
            None,
            2000.0,
        ),
        (
            "stuck mid-plan",
            tu(15, 1.5, 2.75),
            vec![
                steady(800.0, 5.0),
                ramp(800.0, 530.0, 10.0),
                ramp(530.0, 533.75, 5.0),
                steady(533.75, 60.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "split ramp 800 -> 809.7 -> 848.5",
            d,
            vec![
                steady(800.0, 5.0),
                ramp(800.0, 809.7, 0.5),
                ramp(809.7, 848.5, 2.0),
                steady(848.5, 15.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "split fall -19.4 W/s",
            tu(5, 4.0, 2.0),
            vec![
                steady(800.0, 5.0),
                ramp(800.0, 780.6, 1.0),
                ramp(780.6, 770.9, 0.5),
                steady(770.9, 15.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "round-4 #269 plan",
            d,
            vec![
                ramp(548.111_599_801_053_7, 0.0, 30.0),
                ramp(159.109_684_835_569_77, 545.562_804_269_573_9, 10.0),
                steady(65.448_053_879_402_11, 5.0),
                ramp(65.448_053_879_402_11, 0.0, 5.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "ramp 37 -> 49.5 W/s",
            tu(50, 2.0, 1.0),
            vec![
                steady(100.0, 10.0),
                ramp(100.0, 840.0, 20.0),
                ramp(840.0, 840.0 + 49.5 * 5.0, 5.0),
                steady(840.0 + 49.5 * 5.0, 10.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "ramp 10 -> 40 W/s",
            d,
            vec![
                ramp(100.0, 300.0, 20.0),
                ramp(300.0, 500.0, 5.0),
                steady(500.0, 10.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "engagement mid-plan, then a ramp-end vertex",
            d,
            vec![
                steady(150.0, 5.0),
                ramp(150.0, 100.0, 10.0),
                steady(100.0, 20.0),
            ],
            vec![ev(30, Kind::Engage, 95.0)],
            None,
            2000.0,
        ),
        (
            "200 W, 150 W, then 0 W",
            d,
            vec![steady(200.0, 10.0), steady(150.0, 5.0), steady(0.0, 20.0)],
            vec![],
            None,
            2000.0,
        ),
        (
            "hold above a rising line",
            d,
            vec![
                steady(300.0, 10.0),
                ramp(200.0, 400.0, 20.0),
                steady(400.0, 10.0),
            ],
            vec![],
            None,
            2000.0,
        ),
        (
            "engagement above a rising ramp",
            tu(50, 1.5, 1.0),
            vec![
                ramp(8.796_133_428_274_4, 182.115_481_935_318_97, 5.0),
                steady(182.115_481_935_318_97, 10.0),
            ],
            vec![],
            Some(61.496_447_065_334_12),
            2000.0,
        ),
        (
            "ramp into the cap",
            tu(40, 2.0, 2.0),
            vec![steady(580.7, 5.0), ramp(580.7, 1131.0, 60.0)],
            vec![],
            None,
            790.99,
        ),
        (
            "cap drops mid-fall",
            d,
            vec![steady(400.0, 10.0), steady(100.0, 30.0)],
            vec![ev(52, Kind::Cap, 250.0)],
            None,
            2000.0,
        ),
        (
            "falling ramp to 0 W, then a jump up",
            tu(10, 4.0, 3.0),
            vec![
                steady(297.26, 1.0),
                ramp(297.26, 0.0, 30.0),
                steady(501.46, 30.0),
            ],
            vec![],
            None,
            2000.0,
        ),
    ];
    let no_cut = [
        "engagement above a rising ramp",
        "ramp 37 -> 49.5 W/s",
        "ramp 10 -> 40 W/s",
        "hold above a rising line",
        "ramp into the cap",
    ];
    let mut rep = Report::default();
    for (name, t, blocks, events, origin, m0) in cases {
        let mut p = Plan::new(t, blocks).events(events).cap(m0);
        p.origin = origin;
        let tr = run_checked(&p, &mut rep, &format!("pinned: {name}"));
        if no_cut.contains(&name) {
            assert!(
                !tr.exc
                    .iter()
                    .any(|e| matches!(e, Some(Cut::X2 | Cut::X3 | Cut::X4))),
                "{name}: a cut where the design eases"
            );
        }
    }
    // An engagement 2 W off a trajectory at rest, then every calculation refused: degraded at
    // the engagement's state, never a jump back to the old trajectory.
    let tr = run_checked(
        &plain(vec![steady(100.0, 5.0), steady(400.0, 60.0)])
            .events(vec![ev(160, Kind::Engage, 402.0)])
            .faults(Faults::persistent(160)),
        &mut rep,
        "engagement 2 W off a landed trajectory",
    );
    assert!(tr.degraded[160..].iter().all(|x| *x) && (tr.y[160] - 402.0).abs() < 1e-9);
    let t5 = tu(5, 4.0, 4.0);
    let reference = run(&Plan::new(
        t5,
        vec![steady(100.0, 5.0), steady(400.0, 80.0)],
    ));
    let tr = run_checked(
        &Plan::new(t5, vec![steady(100.0, 5.0), steady(400.0, 80.0)])
            .events(vec![ev(100, Kind::Engage, reference.y[99] + 2.0)])
            .faults(Faults::persistent(100)),
        &mut rep,
        "engagement 2 W off a rise in flight",
    );
    assert!(
        tr.degraded[100..].iter().all(|x| *x) && (tr.y[100] - reference.y[99] - 2.0).abs() < 1e-9
    );
    // X3 after the step: a ramp into the cap whose braking plan fails.
    run_checked(
        &Plan::new(
            tu(40, 2.0, 2.0),
            vec![steady(580.7, 5.0), ramp(580.7, 1131.0, 60.0)],
        )
        .cap(790.99)
        .faults(Faults {
            fail_brake: true,
            ..Faults::default()
        }),
        &mut rep,
        "ramp into the cap, braking plan fails",
    );
    rep.assert_clean();
}

/// Every valid tuning (6 955 of them), the model's `boundary` order.
fn all_tunings() -> Vec<SlewTuning> {
    let mut out = Vec::new();
    for r in 5_u16..=50 {
        for f in 4..=16_u8 {
            for e in 4..=16_u8 {
                let t = tu(r, f64::from(f) / 4.0, f64::from(e) / 4.0);
                if f64::from(r) * t.fall_factor >= 20.0 {
                    out.push(t);
                }
            }
        }
    }
    out
}

#[test]
fn steps_of_exactly_rise_times_ease_land_and_are_never_degraded() {
    // The #269 class (§7.10.2 X5): every 35th valid tuning, Δ = 1, 2, 3 × rise·ease both ways.
    let all = all_tunings();
    assert_eq!(all.len(), 6955);
    let mut rep = Report::default();
    let (mut errors, mut twins) = (0, 0);
    for t in all.iter().step_by(35) {
        let lim = Limits::from(t);
        for mul in [1.0, 2.0, 3.0] {
            let d = f64::from(t.rise_w_per_s) * t.ease_s * mul;
            for (a, b) in [(300.0, 300.0 - d), (300.0 - d, 300.0)] {
                if a.min(b) < 0.0 {
                    continue;
                }
                let secs = landing_bound(&lim, b - a, 0.0, 0.0) + 10.0;
                let tr = run_checked(
                    &Plan::new(*t, vec![steady(a, 5.0), steady(b, secs)]),
                    &mut rep,
                    &format!("boundary {mul}·rise·ease {a}->{b} {t:?}"),
                );
                assert!(!tr.degraded.iter().any(|x| *x), "{t:?} {a}->{b}: degraded");
                errors += tr.errors;
                twins += tr.twins;
            }
        }
    }
    rep.assert_clean();
    assert!(
        errors > 0 && twins == errors,
        "refusals {errors}, twin recoveries {twins}"
    );
}

#[test]
fn a_persistent_refusal_stops_by_the_closed_form_stop_and_holds() {
    let mut rep = Report::default();
    for t in std::iter::once(defaults()).chain(corners()) {
        let rise_ease = f64::from(t.rise_w_per_s) * t.ease_s;
        for (a, b) in [
            (100.0, 400.0),
            (400.0, 100.0),
            (233.0, 200.0),
            (150.0, 150.0 + rise_ease),
        ] {
            for onset in [20, 26, 32] {
                let events = if onset > 20 {
                    vec![ev(onset, Kind::Offset, if b > a { 1.03 } else { 0.97 })]
                } else {
                    vec![]
                };
                let name = format!("persistent refusal {a}->{b} from tick {onset} {t:?}");
                let tr = run_checked(
                    &Plan::new(t, vec![steady(a, 5.0), steady(b, 40.0)])
                        .events(events)
                        .faults(Faults::persistent(onset)),
                    &mut rep,
                    &name,
                );
                let first = tr.failed.iter().position(|x| *x).expect("a refusal");
                assert!(
                    tr.degraded[first..].iter().all(|x| *x)
                        && !tr.degraded[..first].iter().any(|x| *x),
                    "{name}: degraded not raised from the first refusal on every tick"
                );
                let n = tr.y.len();
                assert!(
                    tr.v[n - 1].abs() <= 1e-9 && (tr.y[n - 1] - tr.y[n - 2]).abs() <= 1e-9,
                    "{name}: not at rest at the end"
                );
            }
        }
    }
    rep.assert_clean();
    assert!(rep.degraded_runs >= 108);
}

#[test]
fn the_recovery_from_degraded_mode_lands_within_its_bound_on_every_grid_tuning() {
    let mut rep = Report::default();
    let mut n = 0;
    for t in grid() {
        let rise_ease = f64::from(t.rise_w_per_s) * t.ease_s;
        for d in [2.0, 20.0, rise_ease, 300.0] {
            for (a, b) in [(300.0, 300.0 + d), (300.0 + d, 300.0)] {
                for o in [0, 6, 12] {
                    for len in [4, 12] {
                        let k0 = 20 + o;
                        run_checked(
                            &Plan::new(t, vec![steady(a, 5.0), steady(b, 60.0)])
                                .events(vec![ev(k0, Kind::Offset, if b > a { 1.01 } else { 0.99 })])
                                .faults(Faults::on(k0..k0 + len)),
                            &mut rep,
                            &format!("recovery {a}->{b} {t:?} refused {k0}..{}", k0 + len - 1),
                        );
                        n += 1;
                    }
                }
            }
        }
    }
    rep.assert_clean();
    assert_eq!(n, 9792);
    assert!(
        rep.recoveries >= n / 2,
        "only {} landings checked",
        rep.recoveries
    );
}

#[test]
fn ramp_to_ramp_vertices_keep_every_property() {
    let mut rep = Report::default();
    for t in std::iter::once(defaults()).chain(corners()) {
        let lim = Limits::from(&t);
        let mut slopes = vec![0.0];
        for f in [0.1, 0.5, 0.74, 0.99, 1.0, 1.5] {
            slopes.push(f * lim.r_up);
            slopes.push(-f * lim.r_dn);
        }
        for &s1 in &slopes {
            for &s2 in &slopes {
                let (w0, w1) = (1500.0, 1500.0 + s1 * 10.0);
                let w2 = w1 + s2 * 5.0;
                run_checked(
                    &Plan::new(
                        t,
                        vec![
                            steady(w0, 5.0),
                            ramp(w0, w1, 10.0),
                            ramp(w1, w2, 5.0),
                            steady(w2, 15.0),
                        ],
                    )
                    .cap(4000.0),
                    &mut rep,
                    &format!("ramp pair {s1} -> {s2} W/s {t:?}"),
                );
            }
        }
    }
    rep.assert_clean();
}

#[test]
fn the_falsifiers_repros_keep_every_property() {
    let d = defaults();
    let pers = Faults::persistent;
    let none = Faults::default;
    let stair: Vec<Block> = std::iter::once(steady(100.0, 10.0))
        .chain((0..10).map(|i| steady(400.0 - 10.0 * f64::from(i), 2.0)))
        .chain(std::iter::once(steady(300.0, 30.0)))
        .collect();
    let repros: Vec<Repro> = vec![
        (
            "p10 step 233->200",
            tu(11, 2.0, 3.0),
            vec![steady(233.0, 10.0), steady(200.0, 600.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p10 step 200->233",
            tu(11, 2.0, 3.0),
            vec![steady(200.0, 10.0), steady(233.0, 600.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p10 step 275->200",
            tu(20, 1.25, 3.75),
            vec![steady(275.0, 10.0), steady(200.0, 600.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p10 step 233->200 (12)",
            tu(12, 2.0, 2.75),
            vec![steady(233.0, 10.0), steady(200.0, 600.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p2 split -19.4",
            tu(5, 4.0, 2.0),
            vec![
                steady(800.0, 5.0),
                ramp(800.0, 780.6, 1.0),
                ramp(780.6, 770.9, 0.5),
                steady(770.9, 15.0),
            ],
            4000.0,
            none(),
            vec![],
        ),
        (
            "p4 -27 then +0.75",
            tu(15, 1.5, 2.75),
            vec![
                steady(800.0, 5.0),
                ramp(800.0, 530.0, 10.0),
                ramp(530.0, 533.75, 5.0),
                steady(533.75, 300.0),
            ],
            4000.0,
            none(),
            vec![],
        ),
        (
            "p8 200->158.75",
            tu(15, 2.0, 2.75),
            vec![steady(200.0, 10.0), steady(158.75, 120.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p8 158.75->200",
            tu(15, 2.0, 2.75),
            vec![steady(158.75, 10.0), steady(200.0, 120.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p8 300->217.5",
            tu(30, 1.0, 2.75),
            vec![steady(300.0, 10.0), steady(217.5, 120.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p8 217.5->300",
            tu(30, 1.0, 2.75),
            vec![steady(217.5, 10.0), steady(300.0, 120.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "bare #269 input 41.25->0",
            tu(15, 1.5, 2.75),
            vec![steady(41.25, 5.0), steady(0.0, 30.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "probe1 B2b",
            d,
            vec![
                steady(100.0, 10.0),
                ramp(100.0, 300.0, 20.0),
                ramp(300.0, 500.0, 5.0),
                steady(500.0, 10.0),
            ],
            2000.0,
            none(),
            vec![],
        ),
        (
            "q1 fall ramp, persistent from 58",
            d,
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 300.0, 10.0),
                steady(300.0, 30.0),
            ],
            2000.0,
            pers(58),
            vec![],
        ),
        (
            "q1 steep fall, persistent from 40",
            d,
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 100.0, 5.0),
                steady(100.0, 30.0),
            ],
            2000.0,
            pers(40),
            vec![],
        ),
        (
            "q2 fall, persistent from 58",
            tu(5, 4.0, 4.0),
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 300.0, 10.0),
                steady(300.0, 120.0),
            ],
            2000.0,
            pers(58),
            vec![],
        ),
        (
            "q3 jump up to 500, persistent from 60",
            d,
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 300.0, 10.0),
                steady(500.0, 120.0),
            ],
            2000.0,
            pers(60),
            vec![],
        ),
        (
            "q3 jump up to 500, persistent from 59",
            d,
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 300.0, 10.0),
                steady(500.0, 120.0),
            ],
            2000.0,
            pers(59),
            vec![],
        ),
        (
            "q3 300 held, persistent from 58",
            d,
            vec![
                steady(400.0, 5.0),
                ramp(400.0, 300.0, 10.0),
                steady(300.0, 120.0),
            ],
            2000.0,
            pers(58),
            vec![],
        ),
        (
            "q7 cap 250: 300, ramp to 150 in 0.25 s",
            d,
            vec![
                steady(300.0, 10.0),
                ramp(300.0, 150.0, 0.25),
                steady(150.0, 60.0),
            ],
            250.0,
            none(),
            vec![],
        ),
        (
            "q7 cap 250: 300, ramp to 240 in 1 s",
            d,
            vec![
                steady(300.0, 10.0),
                ramp(300.0, 240.0, 1.0),
                steady(240.0, 60.0),
            ],
            250.0,
            none(),
            vec![],
        ),
        (
            "q7 0 W, ramp to 400 in 0.25 s",
            d,
            vec![
                steady(0.0, 10.0),
                ramp(0.0, 400.0, 0.25),
                steady(400.0, 60.0),
            ],
            2000.0,
            none(),
            vec![],
        ),
        (
            "q8 cap 250: 400 4 s, ramp to 150",
            d,
            vec![
                steady(100.0, 10.0),
                steady(400.0, 4.0),
                ramp(400.0, 150.0, 0.25),
                steady(150.0, 30.0),
            ],
            250.0,
            none(),
            vec![],
        ),
        (
            "q9 cap 250: 400 2 s, ramp to 0",
            d,
            vec![
                steady(100.0, 10.0),
                steady(400.0, 2.0),
                ramp(400.0, 0.0, 0.25),
                steady(0.0, 30.0),
            ],
            250.0,
            none(),
            vec![],
        ),
        (
            "segfault plan (seed 585, plan 19613)",
            tu(30, 4.0, 4.0),
            vec![
                steady(391.25, 2.0),
                ramp(391.25, 686.950_055_179_010_4, 2.0),
                steady(494.0, 15.0),
                steady(0.0, 1.0),
                ramp(0.0, 106.091_327_611_175_59, 5.0),
                steady(106.091_327_611_175_59, 5.0),
                steady(554.0, 30.0),
                steady(614.0, 0.5),
                ramp(614.0, 776.975_510_303_075_7, 0.5),
            ],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p3 cap 250, 400 then 300",
            d,
            vec![steady(100.0, 10.0), steady(400.0, 6.0), steady(300.0, 30.0)],
            250.0,
            none(),
            vec![],
        ),
        (
            "p3 cap 250, 400 then 300, persistent from 40",
            d,
            vec![steady(100.0, 10.0), steady(400.0, 6.0), steady(300.0, 30.0)],
            250.0,
            pers(40),
            vec![],
        ),
        (
            "p4 cap 250 staircase",
            d,
            stair.clone(),
            250.0,
            none(),
            vec![],
        ),
        (
            "p4 cap 250 staircase, persistent from 40",
            d,
            stair,
            250.0,
            pers(40),
            vec![],
        ),
        (
            "p5 300, 0, 200",
            d,
            vec![steady(300.0, 10.0), steady(0.0, 4.0), steady(200.0, 40.0)],
            2000.0,
            none(),
            vec![],
        ),
        (
            "p5 300, 0, 200, persistent from 41",
            d,
            vec![steady(300.0, 10.0), steady(0.0, 4.0), steady(200.0, 40.0)],
            2000.0,
            pers(41),
            vec![],
        ),
        (
            "p5 300, 0, 250, persistent from 41",
            d,
            vec![steady(300.0, 10.0), steady(0.0, 4.0), steady(250.0, 40.0)],
            2000.0,
            pers(41),
            vec![],
        ),
        (
            "p6 a near-flat block 8000 s, then a ramp",
            d,
            vec![
                steady(100.0, 10.0),
                Block {
                    ticks: 32000,
                    from_w: 200.0,
                    to_w: 200.0 + 1e-9 * 8000.0 * 0.999,
                },
                ramp(200.0 + 1e-9 * 8000.0 * 0.999, 260.0, 30.0),
                steady(260.0, 30.0),
            ],
            2000.0,
            none(),
            vec![],
        ),
        (
            "D5: xs990000#1063",
            tu(20, 1.0, 2.0),
            vec![
                Block {
                    ticks: 25,
                    from_w: 332.755_878_608_548_9,
                    to_w: 588.723_533_469_982_3,
                },
                Block {
                    ticks: 2,
                    from_w: 588.723_533_469_982_3,
                    to_w: 588.723_533_469_982_3,
                },
                Block {
                    ticks: 126,
                    from_w: 588.723_533_469_982_3,
                    to_w: 0.0,
                },
                Block {
                    ticks: 60,
                    from_w: 0.0,
                    to_w: 0.0,
                },
            ],
            2000.0,
            Faults::on(27..43),
            vec![],
        ),
        (
            "D5: xs990000#2110",
            tu(5, 4.0, 2.75),
            vec![
                Block {
                    ticks: 19,
                    from_w: 629.943_498_540_935,
                    to_w: 629.943_498_540_935,
                },
                Block {
                    ticks: 180,
                    from_w: 875.401_781_780_801_1,
                    to_w: 0.0,
                },
                Block {
                    ticks: 60,
                    from_w: 0.0,
                    to_w: 0.0,
                },
            ],
            2000.0,
            Faults::on(16..41),
            vec![],
        ),
        (
            "degraded mid-rise under a cap of 250 W",
            d,
            vec![steady(100.0, 5.0), steady(400.0, 40.0)],
            250.0,
            pers(28),
            vec![ev(28, Kind::Offset, 1.05)],
        ),
        (
            "2100 W, then a -20 W/s ramp (refusals 40..61)",
            d,
            vec![
                steady(2100.0, 10.0),
                ramp(1800.0, 0.0, 90.0),
                steady(0.0, 30.0),
            ],
            4000.0,
            Faults::on(40..62),
            vec![],
        ),
        (
            "fall to 0 W, refusals 120..127 (10, x2, 1)",
            tu(10, 2.0, 1.0),
            vec![
                steady(400.0, 10.0),
                ramp(400.0, 0.0, 20.0),
                steady(0.0, 30.0),
            ],
            2000.0,
            Faults::on(120..128),
            vec![],
        ),
    ];
    let mut rep = Report::default();
    for (name, t, blocks, m0, faults, events) in repros {
        let tr = run_checked(
            &Plan::new(t, blocks).cap(m0).faults(faults).events(events),
            &mut rep,
            &format!("repro: {name}"),
        );
        if name == "degraded mid-rise under a cap of 250 W" {
            assert!(
                tr.degraded[28]
                    && !tr
                        .exc
                        .iter()
                        .any(|e| matches!(e, Some(Cut::X2 | Cut::X3 | Cut::X4)))
            );
        }
        if name.starts_with("D5: xs990000#1063") {
            assert!(rep.recoveries > 0, "{name}: no recovery checked");
        }
        if name.starts_with("fall to 0 W") {
            assert!(!tr.exc.contains(&Some(Cut::X4)), "{name}: X4");
        }
    }
    // Refusals on a planned 100 -> 400 -> 100 W: no cut, never above the plan.
    for (k0, n) in [(40, 12), (40, 4), (44, 8)] {
        let tr = run_checked(
            &plain(vec![
                steady(100.0, 10.0),
                steady(400.0, 3.0),
                steady(100.0, 30.0),
            ])
            .faults(Faults::on(k0..k0 + n)),
            &mut rep,
            &format!("100/400/100 W, refusals {k0}.."),
        );
        let up = (1..tr.y.len())
            .filter(|&k| tr.y[k] > tr.y[k - 1] + 1e-7)
            .map(|k| tr.y[k] - tr.r[k])
            .fold(0.0, f64::max);
        assert!(
            !tr.exc
                .iter()
                .any(|e| matches!(e, Some(Cut::X2 | Cut::X3 | Cut::X4)))
                && up <= EPS,
            "100/400/100 W refusals {k0}: rose {up} above the plan"
        );
    }
    for hi in [300.0, 400.0] {
        run_checked(
            &plain(vec![
                steady(100.0, 10.0),
                steady(hi, 4.0),
                ramp(hi, 150.0, 0.25),
                steady(150.0, 30.0),
            ])
            .cap(250.0)
            .faults(pers(40)),
            &mut rep,
            &format!("round-6 plan at {hi} W, cap 250, refusals from 40"),
        );
    }
    // Every braking plan refused: degraded, the closed-form stop; P4/P7 after.
    for t in std::iter::once(d).chain(corners()) {
        for (hi, lo) in [(400.0, 0.0), (400.0, 100.0), (600.0, 200.0)] {
            run_checked(
                &Plan::new(t, vec![steady(hi, 10.0), steady(lo, 60.0)]).faults(Faults {
                    fail_brake: true,
                    ..Faults::default()
                }),
                &mut rep,
                &format!("every braking plan refused, {hi} -> {lo} W {t:?}"),
            );
        }
    }
    // Degraded on ramps within the rates (an offset mid-ramp refused): stops, holds, lands back.
    for t in std::iter::once(d).chain(corners()) {
        let lim = Limits::from(&t);
        for f in [0.5, -0.5] {
            let s = f * if f > 0.0 { lim.r_up } else { lim.r_dn };
            let w = 300.0 + s.abs() * 70.0;
            for len in [6, 40] {
                run_checked(
                    &Plan::new(
                        t,
                        vec![
                            steady(w, 5.0),
                            ramp(w, w + s * 60.0, 60.0),
                            steady(w + s * 60.0, 10.0),
                        ],
                    )
                    .events(vec![ev(60, Kind::Offset, 1.0001)])
                    .cap(1e6)
                    .faults(Faults::on(60..60 + len)),
                    &mut rep,
                    &format!("degraded {len} ticks on a {s} W/s ramp {t:?}"),
                );
            }
        }
    }
    // A cap drop below a degraded command at the full falling rate: X3 before the step keeps the
    // falling slope, and the stop continues from the cut state (E21).
    for t in [d, tu(20, 4.0, 2.0)] {
        let lim = Limits::from(&t);
        let blocks = vec![steady(600.0, 10.0), steady(100.0, 40.0)];
        let free = run(&Plan::new(t, blocks.clone()));
        let kc = (40..free.y.len())
            .find(|&k| free.v[k] <= -0.9 * lim.r_dn)
            .expect("the fall reaches 0.9 r↓");
        let ki = i64::try_from(kc).expect("small");
        let tr = run_checked(
            &Plan::new(t, blocks)
                .events(vec![
                    ev(ki, Kind::Offset, 1.0),
                    ev(ki + 2, Kind::Cap, free.y[kc + 1] - 40.0),
                ])
                .faults(pers(ki)),
            &mut rep,
            &format!("cap drop below a degraded command {t:?}"),
        );
        assert!(tr.degraded[kc] && tr.exc[kc + 2] == Some(Cut::X3));
        assert!(
            tr.y[kc + 3] < tr.y[kc + 2] - 1e-6,
            "the stop from the cut state keeps falling"
        );
    }
    // Round 12's falsifier: refusal windows over the vertex of a planned fall.
    for slope in [39.5, 30.0] {
        let blocks = vec![
            steady(400.0, 10.0),
            ramp(400.0, 0.0, 400.0 / slope),
            steady(0.0, 20.0),
        ];
        for k0 in (25..41).step_by(3) {
            for n in [3, 12] {
                run_checked(
                    &plain(blocks.clone()).faults(Faults::on(k0..k0 + n)),
                    &mut rep,
                    &format!("planned fall at -{slope} W/s, refusals {k0}.."),
                );
            }
        }
    }
    // Round 13's falsifier: the lead from far below the line.
    for t in [tu(50, 3.0, 3.0), d] {
        let blocks = vec![
            steady(280.0, 5.0),
            steady(573.0, 1.25),
            ramp(573.0, 0.0, 10.0),
            steady(0.0, 5.0),
        ];
        for (k0, len) in [(16, 80), (16, 20), (20, 12), (22, 8), (16, 8)] {
            run_checked(
                &Plan::new(t, blocks.clone()).faults(Faults::on(k0..k0 + len)),
                &mut rep,
                &format!("280/573 W then a fall to 0 W, refusals {k0}.. {t:?}"),
            );
        }
    }
    rep.assert_clean();
}

#[test]
fn x5s_mechanism_one_refusal_is_one_degraded_tick_and_the_twin_is_never_degraded() {
    let p = plain(vec![steady(100.0, 5.0), steady(300.0, 30.0)]).events(vec![ev(
        25,
        Kind::Offset,
        1.1,
    )]);
    let mut rep = Report::default();
    let tr = run_checked(
        &p.clone().faults(Faults::persistent(25)),
        &mut rep,
        "refused from 25",
    );
    assert!(!tr.degraded[..25].iter().any(|x| *x) && tr.degraded[25..].iter().all(|x| *x));
    let once = run(&p.clone().faults(Faults {
        until: Some(26),
        ..Faults::persistent(25)
    }));
    assert!(
        once.degraded[25]
            && once.exc[25] == Some(Cut::X5)
            && !once.degraded[26..].iter().any(|x| *x)
    );
    let tw = run_checked(
        &p.faults(Faults {
            twin_answers: true,
            ..Faults::persistent(25)
        }),
        &mut rep,
        "the twin answering",
    );
    assert!(!tw.degraded.iter().any(|x| *x) && tw.twins > 0);
    rep.assert_clean();
}

#[test]
fn p7_holds_on_every_line_after_a_downward_step() {
    // f625i: a −50 % offset onto a 300 -> 350 W ramp over 300 s: rounds 1–13 held 303 W for 280 s.
    let mut rep = Report::default();
    let tr = run_checked(
        &plain(vec![steady(300.0, 5.0), ramp(300.0, 350.0, 300.0)]).events(vec![ev(
            100,
            Kind::Offset,
            0.5,
        )]),
        &mut rep,
        "f625i",
    );
    let above = (100..tr.y.len())
        .filter(|&k| tr.y[k] > tr.r[k] + EPS)
        .count();
    assert!(rep.above_checked == 1 && tick_f(i64::try_from(above).expect("small")) * DT <= 10.0);
    // The slope sweep: down to s → 0± and up to 0.99 of the rates.
    let mut checked = 0;
    let mut n = 0;
    let g = grid();
    for t in std::iter::once(defaults())
        .chain(corners())
        .chain(g.iter().step_by(6).copied())
    {
        let lim = Limits::from(&t);
        let fr = [1e-4, 1e-3, 0.01, 0.05, 0.2, 0.5, 0.8, 0.99];
        let slopes = fr
            .iter()
            .map(|f| f * lim.r_up)
            .chain(fr.iter().map(|f| -f * lim.r_dn));
        for s in slopes {
            let d = (1200.0 / s.abs()).min(120.0);
            let blocks = vec![
                steady(1500.0, 5.0),
                ramp(1500.0, 1500.0 + s * d, d),
                steady(1500.0 + s * d, 10.0),
            ];
            #[expect(clippy::cast_possible_truncation, reason = "small")]
            let ke = 20 + ((d / DT) as i64 / 4).max(1);
            for e in [
                ev(ke, Kind::Offset, 0.5),
                ev(ke, Kind::Offset, 0.8),
                ev(ke, Kind::Cap, 1500.0 + s * tick_f(ke) * DT * 0.5 - 400.0),
            ] {
                let mut r2 = Report::default();
                run_checked(
                    &Plan::new(t, blocks.clone()).events(vec![e]).cap(4000.0),
                    &mut r2,
                    &format!("above sweep {t:?} {s}"),
                );
                checked += r2.above_checked;
                rep.fails.extend(r2.fails);
                n += 1;
            }
        }
    }
    assert!(checked >= n / 2, "P7 checked on {checked} of {n}");
    // The check itself: a command held above a rising line fails check_above.
    let mut held = tr.clone();
    for k in 100..held.y.len() {
        (held.y[k], held.v[k], held.a[k]) = (held.y[99], 0.0, 0.0);
    }
    let mut r5 = Report::default();
    check_above(&held, &Limits::from(&defaults()), &mut r5, "self-test");
    assert!(
        r5.fails
            .iter()
            .any(|f| f.contains("P7 still above the line"))
    );
    rep.assert_clean();
}

#[test]
fn a_plan_cap_or_state_that_is_not_a_number_is_degraded_and_the_output_stays_in_range() {
    let (nan, inf) = (f64::NAN, f64::INFINITY);
    let cases: Vec<NanCase> = vec![
        (
            "a NaN offset",
            vec![steady(200.0, 10.0), ramp(200.0, 260.0, 30.0)],
            vec![ev(40, Kind::Offset, nan), ev(60, Kind::Offset, 1.0)],
            40,
            60,
        ),
        (
            "an inf offset",
            vec![steady(200.0, 10.0), ramp(200.0, 260.0, 30.0)],
            vec![ev(40, Kind::Offset, inf), ev(60, Kind::Offset, 1.0)],
            40,
            60,
        ),
        (
            "a NaN cap",
            vec![steady(200.0, 10.0), steady(300.0, 30.0)],
            vec![ev(30, Kind::Cap, nan), ev(70, Kind::Cap, 250.0)],
            30,
            70,
        ),
        (
            "a -inf cap",
            vec![steady(200.0, 10.0), steady(300.0, 30.0)],
            vec![ev(30, Kind::Cap, -inf), ev(70, Kind::Cap, 250.0)],
            30,
            70,
        ),
        (
            "a NaN block",
            vec![
                steady(200.0, 10.0),
                Block {
                    ticks: 20,
                    from_w: nan,
                    to_w: nan,
                },
                steady(150.0, 20.0),
            ],
            vec![],
            0,
            60,
        ),
        (
            "an inf block",
            vec![
                steady(200.0, 10.0),
                Block {
                    ticks: 20,
                    from_w: 200.0,
                    to_w: inf,
                },
                steady(150.0, 20.0),
            ],
            vec![],
            0,
            60,
        ),
    ];
    for (name, blocks, events, k0, k1) in cases {
        let caps: Vec<f64> = std::iter::once(400.0)
            .chain(
                events
                    .iter()
                    .filter(|e| e.kind == Kind::Cap && e.value.is_finite())
                    .map(|e| e.value),
            )
            .collect();
        let top = caps.iter().copied().fold(f64::MIN, f64::max);
        let tr = run(&plain(blocks).events(events).cap(400.0));
        assert!(
            tr.y.iter()
                .all(|y| y.is_finite() && (0.0..=top + 1e-9).contains(y)),
            "{name}: out of range"
        );
        assert!(
            tr.degraded[k0..k1].iter().all(|x| *x),
            "{name}: not degraded throughout"
        );
        assert!(
            tr.exc[k0..k1].iter().all(|e| *e == Some(Cut::X5)),
            "{name}: untagged"
        );
        if k1 < tr.y.len() {
            assert!(
                !tr.degraded[k1 + 1..].iter().any(|x| *x),
                "{name}: still degraded once valid"
            );
        }
    }
    // A valid cap takes effect whatever the fault state (round 15).
    for (name, blocks, events, k0) in [
        (
            "a NaN offset, cap 100 W",
            vec![
                steady(300.0, 10.0),
                steady(300.0, 40.0),
                steady(150.0, 20.0),
            ],
            vec![ev(20, Kind::Offset, nan), ev(30, Kind::Cap, 100.0)],
            30,
        ),
        (
            "an inf offset, cap 50 W",
            vec![steady(250.0, 60.0)],
            vec![ev(8, Kind::Offset, inf), ev(12, Kind::Cap, 50.0)],
            12,
        ),
        (
            "a NaN block ahead, cap 100 W",
            vec![
                steady(300.0, 10.0),
                Block {
                    ticks: 20,
                    from_w: nan,
                    to_w: nan,
                },
                steady(300.0, 20.0),
            ],
            vec![ev(20, Kind::Cap, 100.0)],
            20,
        ),
    ] {
        let tr = run(&plain(blocks).events(events).cap(400.0));
        assert!(
            (0..tr.y.len())
                .all(|k| tr.y[k].is_finite() && 0.0 <= tr.y[k] && tr.y[k] <= tr.m[k] + 1e-9),
            "{name}: outside [0, M]"
        );
        assert!(tr.y[k0] <= tr.m[k0] + 1e-9 && tr.degraded[k0], "{name}");
    }
    // A state that is not a number: the output a number in [0, M], the status raised.
    let st = |p: Plan, name: &str| {
        let tr = run(&p);
        assert!(
            (0..tr.y.len()).all(|k| tr.y[k].is_finite()
                && tr.v[k].is_finite()
                && tr.a[k].is_finite()
                && 0.0 <= tr.y[k]
                && tr.y[k] <= tr.m[k] + 1e-9),
            "{name}: out of range"
        );
        assert!(tr.degraded.iter().any(|x| *x), "{name}: not degraded");
    };
    st(plain(vec![steady(200.0, 10.0)]).y0(nan), "a NaN y0");
    st(
        Plan {
            v0: nan,
            ..plain(vec![steady(200.0, 10.0)])
        },
        "a NaN v0",
    );
    st(plain(vec![steady(200.0, 10.0)]).origin(nan), "a NaN origin");
    st(
        plain(vec![steady(200.0, 10.0)]).origin(inf),
        "an inf origin",
    );
    st(
        plain(vec![steady(200.0, 20.0)]).events(vec![ev(10, Kind::Engage, nan)]),
        "a NaN engagement",
    );
    st(
        plain(vec![steady(200.0, 20.0)]).events(vec![ev(10, Kind::Engage, -inf)]),
        "an inf engagement",
    );
    st(
        plain(vec![
            Block {
                ticks: 8,
                from_w: nan,
                to_w: nan,
            },
            steady(200.0, 10.0),
        ]),
        "a NaN first block",
    );
    // K6: a NaN engagement while riding 200 W holds the last command sent.
    let tr = run(&plain(vec![steady(200.0, 20.0)]).events(vec![ev(10, Kind::Engage, nan)]));
    assert!((tr.y[10] - tr.y[9]).abs() < 1e-9 && (tr.y[9] - 200.0).abs() < 1e-9);
    // An invalid initial cap is a fault from the first tick, and the command holds.
    let tr = run(&plain(vec![steady(100.0, 5.0), steady(300.0, 20.0)]).cap(nan));
    assert!(tr.degraded.iter().all(|x| *x) && tr.y.iter().all(|y| *y <= 100.0 + 1e-9));
    // A NaN cap, then a valid one: every property against the cap in force.
    let mut rep = Report::default();
    for cap in [0.0, 150.0] {
        run_checked(
            &plain(vec![steady(200.0, 10.0), steady(300.0, 30.0)])
                .events(vec![ev(20, Kind::Cap, nan), ev(30, Kind::Cap, cap)]),
            &mut rep,
            &format!("a NaN cap, then {cap} W"),
        );
    }
    // A cap below 0 W: the output is 0 W.
    for (blocks, events, m0) in [
        (
            vec![steady(200.0, 10.0), steady(300.0, 20.0)],
            vec![ev(20, Kind::Cap, -5.0)],
            2000.0,
        ),
        (vec![steady(200.0, 10.0)], vec![], -5.0),
        (
            vec![steady(0.0, 10.0)],
            vec![ev(4, Kind::Cap, -1e-6)],
            100.0,
        ),
    ] {
        let k0 = events
            .first()
            .map_or(0, |e| usize::try_from(e.tick).expect("small"));
        let tr = run_checked(
            &plain(blocks).events(events).cap(m0),
            &mut rep,
            "cap below 0 W",
        );
        assert!(
            tr.y[k0..].iter().all(|y| *y == 0.0),
            "cap below 0 W: not 0 W"
        );
    }
    rep.assert_clean();
}

#[test]
fn the_input_layer_takes_the_last_valid_value_per_tick_and_faults_on_any_invalid_one() {
    let nan = f64::NAN;
    let base = || plain(vec![steady(300.0, 30.0)]).cap(400.0);
    for (name, events, want_m, deg_after) in [
        (
            "cap 100 then NaN",
            vec![ev(10, Kind::Cap, 100.0), ev(10, Kind::Cap, nan)],
            100.0,
            true,
        ),
        (
            "cap NaN then 100",
            vec![ev(10, Kind::Cap, nan), ev(10, Kind::Cap, 100.0)],
            100.0,
            false,
        ),
        (
            "cap 100 then -inf",
            vec![
                ev(10, Kind::Cap, 100.0),
                ev(10, Kind::Cap, f64::NEG_INFINITY),
            ],
            100.0,
            true,
        ),
        (
            "cap -inf then 100",
            vec![
                ev(10, Kind::Cap, f64::NEG_INFINITY),
                ev(10, Kind::Cap, 100.0),
            ],
            100.0,
            false,
        ),
        (
            "cap 100 then 50",
            vec![ev(10, Kind::Cap, 100.0), ev(10, Kind::Cap, 50.0)],
            50.0,
            false,
        ),
        (
            "cap 50 then 100",
            vec![ev(10, Kind::Cap, 50.0), ev(10, Kind::Cap, 100.0)],
            100.0,
            false,
        ),
        (
            "cap -5 then NaN",
            vec![ev(10, Kind::Cap, -5.0), ev(10, Kind::Cap, nan)],
            0.0,
            true,
        ),
    ] {
        let fault = events
            .iter()
            .any(|e| e.value.is_nan() || e.value == f64::NEG_INFINITY);
        let tr = run(&base().events(events));
        assert!(
            (0..tr.y.len())
                .all(|k| tr.y[k].is_finite() && 0.0 <= tr.y[k] && tr.y[k] <= tr.m[k] + 1e-9),
            "{name}: out of [0, M]"
        );
        assert!(
            (tr.m[10] - want_m).abs() < 1e-12 && tr.y[10] <= want_m + 1e-9,
            "{name}: M {}",
            tr.m[10]
        );
        assert_eq!(tr.degraded[10], fault, "{name}: the fault at the tick");
        assert_eq!(
            tr.degraded[11..].iter().any(|x| *x),
            deg_after,
            "{name}: the fault after it"
        );
    }
    for (name, events, deg_after) in [
        (
            "offset 0.5 then NaN",
            vec![ev(10, Kind::Offset, 0.5), ev(10, Kind::Offset, nan)],
            true,
        ),
        (
            "offset NaN then 0.5",
            vec![ev(10, Kind::Offset, nan), ev(10, Kind::Offset, 0.5)],
            false,
        ),
        (
            "offset 0.5 then inf",
            vec![
                ev(10, Kind::Offset, 0.5),
                ev(10, Kind::Offset, f64::INFINITY),
            ],
            true,
        ),
    ] {
        let tr = run(&base().events(events));
        assert!(
            (tr.r[10] - 150.0).abs() < 1e-12 && tr.degraded[10],
            "{name}"
        );
        assert_eq!(
            tr.degraded[11..].iter().any(|x| *x),
            deg_after,
            "{name}: after"
        );
        assert!(tr.y.iter().all(|y| y.is_finite()));
    }
    for (name, events) in [
        (
            "engage NaN then 150",
            vec![ev(10, Kind::Engage, nan), ev(10, Kind::Engage, 150.0)],
        ),
        (
            "engage 150 then NaN",
            vec![ev(10, Kind::Engage, 150.0), ev(10, Kind::Engage, nan)],
        ),
    ] {
        let tr = run(&base().events(events));
        assert!(
            (tr.origin[10] - 150.0).abs() < 1e-9 && tr.degraded[10],
            "{name}"
        );
        assert!(tr.y.iter().all(|y| y.is_finite()));
    }
}

#[test]
fn x3_never_undershoots_the_line_it_caps() {
    let mut rep = Report::default();
    // A cap drop below a command falling fast settles at M.
    let t15 = tu(50, 4.0, 4.0);
    for (sl, cap) in [
        (-100.0, 20.0),
        (-100.0, 60.0),
        (-100.0, 5.0),
        (-180.0, -300.0),
        (-180.0, 0.0),
        (-100.0, 0.0),
    ] {
        let d = (1500.0 / f64::abs(sl)).min(200.0);
        let tr = run_checked(
            &Plan::new(
                t15,
                vec![
                    steady(1600.0, 5.0),
                    ramp(1600.0, 1600.0 + sl * d, d),
                    steady(1600.0 + sl * d, 30.0),
                ],
            )
            .events(vec![ev(60, Kind::Cap, cap)])
            .cap(6000.0),
            &mut rep,
            &format!("cap {cap} W under a {sl} W/s fall"),
        );
        let low = tr.y[60..].iter().copied().fold(f64::MAX, f64::min);
        assert!(
            low >= f64::max(cap, 0.0) - 1e-9,
            "undershoots the cap to {low}"
        );
    }
    // K12: X3 with a curvature at the cut, over the fall's onset.
    let mut curved = 0;
    let mut n12 = 0;
    for t in [
        tu(50, 4.0, 4.0),
        defaults(),
        tu(20, 2.0, 2.0),
        tu(50, 1.0, 1.0),
    ] {
        let blocks = vec![steady(900.0, 5.0), steady(300.0, 40.0)];
        let free = run(&Plan::new(t, blocks.clone()));
        for kc in 21..45_usize {
            let cap = if kc == 36 && t == tu(50, 4.0, 4.0) {
                541.7
            } else {
                f64::midpoint(free.y[kc - 1], 300.0)
            };
            let tr = run_checked(
                &Plan::new(t, blocks.clone()).events(vec![ev(
                    i64::try_from(kc).expect("small"),
                    Kind::Cap,
                    cap,
                )]),
                &mut rep,
                &format!("X3 with curvature, cap {cap} at {kc} {t:?}"),
            );
            n12 += 1;
            curved += usize::from(free.a[kc - 1].abs() > 1e-6);
            assert!(
                tr.y[kc..].iter().all(|y| *y >= 300.0 - EPS),
                "fell below the 300 W line"
            );
        }
    }
    assert!(curved >= n12 / 3);
    // K12's pass-through: the port stays on the 300 W line.
    let tr = run_checked(
        &Plan::new(t15, vec![steady(900.0, 5.0), steady(300.0, 40.0)]).events(vec![ev(
            28,
            Kind::Cap,
            358.33,
        )]),
        &mut rep,
        "X3 at 358.33 W",
    );
    assert!(tr.y[28..].iter().all(|y| *y >= 300.0 - EPS));
    // The check itself: a command 2 W under the capped line after X3 fails.
    let mut tr = run(
        &Plan::new(t15, vec![steady(900.0, 5.0), steady(300.0, 40.0)]).events(vec![ev(
            36,
            Kind::Cap,
            541.7,
        )]),
    );
    let k3 = tr.exc.iter().position(|e| *e == Some(Cut::X3)).expect("X3");
    tr.y[k3 + 1] = tr.r[k3 + 1].min(tr.m[k3 + 1]) - 2.0;
    let mut r13 = Report::default();
    check(&tr, &t15, &mut r13, "self-test");
    assert!(
        r13.fails
            .iter()
            .any(|f| f.contains("below the capped line"))
    );
    rep.assert_clean();
}

#[test]
fn d5_below_a_falling_line_meets_it_one_tick_early() {
    // A353 (f625k): a recovery at rest 50 W below a line falling at 4.1 W/s at (50, x4, 1).
    let t16 = tu(50, 4.0, 1.0);
    let bl = |t: u32, a: f64, b: f64| Block {
        ticks: t,
        from_w: a,
        to_w: b,
    };
    let blocks = vec![
        bl(139, 246.25, 103.630_208_333_334_3),
        bl(46, 103.630_208_333_334_3, 56.432_291_666_666_1),
        bl(55, 56.432_291_666_666_1, 0.0),
        bl(40, 0.0, 0.0),
        bl(25, 0.0, 3.359_356_511_097_046e-14),
        bl(12, 3.359_356_511_097_046e-14, 6.103_328_415_515_826e-13),
        bl(203, 6.103_328_415_515_826e-13, 0.0),
        bl(5, 117.144_513_489_588_49, 114.464_433_772_653_35),
        bl(3, 114.464_433_772_653_35, 112.856_385_942_494_03),
        bl(32, 112.856_385_942_494_03, 95.703_875_754_112_49),
    ];
    let events = vec![
        ev(86, Kind::Skip, 0.0),
        ev(457, Kind::Engage, 105.967_818_582_014_94),
        ev(353, Kind::Cap, 616.598_075_301_440_5),
        ev(373, Kind::Cap, 255.481_772_427_448_42),
        ev(0, Kind::Cap, 151.961_567_764_333_98),
        ev(194, Kind::Cap, 141.554_085_066_686_27),
        ev(484, Kind::Cap, 1e-09),
        ev(101, Kind::Cap, 1e-09),
        ev(102, Kind::Cap, 0.0),
        ev(103, Kind::Cap, 0.0),
        ev(104, Kind::Cap, 546.331_283_382_008_8),
        ev(105, Kind::Cap, -5.0),
        ev(106, Kind::Cap, 0.0),
        ev(107, Kind::Cap, 1e-09),
        ev(108, Kind::Cap, 540.769_366_564_784_4),
        ev(109, Kind::Cap, 1e-09),
        ev(110, Kind::Cap, 190.281_099_361_838_73),
    ];
    let mut rep = Report::default();
    let tr = run_checked(
        &Plan::new(t16, blocks)
            .events(events)
            .faults(Faults::on(100..139)),
        &mut rep,
        "A353",
    );
    let land = (139..tr.y.len())
        .find(|&k| (tr.y[k] - tr.r[k]).abs() <= EPS)
        .expect("lands");
    let s = tr.piece[139].1;
    let bound = ramp_bound(
        &Limits::from(&t16),
        tr.r[139] - DT * s - tr.y[138],
        0.0,
        0.0,
        s,
    );
    assert!(rep.recoveries > 0 && tick_f(i64::try_from(land - 139).expect("small")) * DT <= bound);
    // f625l: at (50, x1, 1), its repro and 360 recoveries below falling lines: no D5 miss.
    let t17 = tu(50, 1.0, 1.0);
    let mut cases = vec![(vec![ramp(1000.0, 600.0, 80.0)], 950.0, 2)];
    for s in [
        -0.5, -1.0, -2.0, -3.75, -5.0, -7.5, -10.0, -15.0, -20.0, -30.0, -40.0, -49.0_f64,
    ] {
        for d in [5.0, 10.0, 20.0, 30.0, 40.0, 52.0, 70.0, 100.0, 150.0, 200.0] {
            for refused in [2, 4, 5] {
                let secs = (950.0 / s.abs()).min(80.0);
                cases.push((
                    vec![
                        ramp(1000.0, 1000.0 + s * secs, secs),
                        steady(1000.0 + s * secs, 20.0),
                    ],
                    1000.0 - d,
                    refused,
                ));
            }
        }
    }
    for (i, (blocks, y0, refused)) in cases.into_iter().enumerate() {
        run_checked(
            &Plan::new(t17, blocks)
                .y0(y0)
                .cap(5000.0)
                .faults(Faults::on(0..refused)),
            &mut rep,
            &format!("D5 below a falling line #{i}"),
        );
    }
    rep.assert_clean();
}

#[test]
fn a_slow_answer_on_a_ramp_frame_is_rejected_and_the_twin_answers() {
    // Seed 1 plan 1270 (round 14): a return onto a −5 W/s ramp answered in 24 s, ramp_bound 18.75 s.
    let bl = |t: u32, a: f64, b: f64| Block {
        ticks: t,
        from_w: a,
        to_w: b,
    };
    let blocks = vec![
        bl(8, 211.316_255_285_117_4, 155.946_782_446_432_34),
        bl(40, 567.75, 567.75),
        bl(2, 447.75, 447.75),
        bl(4, 0.0, 0.0),
        bl(80, 0.0, 389.092_229_331_367_66),
        bl(20, 389.092_229_331_367_66, 389.092_229_331_367_66),
        bl(1, 484.070_417_138_855_8, 485.772_846_542_835_74),
        bl(40, 485.772_846_542_835_74, 769.144_552_688_837_6),
        bl(20, 829.144_552_688_837_6, 829.144_552_688_837_6),
        bl(120, 142.879_907_574_618_76, 0.0),
    ];
    let events = vec![
        ev(167, Kind::Skip, 0.0),
        ev(195, Kind::Offset, 1.05),
        ev(112, Kind::Engage, 65.871_452_344_216_28),
    ];
    let mut rep = Report::default();
    let tr = run_checked(
        &Plan::new(tu(30, 1.25, 2.0), blocks).events(events),
        &mut rep,
        "seed 1 plan 1270",
    );
    rep.assert_clean();
    assert!(
        tr.slow > 0 && !tr.degraded.iter().any(|x| *x),
        "slow {}, degraded",
        tr.slow
    );
}

#[test]
fn the_checks_themselves_catch_what_they_guard() {
    let mut rep = Report::default();
    // a cut is tested against its domain on every tick, also inside the band
    let mut tr = run(&plain(vec![steady(200.0, 20.0)]));
    tr.exc[40] = Some(Cut::X2);
    check(&tr, &defaults(), &mut rep, "self-test");
    assert!(rep.fails.iter().any(|f| f.contains("outside its domain")));
    // degraded mode against the closed-form stop
    let mut tr = run(&plain(vec![steady(100.0, 5.0), steady(300.0, 30.0)])
        .events(vec![ev(25, Kind::Offset, 1.1)])
        .faults(Faults::persistent(25)));
    tr.y[30] += 1e-3;
    let mut r2 = Report::default();
    check(&tr, &defaults(), &mut r2, "self-test");
    assert!(
        r2.fails
            .iter()
            .any(|f| f.contains("off the closed-form stop"))
    );
    // a sent position off the reported state
    let mut tr = run(&plain(vec![steady(100.0, 5.0), steady(300.0, 30.0)]));
    tr.y[30] += 0.5;
    let mut r3 = Report::default();
    check(&tr, &defaults(), &mut r3, "self-test");
    assert!(
        r3.fails
            .iter()
            .any(|f| f.contains("state not the one sent"))
    );
}
