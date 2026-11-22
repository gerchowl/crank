//! Property tests (proptest) over random plans: the model's generators (`random_plan_v6/v7`,
//! `fall_plan`, `cap_event_plan`) ported, drawn from a proptest seed, with events (±, cap, skip,
//! engagement) and fault injection: none, persistent from a random tick, intermittent (the twin
//! failing too, or answering), refusal windows of 1–40 ticks. Every property of ADR-0002 §7.10.6
//! (P1–P7 outside degraded mode, D1–D5 inside it, the bounds and domains of X2–X4) is checked on
//! every tick by `check`. Plus the line's tick-independence and canonical form, and held lines.
use super::harness::{Ev, Faults, Kind, Plan, Report, Rng, check, ev, grid, ramp, run, steady};
use crate::slew::line::collinear;
use crate::slew::{Block, DT, EPS, Limits, VALUE_TOL, segments};
use crate::tuning::SlewTuning;
use proptest::prelude::*;

fn total(blocks: &[Block]) -> i64 {
    blocks.iter().map(|b| i64::from(b.ticks)).sum()
}

fn range_i(rng: &mut Rng, n: i64) -> i64 {
    i64::try_from(rng.below(usize::try_from(n.max(1)).expect("small"))).expect("small")
}

/// The model's `random_plan_v6` (grid-exact steps, split ramps with float noise) and `v7`'s extra
/// shapes (clamp then drop within a tick; ramps from exactly 0 W).
fn random_plan(rng: &mut Rng, t: &SlewTuning) -> (Vec<Block>, Vec<Ev>) {
    let l = Limits::from(t);
    let rise_ease = f64::from(t.rise_w_per_s) * t.ease_s;
    let mut blocks = Vec::new();
    let mut w = (rng.uniform(0.0, 500.0) * 4.0).round() / 4.0;
    for _ in 0..2 + rng.below(7) {
        let secs = rng.pick(&[0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0]);
        let kind = rng.next_f64();
        if kind < 0.45 {
            let pick = rng.next_f64();
            w = if pick < 0.4 {
                let sign = rng.pick(&[-1.0, 1.0]);
                (w + sign * rise_ease * rng.pick(&[1.0, 2.0, 3.0, 0.5])).max(0.0)
            } else if pick < 0.7 {
                (rng.uniform(0.0, 600.0) * 4.0).round() / 4.0
            } else {
                rng.uniform(0.0, 600.0)
            };
            blocks.push(steady(w, secs));
        } else {
            let start = if rng.next_f64() < 0.6 {
                w
            } else {
                rng.uniform(0.0, 600.0)
            };
            let slope = if kind < 0.8 {
                rng.uniform(-l.r_dn, l.r_up)
            } else {
                rng.pick(&[-1.0, 1.0]) * rng.uniform(l.r_up, 3.0 * l.r_dn)
            };
            let end = (start + slope * secs).max(0.0);
            let n = ramp(start, end, secs).ticks;
            if n >= 3 && rng.next_f64() < 0.35 {
                let mut cuts = vec![1 + rng.below(usize::try_from(n - 1).expect("small"))];
                cuts.push(1 + rng.below(usize::try_from(n - 1).expect("small")));
                cuts.sort_unstable();
                cuts.dedup();
                let pts: Vec<u32> = std::iter::once(0)
                    .chain(cuts.into_iter().map(|c| u32::try_from(c).expect("small")))
                    .chain(std::iter::once(n))
                    .collect();
                let vals: Vec<f64> = pts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let v = start + (end - start) * f64::from(*p) / f64::from(n);
                        if i == 0 || i == pts.len() - 1 {
                            v
                        } else {
                            v + rng.uniform(-1e-12, 1e-12)
                        }
                    })
                    .collect();
                for i in 0..pts.len() - 1 {
                    blocks.push(Block {
                        ticks: pts[i + 1] - pts[i],
                        from_w: vals[i],
                        to_w: vals[i + 1],
                    });
                }
            } else {
                blocks.push(ramp(start, end, secs));
            }
            w = end;
        }
    }
    let extra = rng.next_f64();
    if extra < 0.25 {
        let (hi, low) = (
            rng.uniform(800.0, 1500.0),
            if rng.next_f64() < 0.5 {
                0.0
            } else {
                rng.uniform(0.0, 400.0)
            },
        );
        let at = rng.below(blocks.len() + 1);
        let ins = [
            steady(hi, rng.pick(&[1.0, 2.0, 4.0, 8.0])),
            ramp(hi, low, rng.pick(&[0.25, 0.5, 1.0])),
            steady(low, rng.pick(&[5.0, 15.0, 30.0])),
        ];
        blocks.splice(at..at, ins);
    } else if extra < 0.45 {
        let at = rng.below(blocks.len() + 1);
        let top = rng.uniform(50.0, 500.0);
        let ins = [
            steady(0.0, rng.pick(&[1.0, 5.0])),
            ramp(0.0, top, rng.pick(&[0.25, 1.0, 5.0, 20.0])),
            steady(top, rng.pick(&[5.0, 15.0])),
        ];
        blocks.splice(at..at, ins);
    }
    let n = total(&blocks);
    let mut events = Vec::new();
    for _ in 0..rng.below(4) {
        let kind = rng.pick(&[Kind::Offset, Kind::Cap, Kind::Skip, Kind::Engage]);
        let value = match kind {
            Kind::Offset => rng.pick(&[0.9, 0.95, 1.05, 1.1]),
            Kind::Cap => rng.uniform(150.0, 700.0),
            Kind::Skip => 0.0,
            Kind::Engage => rng.uniform(0.0, 400.0),
        };
        events.push(ev(range_i(rng, n), kind, value));
    }
    (blocks, events)
}

