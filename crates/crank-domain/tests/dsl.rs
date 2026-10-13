//! Frozen TypeScript DSL parity and the reference tests (ADR-0001 §3).
use crank_domain::workout::{
    Cadence, OnUnreachable, PowerTarget, Progression, RepeatNode, StepEnd, StepNode, WorkoutNode,
    format_duration, parse_dsl, parse_duration_word, print_dsl,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::collections::HashSet;

fn without_ids(nodes: &[WorkoutNode]) -> Value {
    fn strip(value: &mut Value) {
        for node in value.as_array_mut().expect("workout node array") {
            node.as_object_mut()
                .expect("workout node object")
                .remove("id");
            if let Some(children) = node.get_mut("children") {
                strip(children);
            }
        }
    }
    let mut value = serde_json::to_value(nodes).expect("serializable workout");
    strip(&mut value);
    value
}

// The frozen mock omits optional properties; ADR-0001 §4 requires null in the
// Rust contract. Normalize ONLY the schema's optional fields on the reference.
fn contract_options(nodes: &mut Value) {
    for node in nodes.as_array_mut().expect("workout node array") {
        let repeat = node["type"] == "repeat";
        let obj = node.as_object_mut().expect("workout node object");
        obj.entry("label").or_insert(Value::Null);
        if repeat {
            obj.entry("skipLastRest").or_insert(Value::Null);
            let prog = obj.entry("progression").or_insert(Value::Null);
            if let Some(prog) = prog.as_object_mut() {
                prog.entry("wattsPerRep").or_insert(Value::Null);
                prog.entry("pctPerRep").or_insert(Value::Null);
            }
            contract_options(obj.get_mut("children").expect("repeat children"));
        } else {
            obj.entry("cadence").or_insert(Value::Null);
        }
    }
}

// #74: classify by DSL schema, not JSON number spelling or fract()==0.
// Decimal-capable fields use ADR-0001 §3's tolerance EVEN at integer values;
// count, durations, watts, HR, cadence and error spans are exact integers.
fn parity(a: &Value, b: &Value, path: &str, float: bool) -> Result<(), String> {
    let mismatch = || Err(format!("{path}: {a} != {b}"));
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            if !float {
                // Converting both integers to f64 would equate adjacent values
                // above 2^53. Only canonicalize floats from the Rust schema.
                let exact = |n: &serde_json::Number| {
                    if n.is_f64() {
                        (n.as_f64().expect("finite numeric fixture") + 0.0).to_string()
                    } else {
                        n.to_string()
                    }
                };
                return if exact(a) == exact(b) {
                    Ok(())
                } else {
                    mismatch()
                };
            }
            let (a, b) = (
                a.as_f64().expect("finite numeric fixture"),
                b.as_f64().expect("finite numeric fixture"),
            );
            let tolerance = (1e-12 * a.abs().max(b.abs())).max(1e-15);
            if (a - b).abs() <= tolerance {
                Ok(())
            } else {
                mismatch()
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                parity(a, b, &format!("{path}[{i}]"), false)?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b)) if a.keys().eq(b.keys()) => {
            let kind = b.get("kind").and_then(Value::as_str);
            for (key, a) in a {
                let float = matches!(
                    (kind, key.as_str()),
                    (Some("pct_ftp"), "value")
                        | (Some("range"), "lo" | "hi")
                        | (Some("ramp"), "from" | "to")
                        | (Some("kj"), "kj")
                        | (None, "wattsPerRep" | "pctPerRep")
                );
                parity(a, &b[key], &format!("{path}.{key}"), float)?;
            }
            Ok(())
        }
        _ if a == b => Ok(()),
        _ => mismatch(),
    }
}

