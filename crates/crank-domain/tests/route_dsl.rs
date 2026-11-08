//! Grade-profile text (ADR-0005 §2.2.5, §2.9 R1): grammar, refusals, the route it builds, and
//! the round trip parse → print → parse.
use crank_domain::route::{
    ElevationKind, GradeProfileItem as Item, GradeProfileRepeat, GradeProfileSegment,
    MAX_EXPANDED_LABEL_BYTES, MAX_LABEL_CHARS, MAX_PROFILE_SEGMENTS, MAX_PROFILE_TEXT_BYTES,
    MAX_ROUTE_M, MIN_SEGMENT_M, ProfileGrade, RawPoint, RouteDslErrorKind as K, RouteSource,
    SegmentGrade, Surface, parse_grade_profile, parse_route_dsl, print_grade_profile,
    print_route_dsl, route_from_points,
};
use proptest::prelude::*;
use std::fmt::Write as _;

fn kind(text: &str) -> K {
    match parse_grade_profile(text) {
        Err(e) => e.kind,
        Ok(items) => panic!("{text:?} parsed: {items:?}"),
    }
}

fn message(text: &str) -> String {
    match parse_grade_profile(text) {
        Err(e) => e.message,
        Ok(items) => panic!("{text:?} parsed: {items:?}"),
    }
}

const ADR_EXAMPLE: &str = r#""Flat start" 2km 0%
"Col" 5km 6.5%, 3km ramp 6.5%..9%
3x(0.4km 12% cobbles, 0.6km 2%)
"Descent" 4km -5% gravel"#;

#[test]
fn the_adr_example_parses_and_prints_canonically() {
    let items = parse_grade_profile(ADR_EXAMPLE).expect("the ADR's example");
    assert_eq!(items.len(), 5);
    assert_eq!(
        items[3],
        Item::Repeat(GradeProfileRepeat {
            label: None,
            count: 3,
            children: vec![
                Item::Segment(GradeProfileSegment {
                    label: None,
                    length_m: 400,
                    grade: ProfileGrade::Constant { pct: 12.0 },
                    surface: Some(Surface::Cobbles),
                }),
                Item::Segment(GradeProfileSegment {
                    label: None,
                    length_m: 600,
                    grade: ProfileGrade::Constant { pct: 2.0 },
                    surface: None,
                }),
            ],
        })
    );
    assert_eq!(
        print_grade_profile(&items),
        "\"Flat start\" 2km 0%\n\"Col\" 5km 6.5%\n3km ramp 6.5%..9%\n3x(0.4km 12% cobbles, 0.6km 2%)\n\"Descent\" 4km -5% gravel"
    );
}

#[test]
fn the_route_carries_points_elevation_segments_and_the_canonical_text() {
    let r = parse_route_dsl(ADR_EXAMPLE).expect("the ADR's example");
    assert_eq!(r.name(), "Flat start");
    assert_eq!(r.elevation(), ElevationKind::Exact);
    assert!(r.surface().from_file && r.surface().osm.is_none());
    // 2 + 5 + 3 + 3 × (0.4 + 0.6) + 4 km, one point per segment end plus the start.
    assert_eq!(r.segments().len(), 10);
    assert_eq!(r.points().len(), 11);
    assert_eq!(r.points().last().map(|p| p.dist_m), Some(17_000.0));
    let ele: Vec<f64> = r.points().iter().filter_map(|p| p.ele_m).collect();
    let expected = [
        0.0,
        0.0,
        325.0,
        325.0 + 3_000.0 * 0.0775,
        557.5 + 48.0,
        605.5 + 12.0,
        617.5 + 48.0,
        665.5 + 12.0,
        677.5 + 48.0,
        725.5 + 12.0,
        737.5 - 200.0,
    ];
    for (got, want) in ele.iter().zip(expected) {
        assert!((got - want).abs() < 1e-9, "{ele:?}");
    }
    assert!(r.points().iter().all(|p| p.lat_lon.is_none()));
    assert_eq!(r.segments()[1].label.as_deref(), Some("Col"));
    assert_eq!(r.segments()[2].label, None);
    assert_eq!(
        r.segments()[2].grade,
        SegmentGrade::Ramp {
            from_pct: 6.5,
            to_pct: 9.0
        }
    );
    assert_eq!(
        (r.segments()[3].start_m, r.segments()[3].end_m),
        (10_000.0, 10_400.0)
    );
    assert_eq!(r.segments()[3].surface, Some(Surface::Cobbles));
    assert_eq!(r.segments()[9].surface, Some(Surface::Gravel));
    let RouteSource::GradeProfile { dsl } = r.source() else {
        panic!("{:?}", r.source())
    };
    assert_eq!(
        dsl,
        &print_grade_profile(&parse_grade_profile(ADR_EXAMPLE).expect("parses"))
    );
    assert_eq!(parse_route_dsl(dsl).as_ref(), Ok(&r));
}

