//! Untrusted-input and stack-safety regressions for the workout parser.
use crank_domain::workout::{DslErrorKind, MAX_NESTING_DEPTH, parse_dsl};
use proptest::prelude::*;

fn nested(depth: usize) -> String {
    format!("{}1m 50%{}", "1x(".repeat(depth), ")".repeat(depth))
}

#[test]
fn excessive_nesting_returns_an_error_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let err = parse_dsl(&nested(2000), 280.0, &mut String::new).unwrap_err();
            assert_eq!(
                err.kind,
                DslErrorKind::NestingTooDeep {
                    max: MAX_NESTING_DEPTH
                }
            );
            assert_eq!((err.offset, err.length), (MAX_NESTING_DEPTH * 3, 2));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn maximum_nesting_is_safe_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let nodes = parse_dsl(&nested(MAX_NESTING_DEPTH), 280.0, &mut String::new).unwrap();
            assert_eq!(nodes.len(), 1);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn overflowing_numbers_and_duration_arithmetic_are_typed_errors() {
    let huge = "9".repeat(400);
    let finite = "9".repeat(308);
    for text in [
        format!("{huge}x(1m 50%)"),
        format!("{huge}s free"),
        format!("{huge}kj free"),
        format!("1m {huge}w"),
        format!("1m {huge}bpm"),
        format!("1m free {huge}rpm"),
        format!("1x[+{huge}%](1m free)"),
        format!("{finite}h free"),
        format!("hr<120 1m..{finite}h free"),
    ] {
        let err = parse_dsl(&text, 280.0, &mut String::new).unwrap_err();
        assert_eq!(err.kind, DslErrorKind::NumericOverflow, "{text}");
        assert!(serde_json::to_value(err).is_ok());
    }
}

#[test]
fn long_digit_runs_in_non_number_words_are_unknown_tokens() {
    let huge = "9".repeat(400);
    for text in [
        format!("1m 50% foo{huge}"),
        format!("1m 50% {huge}foo"),
        format!("1m 50% a{huge}rpm"),
        format!("1m 50% !x{huge}"),
    ] {
        let err = parse_dsl(&text, 280.0, &mut String::new).unwrap_err();
        assert_eq!(err.kind, DslErrorKind::UnknownToken, "{text}");
    }
    // Numeric-shaped tokens still overflow, including signed and `hr<` forms.
    for text in [format!("1m hr<{huge}"), format!("1x[-{huge}w](1m free)")] {
        let err = parse_dsl(&text, 280.0, &mut String::new).unwrap_err();
        assert_eq!(err.kind, DslErrorKind::NumericOverflow, "{text}");
    }
}

fn check_input(text: &str) {
    // A panic fails the property; all rejected input must expose a typed error.
    if let Err(err) = parse_dsl(text, 280.0, &mut String::new) {
        let value = serde_json::to_value(&err).expect("serializable typed error");
        assert!(value["kind"]["kind"].is_string());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn arbitrary_unicode_input_does_not_panic(text in any::<String>()) {
        check_input(&text);
    }

    #[test]
    fn dsl_alphabet_and_deep_nesting_do_not_panic(
        body in "[0-9a-zA-Z%+!#<>.,;()\\[\\]\" \\n\\t-]{0,2048}",
        depth in 0usize..=2048,
        closed in any::<bool>(),
    ) {
        check_input(&body);
        check_input(&format!("{}{}{}", "1x(".repeat(depth), body,
            if closed { ")".repeat(depth) } else { String::new() }));
        // Always exercise valid nested syntax as well as arbitrary token sequences.
        check_input(&nested(depth));
    }
}
