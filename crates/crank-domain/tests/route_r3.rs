//! Round 3 of #539: no `Route` that breaks ADR-0005 §2.1 can be deserialised, every accepted
//! grade profile prints to text that parses back (even at the size limit), and GPX structure is
//! read by namespace and schema position (GPX 1.0 and 1.1 XSD).
use crank_domain::route::{
    GpxImport, GradeProfileItem, MAX_PROFILE_TEXT_BYTES, Route, RouteDslErrorKind as K,
    RouteImportError, RouteImportOptions, parse_grade_profile, parse_route_dsl,
    print_grade_profile, print_route_dsl,
};
use proptest::prelude::*;
use serde_json::{Value, json};

// --- A: deserialising checks every invariant ---

fn file_route() -> Route {
    let text = r#"<gpx xmlns="http://www.topografix.com/GPX/1/1"><trk><name>T</name><trkseg>
<trkpt lat="46" lon="8"><ele>1</ele></trkpt><trkpt lat="46.001" lon="8"><ele>2</ele></trkpt>
<trkpt lat="46.002" lon="8"><ele>3</ele></trkpt></trkseg></trk></gpx>"#;
    match crank_domain::route::import_gpx(text.as_bytes(), "t.gpx", &RouteImportOptions::default())
    {
        Ok(GpxImport::Imported(r)) => r.route,
        other => panic!("{other:?}"),
    }
}

/// A change made to a serialised route.
type Change = Box<dyn FnOnce(&mut Value)>;

fn refused(mut v: Value, change: impl FnOnce(&mut Value)) -> bool {
    change(&mut v);
    serde_json::from_value::<Route>(v).is_err()
}

#[test]
fn a_route_deserialises_only_when_it_keeps_every_invariant() {
    let file = file_route();
    let profile = parse_route_dsl("\"A\" 1km 2%, 1km ramp 2%..4%").expect("parses");
    for r in [&file, &profile] {
        let v = serde_json::to_value(r).expect("serialises");
        assert_eq!(
            serde_json::from_value::<Route>(v).ok().as_ref(),
            Some(r),
            "round trip"
        );
    }
    let f = serde_json::to_value(&file).expect("serialises");
    let p = serde_json::to_value(&profile).expect("serialises");
    let cases: [(&str, Value, Change); 11] = [
        ("empty name", f.clone(), Box::new(|v| v["name"] = json!(""))),
        (
            "blank name",
            f.clone(),
            Box::new(|v| v["name"] = json!("  ")),
        ),
        (
            "no points",
            f.clone(),
            Box::new(|v| v["points"] = json!([])),
        ),
        (
            "one point",
            f.clone(),
            Box::new(|v| v["points"].as_array_mut().expect("points").truncate(1)),
        ),
        (
            "first point not at 0",
            f.clone(),
            Box::new(|v| v["points"][0]["distM"] = json!(5.0)),
        ),
        (
            "distance 5 then 1",
            f.clone(),
            Box::new(|v| {
                v["points"][1]["distM"] = json!(5.0);
                v["points"][2]["distM"] = json!(1.0);
            }),
        ),
        (
            "lat 999",
            f.clone(),
            Box::new(|v| v["points"][1]["latLon"] = json!([999.0, 8.0])),
        ),
        (
            "missing elevation in a recorded route",
            f.clone(),
            Box::new(|v| v["points"][1]["eleM"] = Value::Null),
        ),
        (
            "elevation in a route without",
            f.clone(),
            Box::new(|v| v["elevation"] = json!("none")),
        ),
        (
            "coordinates on a grade profile",
            p.clone(),
            Box::new(|v| v["points"][1]["latLon"] = json!([46.0, 8.0])),
        ),
        (
            "segment from 9 to -1 m",
            p.clone(),
            Box::new(|v| {
                v["segments"][0]["startM"] = json!(9.0);
                v["segments"][0]["endM"] = json!(-1.0);
            }),
        ),
    ];
    for (what, v, change) in cases {
        assert!(refused(v, change), "{what} was accepted");
    }
    assert!(
        refused(p, |v| v["segments"][1]["endM"] = json!(2_500.0)),
        "segment past the end"
    );
}

// --- B: every accepted profile prints to text that parses back ---

/// `1x(` + `n` × `0.01km 1%` joined by `sep` + `)`.
fn flat(n: usize, sep: &str) -> String {
    format!("1x({})", vec!["0.01km 1%"; n].join(sep))
}

