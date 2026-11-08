//! Round 4 of #539: a grade profile's stored text is its one source of truth, every label prints
//! to text that reads back as itself, and every GPX element stands where the GPX 1.0/1.1 schema
//! content model puts it.
use crank_domain::route::{
    ElevationKind, GpxImport, GradeProfileItem, GradeProfileSegment, MAX_LABEL_CHARS,
    MAX_PROFILE_TEXT_BYTES, ProfileGrade, Route, RouteImportError, RouteImportOptions,
    RouteSegment, RouteSource, SegmentGrade, import_gpx, parse_grade_profile, parse_route_dsl,
    print_grade_profile, print_route_dsl,
};
use crank_domain::workout::{WorkoutNode, parse_dsl, print_dsl};
use proptest::prelude::*;
use serde_json::json;

// --- 1: the stored grade-profile text is checked ---

fn profile(text: &str) -> Route {
    parse_route_dsl(text).expect("parses")
}

fn with_dsl(route: &Route, dsl: &str) -> Result<Route, String> {
    let mut parts = route.clone().into_parts();
    parts.source = RouteSource::GradeProfile { dsl: dsl.into() };
    Route::from_parts(parts).map_err(|e| e.what)
}

#[test]
fn a_profile_route_must_be_what_its_text_gives() {
    let r = profile("1km 5%");
    assert_eq!(with_dsl(&r, "1km 5%").as_ref(), Ok(&r));
    // The falsifier's three probes.
    let other = with_dsl(&r, "2km 0%").expect_err("other geometry");
    assert!(
        other.contains("not what its grade-profile text gives"),
        "{other}"
    );
    let junk = with_dsl(&r, "not a profile ((").expect_err("junk");
    assert!(junk.contains("does not parse"), "{junk}");
    let mut v = serde_json::to_value(&r).expect("serialises");
    v["source"]["dsl"] = json!("2km 0%");
    assert!(serde_json::from_value::<Route>(v).is_err(), "edited JSON");
    // The stored text is the canonical one: the same geometry written otherwise is refused.
    assert!(with_dsl(&r, "1KM  +5%").is_err());
    // The name is the text's too.
    let mut parts = r.clone().into_parts();
    parts.name = "Renamed".into();
    assert!(Route::from_parts(parts).is_err());
    // Bit for bit: −0 m where the text gives +0 m.
    let mut parts = r.clone().into_parts();
    parts.points[0].ele_m = Some(-0.0);
    assert!(Route::from_parts(parts).is_err());
    let mut parts = r.into_parts();
    parts.segments[0].grade = SegmentGrade::Constant { pct: 5.000_000_1 };
    assert!(Route::from_parts(parts).is_err());
}

#[test]
fn every_parsed_profile_passes_check_and_round_trips_through_serde() {
    for text in [
        "\"Flat start\" 2km 0%\n\"Col\" 5km 6.5%, 3km ramp 6.5%..9%",
        "3x(0.4km 12% cobbles, 0.6km 2%)\n\"Descent\" 4km -5% gravel",
        "\"a\\\nb\" 1km 1%",
    ] {
        let r = profile(text);
        assert_eq!(r.check(), Ok(()));
        let v = serde_json::to_value(&r).expect("serialises");
        assert_eq!(serde_json::from_value::<Route>(v).ok().as_ref(), Some(&r));
    }
}

#[test]
fn a_grade_profile_without_segments_is_refused() {
    let mut parts = profile("1km 5%").into_parts();
    parts.segments.clear();
    let what = Route::from_parts(parts).expect_err("no segments").what;
    assert!(what.contains("without segments"), "{what}");
}

fn file_route() -> Route {
    let text = r#"<gpx xmlns="http://www.topografix.com/GPX/1/1"><trk><name>T</name><trkseg>
<trkpt lat="-90" lon="8"><ele>1</ele></trkpt><trkpt lat="-89.999" lon="8"><ele>2</ele></trkpt>
</trkseg></trk></gpx>"#;
    match import_gpx(text.as_bytes(), "t.gpx", &RouteImportOptions::default()) {
        Ok(GpxImport::Imported(r)) => r.route,
        other => panic!("{other:?}"),
    }
}

