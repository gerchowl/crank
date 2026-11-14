//! The `route` table (ADR-0005 §2.1): a route's points as versioned little-endian columns, its
//! segments and surface provenance as JSON, and the imported file's bytes.
use crate::StoreError;
use crank_domain::route::{
    ElevationKind, Route, RoutePoint, RouteSegment, RouteSource, Surface, SurfaceProvenance,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Version byte of the `route.points` encoding ([`encode_points`]).
const POINTS_V1: u8 = 1;
/// The NaN that stands for an absent `ele_m`, `lat` or `lon`. Only this bit pattern decodes as
/// absent: any other NaN is corruption, so decoding is the exact inverse of encoding.
const ABSENT: u64 = 0x7ff8_0000_0000_0000;
/// Largest original file stored with a route (ADR-0005 §2.2.4, `MAX_FILE_BYTES`).
const MAX_ORIGINAL_BYTES: usize = crank_domain::route::MAX_FILE_BYTES;

/// The imported file a route was built from: its "link to the original file" that survives the
/// file being moved or deleted (ADR-0005 §2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteOriginal {
    /// The file's name as the rider picked it.
    pub name: String,
    /// Its bytes, at most 32 MiB.
    pub bytes: Vec<u8>,
}

/// A stored route ([`crate::Store::route`]).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredRoute {
    /// Stable identifier.
    pub id: String,
    /// The route, every ADR-0005 §2.1 invariant checked on reading.
    pub route: Route,
    /// The imported file's name, if the route came from a file.
    pub original_name: Option<String>,
    /// Lowercase hex SHA-256 of the imported file, if the route came from a file.
    pub original_sha256: Option<String>,
    /// UTC Unix seconds when it was stored.
    pub created_at: i64,
}

/// One row of the route list ([`crate::Store::routes`]): no points, no file.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteListing {
    /// Stable identifier.
    pub id: String,
    /// The route's name.
    pub name: String,
    /// Where it came from (the grade profile's text included).
    pub source: RouteSource,
    /// Whether its points carry elevation.
    pub elevation: ElevationKind,
    /// Its length, metres: the last point's distance.
    pub distance_m: f64,
    /// UTC Unix seconds when it was stored.
    pub created_at: i64,
}

fn corrupt(what: impl Into<String>) -> StoreError {
    StoreError::CorruptRoute(what.into())
}

fn optional_bits(x: Option<f64>) -> u64 {
    x.map_or(ABSENT, f64::to_bits)
}

fn decode_optional(bits: u64, what: &str, i: usize) -> Result<Option<f64>, StoreError> {
    let x = f64::from_bits(bits);
    match (bits == ABSENT, x.is_nan()) {
        (true, _) => Ok(None),
        (false, false) => Ok(Some(x)),
        (false, true) => Err(corrupt(format!(
            "point {i}: {what} is a NaN other than absent"
        ))),
    }
}

const fn surface_code(s: Option<Surface>) -> u8 {
    match s {
        None => 0,
        Some(Surface::SmoothPaved) => 1,
        Some(Surface::RoughPaved) => 2,
        Some(Surface::Gravel) => 3,
        Some(Surface::Grass) => 4,
        Some(Surface::Sand) => 5,
        Some(Surface::Cobbles) => 6,
    }
}

fn surface_from_code(code: u8, i: usize) -> Result<Option<Surface>, StoreError> {
    Ok(match code {
        0 => None,
        1..=6 => Some(Surface::ALL[usize::from(code - 1)]),
        _ => return Err(corrupt(format!("point {i}: surface code {code}"))),
    })
}

/// `route.points`: a version byte, the point count as u32, then one column per field, every
/// f64 little-endian by its bits: `dist_m`, `ele_m`, `lat`, `lon` (an absent value is the one
/// NaN `0x7ff8_0000_0000_0000`), then
/// one surface byte per point (0 = none, else [`Surface::ALL`] index + 1).
///
/// # Errors
/// `InvalidInput` for more points than a u32 counts (a route has at most 200 000).
pub fn encode_points(points: &[RoutePoint]) -> Result<Vec<u8>, StoreError> {
    let n = u32::try_from(points.len())
        .map_err(|_| StoreError::InvalidInput(format!("{} route points", points.len())))?;
    let mut out = Vec::with_capacity(5 + points.len() * 33);
    out.push(POINTS_V1);
    out.extend_from_slice(&n.to_le_bytes());
    let columns: [fn(&RoutePoint) -> u64; 4] = [
        |p| p.dist_m.to_bits(),
        |p| optional_bits(p.ele_m),
        |p| optional_bits(p.lat_lon.map(|(lat, _)| lat)),
        |p| optional_bits(p.lat_lon.map(|(_, lon)| lon)),
    ];
    for column in columns {
        for p in points {
            out.extend_from_slice(&column(p).to_le_bytes());
        }
    }
    out.extend(points.iter().map(|p| surface_code(p.surface)));
    Ok(out)
}

