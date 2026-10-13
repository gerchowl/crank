//! Compiler golden parity, timeline tiling and bounded adversarial inputs.
mod common;
use crank_domain::workout::*;
use proptest::prelude::*;
use serde_json::json;

#[test]
fn compile_vectors() {
    let expected = common::load("compile");
    let mut actual = expected.clone();
    for case in actual["cases"].as_array_mut().unwrap() {
        let input = &case["input"];
        case["output"] = if let Some(power) = input.get("power") {
            json!({"np": normalized_power(&common::numbers(power)).unwrap()})
        } else {
            let workout = serde_json::from_value(input["workout"].clone()).unwrap();
            let options = serde_json::from_value(input["options"].clone()).unwrap();
            let mut value =
                serde_json::to_value(compile_workout(&workout, &options).unwrap()).unwrap();
            // The Rust contract adds machine-readable validation kinds; the frozen
            // reference only contains their presentation messages.
            for issue in value["issues"].as_array_mut().unwrap() {
                issue.as_object_mut().unwrap().remove("kind");
            }
            value
        };
    }
    common::assert_vectors("compile", &actual, &expected);
}

fn workout(nodes: Vec<WorkoutNode>, ftp: f64) -> Workout {
    Workout {
        id: "w".into(),
        name: "test".into(),
        ftp,
        nodes,
    }
}
fn step(s: f64, value: f64) -> WorkoutNode {
    WorkoutNode::Step(StepNode {
        id: "s".into(),
        label: None,
        end: StepEnd::Time { s },
        target: PowerTarget::PctFtp { value },
        cadence: None,
    })
}
fn repeat(children: Vec<WorkoutNode>, count: f64) -> WorkoutNode {
    WorkoutNode::Repeat(RepeatNode {
        id: "r".into(),
        label: None,
        count,
        children,
        progression: Some(Progression {
            watts_per_rep: Some(3.0),
            pct_per_rep: Some(-1.0),
        }),
        skip_last_rest: Some(true),
    })
}
fn valid_tree() -> impl Strategy<Value = Vec<WorkoutNode>> {
    let target = prop_oneof![
        (0u32..180).prop_map(|v| PowerTarget::PctFtp {
            value: f64::from(v)
        }),
        (0u32..600).prop_map(|v| PowerTarget::Watts {
            value: f64::from(v)
        }),
        (0u32..100, 100u32..180).prop_map(|(lo, hi)| PowerTarget::Range {
            lo: f64::from(lo),
            hi: f64::from(hi)
        }),
        (0u32..180, 0u32..180).prop_map(|(from, to)| PowerTarget::Ramp {
            from: f64::from(from),
            to: f64::from(to)
        }),
        (90u32..180).prop_map(|v| PowerTarget::Hr { bpm: f64::from(v) }),
        Just(PowerTarget::Free),
    ];
    let end = prop_oneof![
        (1u32..30).prop_map(|s| StepEnd::Time {
            s: f64::from(s) / 3.
        }),
        (1u32..10).prop_map(|kj| StepEnd::Kj {
            kj: f64::from(kj) / 10.
        }),
        Just(StepEnd::Lap),
        (90u32..150, 1u32..10, 10u32..30).prop_map(|(bpm, min, max)| StepEnd::HrRecovery {
            target_bpm: f64::from(bpm),
            min_s: f64::from(min),
            max_s: f64::from(max),
            on_unreachable: OnUnreachable::Cap
        }),
    ];
    let leaf = (target, end).prop_map(|(target, end)| {
        WorkoutNode::Step(StepNode {
            target,
            end,
            id: "s".into(),
            label: None,
            cadence: None,
        })
    });
    proptest::collection::vec(
        leaf.prop_recursive(3, 32, 3, |inner| {
            (proptest::collection::vec(inner, 1..4), 1u32..4)
                .prop_map(|(nodes, count)| repeat(nodes, f64::from(count)))
        }),
        0..5,
    )
}
fn adversarial_tree() -> impl Strategy<Value = Vec<WorkoutNode>> {
    let target = (0u8..6, any::<f64>(), any::<f64>()).prop_map(|(kind, a, b)| match kind {
        0 => PowerTarget::PctFtp { value: a },
        1 => PowerTarget::Watts { value: a },
        2 => PowerTarget::Range { lo: a, hi: b },
        3 => PowerTarget::Ramp { from: a, to: b },
        4 => PowerTarget::Hr { bpm: a },
        _ => PowerTarget::Free,
    });
    let end =
        (0u8..4, any::<f64>(), any::<f64>(), any::<f64>()).prop_map(|(kind, a, b, c)| match kind {
            0 => StepEnd::Time { s: a },
            1 => StepEnd::Kj { kj: a },
            2 => StepEnd::Lap,
            _ => StepEnd::HrRecovery {
                target_bpm: a,
                min_s: b,
                max_s: c,
                on_unreachable: OnUnreachable::Cap,
            },
        });
    let leaf = (target, end).prop_map(|(target, end)| {
        WorkoutNode::Step(StepNode {
            id: "s".into(),
            label: None,
            cadence: None,
            target,
            end,
        })
    });
    proptest::collection::vec(
        leaf.prop_recursive(4, 32, 3, |inner| {
            (
                proptest::collection::vec(inner, 0..4),
                any::<f64>(),
                any::<f64>(),
                any::<f64>(),
            )
                .prop_map(|(nodes, count, watts, pct)| {
                    let mut node = repeat(nodes, count);
                    if let WorkoutNode::Repeat(r) = &mut node {
                        r.progression = Some(Progression {
                            watts_per_rep: Some(watts),
                            pct_per_rep: Some(pct),
                        });
                    }
                    node
                })
        }),
        0..4,
    )
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn blocks_tile(nodes in valid_tree(), ftp in 50u32..500) {
        let compiled = compile_workout(&workout(nodes,f64::from(ftp)), &CompileOptions::default()).unwrap();
        let mut end = 0_f64;
        for b in &compiled.blocks {
            prop_assert_eq!(b.start_s.to_bits(),end.to_bits());
            prop_assert!(b.dur_s > 0.);
            end = b.start_s+b.dur_s;
            prop_assert!(end > b.start_s);
        }
        prop_assert_eq!(end.to_bits(),compiled.total_s.to_bits());
    }
    #[test]
    fn adversarial_numbers_are_values(nodes in adversarial_tree(), ftp in prop_oneof![Just(280.),any::<f64>()], tau in prop_oneof![Just(55.),any::<f64>()], rest in prop_oneof![Just(50.),any::<f64>()], max in prop_oneof![Just(190.),any::<f64>()]) {
        let w=workout(nodes,ftp);
        let options=CompileOptions { usual_tau_s: Some(tau), athlete:Some(Athlete { rest_hr:rest,max_hr:max,..Athlete::default() }) };
        if let Ok(c)=compile_workout(&w,&options) {
            prop_assert!(c.total_s.is_finite());
            for b in c.blocks { prop_assert!(b.dur_s.is_finite() && b.power_from.is_finite() && b.power_to.is_finite()); }
        }
    }
}
#[test]
fn bounded_expansion_and_depth() {
    let opts = CompileOptions::default();
    assert_eq!(
        compile_workout(
            &workout(vec![repeat(vec![step(1., 100.)], 1e100)], 280.),
            &opts
        ),
        Err(CompileError::ExpansionLimit)
    );
    assert_eq!(
        compile_workout(&workout(vec![step(1e100, 100.)], 280.), &opts),
        Err(CompileError::TraceLimit)
    );
    let mut node = step(1., 100.);
    for _ in 0..=MAX_NESTING_DEPTH {
        node = repeat(vec![node], 1.);
    }
    assert_eq!(
        compile_workout(&workout(vec![node], 280.), &opts),
        Err(CompileError::NestingTooDeep)
    );
}

