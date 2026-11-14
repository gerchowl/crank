//! ADR-0005 §2.9 R3 (#541): `put_route`/`get_route` bit for bit, f64 NaN (absent values)
//! included; the migration is forward-only; deleting a route keeps its rides readable.
use crank_domain::route::{
    ElevationKind, GpxImport, OsmMatch, Route, RouteImportOptions, RoutePoint, RouteSource,
    SegmentGrade, Surface, SurfaceProvenance, import_gpx, parse_route_dsl,
};
use crank_store::{RouteOriginal, Store, StoreError, decode_points, encode_points};
use proptest::prelude::*;
use rusqlite::{Connection, params};

/// A route by the bits of every `f64`, and by its `Debug` text for everything else.
type Bits = (String, Vec<[Option<u64>; 4]>, Vec<[u64; 5]>);

fn bits(r: &Route) -> Bits {
    let points = r
        .points()
        .iter()
        .map(|p| {
            [
                Some(p.dist_m.to_bits()),
                p.ele_m.map(f64::to_bits),
                p.lat_lon.map(|(lat, _)| lat.to_bits()),
                p.lat_lon.map(|(_, lon)| lon.to_bits()),
            ]
        })
        .collect();
    let segments = r
        .segments()
        .iter()
        .map(|s| {
            let (kind, a, b) = match s.grade {
                SegmentGrade::FromPoints => (0, 0, 0),
                SegmentGrade::Constant { pct } => (1, pct.to_bits(), 0),
                SegmentGrade::Ramp { from_pct, to_pct } => {
                    (2, from_pct.to_bits(), to_pct.to_bits())
                }
            };
            [s.start_m.to_bits(), s.end_m.to_bits(), kind, a, b]
        })
        .collect();
    (format!("{r:?}"), points, segments)
}

/// A file route from arbitrary finite values: any bit pattern a finite f64 can have (subnormals
/// and the extremes included), absent elevations (stored as NaN) or none at all, any surface.
fn file_route() -> impl Strategy<Value = Route> {
    let step = (
        prop_oneof![
            1e-300..1e-3f64,
            1e-3..10_000.0f64,
            Just(f64::MIN_POSITIVE),
            Just(5e-324)
        ],
        -90.0..=90.0f64,
        -180.0..=180.0f64,
        prop_oneof![Just(-500.0), -500.0..=9_000.0f64, Just(f64::MIN_POSITIVE)],
        proptest::option::of(0usize..6),
    );
    (
        proptest::collection::vec(step, 2..64),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(steps, has_ele, track, osm)| {
            let mut dist = 0.0;
            let points: Vec<RoutePoint> = steps
                .iter()
                .enumerate()
                .map(|(i, &(d, lat, lon, ele, surface))| {
                    if i > 0 {
                        // A step too small to add at this distance moves one float step instead.
                        let next = dist + d;
                        dist = if next > dist { next } else { dist.next_up() };
                    }
                    RoutePoint {
                        dist_m: dist,
                        ele_m: has_ele.then_some(ele),
                        lat_lon: Some((lat + 0.0, lon + 0.0)),
                        surface: surface.map(|k| Surface::ALL[k]),
                    }
                })
                .collect();
            Route::new(
                "prop".into(),
                if track {
                    RouteSource::GpxTrack
                } else {
                    RouteSource::GpxRoute
                },
                points,
                Vec::new(),
                if has_ele {
                    ElevationKind::Recorded
                } else {
                    ElevationKind::None
                },
                SurfaceProvenance {
                    osm: osm.then(|| OsmMatch {
                        region: "europe/slovenia".into(),
                        extract_at: 1_762_300_000,
                        matched_at: 1_762_400_000,
                        matched_share: 0.123_456_79,
                    }),
                    from_file: true,
                },
            )
            .expect("a valid route")
        })
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open_owner(dir.path()).expect("opens");
    (dir, store)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn put_then_get_is_bit_for_bit(route in file_route()) {
        let (_dir, store) = store();
        store.put_route("r".into(), route.clone(), None, 1).expect("put");
        let got = store.route("r").expect("get").expect("stored");
        prop_assert_eq!(bits(&got.route), bits(&route));
    }

    #[test]
    fn the_points_column_decodes_to_what_was_encoded(route in file_route()) {
        let blob = encode_points(route.points()).expect("encodes");
        let back = decode_points(&blob).expect("decodes");
        prop_assert_eq!(encode_points(&back).expect("encodes"), blob);
    }
}