#[test]
fn exact_elevation_is_a_grade_profile_s_and_only_its() {
    // ADR-0005 §2.1: `elevation: Recorded | None (§2.2.2) | Exact (grade profile)`.
    let mut parts = file_route().into_parts();
    parts.elevation = ElevationKind::Exact;
    let what = Route::from_parts(parts).expect_err("Exact on a file").what;
    assert!(what.contains("Exact elevation for a file route"), "{what}");
    for kind in [ElevationKind::Recorded, ElevationKind::None] {
        let mut parts = profile("1km 5%").into_parts();
        parts.elevation = kind;
        if kind == ElevationKind::None {
            for p in &mut parts.points {
                p.ele_m = None;
            }
        }
        let what = Route::from_parts(parts).expect_err("not Exact").what;
        assert!(what.contains("for a grade-profile route"), "{what}");
    }
}

#[test]
fn segment_grades_match_the_source() {
    let mut parts = file_route().into_parts();
    let end = parts.points[1].dist_m;
    parts.segments.push(RouteSegment {
        start_m: 0.0,
        end_m: end,
        label: Some("Climb".into()),
        grade: SegmentGrade::FromPoints,
        surface: None,
    });
    let file = Route::from_parts(parts.clone()).expect("a labelled stretch of a file");
    parts.segments[0].grade = SegmentGrade::Constant { pct: 1.0 };
    assert!(
        Route::from_parts(parts).is_err(),
        "a stated grade on a file route"
    );
    let mut parts = profile("1km 5%").into_parts();
    parts.segments[0].grade = SegmentGrade::FromPoints;
    let what = Route::from_parts(parts)
        .expect_err("FromPoints in a profile")
        .what;
    assert!(what.contains("segment 0"), "{what}");
    // O8: a segment starts at 0 m or later.
    let mut parts = file.clone().into_parts();
    parts.segments[0].start_m = -f64::MIN_POSITIVE;
    let what = Route::from_parts(parts).expect_err("before the start").what;
    assert!(what.contains("segment 0"), "{what}");
    let mut parts = file.into_parts();
    parts.segments[0].start_m = 0.0;
    assert!(Route::from_parts(parts).is_ok());
}

#[test]
fn latitude_is_within_minus_90_and_90() {
    // O7: the lower bound; the file route's first point is at −90° exactly.
    let r = file_route();
    assert_eq!(r.points()[0].lat_lon, Some((-90.0, 8.0)));
    let mut parts = r.into_parts();
    parts.points[0].lat_lon = Some((-90.000_000_1, 8.0));
    let what = Route::from_parts(parts).expect_err("below −90°").what;
    assert!(what.contains("point 0: position"), "{what}");
}

#[test]
fn a_profile_grade_must_be_finite_before_its_text_is_read() {
    // O14: the shape check names the segment; the text's disagreement alone would not.
    for grade in [
        SegmentGrade::Ramp {
            from_pct: f64::NAN,
            to_pct: 1.0,
        },
        SegmentGrade::Ramp {
            from_pct: 1.0,
            to_pct: f64::INFINITY,
        },
        SegmentGrade::Constant { pct: f64::NAN },
    ] {
        let mut parts = profile("1km ramp 1%..2%").into_parts();
        parts.segments[0].grade = grade;
        let what = Route::from_parts(parts).expect_err("not finite").what;
        assert!(what.contains("segment 0"), "{grade:?}: {what}");
    }
}

#[test]
fn a_canonical_form_of_exactly_the_limit_is_accepted() {
    // O1: 23 831 segments print to 262 143 bytes; one grade written `10%` makes it 262 144.
    let text = format!("1x(0.01km 10%,{})", vec!["0.01km 1%"; 23_830].join(","));
    let items = parse_grade_profile(&text).expect("exactly the limit");
    let printed = print_grade_profile(&items);
    assert_eq!(printed.len(), MAX_PROFILE_TEXT_BYTES);
    let route = parse_route_dsl(&printed).expect("parses");
    assert_eq!(route.check(), Ok(()));
    assert_eq!(print_route_dsl(&route).as_deref(), Some(printed.as_str()));
}

// --- 2: every label prints back as itself ---

