//! Round 5 of #539: a route survives JSON bit for bit, everything inside a GPX `<extensions>` is
//! extension data (skipped, never refused, never read as GPX structure), and the document's
//! `metadata` occurs once.
use crank_domain::route::{
    GpxImport, Route, RouteImportError, RouteImportOptions, RoutePoint, RouteSegment, SegmentGrade,
    Surface, import_gpx, parse_route_dsl,
};
use proptest::prelude::*;
use std::fmt::Write as _;

// --- A: JSON round trip, bit for bit ---

/// A route's `Debug` text, each point's f64s and each segment's f64s, by their bits.
type Bits = (String, Vec<[Option<u64>; 4]>, Vec<[u64; 5]>);

/// Every f64 of a route by its bits, plus everything else by value: two routes are the same
/// exactly when these are equal (−0 is not +0).
fn bits(r: &Route) -> Bits {
    fn point(p: &RoutePoint) -> [Option<u64>; 4] {
        [
            Some(p.dist_m.to_bits()),
            p.ele_m.map(f64::to_bits),
            p.lat_lon.map(|(lat, _)| lat.to_bits()),
            p.lat_lon.map(|(_, lon)| lon.to_bits()),
        ]
    }
    fn segment(s: &RouteSegment) -> [u64; 5] {
        let (kind, a, b) = match s.grade {
            SegmentGrade::FromPoints => (0, 0, 0),
            SegmentGrade::Constant { pct } => (1, pct.to_bits(), 0),
            SegmentGrade::Ramp { from_pct, to_pct } => (2, from_pct.to_bits(), to_pct.to_bits()),
        };
        [s.start_m.to_bits(), s.end_m.to_bits(), kind, a, b]
    }
    (
        format!("{r:?}"),
        r.points().iter().map(point).collect(),
        r.segments().iter().map(segment).collect(),
    )
}

/// `from_json(to_json(r))` is `r` bit for bit, and passes `check`.
fn assert_json_round_trip(r: &Route) {
    assert_eq!(r.check(), Ok(()));
    let text = serde_json::to_string(r).expect("serialises");
    let back: Route = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!(back.check(), Ok(()));
    assert_eq!(bits(&back), bits(r));
}

#[test]
fn the_falsifier_s_profile_corpus_round_trips_through_json() {
    // The 343 profiles of which 167 came back from JSON other than they went in, without
    // serde_json's `float_roundtrip`: `10x(0.01km 1%)` gave an elevation of 0.9999999999999999,
    // which parsed back as 1.0.
    let grades = [
        "1%",
        "3.3%",
        "-2.7%",
        "0.1%",
        "7.77%",
        "ramp 1.1%..4.3%",
        "12.345%",
    ];
    let lens = ["0.01km", "0.123km", "1km", "0.333km", "2.718km"];
    let mut tried = 0;
    for g in grades {
        for l in lens {
            for n in [1u32, 2, 3, 5, 7, 10, 13, 50, 100, 1000] {
                if let Ok(r) = parse_route_dsl(&format!("{n}x({l} {g})")) {
                    assert_json_round_trip(&r);
                    tried += 1;
                }
            }
        }
    }
    assert_eq!(tried, 343);
    assert_json_round_trip(&parse_route_dsl("10x(0.01km 1%)").expect("parses"));
}

