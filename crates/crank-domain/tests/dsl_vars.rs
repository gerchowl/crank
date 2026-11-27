//! Workout variables (ADR-0008 §2.6–§2.11, §2.14, §2.15; #742): round trips, rounding and
//! inlining, every parse error with its span, the bounds, and `resolve_vars`.
use crank_domain::route::parse_grade_profile;
use crank_domain::workout::{
    CompileError, CompileOptions, DslDoc, DslError, DslErrorKind as K, Expr, MAX_EXPRESSION_DEPTH,
    MAX_VARIABLE_NAME, MAX_VARIABLES, Num, OnUnreachable, Op, PowerTarget, Progression, RepeatNode,
    ResolveError, StepEnd, StepNode, Unit, VarDecl, Workout, WorkoutNode, compile_workout,
    parse_dsl, print_dsl, resolve_vars,
};
use proptest::prelude::*;
use serde_json::Value;

const FTP: f64 = 280.0;

fn parse(text: &str) -> Result<DslDoc, DslError> {
    let mut next = 0;
    parse_dsl(text, FTP, &mut || {
        next += 1;
        format!("n:{next}")
    })
}
fn print(doc: &DslDoc) -> String {
    print_dsl(&doc.vars, &doc.nodes)
}
fn workout(doc: &DslDoc) -> Workout {
    Workout {
        id: "w".into(),
        name: "w".into(),
        ftp: FTP,
        nodes: doc.nodes.clone(),
        vars: doc.vars.clone(),
    }
}
fn compile(doc: &DslDoc) -> Result<crank_domain::workout::CompiledWorkout, CompileError> {
    compile_workout(&workout(doc), &CompileOptions::default())
}
fn without_ids(nodes: &[WorkoutNode]) -> Value {
    fn strip(value: &mut Value) {
        for node in value.as_array_mut().expect("node array") {
            node.as_object_mut().expect("node object").remove("id");
            if let Some(children) = node.get_mut("children") {
                strip(children);
            }
        }
    }
    let mut value = serde_json::to_value(nodes).expect("serialisable");
    strip(&mut value);
    value
}
/// The tree with every reference resolved, ids aside.
fn values(doc: &DslDoc) -> Value {
    without_ids(&resolve_vars(&workout(doc)).expect("resolves").nodes)
}
fn error(text: &str) -> DslError {
    parse(text).expect_err(text)
}
/// A UTF-16 span `(offset, length)` of `needle` in `text`, at its `nth` occurrence.
fn span(text: &str, needle: &str, nth: usize) -> (usize, usize) {
    let at = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("{needle:?} in {text:?}"))
        .0;
    (
        text[..at].encode_utf16().count(),
        needle.encode_utf16().count(),
    )
}

// ---- goldens: rounding, exactness, inlining (ADR-0008 §2.8, §5) ----------------------------

fn first_block_power(text: &str) -> (f64, f64) {
    let c = compile(&parse(text).expect(text)).expect(text);
    (c.blocks[0].power_from, c.blocks[0].power_to)
}

#[test]
fn a_watts_use_rounds_to_whole_watts() {
    let doc = parse("$a = 250w / 3\n5m $a").unwrap();
    assert_eq!(values(&doc)[0]["target"]["value"], 83.0);
    assert_eq!(first_block_power("$a = 250w / 3\n5m $a"), (83.0, 83.0));
}

#[test]
fn a_percent_use_rounds_to_three_decimals() {
    let doc = parse("$p = 100% / 3\n5m $p").unwrap();
    assert_eq!(values(&doc)[0]["target"]["value"], 33.333);
}

#[test]
fn no_intermediate_value_is_rounded_and_inlining_keeps_it() {
    let text = "$a = 250w / 3\n$b = $a * 3\n5m $b";
    assert_eq!(first_block_power(text), (250.0, 250.0));
    // Inlining `$a` puts its expression in parentheses; the printer drops the ones `*`/`/`
    // grouping left to right does not need, and the value stays exactly 250 W.
    let inlined = "$b = (250w / 3) * 3\n5m $b";
    let doc = parse(inlined).unwrap();
    assert_eq!(print(&doc), "$b = 250w / 3 * 3\n\n5m $b");
    assert_eq!(first_block_power(inlined), (250.0, 250.0));
    assert_eq!(first_block_power(&print(&doc)), (250.0, 250.0));
}

#[test]
fn a_watts_progression_variable_behaves_like_its_literal() {
    let var = parse("$p = 2.5w\n3x[+$p](5m 100%, 1m 50%)").unwrap();
    let lit = parse("3x[+2.5w](5m 100%, 1m 50%)").unwrap();
    assert_eq!(compile(&var).unwrap(), compile(&lit).unwrap());
    assert_eq!(values(&var), without_ids(&lit.nodes));
    let pct = parse("$p = 2%\n3x[+$p](5m 100%)").unwrap();
    assert_eq!(
        compile(&pct).unwrap(),
        compile(&parse("3x[+2%](5m 100%)").unwrap()).unwrap()
    );
}

#[test]
fn a_zero_duration_variable_is_a_valid_recovery_bound() {
    let doc = parse("$z = 0s\n10m 100%\nhr<118 $z..4m 40%").unwrap();
    assert_eq!(values(&doc)[1]["end"]["minS"], 0.0);
    assert!(compile(&doc).is_ok());
    // As a time step, 0 s is the compiler's issue, not a parse error (ADR-0008 §2.8).
    let zero = compile(&parse("$z = 0s\n$z 50%").unwrap()).unwrap();
    assert!(
        zero.issues
            .iter()
            .any(|i| i.kind == crank_domain::workout::ValidationKind::InvalidDuration)
    );
}

#[test]
fn a_duration_product_is_exact() {
    let doc = parse("$d = 1m30s * 1.1\n$d 50%").unwrap();
    assert_eq!(values(&doc)[0]["end"]["s"], 99.0);
}

#[test]
fn a_literal_beyond_the_exact_range_is_numeric_overflow() {
    // 40 significant digits do not fit `Ratio<i128>` (about 1.7e38).
    let word = "1.000000000000000000000000000000000000001w";
    let text = format!("$a = {word}");
    let e = error(&text);
    assert_eq!(e.kind, K::NumericOverflow);
    assert_eq!((e.offset, e.length), span(&text, word, 0));
    // A product past the range is refused at its operator.
    let product = "$a = 10000000000000000000w * 100000000000000000000";
    let e = error(product);
    assert_eq!(e.kind, K::NumericOverflow);
    assert_eq!((e.offset, e.length), span(product, "*", 0));
}

