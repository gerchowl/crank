//! Frozen TypeScript DSL parity and the reference tests (ADR-0001 §3).
use crank_domain::workout::{
    Cadence, DslError, DslErrorKind as K, OnUnreachable, PowerTarget, Progression, RepeatNode,
    StepEnd, StepNode, WorkoutNode, format_duration, parse_dsl, parse_duration_word, print_dsl,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::collections::HashSet;

fn parse(text: &str, ftp: f64) -> Result<Vec<WorkoutNode>, DslError> {
    let mut next = 0;
    parse_dsl(text, ftp, &mut || {
        next += 1;
        format!("n:{next}")
    })
}

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

// Load #74's portable DSL schema: physical quantities are floats even when
// whole-valued; counts and UTF-16 spans are exact, integral integers.
fn schema() -> Value {
    serde_json::from_str(include_str!("vectors/schema.json")).expect("portable vector schema")
}
fn scalar_kind(shape: &Value) -> Option<&str> {
    shape
        .as_str()
        .or_else(|| shape["kind"].as_str())
        .or_else(|| shape["number"].as_str())
}
fn resolve<'a>(shape: &'a Value, defs: &'a Value, path: &str) -> Result<&'a Value, String> {
    if let Some(name) = shape.get("ref").and_then(Value::as_str) {
        defs.get(name)
            .ok_or_else(|| format!("{path}: unknown schema reference {name}"))
    } else if shape.is_null() {
        Err(format!("{path}: missing schema declaration"))
    } else {
        Ok(shape)
    }
}
fn validate(value: &Value, shape: &Value, defs: &Value, path: &str) -> Result<(), String> {
    let shape = resolve(shape, defs, path)?;
    match value {
        Value::Number(_) if !matches!(scalar_kind(shape), Some("float" | "integer")) => {
            return Err(format!("{path}: numeric classification missing"));
        }
        Value::Array(items) => {
            let item_shape = shape
                .get("items")
                .ok_or_else(|| format!("{path}: array schema missing"))?;
            for (i, item) in items.iter().enumerate() {
                validate(item, item_shape, defs, &format!("{path}[{i}]"))?;
            }
        }
        Value::Object(fields) => {
            let declarations = shape
                .get("fields")
                .ok_or_else(|| format!("{path}: object schema missing"))?;
            for (key, value) in fields {
                let field = declarations
                    .get(key)
                    .ok_or_else(|| format!("{path}.{key}: unclassified field"))?;
                validate(value, field, defs, &format!("{path}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn parity(a: &Value, b: &Value, shape: &Value, defs: &Value, path: &str) -> Result<(), String> {
    // Validate both COMPLETE documents first: an early mismatch must not hide
    // an unknown or unclassified field elsewhere (same rule as the TS loader).
    validate(a, shape, defs, path)?;
    validate(b, shape, defs, path)?;
    compare(a, b, shape, defs, path)
}
fn compare(a: &Value, b: &Value, shape: &Value, defs: &Value, path: &str) -> Result<(), String> {
    let shape = resolve(shape, defs, path)?;
    let mismatch = || Err(format!("{path}: {a} != {b}"));
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            if scalar_kind(shape) == Some("integer") {
                // Preserve adjacent JSON integers above 2^53. Canonicalize only
                // floats emitted by the Rust schema, requiring integrality.
                let exact = |n: &serde_json::Number| {
                    if n.is_f64() {
                        let value = n.as_f64().expect("finite numeric fixture");
                        (value.fract().abs() <= 0.0).then(|| (value + 0.0).to_string())
                    } else {
                        Some(n.to_string())
                    }
                };
                return match (exact(a), exact(b)) {
                    (Some(a), Some(b)) if a == b => Ok(()),
                    _ => mismatch(),
                };
            }
            let (a, b) = (
                a.as_f64().expect("finite numeric fixture"),
                b.as_f64().expect("finite numeric fixture"),
            );
            // ADR-0001 §3: rel 1e-12 / abs 1e-15, including whole-valued floats.
            if (a - b).abs() <= (1e-12 * a.abs().max(b.abs())).max(1e-15) {
                Ok(())
            } else {
                mismatch()
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &shape["items"], defs, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b))
            if a.len() == b.len() && a.keys().all(|key| b.contains_key(key)) =>
        {
            for (key, a) in a {
                compare(
                    a,
                    &b[key],
                    &shape["fields"][key],
                    defs,
                    &format!("{path}.{key}"),
                )?;
            }
            Ok(())
        }
        _ if a == b => Ok(()),
        _ => mismatch(),
    }
}

fn vector_error_kinds() -> [(&'static str, K); 32] {
    [
        ("dsl-13", K::UnclosedRepeat),
        ("dsl-14", K::UnexpectedToken),
        ("dsl-15", K::InvalidStepEnd),
        ("dsl-16", K::MissingPowerTarget),
        ("dsl-17", K::DuplicatePowerTarget),
        ("dsl-18", K::UnknownToken),
        ("dsl-19", K::UnknownRepeatOption),
        ("dsl-20", K::MissingRepeatBody),
        ("dsl-21", K::InvalidRepeatCount { count: 0.0 }),
        ("dsl-22", K::MissingRecoveryBounds),
        (
            "dsl-23",
            K::ReversedRecoveryBounds {
                min_s: 240.0,
                max_s: 120.0,
            },
        ),
        ("dsl-24", K::RecoveryFlagNotApplicable),
        ("dsl-25", K::ImplausiblePower { pct: 900.0 }),
        ("dsl-26", K::ReversedPowerRange),
        ("dsl-27", K::UnterminatedLabel),
        ("dsl-28", K::MissingUnit),
        ("dsl-29", K::UnknownToken),
        ("dsl-30", K::MissingItemAfterLabel),
        ("dsl-31", K::EmptyRepeat),
        ("dsl-32", K::NonPositiveDuration),
        ("dsl-33", K::MissingRampRange),
        ("dsl-34", K::MissingRampRange),
        ("dsl-35", K::DuplicateCadence),
        ("dsl-36", K::ReversedCadenceRange),
        ("dsl-37", K::UnclosedOptions),
        ("dsl-38", K::UnexpectedRepeatOption),
        ("dsl-39", K::UnexpectedToken),
        ("dsl-40", K::MissingSeparator),
        ("dsl-41", K::UnterminatedLabel),
        ("dsl-42", K::MissingRecoveryBounds),
        (
            "dsl-43",
            K::ImplausiblePower {
                pct: 1500.0 / 280.0 * 100.0,
            },
        ),
        ("dsl-44", K::ImplausiblePower { pct: 900.0 }),
    ]
}

#[test]
fn frozen_vectors() {
    let schema = schema();
    assert_eq!(schema["schemaVersion"], 2);
    let defs = &schema["definitions"];
    let output_shape = &schema["areas"]["dsl"]["fields"]["cases"]["items"]["fields"]["output"];
    let error_kinds = vector_error_kinds();
    let vectors: Value = serde_json::from_str(include_str!("vectors/dsl.json")).unwrap();
    assert_eq!(vectors["schemaVersion"], 1);
    validate(&vectors, &schema["areas"]["dsl"], defs, "$").unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let input = &case["input"];
        let result = parse(
            input["text"].as_str().unwrap(),
            input["ftp"].as_f64().expect("finite numeric fixture"),
        );
        let context = case["name"].as_str().unwrap();
        if let Some(error) = case["output"].get("error") {
            let actual = result.unwrap_err();
            let expected_kind = &error_kinds
                .iter()
                .find(|(name, _)| *name == context)
                .unwrap()
                .1;
            assert_eq!(&actual.kind, expected_kind, "{context}");
            let mut reference_fields = serde_json::to_value(&actual).unwrap();
            reference_fields.as_object_mut().unwrap().remove("kind");
            parity(
                &reference_fields,
                error,
                &output_shape["fields"]["error"],
                defs,
                context,
            )
            .unwrap();
        } else {
            let nodes = result.unwrap();
            let mut expected = case["output"]["nodes"].clone();
            contract_options(&mut expected);
            parity(
                &without_ids(&nodes),
                &expected,
                &output_shape["fields"]["nodes"],
                defs,
                context,
            )
            .unwrap();
            let text = print_dsl(&nodes);
            assert_eq!(
                text,
                case["output"]["printed"].as_str().unwrap(),
                "{context}"
            );
            let back = parse(
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
    let schema = schema();
    let defs = &schema["definitions"];
    let fields = &defs["node"]["fields"];
    let check = |a: Value, b: Value, shape: &Value| parity(&a, &b, shape, defs, "$");
    for target in ["pct_ftp", "watts"] {
        assert!(
            check(
                json!({"kind":target,"value":1.0+5e-13}),
                json!({"kind":target,"value":1}),
                &fields["target"]
            )
            .is_ok()
        );
    }
    for (field, value_shape) in [
        ("s", &fields["end"]),
        ("bpm", &fields["target"]),
        ("lo", &fields["cadence"]),
    ] {
        assert!(check(json!({field:1.0+5e-13}), json!({field:1}), value_shape).is_ok());
    }
    assert!(check(json!({"kj":5e-16}), json!({"kj":0}), &fields["end"]).is_ok());
    assert!(check(json!({"kj":2e-15}), json!({"kj":0}), &fields["end"]).is_err());
    for (a, b) in [
        (json!(1.0 + 5e-13), json!(1)),
        (json!(1.5), json!(1.5)),
        (
            json!(9_007_199_254_740_992u64),
            json!(9_007_199_254_740_993u64),
        ),
    ] {
        assert!(check(a, b, &fields["count"]).is_err());
    }
    assert!(check(json!([{}]), json!([]), &fields["children"]).is_err());
    assert!(check(json!({"label":null}), json!({}), &defs["node"]).is_err());
    assert!(check(json!("a"), json!("b"), &fields["label"]).is_err());
    assert!(
        check(
            json!({"label":"a","count":1}),
            json!({"count":1,"label":"a"}),
            &defs["node"]
        )
        .is_ok()
    );
    assert!(
        check(
            json!({"value":1.0+2e-12}),
            json!({"value":1}),
            &fields["target"]
        )
        .unwrap_err()
        .contains("$.value")
    );
    for value in [
        json!({"newField":1}),
        json!({"newField":null}),
        json!({"count":0,"newField":1}),
    ] {
        assert!(
            check(value.clone(), value, &defs["node"])
                .unwrap_err()
                .contains("unclassified")
        );
    }
    assert!(check(json!(1), json!(1), &json!("exact")).is_err());
    assert!(check(json!({}), json!({}), &json!({"ref":"missing"})).is_err());
    assert!(
        check(
            json!({"count":1,"newField":0}),
            json!({"count":2}),
            &defs["node"]
        )
        .unwrap_err()
        .contains("newField")
    );
    assert!(
        check(
            json!({"count":2}),
            json!({"count":1,"newField":0}),
            &defs["node"]
        )
        .unwrap_err()
        .contains("newField")
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
    let mut next = 0;
    let mut source = || {
        next += 1;
        format!("shared:{next}")
    };
    for _ in 0..2 {
        ids(
            &parse_dsl("2x(1m 100%, 1m 50%)", 280.0, &mut source).unwrap(),
            &mut all,
        );
    }
    assert_eq!(all.len(), 6);
    let a = parse("# warm up\n10m 50%\n\n3x(1m 120%;1m 50%) # sets\n", 280.0).unwrap();
    let b = parse("10m 50%, 3x(1m 120%, 1m 50%)", 280.0).unwrap();
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
        (0..=200_000u32).prop_map(|v| PowerTarget::PctFtp {
            value: f64::from(v) / 1000.0
        }),
        (0..=900u32).prop_map(|v| PowerTarget::Watts {
            value: f64::from(v)
        }),
        (30_000..=120_000u32, 0..=30_000u32).prop_map(|(lo, d)| PowerTarget::Range {
            lo: f64::from(lo) / 1000.0,
            hi: f64::from(lo + d) / 1000.0
        }),
        (30_000..=150_000u32, 30_000..=150_000u32).prop_map(|(from, to)| PowerTarget::Ramp {
            from: f64::from(from) / 1000.0,
            to: f64::from(to) / 1000.0
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
        (1..=300_000u32).prop_map(|kj| StepEnd::Kj {
            kj: f64::from(kj) / 1000.0
        }),
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
        let back = parse(&text,280.0).unwrap();
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
    assert_eq!(parse("1m 50% ramp 60-70%", 280.0).unwrap_err().offset, 12);
    assert!(parse("1m 100000w", 0.0).is_ok());
    assert!(parse("1m 400%", 280.0).is_ok());
    assert!(parse("1m 400.001%", 280.0).is_err());
    assert!(parse("\u{feff}1m\u{a0}50%", 280.0).is_ok());
    assert_eq!(parse("1m\u{85}50%", 280.0).unwrap_err().offset, 0);
    assert_eq!(parse("\"🚲\" 1m 50%; \"é\"", 280.0).unwrap_err().offset, 16);
}