fn item() -> impl Strategy<Value = String> {
    let km = (10u32..=5_000).prop_map(|m| format!("{}km", f64::from(m) / 1000.0));
    let pct = (-2500i32..=3000).prop_map(|c| format!("{}%", f64::from(c) / 100.0));
    let grade = prop_oneof![
        pct.clone(),
        (pct.clone(), pct).prop_map(|(a, b)| format!("ramp {a}..{b}"))
    ];
    let leaf = (km, grade).prop_map(|(k, g)| format!("{k} {g}"));
    leaf.prop_recursive(3, 24, 3, |inner| {
        (1u32..=5, proptest::collection::vec(inner, 1..=3))
            .prop_map(|(n, items)| format!("{n}x({})", items.join(", ")))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_generated_profile_round_trips_through_json(items in proptest::collection::vec(item(), 1..=4)) {
        if let Ok(r) = parse_route_dsl(&items.join("\n")) {
            assert_json_round_trip(&r);
        }
    }

    #[test]
    fn every_generated_file_route_round_trips_through_json(
        start in (-80.0..80.0f64, -170.0..170.0f64, -400.0..4000.0f64),
        digits in 1usize..=15,
        steps in proptest::collection::vec((0.001..0.002f64, -0.002..0.002f64, -5.0..5.0f64), 2..60),
    ) {
        // A walk north of steps of about 110 to 300 m: at least 100 m in all even if a start at
        // (0, 0) is dropped, and no step a jump. Written with `digits` decimals as exporters do,
        // or with every digit of the f64.
        let (mut lat, mut lon, mut ele) = start;
        let mut body = String::new();
        for (i, (dlat, dlon, dele)) in std::iter::once(&(0.0, 0.0, 0.0)).chain(&steps).enumerate() {
            lat += dlat;
            lon += dlon;
            ele += dele;
            let written = if i % 2 == 0 {
                write!(body, r#"<trkpt lat="{lat:.digits$}" lon="{lon:.digits$}"><ele>{ele:.digits$}</ele></trkpt>"#)
            } else {
                write!(body, r#"<trkpt lat="{lat}" lon="{lon}"><ele>{ele}</ele></trkpt>"#)
            };
            written.expect("writes to a String");
        }
        let text = format!(r#"<gpx version="1.1" creator="t" xmlns="http://www.topografix.com/GPX/1/1"><trk><trkseg>{body}</trkseg></trk></gpx>"#);
        match import_gpx(text.as_bytes(), "p.gpx", &RouteImportOptions::default()) {
            Ok(GpxImport::Imported(r)) => assert_json_round_trip(&r.route),
            other => panic!("{other:?}"),
        }
    }
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = format!("{}/tests/fixtures/gpx", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "gpx"))
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name, std::fs::read(&p).expect("reads"))
        })
        .collect();
    files.sort();
    files
}

fn import_all(name: &str, bytes: &[u8]) -> Vec<Route> {
    match import_gpx(bytes, name, &RouteImportOptions::default()) {
        Ok(GpxImport::Imported(r)) => vec![r.route],
        Ok(GpxImport::ChooseTrack { tracks }) => tracks
            .iter()
            .map(|t| {
                match import_gpx(
                    bytes,
                    name,
                    &RouteImportOptions {
                        track: Some(t.index),
                    },
                ) {
                    Ok(GpxImport::Imported(r)) => r.route,
                    other => panic!("{name} track {}: {other:?}", t.index),
                }
            })
            .collect(),
        Err(e) => panic!("{name}: {e:?}"),
    }
}

#[test]
fn every_committed_gpx_fixture_imports_and_round_trips_through_json() {
    let files = fixtures();
    assert_eq!(
        files.len(),
        10,
        "{:?}",
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    for (name, bytes) in &files {
        for r in import_all(name, bytes) {
            assert!(r.points().len() >= 2, "{name}");
            assert_json_round_trip(&r);
        }
    }
}

// --- B: everything inside <extensions> is extension data ---

fn fixture(name: &str) -> Route {
    let Some((_, bytes)) = fixtures().into_iter().find(|f| f.0 == name) else {
        panic!("no fixture {name}");
    };
    let mut all = import_all(name, &bytes);
    assert_eq!(all.len(), 1, "{name}");
    all.remove(0)
}

#[test]
fn a_strava_export_with_unprefixed_power_imports() {
    // Strava writes `<power>` unprefixed inside `<extensions>`, so it is in the GPX namespace.
    let r = fixture("strava-power.gpx");
    assert_eq!(r.name(), "Free Ride");
    assert_eq!(r.points().len(), 8);
    let ele: Vec<_> = r.points().iter().map(|p| p.ele_m).collect();
    assert_eq!(ele[0], Some(408.2));
    assert_eq!(ele[7], Some(411.0));
    assert!(r.points().iter().all(|p| p.surface.is_none()));
}

#[test]
fn an_older_osmand_export_with_unprefixed_speed_and_color_imports() {
    let r = fixture("osmand-old.gpx");
    assert_eq!(r.name(), "2024-05-01_ride");
    assert_eq!(r.points().len(), 2);
}

const GPX11: &str = "http://www.topografix.com/GPX/1/1";

fn doc(body: &str) -> String {
    format!(r#"<gpx version="1.1" creator="t" xmlns="{GPX11}" xmlns:o="urn:o">{body}</gpx>"#)
}

const SEG: &str = r#"<trkseg><trkpt lat="47" lon="8"><ele>1</ele></trkpt><trkpt lat="47.001" lon="8"><ele>2</ele></trkpt></trkseg>"#;

fn import(text: &str) -> Result<GpxImport, RouteImportError> {
    import_gpx(text.as_bytes(), "x.gpx", &RouteImportOptions::default())
}

fn imported(text: &str) -> Route {
    match import(text) {
        Ok(GpxImport::Imported(r)) => r.route,
        other => panic!("expected one import, got {other:?}\n{text}"),
    }
}

fn malformed(text: &str) -> String {
    match import(text) {
        Err(RouteImportError::Malformed { what, .. }) => what,
        other => panic!("expected Malformed, got {other:?}\n{text}"),
    }
}

#[test]
fn gpx_structure_inside_extensions_is_skipped_not_imported() {
    let ghost_pts = r#"<trkseg><trkpt lat="10" lon="10"><ele>900</ele></trkpt><trkpt lat="10.001" lon="10"/></trkseg>"#;
    for (body, what) in [
        (
            format!("<trk><name>Real</name>{SEG}<extensions>{ghost_pts}</extensions></trk>"),
            "a trkseg in a trk's extensions",
        ),
        (
            format!(
                "<trk><name>Real</name>{SEG}</trk><extensions><gpx><trk><name>Ghost</name>{ghost_pts}</trk></gpx></extensions>"
            ),
            "a whole gpx in the root's extensions",
        ),
        (
            format!(
                "<trk><name>Real</name>{SEG}</trk><extensions><rte><rtept lat=\"10\" lon=\"10\"/><rtept lat=\"10.001\" lon=\"10\"/></rte></extensions>"
            ),
            "an rte in the root's extensions",
        ),
        (
            r#"<trk><name>Real</name><trkseg><trkpt lat="47" lon="8"><ele>1</ele><extensions><trkpt lat="10" lon="10"/><ele>900</ele><name>x</name></extensions></trkpt><trkpt lat="47.001" lon="8"><ele>2</ele></trkpt></trkseg><extensions><name>Ghost</name></extensions></trk>"#
                .to_owned(),
            "a trkpt, ele and name in a point's or track's extensions",
        ),
        (
            format!(
                "<metadata><name>Doc</name><extensions><metadata><name>Ghost</name></metadata></extensions></metadata><trk><name>Real</name>{SEG}</trk>"
            ),
            "a metadata in the metadata's extensions",
        ),
    ] {
        let r = imported(&doc(&body));
        assert_eq!(r.name(), "Real", "{what}");
        let p: Vec<_> = r.points().iter().map(|p| (p.lat_lon, p.ele_m)).collect();
        assert_eq!(
            p,
            vec![(Some((47.0, 8.0)), Some(1.0)), (Some((47.001, 8.0)), Some(2.0))],
            "{what}"
        );
    }
}

#[test]
fn extension_content_is_never_refused_but_must_be_well_formed() {
    for inner in [
        "<power>230</power>",
        "<ele>5</ele>",
        r#"<x xmlns="">5</x>"#,
        "<o:a><b>x</b></o:a>",
        "text directly in extensions",
        "<speed>5.1</speed><color>#a71de1</color>",
    ] {
        let text = doc(&format!(
            r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions>{inner}</extensions></trkpt><trkpt lat="47.001" lon="8"/></trkseg></trk>"#
        ));
        assert_eq!(imported(&text).points().len(), 2, "{inner}");
    }
    for (inner, want) in [
        ("<y:q/>", "prefix 'y' is not declared"),
        ("<a></b>", "</b>"),
        ("<o:a y:k=\"1\"/>", "prefix 'y' is not declared"),
    ] {
        let text = doc(&format!(
            r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions>{inner}</extensions></trkpt><trkpt lat="47.001" lon="8"/></trkseg></trk>"#
        ));
        let what = malformed(&text);
        assert!(what.contains(want), "{inner}: {what}");
    }
}

#[test]
fn only_a_surface_of_another_namespace_than_gpx_s_sets_the_point_surface() {
    let text = |surface: &str| {
        doc(&format!(
            r#"<trk><trkseg><trkpt lat="47" lon="8"><extensions>{surface}</extensions></trkpt><trkpt lat="47.001" lon="8"/></trkseg></trk>"#
        ))
    };
    let surface = |s: &str| imported(&text(s)).points()[0].surface;
    assert_eq!(
        surface("<o:surface>gravel</o:surface>"),
        Some(Surface::Gravel)
    );
    assert_eq!(
        surface("<o:a><o:surface>gravel</o:surface></o:a>"),
        Some(Surface::Gravel)
    );
    // Unprefixed: the GPX namespace, so extension data like Strava's <power>.
    assert_eq!(surface("<surface>gravel</surface>"), None);
    assert_eq!(surface("<o:a><surface>gravel</surface></o:a>"), None);
    assert_eq!(surface(r#"<surface xmlns="">gravel</surface>"#), None);
    // Skipped, so it does not count as a first surface either.
    assert_eq!(
        surface("<surface>sand</surface><o:surface>gravel</o:surface>"),
        Some(Surface::Gravel)
    );
}

#[test]
fn outside_extensions_the_schema_still_rules() {
    // GPX 1.1 allows other namespaces only inside <extensions>.
    let what = malformed(&doc(&format!("<trk><o:x/>{SEG}</trk>")));
    assert!(what.contains("only inside <extensions>"), "{what}");
    // GPX 1.0's `##other` stands at the end of a trk, but excludes no namespace.
    let v10 = |inner: &str| {
        format!(
            r#"<gpx version="1.0" creator="t" xmlns="http://www.topografix.com/GPX/1/0"><trk>{SEG}{inner}</trk></gpx>"#
        )
    };
    assert_eq!(
        imported(&v10(r#"<o:x xmlns:o="urn:o"/>"#)).points().len(),
        2
    );
    let what = malformed(&v10(r#"<x xmlns=""/>"#));
    assert!(
        what.contains("GPX 1.0 allows other namespaces only"),
        "{what}"
    );
    // M18: trksegType holds trkpt and extensions, no link.
    let what = malformed(&doc(
        r#"<trk><trkseg><link href="h"/><trkpt lat="47" lon="8"/><trkpt lat="47.001" lon="8"/></trkseg></trk>"#,
    ));
    assert!(
        what.contains("<link> is not allowed in <trkseg> (GPX 1.1 trksegType)"),
        "{what}"
    );
    assert_eq!(
        imported(&doc(&format!(r#"<trk><link href="h"/>{SEG}</trk>"#)))
            .points()
            .len(),
        2
    );
}

// --- C: the document name is read once ---

#[test]
fn a_second_metadata_or_document_name_is_malformed() {
    let trk = format!("<trk>{SEG}</trk>");
    for body in [
        format!("<metadata><name>A</name></metadata><metadata><name>B</name></metadata>{trk}"),
        format!("<metadata><name>A</name></metadata><metadata/>{trk}"),
        format!("<metadata/><metadata><name>B</name></metadata>{trk}"),
    ] {
        let what = malformed(&doc(&body));
        assert!(
            what.contains(
                "a second <metadata> in one <gpx> (GPX 1.1 gpxType: metadata maxOccurs 1)"
            ),
            "{body}: {what}"
        );
    }
    let what = malformed(&doc(&format!(
        "<metadata><name>A</name><name>B</name></metadata>{trk}"
    )));
    assert!(what.contains("a second <name> in one <metadata>"), "{what}");
    let v10 = format!(
        r#"<gpx version="1.0" creator="t" xmlns="http://www.topografix.com/GPX/1/0"><name>A</name><name>B</name>{trk}</gpx>"#
    );
    assert!(malformed(&v10).contains("a second <name> in one <gpx>"));
    assert_eq!(
        imported(&doc(&format!("<metadata><name>A</name></metadata>{trk}"))).name(),
        "A"
    );
}

// --- Coverage: M4, the bit comparison covers segments ---

#[test]
fn a_negative_zero_in_a_segment_is_not_the_profile_s_text() {
    let r = parse_route_dsl("1km 0%\n1km 2%").expect("parses");
    assert_eq!(r.segments()[0].grade, SegmentGrade::Constant { pct: 0.0 });
    assert!(
        matches!(r.segments()[0].grade, SegmentGrade::Constant { pct } if pct.is_sign_positive())
    );
    let mut parts = r.clone().into_parts();
    parts.segments[0].grade = SegmentGrade::Constant { pct: -0.0 };
    assert!(
        Route::from_parts(parts).is_err(),
        "−0 % where the text gives +0 %"
    );
    let mut parts = r.clone().into_parts();
    parts.segments[0].start_m = -0.0;
    assert!(
        Route::from_parts(parts).is_err(),
        "a segment starting at −0 m"
    );
    assert_eq!(Route::from_parts(r.clone().into_parts()), Ok(r));
}