/// A random plan (tuning, blocks, events, cap, maybe an engagement origin) from `seed`.
fn plan_from(seed: u64) -> (Plan, Rng) {
    let mut rng = Rng(seed);
    let t = rng.pick(&grid());
    let (blocks, events) = random_plan(&mut rng, &t);
    let origin = (rng.next_f64() < 0.3).then(|| rng.uniform(0.0, 300.0));
    let m0 = if rng.next_f64() < 0.5 {
        2000.0
    } else {
        rng.uniform(200.0, 800.0)
    };
    let mut p = Plan::new(t, blocks).events(events).cap(m0);
    p.origin = origin;
    (p, rng)
}

fn checked(p: &Plan, name: &str) -> Result<(super::harness::Trace, Report), TestCaseError> {
    let tr = run(p);
    let mut rep = Report::default();
    check(&tr, &p.tuning, &mut rep, name);
    prop_assert!(rep.fails.is_empty(), "{}\n{:?}", rep.fails.join("\n"), p);
    Ok((tr, rep))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn random_plans_keep_every_property(seed in any::<u64>()) {
        let (p, _) = plan_from(seed);
        let (tr, _) = checked(&p, "random")?;
        prop_assert!(!tr.degraded.iter().any(|x| *x), "degraded without a fault");
    }

    #[test]
    fn a_persistent_refusal_from_any_tick_keeps_degraded_modes_promises(seed in any::<u64>()) {
        let (p, mut rng) = plan_from(seed);
        let onset = range_i(&mut rng, total(&p.blocks));
        let (tr, _) = checked(&p.faults(Faults::persistent(onset)), "persistent")?;
        // from the onset every calculation is refused: degraded on every tick after the first
        // refused one (a natural double refusal before the onset is not the injection's)
        // (a skip can end the plan before the onset)
        let start = usize::try_from(onset).expect("small").min(tr.failed.len());
        if let Some(first) = tr.failed[start..].iter().position(|x| *x) {
            prop_assert!(tr.degraded[start + first..].iter().all(|x| *x), "degraded from the first refusal on");
        }
    }

    #[test]
    fn intermittent_refusals_keep_every_property_and_the_twin_alone_never_degrades(seed in any::<u64>()) {
        let (p, _) = plan_from(seed);
        let faults = Faults { share: 0.2, seed, ..Faults::default() };
        checked(&p.clone().faults(faults.clone()), "intermittent")?;
        let (tw, _) = checked(&p.faults(Faults { twin_answers: true, ..faults }), "intermittent, twin answering")?;
        prop_assert!(!tw.degraded.iter().any(|x| *x), "the twin answering, degraded");
    }

    #[test]
    fn refusal_windows_of_1_to_40_ticks_keep_every_property(seed in any::<u64>()) {
        let (p, mut rng) = plan_from(seed);
        let k0 = range_i(&mut rng, total(&p.blocks));
        let len = rng.pick(&[1, 2, 3, 4, 5, 6, 8, 12, 20, 40]);
        checked(&p.faults(Faults::on(k0..k0 + len)), "refusal window")?;
    }

    #[test]
    fn refusal_windows_on_planned_falls_keep_every_property(seed in any::<u64>()) {
        // the model's fall_plan: a held or rising line, a ramp down at 20–100 % of r↓, maybe a cap
        let mut rng = Rng(seed);
        let t = rng.pick(&grid());
        let l = Limits::from(&t);
        let w = rng.uniform(50.0, 800.0);
        let lo = if rng.next_f64() < 0.5 { 0.0 } else { rng.uniform(0.0, w) };
        let s = rng.uniform(0.2, 1.0) * l.r_dn;
        let pre = if rng.next_f64() < 0.5 { steady(w, 10.0) } else { ramp(w * 0.6, w, rng.uniform(5.0, 15.0)) };
        let blocks = vec![pre, ramp(w, lo, (w - lo) / s), steady(lo, 20.0)];
        let v = i64::from(pre.ticks);
        let k0 = (v - 30).max(1) + range_i(&mut rng, v + i64::from(blocks[1].ticks) + 8 - (v - 30).max(1));
        let len = rng.pick(&[1, 2, 3, 4, 6, 8, 12, 20, 40]);
        let m0 = if rng.next_f64() < 0.5 { 2000.0 } else { rng.uniform(lo + 1.0, w * 1.2) };
        checked(&Plan::new(t, blocks).cap(m0).faults(Faults::on(k0..k0 + len)), "planned fall")?;
    }

    #[test]
    fn cap_events_during_refusals_keep_every_property(seed in any::<u64>()) {
        // the model's cap_event_plan (round 13's falsifier)
        let mut rng = Rng(seed);
        let t = rng.pick(&grid());
        let lo = rng.uniform(50.0, 300.0);
        let hi = lo + rng.uniform(50.0, 400.0);
        let ke = 41 + range_i(&mut rng, 54);
        let cap = rng.uniform(lo - 20.0, hi);
        let k0 = ke - 8 + range_i(&mut rng, 11);
        let len = rng.pick(&[1, 2, 3, 4, 6, 8, 12, 20, 40]);
        let p = Plan::new(t, vec![steady(lo, 10.0), steady(hi, 15.0), steady(hi, 30.0)])
            .events(vec![ev(ke, Kind::Cap, cap)])
            .faults(Faults::on(k0..k0 + len));
        checked(&p, "cap event during refusals")?;
    }

    #[test]
    fn the_line_is_the_same_whichever_tick_asks_and_canonical(seed in any::<u64>()) {
        let mut rng = Rng(seed);
        let t = rng.pick(&grid());
        let (blocks, _) = random_plan(&mut rng, &t);
        let mult = rng.pick(&[1.0, 1.0, 0.95, 1.1]);
        let m = if rng.next_f64() < 0.5 { 1e9 } else { rng.uniform(150.0, 700.0) };
        let full = segments(&blocks, 0, mult, m);
        let n = total(&blocks);
        for k in (0..n).step_by(usize::try_from((n / 60).max(1)).expect("small")) {
            let want: Vec<_> = full.iter().copied().filter(|p| p.end() > k).collect();
            prop_assert_eq!(segments(&blocks, k, mult, m), want, "tick {}", k);
        }
        for w in full.windows(2) {
            let (p, q) = (w[0], w[1]);
            prop_assert!(
                !(q.joined && q.k0 == p.end() && collinear(p.s, q.s) && (p.at(q.k0) - q.r0).abs() <= VALUE_TOL),
                "two adjacent collinear joined pieces at tick {}", q.k0
            );
        }
    }

    #[test]
    fn a_step_onto_a_held_line_from_rest_lands_within_p4_and_p7(seed in any::<u64>()) {
        // the model's `held`: a rise within Δ/r↑ + τ (+0.75 s), a fall at or below the line within
        // Δ/r↓ + τ(1 + f)/2 (+0.75 s)
        let mut rng = Rng(seed);
        let t = rng.pick(&grid());
        let l = Limits::from(&t);
        let up = rng.next_f64() < 0.5;
        let delta = rng.uniform(1.0, 600.0);
        let a = if up { rng.uniform(0.0, 600.0) } else { rng.uniform(delta, 1200.0) };
        let b = if up { a + delta } else { a - delta };
        let bound = if up { delta / l.r_up + t.ease_s } else { delta / l.r_dn + t.ease_s * (1.0 + t.fall_factor) / 2.0 };
        let (tr, _) = checked(&Plan::new(t, vec![steady(b, bound + 30.0)]).y0(a), "held")?;
        // up: after the last tick off the line (the model's land_time); down: the first tick at or
        // below it, counted to the end of that tick
        let k_end = if up {
            tr.on.iter().rposition(|o| !*o).map_or(0, |k| k + 1)
        } else {
            tr.y.iter().zip(&tr.r).position(|(y, r)| *y <= r + EPS).expect("reaches the line") + 1
        };
        prop_assert!(k_end < tr.y.len(), "never landed");
        #[expect(clippy::cast_precision_loss, reason = "small")]
        let secs = k_end as f64 * DT;
        prop_assert!(secs <= bound + 0.75, "{} s > {} + 0.75 ({:?} {} -> {})", secs, bound, t, a, b);
    }
}