#[test]
fn frozen_vectors() {
    let vectors: Value = serde_json::from_str(include_str!("vectors/dsl.json")).unwrap();
    assert_eq!(vectors["schemaVersion"], 1);
    for case in vectors["cases"].as_array().unwrap() {
        let input = &case["input"];
        let result = parse_dsl(
            input["text"].as_str().unwrap(),
            input["ftp"].as_f64().expect("finite numeric fixture"),
        );
        let context = case["name"].as_str().unwrap();
        if let Some(error) = case["output"].get("error") {
            parity(
                &serde_json::to_value(result.unwrap_err()).unwrap(),
                error,
                context,
                false,
            )
            .unwrap();
        } else {
            let nodes = result.unwrap();
            let mut expected = case["output"]["nodes"].clone();
            contract_options(&mut expected);
            parity(&without_ids(&nodes), &expected, context, false).unwrap();
            let text = print_dsl(&nodes);
            assert_eq!(
                text,
                case["output"]["printed"].as_str().unwrap(),
                "{context}"
            );
            let back = parse_dsl(
                &text,
                input["ftp"].as_f64().expect("finite numeric fixture"),
            )
            .unwrap();
            assert_eq!(without_ids(&back), without_ids(&nodes), "{context}");
            assert_eq!(print_dsl(&back), text, "{context}");
            assert_eq!(
                serde_json::from_value::<Vec<WorkoutNode>>(serde_json::to_value(&nodes).unwrap())
                    .unwrap(),
                nodes
            );
        }
    }
}

#[test]
fn comparator_uses_schema_and_preserves_structure() {
    assert!(
        parity(
            &json!({"kind":"pct_ftp","value":1.0+5e-13}),
            &json!({"kind":"pct_ftp","value":1}),
            "$",
            false
        )
        .is_ok()
    );
    assert!(
        parity(
            &json!({"kind":"watts","value":1.0+5e-13}),
            &json!({"kind":"watts","value":1}),
            "$",
            false
        )
        .is_err()
    );
    assert!(
        parity(
            &json!({"kind":"kj","kj":5e-16}),
            &json!({"kind":"kj","kj":0}),
            "$",
            false
        )
        .is_ok()
    );
    assert!(
        parity(
            &json!({"kind":"kj","kj":2e-15}),
            &json!({"kind":"kj","kj":0}),
            "$",
            false
        )
        .is_err()
    );
    for (a, b) in [
        (json!(1.01), json!(1)),
        (
            json!(9_007_199_254_740_992u64),
            json!(9_007_199_254_740_993u64),
        ),
        (json!([1]), json!([])),
        (json!({"label":null}), json!({})),
        (json!("a"), json!("b")),
    ] {
        assert!(parity(&a, &b, "$", false).is_err());
    }
    assert!(
        parity(
            &json!({"kind":"pct_ftp","value":1.0+2e-12}),
            &json!({"kind":"pct_ftp","value":1}),
            "$",
            false
        )
        .unwrap_err()
        .contains("$.value")
    );
}

#[test]
fn ids_are_fresh_and_unique_and_separators_equivalent() {
    fn ids(nodes: &[WorkoutNode], out: &mut HashSet<String>) {
        for n in nodes {
            match n {
                WorkoutNode::Step(n) => assert!(out.insert(n.id.clone())),
                WorkoutNode::Repeat(n) => {
                    assert!(out.insert(n.id.clone()));
                    ids(&n.children, out);
                }
            }
        }
    }
    let mut all = HashSet::new();
    for _ in 0..2 {
        ids(&parse_dsl("2x(1m 100%, 1m 50%)", 280.0).unwrap(), &mut all);
    }
    assert_eq!(all.len(), 6);
    let a = parse_dsl("# warm up\n10m 50%\n\n3x(1m 120%;1m 50%) # sets\n", 280.0).unwrap();
    let b = parse_dsl("10m 50%, 3x(1m 120%, 1m 50%)", 280.0).unwrap();
    assert_eq!(without_ids(&a), without_ids(&b));
}