#[test]
fn percentage_points_and_relative_are_both_writable() {
    let doc = parse("$thr = 95%\n$a = $thr - 10%\n$b = $thr * 0.9\n5m $a\n5m $b").unwrap();
    let v = values(&doc);
    assert_eq!(v[0]["target"]["value"], 85.0);
    assert_eq!(v[1]["target"]["value"], 85.5);
}

#[test]
fn every_position_takes_a_variable_and_prints_its_name() {
    let text = "$thr = 95%\n$w = 250w\n$lo = 60%\n$hi = 80%\n$on = 4m\n$rest = $on / 2\n$min = 1m\n$sets = 3\n$p = 5w\n$q = 2%\n$drop = -5w\n\n\"Warm-up\" 10m ramp $lo-$hi\n$sets x[+$p, +$q, !last](\"On\" $on $thr 90rpm, \"Off\" $rest 55%)\n5m $w\n5m $lo-80%\n5m 50%-$hi\n2m ramp $lo-75%\n2m ramp 45%-$hi\nhr<118 $min..$on 40%\n2x[+$drop](1m 100%)";
    let doc = parse(text).unwrap();
    assert_eq!(print(&doc), text);
    assert_eq!(doc.vars.len(), 11);
    let WorkoutNode::Repeat(r) = &doc.nodes[1] else {
        panic!("repeat")
    };
    assert_eq!(r.count, Num::var("sets"));
    assert_eq!(
        r.progression,
        Some(Progression {
            watts_per_rep: Some(Num::var("p")),
            pct_per_rep: Some(Num::var("q")),
        })
    );
    let v = values(&doc);
    assert_eq!(v[0]["target"]["from"], 60.0);
    assert_eq!(v[1]["count"], 3.0);
    assert_eq!(v[1]["children"][1]["end"]["s"], 120.0);
    assert_eq!(v[7]["end"]["maxS"], 240.0);
    assert_eq!(v[8]["progression"]["wattsPerRep"], -5.0);
    assert!(compile(&doc).is_ok());
}

#[test]
fn canonical_printing_spaces_operators_and_keeps_needed_parentheses() {
    for (text, printed) in [
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = $a - ($b - $c)",
            "$d = $a - ($b - $c)",
        ),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = ($a * 2) + $b",
            "$d = $a * 2 + $b",
        ),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = $a + ($b + $c)",
            "$d = $a + ($b + $c)",
        ),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = ($a + $b) * 2",
            "$d = ($a + $b) * 2",
        ),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = -($a + 30w)",
            "$d = -($a + 30w)",
        ),
        ("$a = 1w\n$b = 2w\n$c = 3w\n$d = - $a", "$d = -$a"),
        ("$a = 1w\n$b = 2w\n$c = 3w\n$d = $a - -5w", "$d = $a - -5w"),
        ("$a = 1w\n$b = 2w\n$c = 3w\n$d = 90s * 2", "$d = 1m30s * 2"),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = 033.50% + 0.1235%",
            "$d = 33.5% + 0.124%",
        ),
        (
            "$a = 1w\n$b = 2w\n$c = 3w\n$d = 2.50w * 01.10",
            "$d = 2.5w * 1.1",
        ),
    ] {
        let doc = parse(text).unwrap();
        let out = print(&doc);
        assert_eq!(out.lines().nth(3), Some(printed), "{text}");
        assert_eq!(print(&parse(&out).unwrap()), out, "{text}");
    }
    // Declarations alone print without a trailing blank line; a blank line separates them
    // from the blocks.
    assert_eq!(print(&parse("$a = 1m").unwrap()), "$a = 1m");
    assert_eq!(
        print(&parse("$a = 1m, $a 50%").unwrap()),
        "$a = 1m\n\n$a 50%"
    );
}

#[test]
fn a_workout_without_variables_prints_exactly_as_before() {
    // §2.15: today's texts keep their tokens, trees and canonical text.
    for text in [
        "\"Warm-up\" 8m ramp 45-75% 90rpm\n3m 60%\n\"VO2 sets\" 3x[+5w](5x(30s 125%, 15s 50%), \"Recover\" hr<118 2m..4m 40%)\n\"Threshold\" 2x[!last](4m 95% 90-95rpm, 2m 55%)\n\"Cool-down\" 5m ramp 60-40%",
        "10m 50-75%\n1m 250w\n5m 140bpm\nlap free\n10kj 60%",
    ] {
        let doc = parse(text).unwrap();
        assert!(doc.vars.is_empty());
        assert_eq!(print(&doc), text);
        let json = serde_json::to_value(workout(&doc)).unwrap();
        assert_eq!(json["vars"], serde_json::json!([]));
    }
}

#[test]
fn stored_json_without_vars_reads_unchanged() {
    let json = r#"{"id":"w","name":"n","ftp":250,"nodes":[{"type":"repeat","id":"r","label":null,"count":3,"children":[{"type":"step","id":"s","label":null,"end":{"kind":"time","s":60},"target":{"kind":"range","lo":50,"hi":60},"cadence":null}],"progression":{"wattsPerRep":5,"pctPerRep":null},"skipLastRest":true}]}"#;
    let w: Workout = serde_json::from_str(json).unwrap();
    assert!(w.vars.is_empty());
    let WorkoutNode::Repeat(r) = &w.nodes[0] else {
        panic!("repeat")
    };
    assert_eq!(r.count, Num::Lit(3.0));
    // Written back with an empty `vars` (numbers as `f64`), and read back equal.
    let back = serde_json::to_value(&w).unwrap();
    assert_eq!(back["vars"], serde_json::json!([]));
    assert_eq!(back["nodes"][0]["count"], 3.0);
    assert_eq!(serde_json::from_value::<Workout>(back).unwrap(), w);
    // A reference serialises as `{ "var": name }` (ADR-0008 §2.10).
    let doc = parse("$on = 4m\n$on 95%").unwrap();
    let v = serde_json::to_value(workout(&doc)).unwrap();
    assert_eq!(v["nodes"][0]["end"]["s"], serde_json::json!({"var": "on"}));
    assert_eq!(
        v["vars"][0],
        serde_json::json!({"name": "on", "expr": {"kind": "lit", "value": "240", "unit": "seconds"}})
    );
    assert_eq!(serde_json::from_value::<Workout>(v).unwrap(), workout(&doc));
}

// ---- parse errors (ADR-0008 §2.14): kind, message, UTF-16 span -----------------------------