#[test]
fn a_repeat_label_names_its_unlabelled_segments_and_an_unlabelled_profile_has_a_default_name() {
    let r = parse_route_dsl("\"Hills\" 2x(1km 5%, \"Top\" 0.5km 0%)").expect("parses");
    let labels: Vec<_> = r.segments().iter().map(|s| s.label.as_deref()).collect();
    assert_eq!(
        labels,
        [Some("Hills"), Some("Top"), Some("Hills"), Some("Top")]
    );
    assert_eq!(r.name(), "Hills");
    assert_eq!(
        parse_route_dsl("1km 1%").expect("parses").name(),
        "Grade profile"
    );
    assert_eq!(
        parse_route_dsl("1km 1%, \"Late\" 1km 0%")
            .expect("parses")
            .name(),
        "Late"
    );
}

#[test]
fn metres_are_refused_with_the_minutes_message() {
    assert_eq!(kind("500m 3%"), K::MetresNotKm);
    assert_eq!(
        message("500m 3%"),
        "routes are measured in km: write 0.5km ('m' is minutes in a workout)"
    );
    assert_eq!(kind("4M 2%"), K::MetresNotKm);
}

#[test]
fn text_pasted_from_a_workout_fails_on_its_first_token() {
    for (text, k) in [
        ("30s 120%", K::WorkoutStepEnd),
        ("1h 60%", K::WorkoutStepEnd),
        ("1m30s 60%", K::WorkoutStepEnd),
        ("10kj 200w", K::WorkoutStepEnd),
        ("lap free", K::WorkoutStepEnd),
        ("hr<120 2m..4m 50%", K::WorkoutStepEnd),
        ("2km 250w", K::WorkoutTarget),
        ("2km 140bpm", K::WorkoutTarget),
        ("2km free", K::WorkoutTarget),
        ("2km 50-75%", K::WorkoutTarget),
        ("3x[+5w](1km 2%)", K::RepeatOptions),
    ] {
        assert_eq!(kind(text), k, "{text}");
    }
    let e = parse_grade_profile("1km 2%\n4m 120%").expect_err("minutes");
    assert_eq!((e.kind, e.offset, e.length), (K::MetresNotKm, 7, 2));
}

#[test]
fn grade_and_length_limits() {
    assert_eq!(kind("1km 31%"), K::ImplausibleGrade { pct: 31.0 });
    assert_eq!(kind("1km -25.5%"), K::ImplausibleGrade { pct: -25.5 });
    assert_eq!(kind("1km ramp 0%..40%"), K::ImplausibleGrade { pct: 40.0 });
    assert!(parse_grade_profile("1km 30%, 1km -25%, 1km +3%").is_ok());
    assert_eq!(kind("0.009km 1%"), K::SegmentLength);
    assert_eq!(kind("100.001km 1%"), K::SegmentLength);
    assert_eq!(kind("9999999999km 1%"), K::SegmentLength);
    assert!(parse_grade_profile("0.01km 1%, 100km 1%").is_ok());
    assert_eq!(kind("0.0005km 1%"), K::DistanceTooPrecise);
    assert_eq!(kind("2 3%"), K::MissingUnit);
    assert_eq!(kind("far 3%"), K::InvalidDistance);
    assert_eq!(kind("2km"), K::MissingGrade);
    assert_eq!(kind("2km steep"), K::MissingGrade);
    assert_eq!(kind("2km ramp 3%"), K::MissingRampRange);
    assert_eq!(kind("2km ramp"), K::MissingRampRange);
    assert_eq!(kind("2km 3% gravel sand"), K::DuplicateSurface);
    assert_eq!(kind("2km 3% tarmac"), K::UnknownToken);
}

#[test]
fn the_route_is_at_most_1000_km_in_all() {
    assert!(parse_grade_profile("10x(100km 0%)").is_ok());
    assert_eq!(kind("10x(100km 0%), 0.01km 0%"), K::RouteTooLong);
    assert_eq!(kind("2x(10x(50km 0%), 0.01km 1%)"), K::RouteTooLong);
    assert_eq!(kind("100001x(0.01km 0%)"), K::RouteTooLong);
    assert_eq!(kind("99999999999999999999999x(0.01km 0%)"), K::RouteTooLong);
}

