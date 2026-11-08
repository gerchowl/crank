//! Round 2 of #539: the ADR-0005 §2.1 invariants under hostile point sequences, truncated and
//! malformed GPX, and every §2.2.4 limit at its exact boundary.
use crank_domain::route::{
    DEFAULT_ROUTE_NAME, ElevationKind, GpxImport, ImportedRoute, MAX_TRACKS, RawPoint,
    RouteImportError, RouteImportOptions, RouteSource, haversine_m, import_gpx, route_from_points,
};
use proptest::prelude::*;
use std::fmt::Write as _;

fn gpx(body: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1">{body}</gpx>"#
    )
}

/// One track through the points `(lat, lon, ele)`.
fn track(points: &[(f64, f64, Option<f64>)]) -> String {
    let mut body = String::from("<trk><trkseg>");
    for &(lat, lon, ele) in points {
        match ele {
            Some(e) => write!(
                body,
                r#"<trkpt lat="{lat}" lon="{lon}"><ele>{e}</ele></trkpt>"#
            ),
            None => write!(body, r#"<trkpt lat="{lat}" lon="{lon}"/>"#),
        }
        .expect("write to a String");
    }
    body.push_str("</trkseg></trk>");
    gpx(&body)
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

fn malformed(text: &str) -> String {
    match import(text) {
        Err(RouteImportError::Malformed { what, .. }) => what,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

// --- blocker 1: dist_m strictly increases, no NaN elevation ---

#[test]
fn moves_too_small_to_add_distance_are_merged_not_kept_at_the_same_distance() {
    // The falsifier's repro: 9 km to near the pole, then two moves of one float step in longitude
    // (the middle one without elevation). Each is about 1e-20 m: it adds nothing to 9 000 m.
    let lat = 89.999_999_9;
    let start = lat - 9_000.0 / (6_371_008.8 * std::f64::consts::PI / 180.0);
    let lon1 = 10.0_f64.next_up();
    let pts = [
        (start, 10.0, Some(1.0)),
        (lat, 10.0, Some(2.0)),
        (lat, lon1, None),
        (lat, lon1.next_up(), Some(3.0)),
    ];
    let r = imported(&track(&pts));
    assert_eq!(r.report.merged_repeats, 2);
    assert_eq!(r.report.points_kept, 2);
    assert!(
        r.route
            .points()
            .windows(2)
            .all(|w| w[1].dist_m > w[0].dist_m)
    );
    assert!(
        r.route
            .points()
            .iter()
            .all(|p| p.ele_m.is_some_and(f64::is_finite))
    );
    r.route.check().expect("the §2.1 invariants");
}

/// A cursor walk: each step moves by metres, by one float step, not at all, jumps, or emits a
/// hostile value, so near-pole and sub-ULP moves come up often.
fn walk() -> impl Strategy<Value = Vec<RawPoint>> {
    let start = prop_oneof![
        (-90.0..=90.0f64, -180.0..=180.0f64),
        (89.999_99..=90.0f64, -180.0..=180.0f64),
        Just((89.999_999_9, 10.0)),
        Just((0.0, 0.0)),
    ];
    let step = (0u8..9, 0.0..=1.0f64, 0u8..5, -600.0..=9_500.0f64);
    let prefix = any::<bool>();
    (start, prefix, proptest::collection::vec(step, 0..48)).prop_map(
        |((mut lat, mut lon), prefix, steps)| {
            let mut out = Vec::new();
            // Optionally a valid 200 m first, so the hostile steps that follow land on a route that
            // would otherwise import (sub-ULP moves only matter once distance has accumulated).
            if prefix {
                for i in 0..=5u8 {
                    let towards_equator = if lat > 0.0 { -1.0 } else { 1.0 };
                    if i > 0 {
                        lat += towards_equator * 0.000_36;
                    }
                    out.push(RawPoint {
                        at: 0,
                        lat,
                        lon,
                        ele_m: Some(100.0),
                        surface: None,
                    });
                }
            }
            for (i, (kind, x, ele_kind, ele)) in steps.into_iter().enumerate() {
                let (mut plat, mut plon) = (lat, lon);
                // Metre steps go towards the equator, so a walk from near a pole stays valid there.
                let towards_equator = if lat > 0.0 { -1.0 } else { 1.0 };
                match kind {
                    0 => lat += towards_equator * x * 0.000_3,
                    1 => lon += x * 0.000_3,
                    2 => lon = lon.next_up(),
                    3 => lat = lat.next_down(),
                    4 => {}
                    5 => (plat, plon) = (0.0, 0.0),
                    6 => lat += towards_equator * x * 0.2,
                    7 => plat = f64::NAN,
                    _ => lon = -lon,
                }
                if !matches!(kind, 5 | 7) {
                    (plat, plon) = (lat, lon);
                }
                let ele_m = match ele_kind {
                    0 | 1 => None,
                    2 => Some(f64::NAN),
                    _ => Some(ele),
                };
                out.push(RawPoint {
                    at: u64::try_from(i).unwrap_or(u64::MAX),
                    lat: plat,
                    lon: plon,
                    ele_m,
                    surface: None,
                });
            }
            out
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn every_point_sequence_is_refused_or_satisfies_the_invariants(raw in walk()) {
        let result = route_from_points("walk".into(), RouteSource::GpxTrack, &raw);
        // A refusal must be one of §2.2.4's, never the importer breaking §2.1 itself.
        prop_assert!(!matches!(result, Err(RouteImportError::Invariant { .. })), "{:?}", result);
        if let Ok(r) = result {
            prop_assert!(r.route.check().is_ok(), "{:?}", r.route.check());
            let p = &r.route.points();
            prop_assert_eq!(p[0].dist_m.to_bits(), 0f64.to_bits());
            prop_assert!(p.windows(2).all(|w| w[1].dist_m > w[0].dist_m));
            match r.route.elevation() {
                ElevationKind::None => prop_assert!(p.iter().all(|q| q.ele_m.is_none())),
                _ => prop_assert!(p.iter().all(|q| q.ele_m.is_some_and(f64::is_finite))),
            }
            prop_assert_eq!(r.report.points_kept, p.len());
        }
    }
}

#[test]
fn a_blank_name_becomes_the_default() {
    let raw: Vec<RawPoint> = [0.0, 0.001]
        .iter()
        .map(|&d| RawPoint {
            at: 0,
            lat: 46.0 + d,
            lon: 8.0,
            ele_m: None,
            surface: None,
        })
        .collect();
    let r = route_from_points("  ".into(), RouteSource::GpxTrack, &raw).expect("imports");
    assert_eq!(r.route.name(), DEFAULT_ROUTE_NAME);
    let blank = gpx(
        r#"<metadata><name> </name></metadata><trk><name></name><trkseg>
<trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk>"#,
    );
    match import_gpx(blank.as_bytes(), "", &RouteImportOptions::default()) {
        Ok(GpxImport::Imported(r)) => assert_eq!(r.route.name(), DEFAULT_ROUTE_NAME),
        other => panic!("{other:?}"),
    }
}

// --- blocker 2: a truncated file is malformed ---

#[test]
fn a_file_cut_at_any_byte_is_malformed_and_only_the_whole_file_imports() {
    let text = track(&[
        (46.0, 8.0, Some(500.0)),
        (46.001, 8.0, Some(501.0)),
        (46.002, 8.0, None),
        (46.003, 8.0, Some(503.0)),
    ]);
    assert!(text.is_ascii());
    for cut in 0..text.len() {
        assert!(
            matches!(
                import(&text[..cut]),
                Err(RouteImportError::Malformed { .. })
            ),
            "cut at {cut}: {:?} gave {:?}",
            &text[..cut],
            import(&text[..cut])
        );
    }
    assert_eq!(imported(&text).report.points_kept, 4);
}

// --- XML around the root ---

#[test]
fn text_before_or_after_the_root_is_refused() {
    let ok = track(&[(46.0, 8.0, None), (46.01, 8.0, None)]);
    assert!(malformed(&format!("junk{ok}")).contains("outside"));
    assert!(malformed(&format!("{ok}junk")).contains("outside"));
    assert!(malformed(&format!("{ok}&amp;")).contains("outside"));
    assert!(malformed(&format!("{ok}<![CDATA[x]]>")).contains("outside"));
    assert_eq!(
        imported(&format!("\n {ok}\n\t<!-- c -->\n"))
            .report
            .points_kept,
        2
    );
}

#[test]
fn a_utf8_bom_is_skipped() {
    let text = format!(
        "\u{feff}{}",
        track(&[(46.0, 8.0, None), (46.01, 8.0, None)])
    );
    assert_eq!(imported(&text).report.points_kept, 2);
}

#[test]
fn the_root_must_be_gpx_in_the_gpx_1_0_or_1_1_namespace() {
    let body =
        r#"<trk><trkseg><trkpt lat="46" lon="8"/><trkpt lat="46.01" lon="8"/></trkseg></trk>"#;
    for root in [
        r#"gpx xmlns="http://www.topografix.com/GPX/1/0""#,
        r#"gpx xmlns="http://www.topografix.com/GPX/1/1""#,
    ] {
        let text = format!("<{root}>{body}</gpx>");
        assert_eq!(imported(&text).report.points_kept, 2, "{root}");
    }
    let prefixed = r#"<g:gpx xmlns:g="http://www.topografix.com/GPX/1/1"><g:trk><g:trkseg>
<g:trkpt lat="46" lon="8"/><g:trkpt lat="46.01" lon="8"/></g:trkseg></g:trk></g:gpx>"#;
    assert_eq!(imported(prefixed).report.points_kept, 2);
    for root in [
        "gpx",
        r#"gpx xmlns="urn:not-gpx""#,
        r#"k:gpx xmlns:k="http://www.opengis.net/kml/2.2""#,
        "k:gpx",
        r#"k:gpx xmlns="http://www.topografix.com/GPX/1/1""#,
    ] {
        // No body: a root wrongly accepted would give NoTrack, not this refusal of the root.
        let name = root.split(' ').next().unwrap_or(root);
        let what = malformed(&format!("<{root}></{name}>"));
        assert!(
            what.contains("root") || what.contains("not declared"),
            "{root}: {what}"
        );
    }
}

#[test]
fn at_most_1000_tracks_per_file() {
    let tracks = |n: usize| gpx(&"<trk/>".repeat(n));
    match import(&tracks(MAX_TRACKS)) {
        Ok(GpxImport::ChooseTrack { tracks }) => assert_eq!(tracks.len(), MAX_TRACKS),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        import(&tracks(MAX_TRACKS + 1)),
        Err(RouteImportError::TooManyTracks { max: MAX_TRACKS })
    );
}

// --- §2.2.4 boundaries, ±1 unit ---

/// Two latitudes on the meridian 1° E whose haversine distance is exactly `target_m`, found by
/// bisection on the f64 grid (then trying the next start until one hits it exactly).
fn exact_leg(target_m: f64) -> (f64, f64) {
    let d = |a: f64, b: f64| haversine_m((a, 1.0), (b, 1.0));
    let mut a = 0.001_f64;
    for _ in 0..10_000 {
        let (mut lo, mut hi) = (a, a + 1.0);
        while lo.next_up() < hi {
            let mid = lo + (hi - lo) / 2.0;
            if mid <= lo || mid >= hi {
                break;
            }
            if d(a, mid) < target_m {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        for b in [lo, hi] {
            if d(a, b).to_bits() == target_m.to_bits() {
                return (a, b);
            }
        }
        a = a.next_up();
    }
    panic!("no exact leg of {target_m} m");
}

/// Points zigzagging between `a` and `b` (each leg the same exact distance), `legs` legs.
fn zigzag(
    (a, b): (f64, f64),
    legs: usize,
    ele: impl Fn(usize) -> Option<f64>,
) -> Vec<(f64, f64, Option<f64>)> {
    (0..=legs)
        .map(|i| (if i % 2 == 0 { a } else { b }, 1.0, ele(i)))
        .collect()
}

#[test]
fn a_gap_of_exactly_500_m_is_filled_and_one_step_more_is_refused() {
    let leg = exact_leg(250.0);
    // Known at 0 m and 500.0 m exactly, the point between has no elevation.
    let exact = imported(&track(&zigzag(leg, 2, |i| (i != 1).then_some(10.0))));
    assert_eq!(exact.report.gaps_filled.len(), 1);
    assert_eq!(exact.report.gaps_filled[0].to_m.to_bits(), 500f64.to_bits());
    let over = (leg.0, leg.1.next_up());
    assert!(haversine_m((over.0, 1.0), (over.1, 1.0)) > 250.0);
    assert!(matches!(
        import(&track(&zigzag(over, 2, |i| (i != 1).then_some(10.0)))),
        Err(RouteImportError::MissingElevation { .. })
    ));
}

#[test]
fn a_step_of_exactly_10_km_is_accepted_and_one_step_more_is_a_jump() {
    let leg = exact_leg(10_000.0);
    assert_eq!(
        imported(&track(&zigzag(leg, 1, |_| None)))
            .report
            .distance_m
            .to_bits(),
        10_000f64.to_bits()
    );
    let over = (leg.0, leg.1.next_up());
    assert!(matches!(
        import(&track(&zigzag(over, 1, |_| None))),
        Err(RouteImportError::Jump { .. })
    ));
}

#[test]
fn a_route_of_exactly_100_m_and_exactly_1000_km_is_accepted() {
    let leg = exact_leg(100.0);
    assert_eq!(
        imported(&track(&zigzag(leg, 1, |_| None)))
            .report
            .distance_m
            .to_bits(),
        100f64.to_bits()
    );
    let under = (leg.0, leg.1.next_down());
    assert!(matches!(
        import(&track(&zigzag(under, 1, |_| None))),
        Err(RouteImportError::TooShort { .. })
    ));
    let ten_km = exact_leg(10_000.0);
    let r = imported(&track(&zigzag(ten_km, 100, |_| None)));
    assert_eq!(r.report.distance_m.to_bits(), 1_000_000f64.to_bits());
    let mut over = zigzag(ten_km, 100, |_| None);
    over.push((ten_km.0 + 0.001, 1.0, None));
    assert!(matches!(
        import(&track(&over)),
        Err(RouteImportError::TooLong { .. })
    ));
}

#[test]
fn latitude_and_longitude_bounds_are_inclusive() {
    let at = |lat: f64, lon: f64| {
        track(&[
            (lat, lon, None),
            (lat + 0.001_f64.copysign(-lat), lon, None),
        ])
    };
    assert_eq!(imported(&at(-90.0, 8.0)).report.points_kept, 2);
    assert!(malformed(&at((-90.0_f64).next_down(), 8.0)).contains("latitude"));
    assert_eq!(imported(&at(46.0, 180.0)).report.points_kept, 2);
    assert!(malformed(&at(46.0, 180.0_f64.next_up())).contains("longitude"));
    assert_eq!(imported(&at(90.0, 8.0)).report.points_kept, 2);
    assert_eq!(imported(&at(46.0, -180.0)).report.points_kept, 2);
}

#[test]
fn a_non_finite_elevation_is_malformed() {
    for ele in ["1e999", "-1e999"] {
        let text = gpx(&format!(
            r#"<trk><trkseg><trkpt lat="46" lon="8"><ele>{ele}</ele></trkpt><trkpt lat="46.01" lon="8"/></trkseg></trk>"#
        ));
        assert!(malformed(&text).contains("elevation"), "{ele}");
    }
}

#[test]
fn route_new_refuses_every_broken_invariant() {
    use crank_domain::route::{Route, RoutePoint, RouteSegment, SegmentGrade, SurfaceProvenance};
    let pt = |dist_m: f64, ele_m: Option<f64>| RoutePoint {
        dist_m,
        ele_m,
        lat_lon: Some((46.0, 8.0)),
        surface: None,
    };
    let seg = |start_m: f64, end_m: f64| RouteSegment {
        start_m,
        end_m,
        label: None,
        grade: SegmentGrade::FromPoints,
        surface: None,
    };
    let build = |name: &str, points: Vec<RoutePoint>, segments: Vec<RouteSegment>, elevation| {
        Route::new(
            name.into(),
            RouteSource::GpxTrack,
            points,
            segments,
            elevation,
            SurfaceProvenance::default(),
        )
    };
    let good = || vec![pt(0.0, Some(1.0)), pt(10.0, Some(2.0))];
    assert!(build("r", good(), vec![seg(0.0, 10.0)], ElevationKind::Recorded).is_ok());
    let broken = [
        build(" ", good(), vec![], ElevationKind::Recorded),
        build(
            "r",
            vec![pt(0.0, Some(1.0))],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![pt(1.0, Some(1.0)), pt(10.0, Some(1.0))],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![pt(0.0, Some(1.0)), pt(0.0, Some(1.0))],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![pt(0.0, Some(1.0)), pt(f64::NAN, Some(1.0))],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![pt(0.0, Some(1.0)), pt(10.0, Some(f64::NAN))],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![pt(0.0, Some(1.0)), pt(10.0, None)],
            vec![],
            ElevationKind::Recorded,
        ),
        build("r", good(), vec![], ElevationKind::None),
        build("r", good(), vec![seg(0.0, 11.0)], ElevationKind::Recorded),
        build("r", good(), vec![seg(5.0, 5.0)], ElevationKind::Recorded),
        build(
            "r",
            vec![
                pt(0.0, Some(1.0)),
                RoutePoint {
                    lat_lon: Some((91.0, 8.0)),
                    ..pt(10.0, Some(1.0))
                },
            ],
            vec![],
            ElevationKind::Recorded,
        ),
        build(
            "r",
            vec![
                pt(0.0, Some(1.0)),
                RoutePoint {
                    lat_lon: None,
                    ..pt(10.0, Some(1.0))
                },
            ],
            vec![],
            ElevationKind::Recorded,
        ),
    ];
    for (i, b) in broken.iter().enumerate() {
        assert!(b.is_err(), "case {i} was accepted");
    }
}