#[test]
fn a_label_with_an_escaped_newline_prints_back() {
    for text in [
        "\"a\\\nb\" 1km 1%",
        "\"a\\\nb\" 2x(1km 1%)",
        "\"q\\\"\\\\\" 1km 1%",
    ] {
        let items = parse_grade_profile(text).expect("parses");
        let printed = print_grade_profile(&items);
        assert_eq!(
            parse_grade_profile(&printed).as_ref(),
            Ok(&items),
            "{printed:?}"
        );
        assert_eq!(print_grade_profile(&items), printed);
        let r = profile(text);
        assert_eq!(
            parse_route_dsl(&print_route_dsl(&r).expect("profile")).as_ref(),
            Ok(&r)
        );
    }
    let workout = parse_dsl(
        "\"a\\\nb\" 1m 50%, \"c\\\nd\" 2x(1m 50%)",
        250.0,
        &mut String::new,
    )
    .expect("parses");
    let printed = print_dsl(&workout);
    let back = parse_dsl(&printed, 250.0, &mut String::new).expect("reparses");
    assert_eq!(labels(&back), labels(&workout), "{printed:?}");
    assert_eq!(
        labels(&back),
        vec![Some("a\nb".to_owned()), Some("c\nd".to_owned())]
    );
}

fn labels(nodes: &[WorkoutNode]) -> Vec<Option<String>> {
    nodes
        .iter()
        .map(|n| match n {
            WorkoutNode::Step(s) => s.label.clone(),
            WorkoutNode::Repeat(r) => r.label.clone(),
        })
        .collect()
}

/// Labels of any characters, weighted towards the ones the lexer treats specially: control
/// characters, quotes, backslashes, newlines and characters outside the BMP.
fn label() -> impl Strategy<Value = String> {
    let c = prop_oneof![
        3 => proptest::sample::select(vec!['"', '\\', '\n', '\r', '\t', '\0', '#', ',', '(', ')']),
        1 => proptest::char::range('\u{0}', '\u{1f}'),
        1 => proptest::char::range('\u{1F300}', '\u{1FAFF}'),
        2 => any::<char>(),
    ];
    proptest::collection::vec(c, 0..=MAX_LABEL_CHARS).prop_map(|v| v.into_iter().collect())
}

proptest! {
    #[test]
    fn print_then_parse_keeps_any_label(a in label(), b in label()) {
        let items = vec![
            GradeProfileItem::Segment(GradeProfileSegment {
                label: Some(a.clone()),
                length_m: 1000,
                grade: ProfileGrade::Constant { pct: 1.0 },
                surface: None,
            }),
            GradeProfileItem::Repeat(crank_domain::route::GradeProfileRepeat {
                label: Some(b.clone()),
                count: 2,
                children: vec![GradeProfileItem::Segment(GradeProfileSegment {
                    label: None,
                    length_m: 500,
                    grade: ProfileGrade::Ramp { from_pct: 1.0, to_pct: 2.0 },
                    surface: None,
                })],
            }),
        ];
        let printed = print_grade_profile(&items);
        prop_assert_eq!(parse_grade_profile(&printed), Ok(items));
        let mut workout = parse_dsl("\"x\" 1m 50%, \"y\" 2x(1m 50%)", 250.0, &mut String::new)
            .expect("parses");
        for (node, l) in workout.iter_mut().zip([&a, &b]) {
            match node {
                WorkoutNode::Step(s) => s.label = Some(l.clone()),
                WorkoutNode::Repeat(r) => r.label = Some(l.clone()),
            }
        }
        let back = parse_dsl(&print_dsl(&workout), 250.0, &mut String::new).expect("reparses");
        prop_assert_eq!(labels(&back), vec![Some(a), Some(b)]);
    }
}

// --- 3: GPX elements stand where the schema's content model puts them ---

const GPX11: &str = "http://www.topografix.com/GPX/1/1";
const GPX10: &str = "http://www.topografix.com/GPX/1/0";