#[test]
fn repeats_nest_32_levels_and_no_deeper() {
    let nest = |n: usize| format!("{}0.01km 1%{}", "1x(".repeat(n), ")".repeat(n));
    assert!(parse_grade_profile(&nest(32)).is_ok());
    assert_eq!(kind(&nest(33)), K::NestingTooDeep { max: 32 });
}

#[test]
fn structure_errors_match_the_workout_dsl() {
    assert_eq!(kind("\"open"), K::UnterminatedLabel);
    assert_eq!(kind("2x(1km 1%"), K::UnclosedRepeat);
    assert_eq!(kind("1km 1%)"), K::UnexpectedToken);
    assert_eq!(kind("\"label\""), K::MissingItemAfterLabel);
    assert_eq!(kind("0x(1km 1%)"), K::InvalidRepeatCount);
    assert_eq!(kind("2x 1km 1%"), K::MissingRepeatBody);
    assert_eq!(kind("2x()"), K::EmptyRepeat);
    assert_eq!(kind("1km 1% \"b\" 1km 2%"), K::MissingSeparator);
    assert_eq!(
        kind(&format!("1km {}%", "9".repeat(400))),
        K::NumericOverflow
    );
    assert_eq!(
        kind(&format!("{}km 1%", "9".repeat(400))),
        K::NumericOverflow
    );
    assert_eq!(
        parse_route_dsl("  # nothing\n").expect_err("empty").kind,
        K::Empty
    );
    assert_eq!(parse_grade_profile("# only a comment"), Ok(vec![]));
    // UTF-16 offsets, as in the workout DSL.
    let e = parse_grade_profile("\"😀\" 1km 1% x").expect_err("unknown");
    assert_eq!((e.offset, e.length), (12, 1));
}

#[test]
fn the_text_is_at_most_256_kib() {
    let pad = |n: usize| format!("1km 1%\n#{}", "x".repeat(n - "1km 1%\n#".len()));
    assert!(parse_route_dsl(&pad(MAX_PROFILE_TEXT_BYTES)).is_ok());
    assert_eq!(kind(&pad(MAX_PROFILE_TEXT_BYTES + 1)), K::TextTooLong);
}

#[test]
fn a_label_is_at_most_100_characters() {
    let label = |n: usize| format!("\"{}\" 1km 1%", "é".repeat(n));
    assert!(parse_grade_profile(&label(MAX_LABEL_CHARS)).is_ok());
    assert_eq!(kind(&label(MAX_LABEL_CHARS + 1)), K::LabelTooLong);
    // The falsifier's 10 KB text that peaked at 1.1 GB.
    let bomb = format!("100000x(\"{}\" 0.01km 0%)", "a".repeat(10_000));
    assert_eq!(kind(&bomb), K::LabelTooLong);
}

#[test]
fn expanded_labels_are_bounded_before_expanding() {
    // 64-byte labels: 16 384 of them are exactly 1 MiB.
    let label = "a".repeat(64);
    let rep = |n: u64| format!("{n}x(\"{label}\" 0.01km 0%)");
    let fits = MAX_EXPANDED_LABEL_BYTES / 64;
    assert_eq!(fits * 64, MAX_EXPANDED_LABEL_BYTES);
    let r = parse_route_dsl(&rep(fits)).expect("just under the bound");
    assert_eq!(u64::try_from(r.segments().len()).ok(), Some(fits));
    let over = parse_route_dsl(&rep(fits + 1)).expect_err("just over the bound");
    assert_eq!(over.kind, K::LabelsTooLarge);
    // An inherited repeat label counts once per segment it names.
    let inherited = |n: u64| format!("\"{label}\" {n}x(0.01km 0%)");
    assert!(parse_route_dsl(&inherited(fits)).is_ok());
    assert_eq!(
        parse_route_dsl(&inherited(fits + 1))
            .expect_err("over")
            .kind,
        K::LabelsTooLarge
    );
    // 100 000 × 100 bytes would be 10 MB of copies: refused before any is made.
    assert_eq!(
        parse_route_dsl(&format!("100000x(\"{}\" 0.01km 0%)", "a".repeat(100)))
            .expect_err("over")
            .kind,
        K::LabelsTooLarge
    );
}