#[test]
fn explicit_nonfinite_and_helper_edges() {
    let a = Athlete::default();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            compile_workout(
                &workout(vec![step(value, 100.)], 280.),
                &CompileOptions::default()
            ),
            Err(CompileError::NonFinite)
        );
        assert_eq!(
            target_watts(&PowerTarget::Watts { value }, 280., Offset::default(), &a),
            Err(CompileError::NonFinite)
        );
        assert_eq!(normalized_power(&[value]), Err(CompileError::NonFinite));
    }
    let off = Offset { watts: 5., pct: 2. };
    assert_eq!(
        target_watts(&PowerTarget::PctFtp { value: 100. }, 280., off, &a)
            .unwrap()
            .map(f64::to_bits),
        [290.6_f64, 290.6].map(f64::to_bits)
    );
    assert_eq!(
        target_watts(&PowerTarget::Free, 280., off, &a)
            .unwrap()
            .map(f64::to_bits),
        [140_f64, 140.].map(f64::to_bits)
    );
    assert_eq!(
        target_pct(
            &PowerTarget::Ramp {
                from: 110.,
                to: 90.
            },
            280.,
            &a
        )
        .unwrap()
        .to_bits(),
        110_f64.to_bits()
    );
    assert!(
        is_work_step(
            &StepNode {
                id: "s".into(),
                label: None,
                cadence: None,
                end: StepEnd::Time { s: 1. },
                target: PowerTarget::PctFtp { value: 80. }
            },
            280.
        )
        .unwrap()
    );
    let bad = Athlete {
        rest_hr: 171. - 12.,
        ..a
    };
    assert_eq!(
        target_watts(&PowerTarget::Hr { bpm: 171. }, 280., off, &bad),
        Err(CompileError::NonFinite)
    );
    let mut node = step(1., 100.);
    for _ in 0..MAX_NESTING_DEPTH {
        node = repeat(vec![node], 1.);
    }
    assert!(compile_workout(&workout(vec![node], 280.), &CompileOptions::default()).is_ok());
}