#[test]
fn a_profile_whose_canonical_form_is_too_long_is_refused_and_one_that_fits_round_trips() {
    // The falsifier's case: 262 143 bytes of input, but 288 356 written canonically (", ").
    let input = flat(26_214, ",");
    assert!(input.len() <= MAX_PROFILE_TEXT_BYTES);
    assert_eq!(
        parse_grade_profile(&input).map(|_| ()).map_err(|e| e.kind),
        Err(K::CanonicalTooLong)
    );
    // Canonical length is 11 n + 2: n = 23 831 is 262 143 bytes, n = 23 832 is 262 154.
    let fits = parse_grade_profile(&flat(23_831, ",")).expect("fits written canonically");
    let printed = print_grade_profile(&fits);
    assert_eq!(printed.len(), 262_143);
    assert_eq!(parse_grade_profile(&printed).as_ref(), Ok(&fits));
    let route = parse_route_dsl(&printed).expect("parses");
    let dsl = print_route_dsl(&route).expect("a profile");
    assert_eq!(parse_route_dsl(&dsl).as_ref(), Ok(&route));
    assert_eq!(
        parse_grade_profile(&flat(23_832, ","))
            .map(|_| ())
            .map_err(|e| e.kind),
        Err(K::CanonicalTooLong)
    );
}