#[test]
fn the_segment_bound_is_the_route_bound() {
    assert_eq!(
        u32::try_from(MAX_PROFILE_SEGMENTS).map(f64::from).ok(),
        Some(MAX_ROUTE_M / f64::from(MIN_SEGMENT_M))
    );
    let r = parse_route_dsl("100000x(0.01km 0%)").expect("1 000 km of 10 m segments");
    assert_eq!(
        u64::try_from(r.segments().len()).ok(),
        Some(MAX_PROFILE_SEGMENTS)
    );
}

#[test]
fn a_blank_first_label_does_not_name_the_route() {
    assert_eq!(
        parse_route_dsl("\"\" 1km 1%, \" \" 1km 0%")
            .expect("parses")
            .name(),
        "Grade profile"
    );
    assert_eq!(
        parse_route_dsl("\" \" 1km 1%, \"Top\" 1km 0%")
            .expect("parses")
            .name(),
        "Top"
    );
}

#[test]
fn print_route_dsl_is_none_for_a_file_route() {
    let r = parse_route_dsl("1km 1%").expect("parses");
    assert_eq!(print_route_dsl(&r).as_deref(), Some("1km 1%"));
    let raw: Vec<RawPoint> = [46.0, 46.01]
        .iter()
        .map(|&lat| RawPoint {
            at: 0,
            lat,
            lon: 8.0,
            ele_m: None,
            surface: None,
        })
        .collect();
    let file = route_from_points("f".into(), RouteSource::GpxTrack, &raw).expect("imports");
    assert_eq!(print_route_dsl(&file.route), None);
}

// --- property tests: every text round-trips ---

fn grade() -> impl Strategy<Value = ProfileGrade> {
    let pct = prop_oneof![
        (-25i32..=30).prop_map(f64::from),
        (-2500i32..=3000).prop_map(|c| f64::from(c) / 100.0),
        -25.0..=30.0f64,
    ];
    prop_oneof![
        pct.clone().prop_map(|pct| ProfileGrade::Constant { pct }),
        (pct.clone(), pct).prop_map(|(from_pct, to_pct)| ProfileGrade::Ramp { from_pct, to_pct }),
    ]
}

fn label() -> impl Strategy<Value = Option<String>> {
    proptest::option::of("[a-zA-Z0-9 é😀#,;()\\[\\]\"\\\\%.-]{0,8}")
}

fn segment() -> impl Strategy<Value = Item> {
    (
        label(),
        prop_oneof![
            10u32..=100,
            10u32..=2_000,
            (1u32..=20).prop_map(|k| k * 1000)
        ],
        grade(),
        proptest::option::of(proptest::sample::select(Surface::ALL.to_vec())),
    )
        .prop_map(|(label, length_m, grade, surface)| {
            Item::Segment(GradeProfileSegment {
                label,
                length_m,
                grade,
                surface,
            })
        })
}

/// At most 3 × 3 × 3 × 3 × 3 × 20 km = 4 860 km would be too long, so leaves are short enough
/// in practice; texts over 1 000 km are discarded, not counted as failures.
fn profile() -> impl Strategy<Value = Vec<Item>> {
    let item = segment().prop_recursive(3, 48, 3, |inner| {
        (label(), 1u32..=3, proptest::collection::vec(inner, 1..=3)).prop_map(
            |(label, count, children)| {
                Item::Repeat(GradeProfileRepeat {
                    label,
                    count,
                    children,
                })
            },
        )
    });
    proptest::collection::vec(item, 0..=4)
}

/// Deterministic formatting choices from a seed (xorshift64*).
struct Style(u64);
impl Style {
    fn pick(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n
    }
    fn case(&mut self, s: &str) -> String {
        if self.pick(3) == 0 {
            s.to_ascii_uppercase()
        } else {
            s.to_owned()
        }
    }
    fn space(&mut self) -> &'static str {
        ["", " ", "  ", "\t", " # comment\n"][usize::try_from(self.pick(5)).unwrap_or(0)]
    }
    fn label_gap(&mut self) -> &'static str {
        ["", " ", "\t"][usize::try_from(self.pick(3)).unwrap_or(0)]
    }
    fn gap(&mut self) -> &'static str {
        [" ", "  ", "\t"][usize::try_from(self.pick(3)).unwrap_or(0)]
    }
    fn sep(&mut self) -> &'static str {
        [",", ";", "\n", ", ", " ;\n", "\n\n# c\n"][usize::try_from(self.pick(6)).unwrap_or(0)]
    }
    fn pct(&mut self, pct: f64) -> String {
        let plus = if pct >= 0.0 && self.pick(4) == 0 {
            "+"
        } else {
            ""
        };
        let mut n = format!("{}", pct + 0.0);
        if self.pick(3) == 0 {
            n.push_str(if n.contains('.') { "0" } else { ".0" });
        }
        format!("{plus}{n}%")
    }
    fn km(&mut self, m: u32) -> String {
        let text = match self.pick(3) {
            0 => format!("{}.{:03}", m / 1000, m % 1000),
            1 if m.is_multiple_of(1000) => format!("{}", m / 1000),
            _ => format!("{:03}.{:03}", m / 1000, m % 1000),
        };
        format!("{text}{}", self.case("km"))
    }
}

