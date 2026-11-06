//! GPX import, its limits and the missing-elevation rules (ADR-0005 §2.2.1, §2.2.2, §2.2.4, §2.9 R1).
use crank_domain::route::{
    ELE_GAP_M, ElevationKind, GpxImport, ImportedRoute, MAX_FILE_BYTES, MAX_POINTS,
    RouteImportError, RouteImportOptions, RouteSource, Surface, TrackKind, import_gpx,
};
use std::fmt::Write as _;

/// Metres per degree of latitude on the haversine sphere.
const M_PER_DEG: f64 = 6_371_008.8 * std::f64::consts::PI / 180.0;

/// `i` as f64, exactly.
fn n(i: usize) -> f64 {
    f64::from(u32::try_from(i).expect("a small index"))
}

/// A point `m` metres north of 46° N, 8° E.
fn north(m: f64) -> (f64, f64) {
    (46.0 + m / M_PER_DEG, 8.0)
}

fn trkpt(out: &mut String, (lat, lon): (f64, f64), ele: Option<f64>) {
    match ele {
        Some(e) => write!(
            out,
            r#"<trkpt lat="{lat}" lon="{lon}"><ele>{e}</ele></trkpt>"#
        ),
        None => write!(out, r#"<trkpt lat="{lat}" lon="{lon}"/>"#),
    }
    .expect("write to a String");
}

fn gpx(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">{body}</gpx>"#
    )
}

/// One track, one segment, the given points.
fn track(points: &[((f64, f64), Option<f64>)]) -> String {
    let mut body = String::from("<trk><name>Test</name><trkseg>");
    for &(p, e) in points {
        trkpt(&mut body, p, e);
    }
    body.push_str("</trkseg></trk>");
    gpx(&body)
}

/// `n` points `step` metres apart going north, elevation `ele(i)`.
fn line(
    count: usize,
    step: f64,
    ele: impl Fn(usize) -> Option<f64>,
) -> Vec<((f64, f64), Option<f64>)> {
    (0..count).map(|i| (north(step * n(i)), ele(i))).collect()
}

fn import(text: &str) -> Result<GpxImport, RouteImportError> {
    import_gpx(text.as_bytes(), "ride.gpx", &RouteImportOptions::default())
}

fn imported(text: &str) -> ImportedRoute {
    match import(text) {
        Ok(GpxImport::Imported(r)) => *r,
        other => panic!("expected an imported route, got {other:?}"),
    }
}

fn refusal(text: &str) -> RouteImportError {
    match import(text) {
        Err(e) => e,
        Ok(other) => panic!("expected a refusal, got {other:?}"),
    }
}

