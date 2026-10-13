//! Regressions for #68's typed errors, purity and printer contract.
use crank_domain::workout::{Cadence, DslErrorKind, WorkoutNode, parse_dsl, print_dsl};

fn source() -> impl FnMut() -> String {
    let mut next = 0;
    move || {
        next += 1;
        format!("test:{next}")
    }
}

#[test]
fn explicit_id_sources_are_reproducible_and_can_be_shared() {
    let text = "2x(1m 100%, 1m 50%)";
    let mut ids = source();
    let first = parse_dsl(text, 280.0, &mut ids).unwrap();
    let second = parse_dsl(text, 280.0, &mut ids).unwrap();
    assert_ne!(first, second);
    assert_eq!(first, parse_dsl(text, 280.0, &mut source()).unwrap());
}

#[test]
fn errors_expose_typed_data_and_keep_the_reference_message() {
    let err = parse_dsl("0x(1m 50%)", 280.0, &mut source()).unwrap_err();
    assert_eq!(err.kind, DslErrorKind::InvalidRepeatCount { count: 0.0 });
    assert_eq!(err.message, "Repeat count must be ≥ 1");
    let err = parse_dsl("5m 900%", 280.0, &mut source()).unwrap_err();
    assert_eq!(err.kind, DslErrorKind::ImplausiblePower { pct: 900.0 });
    assert_eq!(err.message, "900 % FTP is implausible (max 400 %)");
    let value = serde_json::to_value(&err).unwrap();
    assert_eq!(
        value["kind"],
        serde_json::json!({"kind":"implausible_power","pct":900.0})
    );
    assert_eq!(
        serde_json::from_value::<crank_domain::workout::DslError>(value).unwrap(),
        err
    );
}

#[test]
fn cadence_signed_zero_matches_javascript_equality_and_formatting() {
    for (lo, hi) in [(-0.0, 0.0), (0.0, -0.0), (-0.0, -0.0)] {
        let mut nodes = parse_dsl("1m free", 280.0, &mut source()).unwrap();
        let WorkoutNode::Step(step) = &mut nodes[0] else {
            panic!("step");
        };
        step.cadence = Some(Cadence { lo, hi });
        assert_eq!(print_dsl(&nodes), "1m free 0rpm");
    }
}

#[test]
fn whole_number_printing_uses_shortest_decimal_and_normalizes_zero() {
    // Frozen TS printTarget uses String(Math.round(value)), not fixed precision.
    for (text, expected) in [
        ("1m 1000000000000000128w", "1m 1000000000000000100w"),
        ("1m 9007199254740991bpm", "1m 9007199254740991bpm"),
        ("0.001kj free", "0.001kj free"),
    ] {
        assert_eq!(
            print_dsl(&parse_dsl(text, 0.0, &mut source()).unwrap()),
            expected
        );
    }
}

#[test]
fn id_consumption_on_errors_is_explicit() {
    let mut next = 0;
    {
        let mut ids = || {
            next += 1;
            next.to_string()
        };
        assert!(parse_dsl("2x(1m 50%, bad)", 280.0, &mut ids).is_err());
    }
    assert_eq!(next, 2);
    next = 0;
    {
        let mut ids = || {
            next += 1;
            next.to_string()
        };
        assert!(parse_dsl("1m 50%; \"unterminated", 280.0, &mut ids).is_err());
    }
    assert_eq!(next, 0);
}