/// The inverse of [`encode_points`].
///
/// # Errors
/// `CorruptRoute` for another version, a length that does not match the count, a NaN that is
/// not the absent marker, exactly one of `lat` and `lon` absent, or an unknown surface code.
pub fn decode_points(blob: &[u8]) -> Result<Vec<RoutePoint>, StoreError> {
    let (&version, rest) = blob.split_first().ok_or_else(|| corrupt("empty points"))?;
    if version != POINTS_V1 {
        return Err(corrupt(format!("points version {version}")));
    }
    let (count, columns) = rest
        .split_first_chunk::<4>()
        .ok_or_else(|| corrupt("no point count"))?;
    let n = usize::try_from(u32::from_le_bytes(*count)).map_err(|_| corrupt("point count"))?;
    if Some(columns.len()) != n.checked_mul(33) {
        return Err(corrupt(format!("{} bytes for {n} points", columns.len())));
    }
    let f64_at = |column: usize, i: usize| {
        let at = (column * n + i) * 8;
        let mut b = [0u8; 8];
        b.copy_from_slice(&columns[at..at + 8]);
        u64::from_le_bytes(b)
    };
    (0..n)
        .map(|i| {
            let dist_m = f64::from_bits(f64_at(0, i));
            let ele_m = decode_optional(f64_at(1, i), "elevation", i)?;
            let lat_lon = match (
                decode_optional(f64_at(2, i), "latitude", i)?,
                decode_optional(f64_at(3, i), "longitude", i)?,
            ) {
                (Some(lat), Some(lon)) => Some((lat, lon)),
                (None, None) => None,
                _ => return Err(corrupt(format!("point {i}: half a position"))),
            };
            let surface = surface_from_code(columns[32 * n + i], i)?;
            Ok(RoutePoint {
                dist_m,
                ele_m,
                lat_lon,
                surface,
            })
        })
        .collect()
}

fn source_column(s: &RouteSource) -> (&'static str, Option<&str>) {
    match s {
        RouteSource::GpxTrack => ("gpx_track", None),
        RouteSource::GpxRoute => ("gpx_route", None),
        RouteSource::FitCourse => ("fit_course", None),
        RouteSource::GradeProfile { dsl } => ("grade_profile", Some(dsl)),
    }
}

fn source_from_column(kind: &str, dsl: Option<String>) -> Result<RouteSource, StoreError> {
    Ok(match (kind, dsl) {
        ("gpx_track", None) => RouteSource::GpxTrack,
        ("gpx_route", None) => RouteSource::GpxRoute,
        ("fit_course", None) => RouteSource::FitCourse,
        ("grade_profile", Some(dsl)) => RouteSource::GradeProfile { dsl },
        (kind, dsl) => {
            return Err(corrupt(format!(
                "source {kind:?} with dsl {:?}",
                dsl.is_some()
            )));
        }
    })
}

const fn elevation_column(e: ElevationKind) -> &'static str {
    match e {
        ElevationKind::Recorded => "recorded",
        ElevationKind::None => "none",
        ElevationKind::Exact => "exact",
    }
}

fn elevation_from_column(s: &str) -> Result<ElevationKind, StoreError> {
    Ok(match s {
        "recorded" => ElevationKind::Recorded,
        "none" => ElevationKind::None,
        "exact" => ElevationKind::Exact,
        other => return Err(corrupt(format!("elevation {other:?}"))),
    })
}

fn json_column<T: serde::de::DeserializeOwned>(
    column: &'static str,
    text: &str,
) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError::InvalidJson {
        table: "route",
        column,
        reason: e.to_string(),
    })
}

fn to_json<T: serde::Serialize>(column: &'static str, v: &T) -> Result<String, StoreError> {
    serde_json::to_string(v).map_err(|e| StoreError::InvalidJson {
        table: "route",
        column,
        reason: e.to_string(),
    })
}