/// §2.14's rows (kind, message, the highlighted text and which occurrence of it), then the
/// remaining typed faults of declarations and uses.
const ERROR_ROWS: &[(&str, K, &str, &str, usize)] = &[
    (
        "4m $thr",
        K::UnknownVariable,
        "Unknown variable '$thr' — declare it before the first block, e.g. $thr = 95%",
        "$thr",
        0,
    ),
    (
        "$a = $b * 2\n$b = 1m",
        K::UnknownVariable,
        "Unknown variable '$b' — a declaration can use only variables declared above it",
        "$b",
        0,
    ),
    (
        "3m 60%\n$x = 1m",
        K::DeclarationAfterBlock,
        "Variables are declared before the first block",
        "$x",
        0,
    ),
    (
        "2x($x = 1m)",
        K::DeclarationAfterBlock,
        "Variables are declared before the first block",
        "$x",
        0,
    ),
    (
        "$thr = 95%\n$thr = 90%",
        K::DuplicateVariable,
        "'$thr' is already declared",
        "$thr",
        1,
    ),
    (
        "$d = 4m\n10m $d",
        K::VariableTypeMismatch,
        "'$d' is a duration (4m); a power target is expected here, e.g. 95% or 250w",
        "$d",
        1,
    ),
    (
        "$p = 250w\n5m ramp $p-75%",
        K::VariableTypeMismatch,
        "'$p' is in watts; a ramp is in % FTP",
        "$p-75%",
        0,
    ),
    (
        "$a = 95% + 250w",
        K::MismatchedUnits,
        "Cannot add % FTP and watts",
        "+",
        0,
    ),
    (
        "$a = 4m * 2m",
        K::InvalidProduct,
        "A duration can be multiplied only by a plain number",
        "*",
        0,
    ),
    ("$a = 1m / 0", K::DivisionByZero, "Division by zero", "/", 0),
    (
        "$n = 7 / 2\n$n x(5m 50%)",
        K::InvalidRepeatCount { count: 3.5 },
        "Repeat count '$n' is 3.5; it must be a whole number ≥ 1",
        "$n",
        1,
    ),
    (
        "$d = 4m / 7\n$d 90%",
        K::FractionalDuration,
        "'$d' is 34.286 s; a duration must be whole seconds",
        "$d",
        1,
    ),
    (
        "$a = $b-30s",
        K::UnspacedOperator,
        "Put spaces around '-' in an expression: $b - 30s",
        "$b-30s",
        0,
    ),
    (
        "$1x = 3",
        K::InvalidVariableName,
        "A variable name starts with a letter, e.g. $thr, $rest_2",
        "$1x",
        0,
    ),
    (
        "$a = 50%\n5m ramp $a-75",
        K::MissingPercent,
        "Write the % on the number next to a variable, e.g. ramp $a-75%",
        "$a-75",
        0,
    ),
    (
        "$lo = 50%\n5m $lo-80",
        K::MissingPercent,
        "Write the % on the number next to a variable, e.g. $lo-80%",
        "$lo-80",
        0,
    ),
    (
        "5m ramp 45%-75%",
        K::RedundantPercent,
        "Write a ramp between two numbers as ramp 45-75%",
        "45%-75%",
        0,
    ),
    (
        "5m 60%-80%",
        K::RedundantPercent,
        "Write a range between two numbers as 60-80%",
        "60%-80%",
        0,
    ),
    (
        "$p = 250w\n5m $p-80%",
        K::VariableTypeMismatch,
        "'$p' is in watts; a range is in % FTP",
        "$p-80%",
        0,
    ),
    (
        "$e = 40% - 50%\n5m $e",
        K::NegativePower,
        "'$e' is -10 % FTP; a power must be 0 or more",
        "$e",
        1,
    ),
    (
        "$lo = 80%\n$hi = 60%\n5m $lo-$hi",
        K::ReversedPowerRange,
        "Range low end is above high end (use 'ramp 80-60%' for a descending ramp)",
        "$lo-$hi",
        0,
    ),
    (
        "$d = 1m - 2m\n$d 50%",
        K::NegativeDuration,
        "'$d' is -60 s; a duration must be 0 or more",
        "$d",
        1,
    ),
    (
        "$n = 0\n$n x(5m 50%)",
        K::InvalidRepeatCount { count: 0.0 },
        "Repeat count '$n' is 0; it must be a whole number ≥ 1",
        "$n",
        1,
    ),
    (
        "$thr=95%",
        K::UnspacedEquals,
        "Put spaces around '=': $thr = 95%",
        "$thr=95%",
        0,
    ),
    (
        "$thr =95%",
        K::UnspacedEquals,
        "Put spaces around '=': $thr = 95%",
        "=95%",
        0,
    ),
    (
        "$p = 5w\n3x[-$p](5m 100%)",
        K::InvalidProgressionVariable,
        "Write a falling progression as a negative variable: $p = -5w, then [+$p]",
        "-$p",
        0,
    ),
    (
        "$p = 5w\n3x[$p](5m 100%)",
        K::InvalidProgressionVariable,
        "Write a progression variable with its sign: [+$p]",
        "$p",
        1,
    ),
    // Beyond §2.14's table: the remaining typed faults of declarations and uses.
    (
        "$a = 95% - 4m",
        K::MismatchedUnits,
        "Cannot subtract a duration from % FTP",
        "-",
        0,
    ),
    (
        "$a = 95% / 4m",
        K::InvalidQuotient,
        "A % FTP value can be divided only by a plain number",
        "/",
        0,
    ),
    (
        "$a = 5w 3w",
        K::ExpectedOperator,
        "Expected an operator (+, -, *, /) or the end of the declaration; got '3w'",
        "3w",
        0,
    ),
    (
        "$a = 5q",
        K::ExpectedExpression,
        "Expected a value (95%, 250w, 4m, 3) or a variable; got '5q'",
        "5q",
        0,
    ),
    (
        "$a = (5w + 3w",
        K::UnclosedParenthesis,
        "Unclosed '(' — missing ')'",
        "(",
        0,
    ),
    (
        "$a_b.c = 3",
        K::InvalidVariableName,
        "A variable name has only letters, digits and '_', e.g. $thr, $rest_2",
        "$a_b.c",
        0,
    ),
    (
        "$d = 4m\n3x[+$d](5m 100%)",
        K::VariableTypeMismatch,
        "'$d' is a duration (4m); a progression is in watts or % FTP, e.g. $p = 5w",
        "$d",
        1,
    ),
    (
        "$p = 95%\n$p 50%",
        K::VariableTypeMismatch,
        "'$p' is in % FTP (95%); a duration is expected here, e.g. 4m",
        "$p",
        1,
    ),
    (
        "$d = 4m\n$d x(5m 50%)",
        K::VariableTypeMismatch,
        "'$d' is a duration (4m); a repeat count is a plain number, e.g. $n = 3",
        "$d",
        1,
    ),
    (
        "$p = 500%\n5m $p",
        K::ImplausiblePower { pct: 500.0 },
        "500 % FTP is implausible (max 400 %)",
        "$p",
        1,
    ),
    (
        "$a = 1m\n$b = 2m\nhr<118 $b..$a 40%",
        K::ReversedRecoveryBounds {
            min_s: 120.0,
            max_s: 60.0,
        },
        "Min duration is longer than max",
        "$b..$a",
        0,
    ),
    (
        "\"🚲\" 4m $nope",
        K::UnknownVariable,
        "Unknown variable '$nope' — declare it before the first block, e.g. $nope = 95%",
        "$nope",
        0,
    ),
];