fn write_label(style: &mut Style, label: Option<&String>) -> String {
    label.map_or_else(String::new, |l| {
        format!(
            "\"{}\"{}",
            l.replace('\\', "\\\\").replace('"', "\\\""),
            style.label_gap()
        )
    })
}

/// A non-canonical but equivalent text of `items`.
fn write(style: &mut Style, items: &[Item]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(style.sep());
        }
        out.push_str(style.space());
        match item {
            Item::Segment(s) => {
                out.push_str(&write_label(style, s.label.as_ref()));
                out.push_str(&style.km(s.length_m));
                out.push_str(style.gap());
                match s.grade {
                    ProfileGrade::Constant { pct } => out.push_str(&style.pct(pct)),
                    ProfileGrade::Ramp { from_pct, to_pct } => {
                        let ramp = style.case("ramp");
                        let gap = style.gap();
                        let (from, to) = (style.pct(from_pct), style.pct(to_pct));
                        write!(out, "{ramp}{gap}{from}..{to}").expect("write to a String");
                    }
                }
                if let Some(surface) = s.surface {
                    out.push_str(style.gap());
                    out.push_str(&style.case(surface.name()));
                }
            }
            Item::Repeat(r) => {
                out.push_str(&write_label(style, r.label.as_ref()));
                let x = style.case("x");
                let open = style.space();
                let body = write(style, &r.children);
                let close = style.space();
                write!(out, "{}{x}({open}{body}{close})", r.count).expect("write to a String");
            }
        }
        out.push_str(style.space());
    }
    out
}

fn total_m(items: &[Item]) -> u64 {
    items
        .iter()
        .map(|i| match i {
            Item::Segment(s) => u64::from(s.length_m),
            Item::Repeat(r) => u64::from(r.count) * total_m(&r.children),
        })
        .sum()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn every_text_round_trips_and_printing_is_idempotent(items in profile(), seed in 1u64..) {
        prop_assume!(total_m(&items) <= 1_000_000);
        let text = write(&mut Style(seed), &items);
        // parse → the items the text was written from
        let parsed = parse_grade_profile(&text).map_err(|e| TestCaseError::fail(format!("{text:?}: {e}")))?;
        prop_assert_eq!(&parsed, &items, "{:?}", text);
        // parse → print → parse equal, and print idempotent
        let printed = print_grade_profile(&parsed);
        let reparsed = parse_grade_profile(&printed).map_err(|e| TestCaseError::fail(format!("{printed:?}: {e}")))?;
        prop_assert_eq!(&reparsed, &parsed);
        prop_assert_eq!(print_grade_profile(&reparsed), printed.clone());
        // the same at the route level (ADR-0005 §2.1 "Round trip")
        if !items.is_empty() {
            let route = parse_route_dsl(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
            let dsl = print_route_dsl(&route).ok_or_else(|| TestCaseError::fail("no dsl"))?;
            prop_assert_eq!(&dsl, &printed);
            prop_assert_eq!(parse_route_dsl(&dsl), Ok(route));
        }
    }

    #[test]
    fn route_points_are_strictly_increasing_from_zero(items in profile()) {
        prop_assume!(!items.is_empty() && total_m(&items) <= 1_000_000);
        let route = parse_route_dsl(&print_grade_profile(&items)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(route.points()[0].dist_m.to_bits(), 0f64.to_bits());
        prop_assert!(route.points().windows(2).all(|w| w[1].dist_m > w[0].dist_m));
        prop_assert_eq!(route.points().len(), route.segments().len() + 1);
        let total = u32::try_from(total_m(&items)).map(f64::from).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(route.points().last().map(|p| p.dist_m), Some(total));
    }
}