/// Texts whose length lands within 2 000 bytes under the limit, so their canonical form (`,` and
/// `;` and newlines become `, `; case and `+` change) is often over it.
fn near_limit() -> impl Strategy<Value = String> {
    let segment = proptest::sample::select(vec![
        "0.01km 1%",
        "0.02km -2.5%",
        "0.03km ramp 1%..2%",
        "\"L\" 0.01km 0%",
        "0.01KM +3% gravel",
    ]);
    let sep = proptest::sample::select(vec![",", ", ", ";", "\n", " ,  "]);
    (segment, sep, 0usize..2_000, any::<bool>()).prop_map(|(seg, sep, slack, wrap)| {
        let n = (MAX_PROFILE_TEXT_BYTES - slack - 5) / (seg.len() + sep.len());
        let body = vec![seg; n].join(sep);
        if wrap { format!("2x({body})") } else { body }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn near_the_size_limits_a_profile_is_refused_or_round_trips(text in near_limit()) {
        match parse_grade_profile(&text) {
            Err(e) => prop_assert!(
                matches!(e.kind, K::TextTooLong | K::CanonicalTooLong | K::RouteTooLong),
                "{:?}", e.kind
            ),
            Ok(items) => {
                let printed = print_grade_profile(&items);
                prop_assert!(printed.len() <= MAX_PROFILE_TEXT_BYTES);
                let again: Vec<GradeProfileItem> = parse_grade_profile(&printed)
                    .map_err(|e| TestCaseError::fail(format!("printed text refused: {e}")))?;
                prop_assert_eq!(&again, &items);
                prop_assert_eq!(print_grade_profile(&again), printed.clone());
                if !items.is_empty() {
                    let route = parse_route_dsl(&printed).map_err(|e| TestCaseError::fail(e.to_string()))?;
                    let dsl = print_route_dsl(&route).ok_or_else(|| TestCaseError::fail("no dsl"))?;
                    prop_assert_eq!(parse_route_dsl(&dsl), Ok(route));
                }
            }
        }
    }
}

// --- C: GPX structure by namespace and schema position ---

const GPX11: &str = "http://www.topografix.com/GPX/1/1";
const GPX10: &str = "http://www.topografix.com/GPX/1/0";

fn doc(ns: &str, body: &str) -> String {
    format!(r#"<gpx version="1.1" xmlns="{ns}">{body}</gpx>"#)
}

fn import(text: &str) -> Result<GpxImport, RouteImportError> {
    crank_domain::route::import_gpx(text.as_bytes(), "x.gpx", &RouteImportOptions::default())
}

fn malformed(text: &str) -> String {
    match import(text) {
        Err(RouteImportError::Malformed { what, .. }) => what,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

fn kept(text: &str) -> usize {
    match import(text) {
        Ok(GpxImport::Imported(r)) => r.report.points_kept,
        other => panic!("expected an import, got {other:?}"),
    }
}

const PTS: &str = r#"<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/>"#;

#[test]
fn a_foreign_element_outside_extensions_is_malformed_in_gpx_1_1() {
    let foreign_trk = doc(
        GPX11,
        &format!(r#"<o:trk xmlns:o="urn:other"><o:trkseg>{PTS}</o:trkseg></o:trk>"#),
    );
    assert!(malformed(&foreign_trk).contains("extensionsType"));
    let redeclared = doc(
        GPX11,
        &format!(r#"<trk xmlns="urn:other"><trkseg>{PTS}</trkseg></trk>"#),
    );
    assert!(malformed(&redeclared).contains("urn:other"));
    let in_point = doc(
        GPX11,
        r#"<trk><trkseg><trkpt lat="46" lon="8"><o:x xmlns:o="urn:o"/></trkpt>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    assert!(malformed(&in_point).contains("extensions"));
    // The same element inside <extensions> is an extension, skipped.
    let ext = doc(
        GPX11,
        r#"<trk><extensions><o:x xmlns:o="urn:o"><trk/></o:x></extensions><trkseg>
<trkpt lat="46" lon="8"><extensions><o:x xmlns:o="urn:o"><o:trkpt lat="1" lon="1"/></o:x></extensions></trkpt>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    assert_eq!(kept(&ext), 2);
}

#[test]
fn gpx_1_0_allows_foreign_elements_where_its_schema_does() {
    // GPX 1.0: `xsd:any namespace="##other"` at the end of gpx, wpt, rte, rtept, trk and trkpt.
    let ok = doc(
        GPX10,
        r#"<o:a xmlns:o="urn:o"/><trk><o:b xmlns:o="urn:o"/><trkseg>
<trkpt lat="46" lon="8"><o:surface xmlns:o="urn:o">gravel</o:surface></trkpt>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    match import(&ok) {
        Ok(GpxImport::Imported(r)) => {
            assert_eq!(r.report.points_kept, 2);
            assert_eq!(r.report.surfaces_from_file, 1);
        }
        other => panic!("{other:?}"),
    }
    // ...but not in trkseg, whose 1.0 type holds only trkpt.
    let in_seg = doc(
        GPX10,
        &format!(r#"<trk><trkseg><o:x xmlns:o="urn:o"/>{PTS}</trkseg></trk>"#),
    );
    assert!(malformed(&in_seg).contains("GPX 1.0"));
    // A GPX 1.1 element in a 1.0 document is foreign too.
    let mixed = doc(
        GPX10,
        &format!(r#"<trk><trkseg>{PTS}</trkseg></trk><g:trk xmlns:g="{GPX11}"/>"#),
    );
    assert_eq!(
        kept(&mixed),
        2,
        "skipped as a ##other element at the end of gpx"
    );
}

#[test]
fn only_the_unqualified_lat_and_lon_are_coordinates() {
    let prefixed = doc(
        GPX11,
        r#"<trk><trkseg><trkpt xmlns:x="urn:x" x:lat="46" lon="8"/></trkseg></trk>"#,
    );
    assert!(malformed(&prefixed).contains("lat and lon"));
    let both = doc(
        GPX11,
        r#"<trk><trkseg><trkpt xmlns:x="urn:x" lat="46" x:lat="0" lon="8"/>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    assert_eq!(kept(&both), 2);
    let duplicate = doc(
        GPX11,
        r#"<trk><trkseg><trkpt lat="46" lat="47" lon="8"/></trkseg></trk>"#,
    );
    let _ = malformed(&duplicate);
}

#[test]
fn a_second_ele_in_one_point_is_malformed() {
    let two = doc(
        GPX11,
        r#"<trk><trkseg><trkpt lat="46" lon="8"><ele>1</ele><ele>2</ele></trkpt>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    assert!(malformed(&two).contains("maxOccurs"));
    let one_each = doc(
        GPX11,
        r#"<trk><trkseg><trkpt lat="46" lon="8"><ele>1</ele></trkpt>
<trkpt lat="46.01" lon="8"><ele>2</ele></trkpt></trkseg></trk>"#,
    );
    assert_eq!(kept(&one_each), 2);
}

#[test]
fn structural_elements_must_sit_where_the_schema_puts_them() {
    for (body, want) in [
        (
            format!("<trk>{PTS}</trk>"),
            "<trkpt> is not allowed in <trk> (GPX 1.1 trkType)",
        ),
        (
            format!("<rte><trkseg>{PTS}</trkseg></rte>"),
            "<trkseg> is not allowed in <rte> (GPX 1.1 rteType)",
        ),
        (
            r#"<trk><trkseg><rtept lat="46" lon="8"/></trkseg></trk>"#.to_owned(),
            "<rtept> is not allowed in <trkseg> (GPX 1.1 trksegType)",
        ),
        (
            format!("<metadata><trk><trkseg>{PTS}</trkseg></trk></metadata>"),
            "<trk> is not allowed in <metadata> (GPX 1.1 metadataType)",
        ),
        (
            format!("<trk><trk><trkseg>{PTS}</trkseg></trk></trk>"),
            "<trk> is not allowed in <trk> (GPX 1.1 trkType)",
        ),
    ] {
        let what = malformed(&doc(GPX11, &body));
        assert!(what.contains(want), "{body}: {what}");
    }
}