#[test]
fn a_grade_profile_and_a_gpx_import_with_its_file_round_trip() {
    let (_dir, store) = store();
    let profile = parse_route_dsl("\"Col\" 2km 0%, 3km ramp 6.5%..9% cobbles, 3x(0.4km 12%)")
        .expect("parses");
    store
        .put_route("p".into(), profile.clone(), None, 10)
        .expect("put");
    let gpx = br#"<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><trk><name>T</name>
<trkseg><trkpt lat="46" lon="8"><ele>1</ele></trkpt><trkpt lat="46.001" lon="8"/>
<trkpt lat="46.002" lon="8"><ele>3</ele></trkpt></trkseg></trk></gpx>"#;
    let Ok(GpxImport::Imported(file)) = import_gpx(gpx, "t.gpx", &RouteImportOptions::default())
    else {
        panic!("imports")
    };
    let original = RouteOriginal {
        name: "t.gpx".into(),
        bytes: gpx.to_vec(),
    };
    store
        .put_route("f".into(), file.route.clone(), Some(original), 20)
        .expect("put");

    let p = store.route("p").expect("get").expect("stored");
    assert_eq!(bits(&p.route), bits(&profile));
    assert_eq!((p.original_name, p.original_sha256), (None, None));
    let f = store.route("f").expect("get").expect("stored");
    assert_eq!(bits(&f.route), bits(&file.route));
    assert_eq!(f.original_name.as_deref(), Some("t.gpx"));
    assert_eq!(f.original_sha256.as_deref().map(str::len), Some(64));
    assert_eq!(store.route_original("f").expect("read"), Some(gpx.to_vec()));

    let list = store.routes().expect("list");
    assert_eq!(
        list.iter()
            .map(|r| (r.id.as_str(), r.distance_m.to_bits()))
            .collect::<Vec<_>>(),
        [
            (
                "f",
                file.route.points().last().expect("points").dist_m.to_bits()
            ),
            (
                "p",
                profile.points().last().expect("points").dist_m.to_bits()
            ),
        ]
    );
    assert!(matches!(
        store.put_route("p".into(), profile, None, 30),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(store.delete_route("p").expect("delete"));
    assert!(!store.delete_route("p").expect("delete"));
    assert!(store.route("p").expect("get").is_none());
}

#[test]
fn a_corrupt_points_column_is_refused_not_misread() {
    let route = parse_route_dsl("1km 2%").expect("parses");
    let good = encode_points(route.points()).expect("encodes");
    let mut other_nan = good.clone();
    // Point 0's elevation: a NaN with another payload than the absent marker.
    let ele0 = 5 + 8 * route.points().len();
    other_nan[ele0..ele0 + 8].copy_from_slice(&0x7ff8_0000_0000_0001u64.to_le_bytes());
    let mut version = good.clone();
    version[0] = 2;
    let mut surface = good.clone();
    *surface.last_mut().expect("bytes") = 7;
    for blob in [
        other_nan,
        version,
        surface,
        good[..good.len() - 1].to_vec(),
        Vec::new(),
    ] {
        assert!(matches!(
            decode_points(&blob),
            Err(StoreError::CorruptRoute(_))
        ));
    }
}

#[test]
fn deleting_a_route_keeps_its_rides_readable() {
    let (dir, store) = store();
    let route = parse_route_dsl("1km 2%").expect("parses");
    store.put_route("r".into(), route, None, 1).expect("put");
    // R3 adds the columns; a route ride writes them in R7 (#545). Write one here by hand.
    let db = Connection::open(dir.path().join("crank.db")).expect("open");
    db.pragma_update(None, "foreign_keys", true).expect("fk");
    db.execute(
        "INSERT INTO ride (id,started_at,ended_at,state,workout_json,compiled_json,telemetry_source,
                           route_id,speed_source,route_json)
         VALUES ('ride',1,2,'aborted','{}','{}','sim','r','model','{\"profileVersion\":1}')",
        [],
    )
    .expect("insert a route ride");
    assert!(store.delete_route("r").expect("delete"));
    let (route_id, speed, json): (Option<String>, String, String) = db
        .query_row(
            "SELECT route_id,speed_source,route_json FROM ride WHERE id='ride'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("the ride is still there");
    assert_eq!(
        (route_id, speed.as_str(), json.as_str()),
        (None, "model", "{\"profileVersion\":1}")
    );
    // A route ride without its snapshot is refused by the schema.
    assert!(
        db.execute(
            "INSERT INTO ride (id,started_at,workout_json,compiled_json,telemetry_source,speed_source)
             VALUES ('bad',1,'{}','{}','sim','model')",
            params![],
        )
        .is_err()
    );
}

#[test]
fn the_route_migration_is_forward_only() {
    let (dir, store) = store();
    let version = store.schema_version().expect("version");
    drop(store);
    // A build that knows one migration fewer must refuse this database, never read it.
    let db = Connection::open(dir.path().join("crank.db")).expect("open");
    db.pragma_update(None, "user_version", version + 1)
        .expect("bump");
    drop(db);
    assert!(matches!(
        Store::open_read_only(dir.path()),
        Err(StoreError::SchemaTooNew { .. })
    ));
}

#[test]
fn a_v3_ride_migrates_with_no_route() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("fixtures/v3.db"),
    )
    .expect("copy");
    let store = Store::open_owner(dir.path()).expect("migrates");
    assert_eq!(store.schema_version().expect("version"), 4);
    let rides = store.rides().expect("rides");
    assert_eq!(
        rides.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["v3-ride"]
    );
    drop(store);
    let db = Connection::open(dir.path().join("crank.db")).expect("open");
    let route: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT route_id,speed_source,route_json FROM ride",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("read");
    assert_eq!(route, (None, None, None));
}
