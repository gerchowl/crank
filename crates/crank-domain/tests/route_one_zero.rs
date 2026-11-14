//! #578: a route holds one zero (ADR-0005 §2.1). JSON in JavaScript cannot carry −0.0
//! (`JSON.stringify(-0)` is `0`), so the importers write +0.0 for a file's `-0` and `Route::check`
//! refuses −0.0 in any `f64`. `app/tests/route-json.test.ts` round-trips the pinned JSON of
//! `fixtures/gpx/negative-zero.gpx` through JavaScript itself.
use crank_domain::route::{
    GpxImport, RawPoint, Route, RouteImportOptions, RouteSource, import_gpx, parse_route_dsl,
    route_from_points,
};
use proptest::prelude::*;
use serde_json::Value;

fn fixture_dir() -> String {
    format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))
}

fn negative_zero_route() -> Route {
    let bytes = std::fs::read(format!("{}/gpx/negative-zero.gpx", fixture_dir())).expect("fixture");
    match import_gpx(&bytes, "negative-zero.gpx", &RouteImportOptions::default()) {
        Ok(GpxImport::Imported(r)) => r.route,
        other => panic!("{other:?}"),
    }
}

/// Every number of a JSON value as JavaScript's `JSON.stringify(JSON.parse(..))` gives it back:
/// the same double (`serde_json`'s `float_roundtrip` and JavaScript both print the shortest text
/// that parses back), except −0, which comes back as 0.
fn through_javascript(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if n.as_f64().is_some_and(|x| x == 0.0 && x.is_sign_negative()) {
                *v = Value::from(0);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(through_javascript),
        Value::Object(o) => o.values_mut().for_each(through_javascript),
        _ => {}
    }
}

/// The route after a trip through the UI: serialised, read and written by JavaScript, read back.
fn via_javascript(r: &Route) -> Route {
    let mut v = serde_json::to_value(r).expect("serialises");
    through_javascript(&mut v);
    serde_json::from_value(v).expect("deserialises")
}

/// Bits of every f64 of the route's JSON, in document order: equal exactly when no f64 changed.
fn json_bits(r: &Route) -> Vec<u64> {
    fn walk(v: &Value, out: &mut Vec<u64>) {
        match v {
            Value::Number(n) => out.push(n.as_f64().map_or(0, f64::to_bits)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&serde_json::to_value(r).expect("serialises"), &mut out);
    out
}

#[test]
fn a_file_route_written_with_negative_zeros_holds_plus_zero() {
    let r = negative_zero_route();
    let first = &r.points()[0];
    assert_eq!(first.ele_m.map(f64::to_bits), Some(0f64.to_bits()));
    assert_eq!(
        first.lat_lon.map(|(_, lon)| lon.to_bits()),
        Some(0f64.to_bits())
    );
    assert_eq!(json_bits(&via_javascript(&r)), json_bits(&r));
}

/// The fixture `app/tests/route-json.test.ts` reads: this route's JSON as the core sends it.
/// `CRANK_WRITE_ROUTE_FIXTURE=1` rewrites it.
#[test]
fn the_pinned_json_of_the_negative_zero_fixture_is_current() {
    let path = format!("{}/route-negative-zero.json", fixture_dir());
    let text = serde_json::to_string_pretty(&negative_zero_route()).expect("serialises") + "\n";
    if std::env::var_os("CRANK_WRITE_ROUTE_FIXTURE").is_some() {
        std::fs::write(&path, &text).expect("write the fixture");
    }
    let pinned = std::fs::read_to_string(&path).expect("the fixture exists");
    assert_eq!(pinned, text, "rewrite it with CRANK_WRITE_ROUTE_FIXTURE=1");
}

#[test]
fn an_empty_ascent_is_plus_zero() {
    // A route without elevation sums no rise; an empty f64 `sum()` would be −0.0.
    let raw: Vec<RawPoint> = (0..3)
        .map(|i| RawPoint {
            at: i,
            lat: 46.0 + 0.001 * f64::from(u32::try_from(i).expect("small")),
            lon: 8.0,
            ele_m: None,
            surface: None,
        })
        .collect();
    let r = route_from_points("flat".into(), RouteSource::GpxTrack, &raw).expect("imports");
    assert_eq!(r.report.ascent_m.to_bits(), 0f64.to_bits());
}

/// A route and a change made to its JSON.
type Change<'a> = (&'a Route, fn(&mut Value));

#[test]
fn check_refuses_negative_zero_in_every_f64() {
    let file = negative_zero_route();
    let profile = parse_route_dsl("\"A\" 1km 0%, 1km ramp 0%..2%").expect("parses");
    let changes: [Change<'_>; 6] = [
        (&file, |v| v["points"][1]["eleM"] = (-0.0f64).into()),
        (&file, |v| v["points"][1]["latLon"][0] = (-0.0f64).into()),
        (&file, |v| v["points"][1]["latLon"][1] = (-0.0f64).into()),
        (&file, |v| v["points"][0]["distM"] = (-0.0f64).into()),
        (&profile, |v| {
            v["segments"][0]["grade"]["pct"] = (-0.0f64).into();
        }),
        (&profile, |v| {
            v["segments"][1]["grade"]["fromPct"] = (-0.0f64).into();
        }),
    ];
    for (i, (r, change)) in changes.into_iter().enumerate() {
        let mut v = serde_json::to_value(r).expect("serialises");
        change(&mut v);
        let e = serde_json::from_value::<Route>(v).expect_err(&format!("change {i} refused"));
        assert!(
            e.to_string().contains("-0") || e.to_string().contains("first point"),
            "{e}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// A walk along the equator and the meridian whose coordinates and elevations are often ±0.
    #[test]
    fn every_imported_route_survives_javascript_bit_for_bit(
        steps in proptest::collection::vec((0u8..4, 0u8..4, 0u8..4), 2..24),
    ) {
        let zero = |k: u8, x: f64| match k { 0 => -0.0, 1 => 0.0, _ => x };
        let raw: Vec<RawPoint> = steps.iter().enumerate().map(|(i, &(a, b, e))| {
            let x = 0.001 * f64::from(u32::try_from(i).expect("small"));
            RawPoint {
                at: u64::try_from(i).expect("small"),
                // One of lat and lon stays ±0 only where the other moves, so the walk is not (0, 0).
                lat: if i % 2 == 0 { zero(a, -x) } else { -x },
                lon: if i % 2 == 0 { x + 0.01 } else { zero(b, x) },
                ele_m: Some(zero(e, 3.0)),
                surface: None,
            }
        }).collect();
        if let Ok(r) = route_from_points("walk".into(), RouteSource::GpxTrack, &raw) {
            prop_assert_eq!(json_bits(&via_javascript(&r.route)), json_bits(&r.route));
        }
    }
}