fn doc(ns: &str, body: &str) -> String {
    format!(r#"<gpx version="1.1" creator="t" xmlns="{ns}">{body}</gpx>"#)
}

fn import(text: &str, track: Option<usize>) -> Result<GpxImport, RouteImportError> {
    import_gpx(text.as_bytes(), "x.gpx", &RouteImportOptions { track })
}

fn malformed(text: &str) -> String {
    match import(text, None) {
        Err(RouteImportError::Malformed { what, .. }) => what,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn imported(text: &str) -> Route {
    match import(text, None) {
        Ok(GpxImport::Imported(r)) => r.route,
        other => panic!("expected an import, got {other:?}"),
    }
}

fn pt(lat: f64) -> String {
    format!(r#"<trkpt lat="{lat}" lon="8.0"/>"#)
}

#[test]
fn the_falsifier_s_nested_documents_are_malformed() {
    let real = format!(
        "<trk><name>Real</name><trkseg>{}{}</trkseg></trk>",
        pt(47.0),
        pt(47.001)
    );
    let hidden = format!(
        "<metadata><gpx><trk><name>Hidden</name><trkseg>{}{}</trkseg></trk></gpx></metadata>",
        pt(10.0),
        pt(10.001)
    );
    for (body, want) in [
        (
            hidden.clone(),
            "<gpx> is not allowed in <metadata> (GPX 1.1 metadataType)",
        ),
        (
            format!("{hidden}{real}"),
            "<gpx> is not allowed in <metadata>",
        ),
        (
            format!(
                r#"<trk><name>Real</name><trkseg>{}<trkpt lat="47.0005" lon="8.0"><gpx><rte><name>Ghost</name></rte></gpx></trkpt>{}</trkseg></trk>"#,
                pt(47.0),
                pt(47.001)
            ),
            "<gpx> is not allowed in <trkpt> (GPX 1.1 wptType)",
        ),
        (
            format!(
                r#"<trk><trkseg>{}<trkpt lat="47.0005" lon="8.0"><trkpt lat="47.0007" lon="8.0"/></trkpt>{}</trkseg></trk>"#,
                pt(47.0),
                pt(47.001)
            ),
            "<trkpt> is not allowed in <trkpt>",
        ),
        (
            format!(r#"<wpt lat="1" lon="1"><gpx>{real}</gpx></wpt>"#),
            "<gpx> is not allowed in <wpt>",
        ),
        (
            format!(
                r#"<trk><trkseg><trkpt lat="47" lon="8"><desc><ele>500</ele></desc></trkpt>{}</trkseg></trk>"#,
                pt(47.001)
            ),
            "<ele> is not allowed in <desc> (GPX 1.1 simple type, no child elements)",
        ),
        (
            format!(
                r#"<trk><trkseg><trkpt lat="47" lon="8"><surface>gravel</surface></trkpt>{}</trkseg></trk>"#,
                pt(47.001)
            ),
            "<surface> is not allowed in <trkpt>",
        ),
        (
            format!("{real}<gpx/>"),
            "<gpx> is not allowed in <gpx> (GPX 1.1 gpxType)",
        ),
        (
            format!(
                "<trk><name>Real<b>!</b></name><trkseg>{}{}</trkseg></trk>",
                pt(47.0),
                pt(47.001)
            ),
            "<b> is not allowed in <name>",
        ),
    ] {
        let what = malformed(&doc(GPX11, &body));
        assert!(what.contains(want), "{body}\n  -> {what}");
    }
    // A child element in a simple-typed element is refused in any namespace.
    let foreign_in_desc = doc(
        GPX11,
        &format!(
            r#"<trk><desc><o:x xmlns:o="urn:o"/></desc><trkseg>{}{}</trkseg></trk>"#,
            pt(47.0),
            pt(47.001)
        ),
    );
    assert!(malformed(&foreign_in_desc).contains("extensionsType"));
    let nested_10 = doc(
        GPX10,
        &format!(
            "<trk><trkseg>{}<trkpt lat=\"47.0005\" lon=\"8\"><gpx/></trkpt>{}</trkseg></trk>",
            pt(47.0),
            pt(47.001)
        ),
    );
    assert!(malformed(&nested_10).contains("GPX 1.0"));
}

#[test]
fn the_document_name_comes_from_where_the_schema_puts_it() {
    let seg = format!("<trk><trkseg>{}{}</trkseg></trk>", pt(47.0), pt(47.001));
    // O4: a 1.1 root has no <name>; its document name is the metadata's.
    let what = malformed(&doc(GPX11, &format!("<name>Root</name>{seg}")));
    assert!(
        what.contains("<name> is not allowed in <gpx> (GPX 1.1 gpxType)"),
        "{what}"
    );
    let meta = doc(
        GPX11,
        &format!("<metadata><name>Meta</name><author><name>Author</name></author></metadata>{seg}"),
    );
    assert_eq!(imported(&meta).name(), "Meta");
    let author_only = doc(
        GPX11,
        &format!("<metadata><author><name>Author</name></author></metadata>{seg}"),
    );
    assert_eq!(
        imported(&author_only).name(),
        "x",
        "an author's name is not the route's"
    );
    // GPX 1.0 has no metadata: the root's <name> is the document's.
    assert_eq!(
        imported(&doc(GPX10, &format!("<name>Root</name>{seg}"))).name(),
        "Root"
    );
    assert!(malformed(&doc(GPX10, &format!("<metadata/>{seg}"))).contains("GPX 1.0"));
    // Each name is read once.
    let two = doc(
        GPX11,
        &format!(
            "<trk><name>A</name><name>B</name><trkseg>{}{}</trkseg></trk>",
            pt(47.0),
            pt(47.001)
        ),
    );
    assert!(malformed(&two).contains("a second <name> in one <trk>"));
    let two_meta = doc(
        GPX11,
        &format!("<metadata><name>A</name><name>B</name></metadata>{seg}"),
    );
    assert!(malformed(&two_meta).contains("a second <name> in one <metadata>"));
}

#[test]
fn extensions_are_gpx_1_1_only() {
    // O12: GPX 1.0 has no <extensions>; its foreign elements stand directly in their parent.
    let ext = |ns| {
        doc(
            ns,
            &format!(
                r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions><o:surface xmlns:o="urn:o">gravel</o:surface></extensions></trkpt>{}</trkseg></trk>"#,
                pt(47.001)
            ),
        )
    };
    assert_eq!(
        imported(&ext(GPX11)).points()[0].surface,
        Some(crank_domain::route::Surface::Gravel)
    );
    let what = malformed(&ext(GPX10));
    assert!(
        what.contains("<extensions> is not allowed in <trkpt> (GPX 1.0 wptType)"),
        "{what}"
    );
}

// Round 5 replaced `##other` inside <extensions>: everything there is extension data, skipped
// whatever its namespace (`route_r5::extension_content_is_never_refused_but_must_be_well_formed`).

#[test]
fn an_undeclared_prefix_is_malformed_wherever_it_stands() {
    // O5: inside <extensions> a foreign element is skipped, so only the prefix check refuses it.
    let in_ext = doc(
        GPX11,
        &format!(
            r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions><y:q/></extensions></trkpt>{}</trkseg></trk>"#,
            pt(47.001)
        ),
    );
    assert!(malformed(&in_ext).contains("prefix 'y' is not declared"));
    let deep = doc(
        GPX11,
        &format!(
            r#"<trk><extensions><o:a xmlns:o="urn:o"><y:q/></o:a></extensions><trkseg>{}{}</trkseg></trk>"#,
            pt(47.0),
            pt(47.001)
        ),
    );
    assert!(malformed(&deep).contains("prefix 'y' is not declared"));
    let attr = doc(
        GPX11,
        &format!(
            r#"<trk><trkseg><trkpt y:q="1" lat="47" lon="8"/>{}</trkseg></trk>"#,
            pt(47.001)
        ),
    );
    assert!(malformed(&attr).contains("prefix 'y' is not declared"));
    // The xml prefix and declared prefixes are fine.
    let fine = doc(
        GPX11,
        &format!(
            r#"<trk xml:lang="de" xmlns:o="urn:o" o:k="v"><trkseg>{}{}</trkseg></trk>"#,
            pt(47.0),
            pt(47.001)
        ),
    );
    assert_eq!(imported(&fine).points().len(), 2);
}

#[test]
fn a_second_surface_in_one_point_is_malformed() {
    let text = |surfaces: &str| {
        doc(
            GPX11,
            &format!(
                r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions xmlns:o="urn:o">{surfaces}</extensions></trkpt><trkpt lat="47.001" lon="8"><extensions xmlns:o="urn:o"><o:surface>sand</o:surface></extensions></trkpt></trkseg></trk>"#
            ),
        )
    };
    let one = imported(&text("<o:surface>gravel</o:surface>"));
    assert_eq!(
        one.points().iter().map(|p| p.surface).collect::<Vec<_>>(),
        vec![
            Some(crank_domain::route::Surface::Gravel),
            Some(crank_domain::route::Surface::Sand)
        ]
    );
    let what = malformed(&text(
        "<o:surface>gravel</o:surface><o:surface>sand</o:surface>",
    ));
    assert!(what.contains("a second <surface> in one <trkpt>"), "{what}");
    let nested = malformed(&text(
        "<o:a><o:surface>gravel</o:surface></o:a><o:surface>sand</o:surface>",
    ));
    assert!(nested.contains("a second <surface>"), "{nested}");
}

#[test]
fn elements_the_schema_allows_but_the_importer_does_not_read_are_ignored() {
    let v11 = doc(
        GPX11,
        r#"<metadata><name>M</name><desc>d</desc><author><name>A</name><email id="a" domain="b"/><link href="h"><text>t</text><type>x</type></link></author>
<copyright author="a"><year>2020</year><license>l</license></copyright><link href="h"/><time>2020-01-01T00:00:00Z</time><keywords>k</keywords>
<bounds minlat="1" minlon="1" maxlat="2" maxlon="2"/><extensions/></metadata>
<wpt lat="1" lon="1"><ele>1</ele><name>w</name><link href="h"/><sym>s</sym><extensions/></wpt>
<trk><name>T</name><cmt>c</cmt><desc>d</desc><src>s</src><link href="h"/><number>1</number><type>t</type><extensions/><trkseg>
<trkpt lat="47" lon="8"><ele>1</ele><time>2020-01-01T00:00:00Z</time><magvar>1</magvar><geoidheight>1</geoidheight><name>n</name><cmt>c</cmt><desc>d</desc><src>s</src><link href="h"/><sym>s</sym><type>t</type><fix>3d</fix><sat>4</sat><hdop>1</hdop><vdop>1</vdop><pdop>1</pdop><ageofdgpsdata>1</ageofdgpsdata><dgpsid>1</dgpsid><extensions/></trkpt>
<trkpt lat="47.001" lon="8"/><extensions/></trkseg></trk>
<rte><name>R</name><rtept lat="47" lon="8"/><rtept lat="47.001" lon="8"/></rte><extensions/>"#,
    );
    assert!(matches!(import(&v11, Some(0)), Ok(GpxImport::Imported(_))));
    let v10 = doc(
        GPX10,
        r#"<name>N</name><desc>d</desc><author>a</author><email>e</email><url>u</url><urlname>n</urlname><time>2020-01-01T00:00:00Z</time><keywords>k</keywords>
<bounds minlat="1" minlon="1" maxlat="2" maxlon="2"/><wpt lat="1" lon="1"><url>u</url></wpt>
<trk><name>T</name><url>u</url><urlname>n</urlname><number>1</number><trkseg>
<trkpt lat="47" lon="8"><ele>1</ele><course>1</course><speed>1</speed><url>u</url></trkpt><trkpt lat="47.001" lon="8"/></trkseg></trk>"#,
    );
    assert_eq!(imported(&v10).name(), "T");
    // `course` and `speed` are 1.0 track-point elements only.
    let rte_speed = doc(
        GPX10,
        r#"<rte><rtept lat="47" lon="8"><speed>1</speed></rtept><rtept lat="47.001" lon="8"/></rte>"#,
    );
    assert!(malformed(&rte_speed).contains("<speed> is not allowed in <rtept>"));
    assert!(malformed(&doc(GPX11, &format!("<trk><trkseg><trkpt lat=\"47\" lon=\"8\"><speed>1</speed></trkpt>{}</trkseg></trk>", pt(47.001)))).contains("<speed>"));
}

#[test]
fn text_inside_an_element_only_type_is_malformed() {
    let what = malformed(&doc(
        GPX11,
        &format!("<trk>junk<trkseg>{}{}</trkseg></trk>", pt(47.0), pt(47.001)),
    ));
    assert!(
        what.contains("text inside <trk> (GPX 1.1 trkType: no character content)"),
        "{what}"
    );
    let what = malformed(&doc(
        GPX11,
        &format!("<trk><trkseg>{}{}</trkseg></trk>x", pt(47.0), pt(47.001)),
    ));
    assert!(what.contains("text inside <gpx>"), "{what}");
    let what = malformed(&doc(
        GPX11,
        &format!(
            "<trk><trkseg><trkpt lat=\"47\" lon=\"8\">&amp;</trkpt>{}</trkseg></trk>",
            pt(47.001)
        ),
    ));
    assert!(what.contains("text inside <trkpt>"), "{what}");
    // Whitespace between elements, and text in a simple type or an extension, are fine.
    let fine = doc(
        GPX11,
        &format!(
            "\n <trk>\n <desc>a &amp; b</desc><extensions><o:a xmlns:o=\"urn:o\">text</o:a></extensions>\t<trkseg>{}\n{}</trkseg></trk>\n",
            pt(47.0),
            pt(47.001)
        ),
    );
    assert_eq!(imported(&fine).points().len(), 2);
}