fn malformed_what(text: &str) -> String {
    match refusal(text) {
        RouteImportError::Malformed { what, .. } => what,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn a_plain_track_imports_with_haversine_distance_and_its_name() {
    let r = imported(&track(&line(11, 100.0, |i| Some(500.0 + n(i)))));
    assert_eq!(r.route.name, "Test");
    assert_eq!(r.route.source, RouteSource::GpxTrack);
    assert_eq!(r.route.elevation, ElevationKind::Recorded);
    assert_eq!(r.report.points_kept, 11);
    assert!(
        (r.report.distance_m - 1000.0).abs() < 1e-6,
        "{}",
        r.report.distance_m
    );
    assert!((r.report.ascent_m - 10.0).abs() < 1e-9);
    assert!(r.route.segments.is_empty());
    let d: Vec<f64> = r.route.points.iter().map(|p| p.dist_m).collect();
    assert_eq!(d[0].to_bits(), 0f64.to_bits());
    assert!(d.windows(2).all(|w| w[1] > w[0]), "strictly increasing");
}

// --- §2.2.4 limits: each test fails without its limit ---

#[test]
fn a_file_over_32_mib_is_refused_before_parsing() {
    let valid = track(&line(11, 100.0, |_| Some(0.0)));
    // A valid file, padded with a comment: without the size limit it would import.
    let pad = MAX_FILE_BYTES + 1 - valid.len() - "<!---->".len();
    let big = format!("{valid}<!--{}-->", " ".repeat(pad));
    assert_eq!(big.len(), MAX_FILE_BYTES + 1);
    assert_eq!(
        refusal(&big),
        RouteImportError::TooLarge {
            bytes: MAX_FILE_BYTES + 1
        }
    );
    let at_limit = format!("{valid}<!--{}-->", " ".repeat(pad - 1));
    assert_eq!(imported(&at_limit).report.points_kept, 11);
}

#[test]
fn more_than_200_000_points_after_dropping_are_refused() {
    let at_limit = track(&line(MAX_POINTS, 4.0, |_| None));
    assert_eq!(imported(&at_limit).report.points_kept, MAX_POINTS);
    // One more point, plus (0, 0) points that are dropped before counting.
    let mut pts = line(MAX_POINTS + 1, 4.0, |_| None);
    let over = track(&pts);
    assert_eq!(
        refusal(&over),
        RouteImportError::TooManyPoints { max: MAX_POINTS }
    );
    pts.truncate(MAX_POINTS);
    pts.insert(1, ((0.0, 0.0), None));
    assert_eq!(imported(&track(&pts)).report.points_kept, MAX_POINTS);
}

#[test]
fn a_point_at_exactly_0_0_is_dropped_and_counted() {
    let mut pts = line(11, 100.0, |_| Some(10.0));
    pts.insert(5, ((0.0, 0.0), Some(0.0)));
    pts.insert(0, ((0.0, 0.0), None));
    let r = imported(&track(&pts));
    assert_eq!(r.report.dropped_null_island, 2);
    assert_eq!(r.report.points_kept, 11);
    assert!((r.report.distance_m - 1000.0).abs() < 1e-6);
    // Near (0, 0) is a real place, not the "no fix" value.
    let near = track(&[((0.0, 1e-7), None), ((0.0, 0.01), None)]);
    assert_eq!(imported(&near).report.dropped_null_island, 0);
}

#[test]
fn a_jump_of_more_than_10_km_is_refused_with_its_position() {
    let pts = [
        (north(0.0), None),
        (north(500.0), None),
        (north(10_600.0), None),
    ];
    match refusal(&track(&pts)) {
        RouteImportError::Jump { at_m, len_m } => {
            assert!((at_m - 500.0).abs() < 1e-6, "{at_m}");
            assert!((len_m - 10_100.0).abs() < 1e-6, "{len_m}");
        }
        other => panic!("expected Jump, got {other:?}"),
    }
    let ok = [(north(0.0), None), (north(9_999.0), None)];
    assert!((imported(&track(&ok)).report.distance_m - 9_999.0).abs() < 1e-6);
}

#[test]
fn an_xml_bomb_doctype_with_nested_entities_is_refused() {
    let bomb = r#"<?xml version="1.0"?>
<!DOCTYPE gpx [
  <!ENTITY a "aaaaaaaaaa">
  <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
  <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
  <!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
]>
<gpx version="1.1"><trk><name>&d;</name><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk></gpx>"#;
    match refusal(bomb) {
        RouteImportError::Malformed { at, what } => {
            assert!(what.contains("DOCTYPE"), "{what}");
            assert_eq!(
                at,
                u64::try_from(bomb.find("<!DOCTYPE").expect("doctype")).expect("small")
            );
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
    // A DOCTYPE whose entities are never used is refused all the same: GPX needs none.
    let unused = r#"<!DOCTYPE gpx [<!ENTITY a "a">]><gpx><trk><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk></gpx>"#;
    assert!(malformed_what(unused).contains("DOCTYPE"));
}

#[test]
fn an_entity_other_than_xmls_five_is_not_expanded() {
    let text = gpx(r#"<trk><name>&ext;</name><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk>"#);
    assert!(malformed_what(&text).contains("&ext;"));
    let ok = gpx(r#"<trk><name>A &amp; B &#x263A;</name><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk>"#);
    assert_eq!(imported(&ok).route.name, "A & B ☺");
}

/// A track whose first point holds `extra` nested elements inside `<extensions>`.
fn nested(extra: usize) -> String {
    // gpx(1) trk(2) trkseg(3) trkpt(4) extensions(5), then `extra` more.
    let (mut open, mut close) = (String::new(), String::new());
    for i in 0..extra {
        write!(open, "<x{i}>").expect("write to a String");
        write!(close, "</x{}>", extra - 1 - i).expect("write to a String");
    }
    gpx(&format!(
        r#"<trk><trkseg><trkpt lat="46" lon="8"><extensions>{open}{close}</extensions></trkpt>
<trkpt lat="46.01" lon="8"/></trkseg></trk>"#
    ))
}

#[test]
fn depth_33_is_refused_and_32_accepted() {
    assert_eq!(imported(&nested(27)).report.points_kept, 2, "depth 32");
    let what = malformed_what(&nested(28));
    assert!(what.contains("deeper than 32"), "{what}");
}

// --- §2.2.2 missing elevation ---

#[test]
fn a_400_m_gap_is_filled_linearly_and_counted() {
    // Known at 100 m (10 m) and 500 m (50 m); 200, 300, 400 m have none.
    let r = imported(&track(&line(11, 100.0, |i| {
        (!(2..=4).contains(&i)).then_some(n(i) * 10.0)
    })));
    assert_eq!(r.route.elevation, ElevationKind::Recorded);
    assert_eq!(r.report.gaps_filled.len(), 1);
    let gap = r.report.gaps_filled[0];
    assert_eq!(gap.points, 3);
    assert!((gap.from_m - 100.0).abs() < 1e-6 && (gap.to_m - 500.0).abs() < 1e-6);
    for i in 2..=4 {
        let e = r.route.points[i].ele_m.expect("filled");
        assert!((e - n(i) * 10.0).abs() < 1e-6, "point {i}: {e}");
    }
}

#[test]
fn a_600_m_gap_is_refused() {
    let err = refusal(&track(&line(11, 100.0, |i| {
        (!(2..=6).contains(&i)).then_some(0.0)
    })));
    match err {
        RouteImportError::MissingElevation { from_m, to_m } => {
            assert!((from_m - 100.0).abs() < 1e-6 && (to_m - 700.0).abs() < 1e-6);
            assert!(to_m - from_m > ELE_GAP_M);
        }
        other => panic!("expected MissingElevation, got {other:?}"),
    }
}

#[test]
fn a_missing_start_or_end_takes_the_nearest_value_under_the_same_rule() {
    let r = imported(&track(&line(11, 100.0, |i| {
        (2..=8).contains(&i).then_some(7.0)
    })));
    assert!(r.route.points.iter().all(|p| p.ele_m == Some(7.0)));
    assert_eq!(r.report.gaps_filled.len(), 2);
    assert!(matches!(
        refusal(&track(&line(11, 100.0, |i| (i >= 6).then_some(7.0)))),
        RouteImportError::MissingElevation { from_m, .. } if from_m == 0.0
    ));
    assert!(matches!(
        refusal(&track(&line(11, 100.0, |i| (i <= 4).then_some(7.0)))),
        RouteImportError::MissingElevation { to_m, .. } if (to_m - 1000.0).abs() < 1e-6
    ));
}

#[test]
fn no_elevation_at_all_is_elevation_none() {
    let r = imported(&track(&line(11, 100.0, |_| None)));
    assert_eq!(r.route.elevation, ElevationKind::None);
    assert_eq!(r.report.elevation, ElevationKind::None);
    assert!(r.route.points.iter().all(|p| p.ele_m.is_none()));
    assert!(r.report.gaps_filled.is_empty());
}

#[test]
fn an_implausible_elevation_is_a_missing_one_counted() {
    let r = imported(&track(&line(11, 100.0, |i| {
        Some(match i {
            3 => -999.0,
            4 => 9_001.0,
            _ => 100.0,
        })
    })));
    assert_eq!(r.report.elevation_out_of_range, 2);
    assert_eq!(r.report.gaps_filled.len(), 1);
    assert!(r.route.points.iter().all(|p| p.ele_m == Some(100.0)));
}

// --- the rest of §2.2.1 and §2.2.4 ---

#[test]
fn a_repeated_point_is_merged_and_counted() {
    let mut pts = line(11, 100.0, |_| Some(1.0));
    pts.insert(3, pts[2]);
    pts.insert(3, pts[2]);
    let r = imported(&track(&pts));
    assert_eq!(r.report.merged_repeats, 2);
    assert_eq!(r.report.points_kept, 11);
}

#[test]
fn route_length_is_bounded_to_100_m_to_1000_km() {
    assert!(matches!(
        refusal(&track(&line(2, 99.0, |_| None))),
        RouteImportError::TooShort { .. }
    ));
    assert!(matches!(
        refusal(&track(&line(1, 1.0, |_| None))),
        RouteImportError::TooShort { len_m } if len_m == 0.0
    ));
    assert!(matches!(
        refusal(&track(&line(102, 9_950.0, |_| None))),
        RouteImportError::TooLong { .. }
    ));
}

#[test]
fn coordinates_out_of_range_are_malformed_at_the_point() {
    for (lat, lon) in [
        ("91", "8"),
        ("46", "-180.5"),
        ("NaN", "8"),
        ("46", "inf"),
        ("x", "8"),
    ] {
        let text = gpx(&format!(
            r#"<trk><trkseg><trkpt lat="46" lon="8"/><trkpt lat="{lat}" lon="{lon}"/></trkseg></trk>"#
        ));
        match refusal(&text) {
            RouteImportError::Malformed { at, .. } => {
                assert_eq!(
                    at,
                    u64::try_from(text.rfind("<trkpt").expect("point")).expect("small"),
                    "{lat} {lon}"
                );
            }
            other => panic!("{lat} {lon}: expected Malformed, got {other:?}"),
        }
    }
    let missing = gpx(r#"<trk><trkseg><trkpt lat="46"/></trkseg></trk>"#);
    assert!(malformed_what(&missing).contains("lat and lon"));
    let bad_ele =
        gpx(r#"<trk><trkseg><trkpt lat="46" lon="8"><ele>NaN</ele></trkpt></trkseg></trk>"#);
    assert!(malformed_what(&bad_ele).contains("elevation"));
}

#[test]
fn not_well_formed_or_not_gpx_is_malformed() {
    for text in [
        "",
        "<kml></kml>",
        "<gpx><trk></gpx>",
        "<gpx></gpx><gpx></gpx>",
    ] {
        assert!(
            matches!(refusal(text), RouteImportError::Malformed { .. }),
            "{text:?}"
        );
    }
    assert!(matches!(
        import_gpx(b"<gpx>\xff</gpx>", "x.gpx", &RouteImportOptions::default()),
        Err(RouteImportError::Malformed { at: 5, .. })
    ));
    assert_eq!(
        refusal(&gpx("<wpt lat=\"46\" lon=\"8\"/>")),
        RouteImportError::NoTrack
    );
}

#[test]
fn segments_of_a_track_join_in_order_and_the_join_counts() {
    let mut body = String::from("<trk><trkseg>");
    trkpt(&mut body, north(0.0), None);
    trkpt(&mut body, north(100.0), None);
    body.push_str("</trkseg><trkseg>");
    trkpt(&mut body, north(300.0), None);
    trkpt(&mut body, north(400.0), None);
    body.push_str("</trkseg></trk>");
    let r = imported(&gpx(&body));
    assert_eq!(r.report.points_kept, 4);
    assert!((r.report.distance_m - 400.0).abs() < 1e-6);
}

#[test]
fn a_route_imports_and_waypoints_and_extensions_are_ignored() {
    let mut body = String::from(
        r#"<wpt lat="10" lon="10"><ele>1</ele><name>W</name></wpt><rte><name>R</name>"#,
    );
    for i in 0..3 {
        let (lat, lon) = north(100.0 * f64::from(i));
        write!(body, r#"<rtept lat="{lat}" lon="{lon}"><ele>5</ele><time>2020-01-01T00:00:00Z</time></rtept>"#)
            .expect("write to a String");
    }
    body.push_str("</rte>");
    let r = imported(&gpx(&body));
    assert_eq!(r.route.source, RouteSource::GpxRoute);
    assert_eq!(r.route.name, "R");
    assert_eq!(r.report.points_kept, 3);
}

#[test]
fn a_surface_extension_in_any_namespace_sets_the_point_surface() {
    let text = gpx(r#"<trk><trkseg>
<trkpt lat="46" lon="8"><extensions><a:surface xmlns:a="urn:a">gravel</a:surface></extensions></trkpt>
<trkpt lat="46.001" lon="8"><extensions><x><surface>sett</surface></x></extensions></trkpt>
<trkpt lat="46.002" lon="8"><extensions><surface>moon dust</surface></extensions></trkpt>
</trkseg></trk>"#);
    let r = imported(&text);
    let s: Vec<_> = r.route.points.iter().map(|p| p.surface).collect();
    assert_eq!(s, [Some(Surface::Gravel), Some(Surface::Cobbles), None]);
    assert_eq!(r.report.surfaces_from_file, 2);
    assert_eq!(r.report.unknown_surfaces, 1);
    assert!(r.route.surface.from_file);
}

#[test]
fn gpx_1_0_names_come_from_the_file_then_the_file_name() {
    let one_oh = r#"<gpx version="1.0"><name>Old</name><trk><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk></gpx>"#;
    assert_eq!(imported(one_oh).route.name, "Old");
    let unnamed =
        gpx(r#"<trk><trkseg><trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk>"#);
    match import_gpx(
        unnamed.as_bytes(),
        "dir/Col du Test.gpx",
        &RouteImportOptions::default(),
    ) {
        Ok(GpxImport::Imported(r)) => assert_eq!(r.route.name, "Col du Test"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_multi_track_file_lists_its_tracks_and_imports_the_chosen_one() {
    let mut body = String::new();
    for (name, len) in [("Day 1", 1_000.0), ("Day 2", 2_000.0)] {
        write!(body, "<trk><name>{name}</name><trkseg>").expect("write to a String");
        trkpt(&mut body, north(0.0), None);
        trkpt(&mut body, north(len), None);
        body.push_str("</trkseg></trk>");
    }
    body.push_str("<rte>");
    for m in [0.0, 300.0] {
        let (lat, lon) = north(m);
        write!(body, r#"<rtept lat="{lat}" lon="{lon}"/>"#).expect("write to a String");
    }
    body.push_str("</rte>");
    let text = gpx(&body);
    let Ok(GpxImport::ChooseTrack { tracks }) = import(&text) else {
        panic!("expected a track list");
    };
    let listed: Vec<_> = tracks
        .iter()
        .map(|t| (t.index, t.kind, t.name.as_deref(), t.distance_m.round()))
        .collect();
    assert_eq!(
        listed,
        [
            (0, TrackKind::Track, Some("Day 1"), 1_000.0),
            (1, TrackKind::Track, Some("Day 2"), 2_000.0),
            (2, TrackKind::Route, None, 300.0),
        ]
    );
    let pick = |track| {
        import_gpx(
            text.as_bytes(),
            "x.gpx",
            &RouteImportOptions { track: Some(track) },
        )
    };
    match pick(1) {
        Ok(GpxImport::Imported(r)) => {
            assert_eq!(r.route.name, "Day 2");
            assert!((r.report.distance_m - 2_000.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(pick(3), Err(RouteImportError::NoTrack));
}

/// Model distances: the haversine sum of `docs/adr/models/route_grade.py` (`haversine`) over
/// each segment of its fixture, printed by
/// `python3 -c "import route_grade as m; s = m.load('fixtures/gpx-elevation-samples.csv'); ..."`
/// in `docs/adr/models` (`sum(m.haversine(a, b) for a, b in zip(p, p[1:]))` per segment).
const MODEL_DISTANCE_M: [(&str, f64); 4] = [
    ("korita-1", 8_635.553_802_504_208),
    ("korita-2", 2_282.074_375_138_473_4),
    ("korita-3", 3_979.487_102_700_827_6),
    ("cerknica-1", 1_912.360_478_042_139_2),
];

#[test]
fn the_route_grade_model_fixture_imports_to_the_models_distance() {
    let csv = include_str!("../../../docs/adr/models/fixtures/gpx-elevation-samples.csv");
    for (segment, model_m) in MODEL_DISTANCE_M {
        let mut body = String::from("<trk><trkseg>");
        for row in csv
            .lines()
            .filter(|l| l.starts_with(&format!("{segment},")))
        {
            let f: Vec<&str> = row.split(',').collect();
            write!(
                body,
                r#"<trkpt lat="{}" lon="{}"><ele>{}</ele></trkpt>"#,
                f[1], f[2], f[3]
            )
            .expect("write to a String");
        }
        body.push_str("</trkseg></trk>");
        let r = imported(&gpx(&body));
        let rel = (r.report.distance_m - model_m).abs() / model_m;
        assert!(
            rel <= 1e-3,
            "{segment}: {} m against {model_m} m",
            r.report.distance_m
        );
        assert!(
            rel <= 1e-12,
            "{segment}: same formula, so equal to rounding: {rel:e}"
        );
        assert_eq!(r.route.elevation, ElevationKind::Recorded);
        assert_eq!(r.report.dropped_null_island + r.report.merged_repeats, 0);
    }
}