fn label() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(prop_oneof![
        Just("say \"hi\"".into()),
        Just("back\\slash".into()),
        Just("🚲, max; (hard)".into()),
        "[a-zA-Z0-9 ]{0,20}"
    ])
}
fn target() -> impl Strategy<Value = PowerTarget> {
    prop_oneof![
        (0..=2000u32).prop_map(|v| PowerTarget::PctFtp {
            value: f64::from(v) / 10.0
        }),
        (0..=900u32).prop_map(|v| PowerTarget::Watts {
            value: f64::from(v)
        }),
        (30..=120u32, 0..=30u32).prop_map(|(lo, d)| PowerTarget::Range {
            lo: f64::from(lo),
            hi: f64::from(lo + d)
        }),
        (30..=150u32, 30..=150u32).prop_map(|(from, to)| PowerTarget::Ramp {
            from: f64::from(from),
            to: f64::from(to)
        }),
        (100..=180u32).prop_map(|bpm| PowerTarget::Hr {
            bpm: f64::from(bpm)
        }),
        Just(PowerTarget::Free),
    ]
}
fn end() -> impl Strategy<Value = StepEnd> {
    prop_oneof![
        (1..=7200u32).prop_map(|s| StepEnd::Time { s: f64::from(s) }),
        (1..=300u32).prop_map(|kj| StepEnd::Kj { kj: f64::from(kj) }),
        Just(StepEnd::Lap),
        (
            90..=150u32,
            0..=120u32,
            121..=600u32,
            prop_oneof![
                Just(OnUnreachable::LowerPower),
                Just(OnUnreachable::Cap),
                Just(OnUnreachable::AskCoach)
            ]
        )
            .prop_map(|(bpm, min, max, policy)| StepEnd::HrRecovery {
                target_bpm: f64::from(bpm),
                min_s: f64::from(min),
                max_s: f64::from(max),
                on_unreachable: policy
            }),
    ]
}
fn tree() -> impl Strategy<Value = Vec<WorkoutNode>> {
    let cadence = proptest::option::of((60..=100u32, 0..=20u32).prop_map(|(lo, d)| Cadence {
        lo: f64::from(lo),
        hi: f64::from(lo + d),
    }));
    let leaf = (end(), target(), label(), cadence).prop_map(|(end, target, label, cadence)| {
        WorkoutNode::Step(StepNode {
            id: "s".into(),
            end,
            target,
            label,
            cadence,
        })
    });
    let node = leaf.prop_recursive(3, 64, 4, |inner| {
        // Every generated progression has at least one field: empty options have
        // no DSL representation, as do false skipLastRest and fractional times.
        let progression = proptest::option::of(prop_oneof![
            (-100..=100i32).prop_map(|w| Progression {
                watts_per_rep: Some(f64::from(w) / 10.0),
                pct_per_rep: None
            }),
            (-300..=300i32).prop_map(|p| Progression {
                watts_per_rep: None,
                pct_per_rep: Some(f64::from(p) / 10.0)
            }),
            (-10..=10i32, -300..=300i32).prop_map(|(w, p)| Progression {
                watts_per_rep: Some(f64::from(w)),
                pct_per_rep: Some(f64::from(p) / 10.0)
            }),
        ]);
        (
            1..=12u32,
            proptest::collection::vec(inner, 1..=4),
            label(),
            progression,
            any::<bool>(),
        )
            .prop_map(|(count, children, label, progression, skip)| {
                WorkoutNode::Repeat(RepeatNode {
                    id: "r".into(),
                    count: f64::from(count),
                    children,
                    label,
                    progression,
                    skip_last_rest: skip.then_some(true),
                })
            })
    });
    proptest::collection::vec(node, 0..=4)
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]
    #[test]
    fn generated_workouts_round_trip(nodes in tree()) {
        let text = print_dsl(&nodes);
        let back = parse_dsl(&text,280.0).unwrap();
        prop_assert_eq!(without_ids(&back),without_ids(&nodes));
        prop_assert_eq!(print_dsl(&back),text);
    }
}

#[test]
fn durations_and_additional_error_edges() {
    for (word, s) in [("1H5M30S", 3930.0), ("0s", 0.0), ("90s", 90.0)] {
        assert_eq!(parse_duration_word(word), Some(s));
    }
    for word in ["", "1", "1.5m", "1s1m", "1h1h", "-1m", "1m2"] {
        assert_eq!(parse_duration_word(word), None);
    }
    for (s, word) in [
        (-1.0, "0s"),
        (0.49, "0s"),
        (0.499_999_999_999_999_94, "0s"),
        (0.5, "1s"),
        (3661.0, "1h1m1s"),
    ] {
        assert_eq!(format_duration(s), word);
    }
    assert_eq!(
        parse_dsl("1m 50% ramp 60-70%", 280.0).unwrap_err().offset,
        12
    );
    assert!(parse_dsl("1m 100000w", 0.0).is_ok());
    assert!(parse_dsl("1m 400%", 280.0).is_ok());
    assert!(parse_dsl("1m 400.001%", 280.0).is_err());
    assert!(parse_dsl("\u{feff}1m\u{a0}50%", 280.0).is_ok());
    assert_eq!(parse_dsl("1m\u{85}50%", 280.0).unwrap_err().offset, 0);
    assert_eq!(
        parse_dsl("\"🚲\" 1m 50%; \"é\"", 280.0).unwrap_err().offset,
        16
    );
}