/// Insert a new route. The caller checked nothing: the route is a `Route`, so its invariants
/// hold, and the original's hash is computed here, so it always matches its bytes.
pub(crate) fn insert(
    db: &Connection,
    id: &str,
    route: &Route,
    original: Option<&RouteOriginal>,
    created_at: i64,
) -> Result<(), StoreError> {
    if let Some(o) = original
        && o.bytes.len() > MAX_ORIGINAL_BYTES
    {
        return Err(StoreError::InvalidInput(format!(
            "original file of {} bytes, more than {MAX_ORIGINAL_BYTES}",
            o.bytes.len()
        )));
    }
    let (source, dsl) = source_column(route.source());
    let sha = original.map(|o| {
        Sha256::digest(&o.bytes)
            .iter()
            .fold(String::with_capacity(64), |mut hex, b| {
                // Writing to a String cannot fail.
                let _ = write!(hex, "{b:02x}");
                hex
            })
    });
    let inserted = db.execute(
        "INSERT INTO route (id,name,source,elevation,points,segments_json,surface_json,dsl,
                            original_name,original_sha256,original,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO NOTHING",
        params![
            id,
            route.name(),
            source,
            elevation_column(route.elevation()),
            encode_points(route.points())?,
            to_json("segments_json", &route.segments())?,
            to_json("surface_json", route.surface())?,
            dsl,
            original.map(|o| &o.name),
            sha,
            original.map(|o| &o.bytes),
            created_at,
        ],
    )?;
    if inserted == 0 {
        return Err(StoreError::InvalidInput(format!(
            "route id {id:?} is taken"
        )));
    }
    Ok(())
}

/// One route by id, rebuilt through `Route::new` (every invariant checked).
pub(crate) fn select(db: &Connection, id: &str) -> Result<Option<StoredRoute>, StoreError> {
    let row = db
        .query_row(
            "SELECT name,source,elevation,points,segments_json,surface_json,dsl,
                    original_name,original_sha256,created_at FROM route WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        name,
        source,
        elevation,
        points,
        segments,
        surface,
        dsl,
        original_name,
        sha,
        created_at,
    )) = row
    else {
        return Ok(None);
    };
    let segments: Vec<RouteSegment> = json_column("segments_json", &segments)?;
    let surface: SurfaceProvenance = json_column("surface_json", &surface)?;
    let route = Route::new(
        name,
        source_from_column(&source, dsl)?,
        decode_points(&points)?,
        segments,
        elevation_from_column(&elevation)?,
        surface,
    )
    .map_err(|e| corrupt(e.to_string()))?;
    Ok(Some(StoredRoute {
        id: id.to_owned(),
        route,
        original_name,
        original_sha256: sha,
        created_at,
    }))
}

/// Every route, newest first, without points or file: the length is read from the last
/// `dist_m` of the points column alone.
pub(crate) fn list(db: &Connection) -> Result<Vec<RouteListing>, StoreError> {
    // The count is bytes 2..=5 of `points`; the last distance is the n-th f64 after them.
    let mut stmt = db.prepare(
        "SELECT id,name,source,dsl,elevation,created_at,substr(points,1,5) FROM route
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Vec<u8>>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut last = db.prepare("SELECT substr(points,?2,8) FROM route WHERE id=?1")?;
    rows.into_iter()
        .map(|(id, name, source, dsl, elevation, created_at, header)| {
            let n = match header.as_slice() {
                [POINTS_V1, a, b, c, d] => u32::from_le_bytes([*a, *b, *c, *d]),
                _ => return Err(corrupt(format!("route {id}: points header"))),
            };
            let at = n
                .checked_sub(1)
                .and_then(|k| k.checked_mul(8))
                .and_then(|o| o.checked_add(6))
                .ok_or_else(|| corrupt(format!("route {id}: {n} points")))?;
            let bytes: Vec<u8> = last.query_row(params![id, at], |r| r.get(0))?;
            let distance_m = <[u8; 8]>::try_from(bytes.as_slice())
                .map(f64::from_le_bytes)
                .map_err(|_| corrupt(format!("route {id}: points shorter than their count")))?;
            Ok(RouteListing {
                source: source_from_column(&source, dsl)?,
                elevation: elevation_from_column(&elevation)?,
                id,
                name,
                distance_m,
                created_at,
            })
        })
        .collect()
}

/// Delete a route; rides ridden on it keep their `route_json` and lose only the link
/// (`ride.route_id` is `ON DELETE SET NULL`). Returns whether a row existed.
pub(crate) fn delete(db: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(db.execute("DELETE FROM route WHERE id=?1", [id])? > 0)
}