// Open design findings of #620 (PR body), pinned by the seeds the properties above drew. Both
// reproduce in the model (`docs/adr/models/erg_slew.py`, upstream Ruckig) on the same input.

#[test]
fn finding_1_a_twin_answered_trajectory_keeps_the_cut_domains() {
    // 20 % of first calculations refused, the twin answering: at (50, ×3, 1) the twin's brake onto
    // 0 W (its limits 2⁻²⁰ tighter than the probes') ends 1.6e-6 W below 0 W, past ROUND_SNAP: X4
    // at tick 404 with no event pending. The model, replaying this refusal pattern, cuts the same.
    let seed = 1_996_470_693_720_483_841;
    let (p, _) = plan_from(seed);
    let p = p.faults(Faults {
        share: 0.2,
        seed,
        twin_answers: true,
        ..Faults::default()
    });
    let tr = run(&p);
    let mut rep = Report::default();
    check(&tr, &p.tuning, &mut rep, "finding 1");
    rep.assert_clean();
}

#[test]
fn finding_2_a_fault_free_plan_is_never_degraded() {
    // No fault injected, (10, ×4, 1.5): at tick 20, a jump up of 7.5 W while the command falls at
    // −31 W/s; Ruckig's answer and the twin's are both slower than landing_bound, so the easer is
    // degraded (erg_easer_degraded raised) in fault-free riding. The model degrades the same tick.
    let (p, _) = plan_from(4_795_051_404_215_482_607);
    let tr = run(&p);
    assert!(
        !tr.degraded.iter().any(|x| *x),
        "degraded without a fault at tick {:?}",
        tr.degraded.iter().position(|x| *x)
    );
}
