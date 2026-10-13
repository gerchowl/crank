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
fn finite_number() -> BoxedStrategy<f64> {
    prop_oneof![
        Just(0.),
        Just(-0.),
        Just(f64::MAX),
        Just(f64::MIN),
        (-1000i32..1000).prop_map(f64::from),
        any::<f64>().prop_filter("finite numeric input", |n| n.is_finite())
    ]
    .boxed()
}
fn adversarial_tree(finite_only: bool) -> impl Strategy<Value = Vec<WorkoutNode>> {
    let number = if finite_only {
        finite_number()
    } else {
        any::<f64>().boxed()
    };
    let target = (0u8..6, number.clone(), number.clone()).prop_map(|(kind, a, b)| match kind {
        0 => PowerTarget::PctFtp { value: a },
        1 => PowerTarget::Watts { value: a },
        2 => PowerTarget::Range { lo: a, hi: b },
        3 => PowerTarget::Ramp { from: a, to: b },
        4 => PowerTarget::Hr { bpm: a },
        _ => PowerTarget::Free,
    });
    let end =
        (0u8..4, number.clone(), number.clone(), number.clone()).prop_map(|(kind, a, b, c)| {
            match kind {
                0 => StepEnd::Time { s: a },
                1 => StepEnd::Kj { kj: a },
                2 => StepEnd::Lap,
                _ => StepEnd::HrRecovery {
                    target_bpm: a,
                    min_s: b,
                    max_s: c,
                    on_unreachable: OnUnreachable::Cap,
                },
            }
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
        leaf.prop_recursive(4, 32, 3, move |inner| {
            (
                proptest::collection::vec(inner, 0..4),
                number.clone(),
                number.clone(),
                number.clone(),
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
    fn adversarial_numbers_are_values(input in prop_oneof![(adversarial_tree(true),finite_number(),finite_number(),finite_number(),finite_number()),(adversarial_tree(false),any::<f64>(),any::<f64>(),any::<f64>(),any::<f64>())]) {
        let (nodes,ftp,tau,rest,max)=input;
        let w=workout(nodes,ftp);
        let options=CompileOptions { usual_tau_s: Some(tau), athlete:Some(Athlete { rest_hr:rest,max_hr:max,..Athlete::default() }) };
        let finite_input=ftp.is_finite() && tau.is_finite() && rest.is_finite() && max.is_finite() && nodes_are_finite(&w.nodes);
        let result=compile_workout(&w,&options);
        if let Err(error)=&result {
            match error {
                CompileError::NonFinite => prop_assert!(!finite_input, "finite input rejected as NonFinite: {:?}", w),
                CompileError::InvalidArithmetic | CompileError::NestingTooDeep | CompileError::ExpansionLimit | CompileError::TraceLimit | CompileError::TimelinePrecision => {}
            }
        }
        if let Ok(c)=result {
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
        Err(CompileError::InvalidArithmetic)
    );
    let mut node = step(1., 100.);
    for _ in 0..MAX_NESTING_DEPTH {
        node = repeat(vec![node], 1.);
    }
    assert!(compile_workout(&workout(vec![node], 280.), &CompileOptions::default()).is_ok());
}

// Captured from frozen shared/src/workout/compile.ts with Vitest, 2026-10-13.
// Additional review cases live here; the frozen vector files remain unchanged.
const REVIEW_REFERENCE: &str = r#"{
  "schemaVersion":1,
  "cases":[
    {"name":"ftp-0-watts-200","input":{"workout":{"id":"w","name":"test","ftp":0,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"watts","value":200}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":200,"powerTo":200,"phase":"work"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"},{"severity":"error","nodeId":"s","message":"Target 200 W exceeds max 0 W"}]}},
    {"name":"ftp-0-watts-0","input":{"workout":{"id":"w","name":"test","ftp":0,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"watts","value":0}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"recovery"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"}]}},
    {"name":"ftp-0-hr-120","input":{"workout":{"id":"w","name":"test","ftp":0,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"hr","bpm":120}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"recovery"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"}]}},
    {"name":"ftp--280-watts-200","input":{"workout":{"id":"w","name":"test","ftp":-280,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"watts","value":200}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":200,"powerTo":200,"phase":"recovery"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"},{"severity":"error","nodeId":"s","message":"Target 200 W exceeds max -1120 W"},{"severity":"error","nodeId":"s","message":"W'bal drops to -44 % — not sustainable as prescribed"}]}},
    {"name":"ftp--280-watts-0","input":{"workout":{"id":"w","name":"test","ftp":-280,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"watts","value":0}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"work"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"},{"severity":"error","nodeId":"s","message":"Target 0 W exceeds max -1120 W"}]}},
    {"name":"ftp--280-hr-120","input":{"workout":{"id":"w","name":"test","ftp":-280,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"hr","bpm":120}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"work"}],"totalS":60,"totalRangeS":[60,60],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"},{"severity":"error","nodeId":"s","message":"Target 0 W exceeds max -1120 W"}]}},
    {"name":"mixed-nan-percentage","input":{"workout":{"id":"w","name":"test","ftp":0,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":60},"target":{"kind":"watts","value":0}},{"type":"step","id":"s2","end":{"kind":"time","s":60},"target":{"kind":"pct_ftp","value":50}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"recovery"},{"path":[1],"iteration":[],"nodeId":"s2","startS":60,"durS":60,"powerFrom":0,"powerTo":0,"phase":"recovery"}],"totalS":120,"totalRangeS":[120,120],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,120],"issues":[{"severity":"error","message":"FTP must be positive"}]}},
    {"name":"negative-zero-0","input":{"workout":{"id":"w","name":"test","ftp":280,"nodes":[{"type":"repeat","id":"r","count":0,"children":[]}]},"options":{}},"output":{"blocks":[],"totalS":0,"totalRangeS":[0,0],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,0],"issues":[{"severity":"error","nodeId":"r","message":"Repeat count must be a whole number ≥ 1 (got 0)"}]}},
    {"name":"negative-zero-1","input":{"workout":{"id":"w","name":"test","ftp":280,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":1},"target":{"kind":"watts","value":20280.1}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":1,"powerFrom":20280.1,"powerTo":20280.1,"phase":"work"}],"totalS":1,"totalRangeS":[1,1],"tss":145.7,"if":72.429,"timeInZoneS":[0,0,0,0,0,0,1],"issues":[{"severity":"error","nodeId":"s","message":"Target 20280 W exceeds max 1120 W"},{"severity":"error","nodeId":"s","message":"W'bal drops to 0 % — not sustainable as prescribed"}]}},
    {"name":"zero-ftp-initial-hr-recovery","input":{"workout":{"id":"w","name":"test","ftp":0,"nodes":[{"type":"step","id":"s","end":{"kind":"hr_recovery","targetBpm":120,"minS":60,"maxS":240,"onUnreachable":"cap"},"target":{"kind":"pct_ftp","value":40}}]},"options":{}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":60,"powerFrom":0,"powerTo":0,"phase":"recovery","durRangeS":[60,240]}],"totalS":60,"totalRangeS":[60,240],"tss":0,"if":0,"timeInZoneS":[0,0,0,0,0,0,60],"issues":[{"severity":"error","message":"FTP must be positive"}]}},
    {"name":"infinite-estimate-finite-cap","input":{"workout":{"id":"w","name":"test","ftp":280,"nodes":[{"type":"step","id":"s","end":{"kind":"time","s":300},"target":{"kind":"pct_ftp","value":110}},{"type":"step","id":"r","end":{"kind":"hr_recovery","targetBpm":120,"minS":60,"maxS":240,"onUnreachable":"cap"},"target":{"kind":"pct_ftp","value":40}}]},"options":{"usualTauS":1.7976931348623157e+308}},"output":{"blocks":[{"path":[0],"iteration":[],"nodeId":"s","startS":0,"durS":300,"powerFrom":308,"powerTo":308,"phase":"work"},{"path":[1],"iteration":[],"nodeId":"r","startS":300,"durS":240,"powerFrom":112,"powerTo":112,"phase":"recovery","durRangeS":[60,240]}],"totalS":540,"totalRangeS":[360,540],"tss":13.5,"if":0.949,"timeInZoneS":[240,0,0,0,300,0,0],"issues":[]}}
  ]
}"#;
fn assert_review_cases(indices: &[usize]) {
    let expected: serde_json::Value =
        serde_json::from_str(REVIEW_REFERENCE).expect("valid finite TS review case");
    let mut actual = expected.clone();
    for &index in indices {
        let input = &expected["cases"][index]["input"];
        let mut w: Workout =
            serde_json::from_value(input["workout"].clone()).expect("valid finite TS review case");
        if index == 7 {
            // JSON.stringify normalizes -0; supply its original sign to Rust.
            if let WorkoutNode::Repeat(r) = &mut w.nodes[0] {
                r.count = -0.;
            }
        }
        let options = serde_json::from_value(input["options"].clone()).expect("valid TS options");
        let compiled = compile_workout(&w, &options).expect("valid finite TS review case");
        if w.ftp <= 0. {
            assert_eq!(compiled.issues[0].kind, ValidationKind::InvalidFtp);
        }
        let mut value = serde_json::to_value(compiled).expect("valid finite TS review case");
        for issue in value["issues"]
            .as_array_mut()
            .expect("valid finite TS review case")
        {
            issue
                .as_object_mut()
                .expect("valid finite TS review case")
                .remove("kind");
        }
        actual["cases"][index]["output"] = value;
    }
    common::assert_vectors("compile", &actual, &expected);
}
#[test]
fn invalid_ftp_matches_ts_reference() {
    assert_review_cases(&[0, 1, 2, 3, 4, 5, 6, 9, 10]);
}
#[test]
fn negative_zero_repeat_message_matches_ts() {
    assert_review_cases(&[7]);
}
#[test]
fn negative_zero_reserve_message_matches_ts() {
    assert_review_cases(&[8]);
}
#[test]
fn finite_arithmetic_failure_is_not_nonfinite_input() {
    let error = compile_workout(
        &workout(vec![step(1., f64::MAX)], 280.),
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert_ne!(error, CompileError::NonFinite);
}

fn nodes_are_finite(nodes: &[WorkoutNode]) -> bool {
    nodes.iter().all(|node| match node {
        WorkoutNode::Repeat(r) => {
            r.count.is_finite()
                && r.progression.as_ref().is_none_or(|p| {
                    p.watts_per_rep.is_none_or(f64::is_finite)
                        && p.pct_per_rep.is_none_or(f64::is_finite)
                })
                && nodes_are_finite(&r.children)
        }
        WorkoutNode::Step(s) => {
            let target = match s.target {
                PowerTarget::PctFtp { value } | PowerTarget::Watts { value } => value.is_finite(),
                PowerTarget::Range { lo, hi } => lo.is_finite() && hi.is_finite(),
                PowerTarget::Ramp { from, to } => from.is_finite() && to.is_finite(),
                PowerTarget::Hr { bpm } => bpm.is_finite(),
                PowerTarget::Free => true,
            };
            let end = match s.end {
                StepEnd::Time { s } => s.is_finite(),
                StepEnd::Kj { kj } => kj.is_finite(),
                StepEnd::Lap => true,
                StepEnd::HrRecovery {
                    target_bpm,
                    min_s,
                    max_s,
                    ..
                } => target_bpm.is_finite() && min_s.is_finite() && max_s.is_finite(),
            };
            target && end
        }
    })
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn finite_invalid_ftp_is_a_diagnostic(ftp in -500i32..=0, watts in 0u32..1000, hr_target in any::<bool>()) {
        let target=if hr_target { PowerTarget::Hr { bpm:120. } } else { PowerTarget::Watts { value:f64::from(watts) } };
        // Include zero in every case so this class cannot miss its boundary.
        for ftp in [f64::from(ftp),0.] {
            let w=workout(vec![WorkoutNode::Step(StepNode { id:"s".into(),label:None,cadence:None,end:StepEnd::Time { s:60. },target:target.clone() })],ftp);
            let compiled=compile_workout(&w,&CompileOptions::default()).unwrap();
            prop_assert_eq!(compiled.issues[0].kind,ValidationKind::InvalidFtp);
            prop_assert!(compiled.total_s.is_finite());
        }
    }
}