#[test]
fn every_error_row_has_its_kind_message_and_span() {
    for &(text, ref kind, message, needle, nth) in ERROR_ROWS {
        let e = error(text);
        assert_eq!(&e.kind, kind, "{text}");
        assert_eq!(e.message, message, "{text}");
        assert_eq!((e.offset, e.length), span(text, needle, nth), "{text}");
    }
}

#[test]
fn a_variable_inside_a_word_is_selected_alone() {
    // A fault of the variable itself selects the variable, not the whole range word.
    let text = "$lo = 50%\n5m $lo-$nope";
    let e = error(text);
    assert_eq!(e.kind, K::UnknownVariable);
    assert_eq!((e.offset, e.length), span(text, "$nope", 0));
    let text = "$min = 1m\nhr<118 $min..$max 40%";
    assert_eq!(
        (error(text).offset, error(text).length),
        span(text, "$max", 0)
    );
}

#[test]
fn each_bound_is_accepted_at_its_limit_and_refused_one_past() {
    let decls = |n: usize| {
        (0..n)
            .map(|i| format!("$v{i} = {i}w"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(parse(&decls(MAX_VARIABLES)).unwrap().vars.len(), 100);
    let text = decls(MAX_VARIABLES + 1);
    let e = error(&text);
    assert_eq!(e.kind, K::TooManyVariables { max: 100 });
    assert_eq!((e.offset, e.length), span(&text, "$v100", 0));

    // Expression depth: unary minus, parentheses and binary chains are levels alike.
    let minus = |n: usize| format!("$a = {}5w", "-".repeat(n));
    let parens = |n: usize| format!("$a = {}5w{}", "(".repeat(n), ")".repeat(n));
    let chain = |n: usize| format!("$a = 1w{}", " + 1w".repeat(n));
    let deep = K::ExpressionTooDeep { max: 32 };
    for make in [minus, parens, chain] {
        assert!(
            parse(&make(MAX_EXPRESSION_DEPTH)).is_ok(),
            "{}",
            make(MAX_EXPRESSION_DEPTH)
        );
        assert_eq!(error(&make(MAX_EXPRESSION_DEPTH + 1)).kind, deep);
    }
    let text = chain(33);
    let e = error(&text);
    assert_eq!(e.offset, text.rfind('+').unwrap());

    let name = "a".repeat(MAX_VARIABLE_NAME);
    assert!(parse(&format!("${name} = 3")).is_ok());
    let text = format!("${name}b = 3");
    let e = error(&text);
    assert_eq!(e.kind, K::VariableNameTooLong { max: 32 });
    assert_eq!((e.offset, e.length), (0, 34));
}

#[test]
fn deep_expressions_are_refused_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            for text in [
                format!("$a = {}5w", "-".repeat(20_000)),
                format!("$a = {}5w", "(".repeat(20_000)),
                format!("$a = {}5w", "- ".repeat(20_000)),
                format!("$a = 1w{}", " + 1w".repeat(20_000)),
            ] {
                assert_eq!(
                    parse(&text).unwrap_err().kind,
                    K::ExpressionTooDeep { max: 32 }
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn grade_profiles_refuse_variables_with_todays_message() {
    // §2.15: the route grammar gains neither `$` nor `auto`; its refusal is unchanged.
    for word in ["$x", "auto"] {
        let today = parse_grade_profile("zz").unwrap_err();
        let e = parse_grade_profile(word).unwrap_err();
        assert_eq!(e.kind, today.kind, "{word}");
        assert_eq!(e.message, today.message.replace("zz", word), "{word}");
        assert_eq!((e.offset, e.length), (0, word.len()), "{word}");
    }
}

// ---- resolve_vars on trees built or deserialised directly (ADR-0008 §2.10) ------------------

fn step(end: StepEnd, target: PowerTarget) -> WorkoutNode {
    WorkoutNode::Step(StepNode {
        id: "s1".into(),
        label: None,
        end,
        target,
        cadence: None,
    })
}
fn tree(vars: Vec<VarDecl>, nodes: Vec<WorkoutNode>) -> Workout {
    Workout {
        id: "w".into(),
        name: "w".into(),
        ftp: FTP,
        nodes,
        vars,
    }
}
fn lit(value: &str, unit: Unit) -> Expr {
    Expr::Lit {
        value: value.into(),
        unit,
    }
}
fn decl(name: &str, expr: Expr) -> VarDecl {
    VarDecl {
        name: name.into(),
        expr,
    }
}

#[test]
fn a_tree_with_an_unknown_name_is_a_typed_compile_refusal() {
    let w = tree(
        vec![],
        vec![step(
            StepEnd::Time { s: 60.0.into() },
            PowerTarget::PctFtp {
                value: Num::var("thr"),
            },
        )],
    );
    let unknown = ResolveError::UnknownVariable { name: "thr".into() };
    assert_eq!(resolve_vars(&w), Err(unknown.clone()));
    assert_eq!(
        compile_workout(&w, &CompileOptions::default()),
        Err(CompileError::Resolve(unknown))
    );
    // Deserialised the same way.
    let json = r#"{"id":"w","name":"w","ftp":280,"nodes":[{"type":"step","id":"s1","label":null,"end":{"kind":"time","s":60},"target":{"kind":"pct_ftp","value":{"var":"thr"}},"cadence":null}]}"#;
    let w: Workout = serde_json::from_str(json).unwrap();
    assert!(matches!(
        compile_workout(&w, &CompileOptions::default()),
        Err(CompileError::Resolve(ResolveError::UnknownVariable { .. }))
    ));
}

#[test]
fn a_tree_with_a_mismatched_type_is_a_typed_compile_refusal() {
    let vars = vec![decl("d", lit("240", Unit::Seconds))];
    let w = tree(
        vars.clone(),
        vec![step(
            StepEnd::Time { s: 60.0.into() },
            PowerTarget::PctFtp {
                value: Num::var("d"),
            },
        )],
    );
    let e = ResolveError::TypeMismatch {
        name: "d".into(),
        node_id: "s1".into(),
        found: Unit::Seconds,
    };
    assert_eq!(resolve_vars(&w), Err(e.clone()));
    assert_eq!(
        compile_workout(&w, &CompileOptions::default()),
        Err(CompileError::Resolve(e))
    );
    // A watts variable in a % field, and a % one in a watts progression.
    let w = tree(
        vec![decl("p", lit("250", Unit::Watts))],
        vec![step(
            StepEnd::Time { s: 60.0.into() },
            PowerTarget::PctFtp {
                value: Num::var("p"),
            },
        )],
    );
    assert!(matches!(
        resolve_vars(&w),
        Err(ResolveError::TypeMismatch {
            found: Unit::Watts,
            ..
        })
    ));
    let w = tree(
        vec![decl("q", lit("2", Unit::Pct))],
        vec![WorkoutNode::Repeat(RepeatNode {
            id: "r".into(),
            label: None,
            count: 2.0.into(),
            children: vec![step(
                StepEnd::Time { s: 60.0.into() },
                PowerTarget::PctFtp { value: 90.0.into() },
            )],
            progression: Some(Progression {
                watts_per_rep: Some(Num::var("q")),
                pct_per_rep: None,
            }),
            skip_last_rest: None,
        })],
    );
    assert!(matches!(
        resolve_vars(&w),
        Err(ResolveError::TypeMismatch {
            found: Unit::Pct,
            ..
        })
    ));
}

#[test]
fn resolve_vars_checks_what_the_parser_checks() {
    let at = |vars: Vec<VarDecl>, end: StepEnd, target: PowerTarget| {
        resolve_vars(&tree(vars, vec![step(end, target)])).unwrap_err()
    };
    let time = || StepEnd::Time { s: 60.0.into() };
    let pct = |v: f64| PowerTarget::PctFtp { value: v.into() };
    let neg = Expr::Bin {
        op: Op::Sub,
        l: Box::new(lit("40", Unit::Pct)),
        r: Box::new(lit("50", Unit::Pct)),
    };
    assert!(matches!(
        at(vec![decl("e", neg)], time(), PowerTarget::PctFtp { value: Num::var("e") }),
        ResolveError::InvalidValue { ref message, .. } if message == "'$e' is -10 % FTP; a power must be 0 or more"
    ));
    let frac = Expr::Bin {
        op: Op::Div,
        l: Box::new(lit("240", Unit::Seconds)),
        r: Box::new(lit("7", Unit::Number)),
    };
    assert!(matches!(
        at(vec![decl("d", frac)], StepEnd::Time { s: Num::var("d") }, pct(50.0)),
        ResolveError::InvalidValue { ref message, .. } if message == "'$d' is 34.286 s; a duration must be whole seconds"
    ));
    let zero = Expr::Bin {
        op: Op::Div,
        l: Box::new(lit("1", Unit::Watts)),
        r: Box::new(lit("0", Unit::Number)),
    };
    assert_eq!(
        at(vec![decl("z", zero)], time(), pct(50.0)),
        ResolveError::InvalidExpression {
            name: "z".into(),
            message: "Division by zero".into()
        }
    );
    assert_eq!(
        at(vec![decl("x", lit("1e3", Unit::Watts))], time(), pct(50.0)),
        ResolveError::InvalidLiteral {
            name: "x".into(),
            value: "1e3".into()
        }
    );
    assert_eq!(
        at(
            vec![decl("x", lit("1.5", Unit::Seconds))],
            time(),
            pct(50.0)
        ),
        ResolveError::InvalidLiteral {
            name: "x".into(),
            value: "1.5".into()
        }
    );
}

#[test]
fn resolve_vars_checks_declarations_as_the_parser_does() {
    let at = |vars: Vec<VarDecl>| {
        resolve_vars(&tree(
            vars,
            vec![step(
                StepEnd::Time { s: 60.0.into() },
                PowerTarget::PctFtp { value: 50.0.into() },
            )],
        ))
        .unwrap_err()
    };
    let fwd = vec![
        decl("a", Expr::Var { name: "b".into() }),
        decl("b", lit("1", Unit::Watts)),
    ];
    assert_eq!(at(fwd), ResolveError::UnknownVariable { name: "b".into() });
    let twice = vec![
        decl("a", lit("1", Unit::Watts)),
        decl("a", lit("2", Unit::Watts)),
    ];
    assert_eq!(
        at(twice),
        ResolveError::DuplicateVariable { name: "a".into() }
    );
    assert_eq!(
        at(vec![decl("1x", lit("1", Unit::Number))]),
        ResolveError::InvalidName { name: "1x".into() }
    );
    let many = (0..=MAX_VARIABLES)
        .map(|i| decl(&format!("v{i}"), lit("1", Unit::Number)))
        .collect();
    assert_eq!(at(many), ResolveError::TooManyVariables { max: 100 });
    let mut deep = lit("5", Unit::Watts);
    for _ in 0..=MAX_EXPRESSION_DEPTH {
        deep = Expr::Neg {
            expr: Box::new(deep),
        };
    }
    assert_eq!(
        at(vec![decl("d", deep)]),
        ResolveError::ExpressionTooDeep {
            name: "d".into(),
            max: 32
        }
    );
    let time = || StepEnd::Time { s: 60.0.into() };
    let pct = |v: f64| PowerTarget::PctFtp { value: v.into() };
    let count = |c: &str| {
        resolve_vars(&tree(
            vec![decl("n", lit(c, Unit::Number))],
            vec![WorkoutNode::Repeat(RepeatNode {
                id: "r".into(),
                label: None,
                count: Num::var("n"),
                children: vec![step(time(), pct(50.0))],
                progression: None,
                skip_last_rest: None,
            })],
        ))
    };
    assert!(count("3").is_ok());
    assert!(matches!(
        count("3.5"),
        Err(ResolveError::InvalidValue { .. })
    ));
    assert!(matches!(count("0"), Err(ResolveError::InvalidValue { .. })));
    // Literal fields are left to the compiler's own diagnostics.
    let w = tree(vec![], vec![step(time(), pct(-5.0))]);
    assert_eq!(resolve_vars(&w), Ok(w));
}

// ---- properties -----------------------------------------------------------------------------

/// A variable a block may use, with its exact value (thousandths for powers, seconds,
/// a count), so the generator can write only valid uses.
#[derive(Clone, Debug)]
struct Pool {
    pct: Vec<(String, u32)>,
    watts: Vec<(String, u32)>,
    secs: Vec<(String, u32)>,
    counts: Vec<String>,
    prog_watts: Vec<String>,
    prog_pct: Vec<String>,
}

fn name(i: usize) -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_]{0,24}".prop_map(move |tail| format!("v{i}{tail}"))
}
fn number_text(max: u32, places: u32) -> impl Strategy<Value = (String, u32)> {
    let scale = 10_u32.pow(places);
    (0..=max * scale).prop_map(move |n| {
        let int = n / scale;
        let frac = n % scale;
        let text = if frac == 0 {
            int.to_string()
        } else {
            let f = format!("{frac:0width$}", width = places as usize);
            format!("{int}.{}", f.trim_end_matches('0'))
        };
        // Thousandths of the value, for the generator's own bookkeeping.
        (text, n * (1000 / scale))
    })
}
/// One literal of `unit` (canonical text) and its value in thousandths (seconds: whole).
fn literal(unit: Unit) -> BoxedStrategy<(Expr, u32)> {
    match unit {
        Unit::Pct => number_text(150, 3)
            .prop_map(|(t, v)| (lit(&t, Unit::Pct), v))
            .boxed(),
        Unit::Watts => number_text(400, 0)
            .prop_map(|(t, v)| (lit(&t, Unit::Watts), v))
            .boxed(),
        Unit::Seconds => (0..=3600u32)
            .prop_map(|s| (lit(&s.to_string(), Unit::Seconds), s))
            .boxed(),
        Unit::Number => number_text(20, 1)
            .prop_map(|(t, v)| (lit(&t, Unit::Number), v))
            .boxed(),
    }
}
/// A typed expression over literals and `refs` (names of `unit`), with every operator,
/// unary minus and parentheses; divisors are non-zero literals, so it always evaluates.
fn expr(unit: Unit, refs: &[String]) -> BoxedStrategy<Expr> {
    let leaf = {
        let lits = literal(unit).prop_map(|(e, _)| e);
        if refs.is_empty() {
            lits.boxed()
        } else {
            prop_oneof![
                lits,
                proptest::sample::select(refs.to_vec()).prop_map(|name| Expr::Var { name })
            ]
            .boxed()
        }
    };
    let factor = (1..=200u32).prop_map(|n| {
        let text = if n % 10 == 0 {
            (n / 10).to_string()
        } else {
            format!("{}.{}", n / 10, n % 10)
        };
        lit(&text, Unit::Number)
    });
    leaf.prop_recursive(4, 24, 2, move |inner| {
        prop_oneof![
            inner.clone().prop_map(|e| Expr::Neg { expr: Box::new(e) }),
            (
                inner.clone(),
                inner.clone(),
                prop_oneof![Just(Op::Add), Just(Op::Sub)]
            )
                .prop_map(|(l, r, op)| Expr::Bin {
                    op,
                    l: Box::new(l),
                    r: Box::new(r)
                }),
            (inner.clone(), factor.clone(), any::<bool>()).prop_map(|(e, f, swap)| {
                let (l, r) = if swap { (f, e) } else { (e, f) };
                Expr::Bin {
                    op: Op::Mul,
                    l: Box::new(l),
                    r: Box::new(r),
                }
            }),
            (inner, factor.clone()).prop_map(|(e, f)| Expr::Bin {
                op: Op::Div,
                l: Box::new(e),
                r: Box::new(f)
            }),
        ]
    })
    .boxed()
}
/// `l0 - (l1 - (l2 - …))`: literals only, up to the depth bound.
fn chain() -> impl Strategy<Value = Expr> {
    proptest::collection::vec(number_text(100, 3), 2..=MAX_EXPRESSION_DEPTH + 1).prop_map(|terms| {
        let mut it = terms.into_iter().rev();
        let mut e = it.next().map(|(t, _)| lit(&t, Unit::Pct)).expect("terms");
        for (t, _) in it {
            e = Expr::Bin {
                op: Op::Sub,
                l: Box::new(lit(&t, Unit::Pct)),
                r: Box::new(e),
            };
        }
        e
    })
}
const UNITS: [Unit; 4] = [Unit::Pct, Unit::Watts, Unit::Seconds, Unit::Number];

/// Declarations, in order: usable ones (a literal, or an earlier usable variable of the same
/// type plus a literal: non-negative, whole seconds, counts 1 to 4), then expression-only
/// ones over every operator, which may use the usable ones and are used by no block.
fn declarations() -> impl Strategy<Value = (Vec<VarDecl>, Pool)> {
    let usable = proptest::collection::vec((0..UNITS.len(), any::<bool>(), any::<u8>()), 0..=8);
    let extra = proptest::collection::vec((0..UNITS.len(), any::<bool>()), 0..=3);
    let signed = proptest::collection::vec((any::<bool>(), 0..=20u32), 0..=2);
    (usable, extra, signed)
        .prop_flat_map(|(usable, extra, signed)| {
            let n = usable.len() + signed.len() + extra.len();
            let names = (0..n).map(name).collect::<Vec<_>>();
            let lits = usable
                .iter()
                .map(|(u, _, _)| literal(UNITS[*u]))
                .collect::<Vec<_>>();
            (Just(usable), Just(extra), Just(signed), names, lits)
        })
        .prop_flat_map(|(usable, extra, signed, names, lits)| {
            let mut vars = Vec::new();
            let mut pool = Pool {
                pct: vec![],
                watts: vec![],
                secs: vec![],
                counts: vec![],
                prog_watts: vec![],
                prog_pct: vec![],
            };
            let mut by_unit: Vec<Vec<(String, u32)>> = vec![vec![]; 4];
            for (i, ((u, sum, pick), (lit_expr, lit_value))) in usable.iter().zip(lits).enumerate()
            {
                let unit = UNITS[*u];
                let name = names[i].clone();
                // Counts stay small; powers stay plausible at 280 W FTP.
                let (lit_expr, lit_value) = match unit {
                    Unit::Number => {
                        let c = lit_value / 1000 % 4 + 1;
                        (lit(&c.to_string(), Unit::Number), c * 1000)
                    }
                    _ => (lit_expr, lit_value),
                };
                let prev = &by_unit[*u];
                let (expr, value) = if *sum && !prev.is_empty() && unit != Unit::Number {
                    let (p, pv) = &prev[usize::from(*pick) % prev.len()];
                    (
                        Expr::Bin {
                            op: Op::Add,
                            l: Box::new(Expr::Var { name: p.clone() }),
                            r: Box::new(lit_expr),
                        },
                        pv + lit_value,
                    )
                } else {
                    (lit_expr, lit_value)
                };
                by_unit[*u].push((name.clone(), value));
                vars.push(VarDecl { name, expr });
            }
            pool.pct.clone_from(&by_unit[0]);
            pool.watts.clone_from(&by_unit[1]);
            pool.secs = by_unit[2].iter().map(|(n, v)| (n.clone(), *v)).collect();
            pool.counts = by_unit[3].iter().map(|(n, _)| n.clone()).collect();
            pool.prog_watts = pool.watts.iter().map(|(n, _)| n.clone()).collect();
            pool.prog_pct = pool.pct.iter().map(|(n, _)| n.clone()).collect();
            let mut i = usable.len();
            for (watts, v) in &signed {
                // A falling progression: `$drop = -5w`, used only as `[+$drop]`.
                let unit = if *watts { Unit::Watts } else { Unit::Pct };
                let expr = Expr::Neg {
                    expr: Box::new(lit(&v.to_string(), unit)),
                };
                if *watts {
                    pool.prog_watts.push(names[i].clone());
                } else {
                    pool.prog_pct.push(names[i].clone());
                }
                vars.push(VarDecl {
                    name: names[i].clone(),
                    expr,
                });
                i += 1;
            }
            let extras = extra
                .iter()
                .enumerate()
                .map(|(k, (u, deep))| {
                    let refs: Vec<String> = by_unit[*u].iter().map(|(n, _)| n.clone()).collect();
                    let e = if *deep {
                        chain().boxed()
                    } else {
                        expr(UNITS[*u], &refs)
                    };
                    (Just(names[i + k].clone()), e).prop_map(|(name, expr)| VarDecl { name, expr })
                })
                .collect::<Vec<_>>();
            (Just(vars), Just(pool), extras)
        })
        .prop_map(|(mut vars, pool, extras)| {
            vars.extend(extras);
            (vars, pool)
        })
}

fn pick(names: &[(String, u32)]) -> BoxedStrategy<(Num, u32)> {
    proptest::sample::select(names.to_vec())
        .prop_map(|(n, v)| (Num::var(n), v))
        .boxed()
}
/// Targets using the % FTP variables `p`: a bare `%` target, the three range forms (ends
/// ordered by value) and the three ramp forms, each number with its own `%`.
fn pct_var_targets(p: &[(String, u32)]) -> Vec<BoxedStrategy<PowerTarget>> {
    let pct = |v: u32| Num::Lit(f64::from(v) / 1000.0);
    let mut out = Vec::new();
    out.push(
        pick(p)
            .prop_map(|(value, _)| PowerTarget::PctFtp { value })
            .boxed(),
    );
    // Ranges: var-var (ordered by value), var-lit and lit-var, each end with its `%`.
    out.push(
        (pick(p), pick(p))
            .prop_map(|((a, av), (b, bv))| {
                let (lo, hi) = if av <= bv { (a, b) } else { (b, a) };
                PowerTarget::Range { lo, hi }
            })
            .boxed(),
    );
    out.push(
        (pick(p), 0..=50_000u32)
            .prop_map(move |((lo, v), d)| PowerTarget::Range { lo, hi: pct(v + d) })
            .boxed(),
    );
    out.push(
        pick(p)
            .prop_flat_map(move |(hi, v)| (Just(hi), 0..=v))
            .prop_map(move |(hi, lo)| PowerTarget::Range { lo: pct(lo), hi })
            .boxed(),
    );
    // Ramps: var-var, var-lit, lit-var.
    out.push(
        (pick(p), pick(p), 0..=150_000u32, 0..3u8)
            .prop_map(move |((a, _), (b, _), l, form)| match form {
                0 => PowerTarget::Ramp { from: a, to: b },
                1 => PowerTarget::Ramp {
                    from: a,
                    to: pct(l),
                },
                _ => PowerTarget::Ramp {
                    from: pct(l),
                    to: b,
                },
            })
            .boxed(),
    );
    out
}
fn target(pool: &Pool) -> BoxedStrategy<PowerTarget> {
    let pct = |v: u32| Num::Lit(f64::from(v) / 1000.0);
    let mut options: Vec<BoxedStrategy<PowerTarget>> = vec![
        (0..=150_000u32)
            .prop_map(move |v| PowerTarget::PctFtp { value: pct(v) })
            .boxed(),
        (0..=900u32)
            .prop_map(|v| PowerTarget::Watts {
                value: Num::Lit(f64::from(v)),
            })
            .boxed(),
        (30_000..=120_000u32, 0..=30_000u32)
            .prop_map(move |(lo, d)| PowerTarget::Range {
                lo: pct(lo),
                hi: pct(lo + d),
            })
            .boxed(),
        (30_000..=150_000u32, 30_000..=150_000u32)
            .prop_map(move |(from, to)| PowerTarget::Ramp {
                from: pct(from),
                to: pct(to),
            })
            .boxed(),
        (100..=180u32)
            .prop_map(|bpm| PowerTarget::Hr {
                bpm: f64::from(bpm),
            })
            .boxed(),
        Just(PowerTarget::Free).boxed(),
    ];
    if !pool.pct.is_empty() {
        options.extend(pct_var_targets(&pool.pct));
    }
    if !pool.watts.is_empty() {
        options.push(
            pick(&pool.watts)
                .prop_map(|(value, _)| PowerTarget::Watts { value })
                .boxed(),
        );
    }
    proptest::strategy::Union::new(options).boxed()
}
fn end(pool: &Pool) -> BoxedStrategy<StepEnd> {
    let policy = prop_oneof![
        Just(OnUnreachable::LowerPower),
        Just(OnUnreachable::Cap),
        Just(OnUnreachable::AskCoach)
    ];
    let mut options: Vec<BoxedStrategy<StepEnd>> = vec![
        (1..=7200u32)
            .prop_map(|s| StepEnd::Time {
                s: Num::Lit(f64::from(s)),
            })
            .boxed(),
        (1..=300_000u32)
            .prop_map(|kj| StepEnd::Kj {
                kj: f64::from(kj) / 1000.0,
            })
            .boxed(),
        Just(StepEnd::Lap).boxed(),
        (90..=150u32, 0..=120u32, 121..=600u32, policy.clone())
            .prop_map(|(bpm, min, max, on_unreachable)| StepEnd::HrRecovery {
                target_bpm: f64::from(bpm),
                min_s: Num::Lit(f64::from(min)),
                max_s: Num::Lit(f64::from(max)),
                on_unreachable,
            })
            .boxed(),
    ];
    if !pool.secs.is_empty() {
        let s = pool.secs.clone();
        options.push(pick(&s).prop_map(|(s, _)| StepEnd::Time { s }).boxed());
        // hr< bounds: var-var (ordered), var-lit, lit-var.
        options.push(
            (pick(&s), pick(&s), 90..=150u32, 0..3u8, policy)
                .prop_map(|((a, av), (b, bv), bpm, form, on_unreachable)| {
                    let (lo, hi) = if av <= bv { (a, b) } else { (b, a) };
                    let (min_s, max_s) = match form {
                        0 => (lo, hi),
                        1 => (lo, Num::Lit(7200.0)),
                        _ => (Num::Lit(0.0), hi),
                    };
                    StepEnd::HrRecovery {
                        target_bpm: f64::from(bpm),
                        min_s,
                        max_s,
                        on_unreachable,
                    }
                })
                .boxed(),
        );
    }
    proptest::strategy::Union::new(options).boxed()
}
fn nodes(pool: &Pool) -> BoxedStrategy<Vec<WorkoutNode>> {
    let leaf = (end(pool), target(pool), proptest::option::of("[a-z ]{0,8}")).prop_map(
        |(end, target, label)| {
            WorkoutNode::Step(StepNode {
                id: "s".into(),
                label,
                end,
                target,
                cadence: None,
            })
        },
    );
    let count: BoxedStrategy<Num> = if pool.counts.is_empty() {
        (1..=4u32).prop_map(|c| Num::Lit(f64::from(c))).boxed()
    } else {
        prop_oneof![
            (1..=4u32).prop_map(|c| Num::Lit(f64::from(c))),
            proptest::sample::select(pool.counts.clone()).prop_map(Num::var)
        ]
        .boxed()
    };
    let prog = |lits: BoxedStrategy<Num>, names: &[String]| -> BoxedStrategy<Option<Num>> {
        if names.is_empty() {
            proptest::option::of(lits).boxed()
        } else {
            proptest::option::of(prop_oneof![
                lits,
                proptest::sample::select(names.to_vec()).prop_map(Num::var)
            ])
            .boxed()
        }
    };
    let watts = prog(
        (-100..=100i32)
            .prop_map(|w| Num::Lit(f64::from(w) / 10.0))
            .boxed(),
        &pool.prog_watts,
    );
    let pct = prog(
        (-300..=300i32)
            .prop_map(|p| Num::Lit(f64::from(p) / 10.0))
            .boxed(),
        &pool.prog_pct,
    );
    let progression = (watts, pct).prop_map(|(w, p)| {
        (w.is_some() || p.is_some()).then_some(Progression {
            watts_per_rep: w,
            pct_per_rep: p,
        })
    });
    leaf.prop_recursive(3, 32, 3, move |inner| {
        (
            count.clone(),
            proptest::collection::vec(inner, 1..=3),
            progression.clone(),
            any::<bool>(),
        )
            .prop_map(|(count, children, progression, skip)| {
                WorkoutNode::Repeat(RepeatNode {
                    id: "r".into(),
                    label: None,
                    count,
                    children,
                    progression,
                    skip_last_rest: skip.then_some(true),
                })
            })
    })
    .prop_map(|n| vec![n])
    .boxed()
}
fn document() -> impl Strategy<Value = DslDoc> {
    declarations().prop_flat_map(|(vars, pool)| {
        (
            Just(vars),
            proptest::collection::vec(nodes(&pool), 0..=4)
                .prop_map(|groups| groups.into_iter().flatten().collect::<Vec<_>>()),
        )
            .prop_map(|(vars, nodes)| DslDoc { vars, nodes })
    })
}

/// `doc` with `$name` inlined (ADR-0008 §2.8, §2.12): a block's use becomes the literal its
/// use resolves to, a declaration's use the inlined expression (a subtree: parentheses where
/// printing needs them), and the declaration is deleted.
fn inline(doc: &DslDoc, name: &str) -> DslDoc {
    fn fields(original: &mut Value, resolved: &Value, name: &str) {
        if original.get("var").and_then(Value::as_str) == Some(name) {
            *original = resolved.clone();
            return;
        }
        match (original, resolved) {
            (Value::Object(o), Value::Object(r)) => {
                for (k, v) in o.iter_mut() {
                    fields(v, &r[k], name);
                }
            }
            (Value::Array(o), Value::Array(r)) => {
                for (v, rv) in o.iter_mut().zip(r) {
                    fields(v, rv, name);
                }
            }
            _ => {}
        }
    }
    fn exprs(e: &mut Value, name: &str, with: &Value) {
        if e["kind"] == "var" && e["name"] == name {
            *e = with.clone();
            return;
        }
        for key in ["expr", "l", "r"] {
            if let Some(child) = e.get_mut(key) {
                exprs(child, name, with);
            }
        }
    }
    let resolved = resolve_vars(&workout(doc)).expect("resolves");
    let mut nodes = serde_json::to_value(&doc.nodes).expect("serialisable");
    fields(
        &mut nodes,
        &serde_json::to_value(&resolved.nodes).expect("serialisable"),
        name,
    );
    let with = serde_json::to_value(
        &doc.vars
            .iter()
            .find(|d| d.name == name)
            .expect("declared")
            .expr,
    )
    .expect("serialisable");
    let vars = doc
        .vars
        .iter()
        .filter(|d| d.name != name)
        .map(|d| {
            let mut e = serde_json::to_value(&d.expr).expect("serialisable");
            exprs(&mut e, name, &with);
            VarDecl {
                name: d.name.clone(),
                expr: serde_json::from_value(e).expect("serialisable"),
            }
        })
        .collect();
    DslDoc {
        vars,
        nodes: serde_json::from_value(nodes).expect("serialisable"),
    }
}
fn depth(e: &Expr) -> usize {
    match e {
        Expr::Lit { .. } | Expr::Var { .. } => 0,
        Expr::Neg { expr } => 1 + depth(expr),
        Expr::Bin { l, r, .. } => 1 + depth(l).max(depth(r)),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn documents_with_variables_round_trip(doc in document()) {
        let text = print(&doc);
        let back = parse(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
        prop_assert_eq!(&back.vars, &doc.vars, "{}", text);
        prop_assert_eq!(without_ids(&back.nodes), without_ids(&doc.nodes), "{}", text);
        prop_assert_eq!(print(&back), text);
    }

    #[test]
    fn inlining_a_variable_changes_nothing(doc in document()) {
        let compiled = compile(&doc);
        for d in &doc.vars {
            let inlined = inline(&doc, &d.name);
            if inlined.vars.iter().any(|v| depth(&v.expr) > MAX_EXPRESSION_DEPTH) {
                continue; // inlined past the text's depth bound: not a text the DSL can hold
            }
            prop_assert_eq!(&compile(&inlined), &compiled, "${}", d.name);
            let text = print(&inlined);
            let back = parse(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
            prop_assert_eq!(values(&back), values(&doc), "${}\n{}", d.name, text);
        }
    }

    #[test]
    fn the_extended_alphabet_ends_in_ok_or_a_typed_error(
        text in "[$=*/+\\-() 0-9a-z%.,;\n\\[\\]!<\"_]{0,256}"
    ) {
        let _ = parse(&text);
    }

    #[test]
    fn declarations_over_the_extended_alphabet_end_in_ok_or_a_typed_error(
        body in "[$=*/+\\-() 0-9wmsx%.]{0,128}",
        name in "[a-z][a-z0-9_]{0,40}",
    ) {
        let _ = parse(&format!("$b = 3\n${name} = {body}\n5m $b x(1m 50%)"));
        let _ = parse(&format!("${name} = {body}"));
    }
}
