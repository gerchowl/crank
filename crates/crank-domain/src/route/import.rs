//! From a file's points to a [`Route`]: the limits of ADR-0005 §2.2.4 and the missing-elevation
//! rules of §2.2.2, shared by every file format (GPX here, FIT in #540).
use super::{
    DEFAULT_ROUTE_NAME, ELE_GAP_M, ELE_MAX_M, ELE_MIN_M, ElevationKind, MAX_JUMP_M, MAX_POINTS,
    MAX_ROUTE_M, MIN_ROUTE_M, Route, RouteInvariantError, RoutePoint, RouteSource, Surface,
    SurfaceProvenance, haversine_m,
};
use serde::{Deserialize, Serialize};

/// One point as a file states it, before any limit is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawPoint {
    /// Where the point is in the file (a byte offset for GPX), named by [`RouteImportError::Malformed`].
    pub at: u64,
    /// Latitude, degrees.
    pub lat: f64,
    /// Longitude, degrees.
    pub lon: f64,
    /// Elevation, metres, if the file gives one.
    pub ele_m: Option<f64>,
    /// Surface, if the file gives one.
    pub surface: Option<Surface>,
}

/// Choices the rider makes for one import (ADR-0005 §2.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RouteImportOptions {
    /// Which track or route of a file with several to import, in document order ([`super::TrackChoice::index`]).
    pub track: Option<usize>,
}

/// A run of points without elevation that was filled (ADR-0005 §2.2.2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FilledGap {
    /// Distance of the known elevation before the run (0 for a missing start), metres.
    pub from_m: f64,
    /// Distance of the known elevation after the run (the route's end for a missing end), metres.
    pub to_m: f64,
    /// Points filled.
    pub points: usize,
}

/// What an import kept, dropped and filled, shown before the route is saved (ADR-0005 §2.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RouteImportReport {
    /// Points in the route.
    pub points_kept: usize,
    /// Points at exactly (0, 0), a logger's "no fix yet", dropped.
    pub dropped_null_island: usize,
    /// Points that add no distance to the previous one (the same position, or a move too small
    /// to change the along-route distance at its magnitude), merged into it.
    pub merged_repeats: usize,
    /// Elevations outside [`ELE_MIN_M`]..=[`ELE_MAX_M`], treated as missing (a sensor fault).
    pub elevation_out_of_range: usize,
    /// Runs of missing elevation that were filled.
    pub gaps_filled: Vec<FilledGap>,
    /// The route's elevation kind.
    pub elevation: ElevationKind,
    /// The route's length, metres.
    pub distance_m: f64,
    /// Sum of the rises between consecutive points, metres, before the smoothing of ADR-0005 §2.3
    /// (the profile's smoothed ascent is #542's).
    pub ascent_m: f64,
    /// Points whose surface the file gives.
    pub surfaces_from_file: usize,
    /// Surface values in the file that name no class of ADR-0005 §2.6.2, ignored.
    pub unknown_surfaces: usize,
}

/// A route and its import report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImportedRoute {
    /// The route.
    pub route: Route,
    /// What the import did.
    pub report: RouteImportReport,
}

/// Why a file was refused (ADR-0005 §2.2.4); each maps to `ApiError::InvalidInput` with its position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum RouteImportError {
    /// The file is larger than [`super::MAX_FILE_BYTES`].
    TooLarge {
        /// The file's size, bytes.
        bytes: usize,
    },
    /// More than [`MAX_POINTS`] points remain after dropping and merging.
    TooManyPoints {
        /// The limit that was passed.
        max: usize,
    },
    /// The route is shorter than [`MIN_ROUTE_M`].
    TooShort {
        /// Its length, metres.
        len_m: f64,
    },
    /// The route is longer than [`MAX_ROUTE_M`].
    TooLong {
        /// Its length, metres.
        len_m: f64,
    },
    /// The file is not well-formed or a point is out of range.
    Malformed {
        /// Where in the file (a byte offset for GPX).
        at: u64,
        /// What is wrong.
        what: String,
    },
    /// A run without elevation spans more than [`ELE_GAP_M`].
    MissingElevation {
        /// Distance of the known elevation before the run (0 for a missing start), metres.
        from_m: f64,
        /// Distance of the known elevation after the run (the end for a missing end), metres.
        to_m: f64,
    },
    /// Two consecutive points are more than [`MAX_JUMP_M`] apart.
    Jump {
        /// Distance along the route where the jump starts, metres.
        at_m: f64,
        /// The jump's length, metres.
        len_m: f64,
    },
    /// A FIT file that is not a course (#540).
    NotACourse {
        /// Its `file_id.type`.
        file_type: u8,
    },
    /// The file has no track or route, or none at the requested index.
    NoTrack,
    /// The file holds more than [`super::MAX_TRACKS`] tracks and routes.
    TooManyTracks {
        /// The limit that was passed.
        max: usize,
    },
    /// The importer built a route that breaks an ADR-0005 §2.1 invariant: a bug, never a property
    /// of the file.
    Invariant {
        /// The broken invariant.
        what: String,
    },
}

impl From<RouteInvariantError> for RouteImportError {
    fn from(e: RouteInvariantError) -> Self {
        Self::Invariant { what: e.what }
    }
}

impl std::fmt::Display for RouteImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "file is {bytes} bytes, more than {} MiB",
                super::MAX_FILE_BYTES / (1024 * 1024)
            ),
            Self::TooManyPoints { max } => write!(f, "more than {max} points"),
            Self::TooShort { len_m } => {
                write!(
                    f,
                    "route is {len_m:.0} m long, shorter than {MIN_ROUTE_M} m"
                )
            }
            Self::TooLong { len_m } => write!(
                f,
                "route is {:.1} km long, longer than {} km",
                len_m / 1000.0,
                MAX_ROUTE_M / 1000.0
            ),
            Self::Malformed { at, what } => write!(f, "malformed file at byte {at}: {what}"),
            Self::MissingElevation { from_m, to_m } => write!(
                f,
                "no elevation from {from_m:.0} m to {to_m:.0} m, more than {ELE_GAP_M} m"
            ),
            Self::Jump { at_m, len_m } => write!(
                f,
                "the route jumps {:.1} km at {:.1} km",
                len_m / 1000.0,
                at_m / 1000.0
            ),
            Self::NotACourse { file_type } => {
                write!(f, "FIT file type {file_type} is not a course")
            }
            Self::NoTrack => write!(f, "no track or route to import"),
            Self::TooManyTracks { max } => write!(f, "more than {max} tracks and routes"),
            Self::Invariant { what } => write!(f, "route invariant broken: {what}"),
        }
    }
}
impl std::error::Error for RouteImportError {}

fn malformed(at: u64, what: impl Into<String>) -> RouteImportError {
    RouteImportError::Malformed {
        at,
        what: what.into(),
    }
}

/// One zero: −0.0 becomes +0.0, any other value is kept (ADR-0005 §2.1, #578). A file's `-0` or
/// `-0.000000` (printf's form just west of Greenwich) is the same position or height as `0`, and
/// JSON in JavaScript cannot carry the sign (`JSON.stringify(-0)` is `0`), so a route never holds it.
pub(super) fn one_zero(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

/// A finite latitude within ±90° and longitude within ±180°, else the point is malformed.
fn check_position(p: &RawPoint) -> Result<(), RouteImportError> {
    if !(p.lat.is_finite() && (-90.0..=90.0).contains(&p.lat)) {
        return Err(malformed(p.at, format!("latitude {} outside ±90°", p.lat)));
    }
    if !(p.lon.is_finite() && (-180.0..=180.0).contains(&p.lon)) {
        return Err(malformed(
            p.at,
            format!("longitude {} outside ±180°", p.lon),
        ));
    }
    Ok(())
}

/// The point's elevation; one outside [`ELE_MIN_M`]..=[`ELE_MAX_M`] is a sensor fault, counted
/// and treated as missing (then filled or refused by the §2.2.2 rules).
fn plausible_elevation(
    p: &RawPoint,
    out_of_range: &mut usize,
) -> Result<Option<f64>, RouteImportError> {
    match p.ele_m {
        Some(e) if !e.is_finite() => Err(malformed(p.at, "elevation is not a number")),
        Some(e) if !(ELE_MIN_M..=ELE_MAX_M).contains(&e) => {
            *out_of_range += 1;
            Ok(None)
        }
        e => Ok(e.map(one_zero)),
    }
}

/// Build a route from a file's points in order, applying the limits of ADR-0005 §2.2.4 and the
/// missing-elevation rules of §2.2.2. `segments` is empty; a format with labels adds them. A
/// blank `name` becomes [`DEFAULT_ROUTE_NAME`]. A point whose distance would not increase the
/// along-route distance is merged into the previous one, so `dist_m` strictly increases (§2.1).
///
/// # Errors
/// The first refusal: a malformed point, a [`RouteImportError::Jump`], too many points, a route
/// too short or too long, or a run without elevation longer than [`ELE_GAP_M`].
pub fn route_from_points(
    name: String,
    source: RouteSource,
    raw: &[RawPoint],
) -> Result<ImportedRoute, RouteImportError> {
    let mut points: Vec<RoutePoint> = Vec::new();
    let mut dropped_null_island = 0;
    let mut merged_repeats = 0;
    let mut elevation_out_of_range = 0;
    let mut last_lat_lon = (0.0, 0.0);
    for p in raw {
        check_position(p)?;
        if p.lat == 0.0 && p.lon == 0.0 {
            dropped_null_island += 1;
            continue;
        }
        let ele_m = plausible_elevation(p, &mut elevation_out_of_range)?;
        let lat_lon = (one_zero(p.lat), one_zero(p.lon));
        let Some(prev) = points.last_mut() else {
            last_lat_lon = lat_lon;
            points.push(RoutePoint {
                dist_m: 0.0,
                ele_m,
                lat_lon: Some(lat_lon),
                surface: p.surface,
            });
            continue;
        };
        let d = haversine_m(last_lat_lon, lat_lon);
        if d > MAX_JUMP_M {
            return Err(RouteImportError::Jump {
                at_m: prev.dist_m,
                len_m: d,
            });
        }
        // Merge on the accumulated distance, not on `d == 0`: far from 0 a tiny `d` adds nothing
        // in f64, and keeping that point would repeat a distance (and divide by 0 when filling).
        let dist_m = prev.dist_m + d;
        if dist_m <= prev.dist_m {
            merged_repeats += 1;
            prev.ele_m = prev.ele_m.or(ele_m);
            prev.surface = prev.surface.or(p.surface);
            continue;
        }
        if points.len() == MAX_POINTS {
            return Err(RouteImportError::TooManyPoints { max: MAX_POINTS });
        }
        last_lat_lon = lat_lon;
        points.push(RoutePoint {
            dist_m,
            ele_m,
            lat_lon: Some(lat_lon),
            surface: p.surface,
        });
    }
    let len_m = points.last().map_or(0.0, |p| p.dist_m);
    if len_m < MIN_ROUTE_M {
        return Err(RouteImportError::TooShort { len_m });
    }
    if len_m > MAX_ROUTE_M {
        return Err(RouteImportError::TooLong { len_m });
    }
    let (elevation, gaps_filled) = fill_elevation(&mut points)?;
    let ascent_m = points
        .windows(2)
        .filter_map(|w| Some((w[1].ele_m? - w[0].ele_m?).max(0.0)))
        // Not `sum()`: an empty `f64` sum is −0.0, and a route without elevation would report it.
        .fold(0.0, |a, b| a + b);
    let surfaces_from_file = points.iter().filter(|p| p.surface.is_some()).count();
    let report = RouteImportReport {
        points_kept: points.len(),
        dropped_null_island,
        merged_repeats,
        elevation_out_of_range,
        gaps_filled,
        elevation,
        distance_m: len_m,
        ascent_m,
        surfaces_from_file,
        unknown_surfaces: 0,
    };
    let name = if name.trim().is_empty() {
        DEFAULT_ROUTE_NAME.to_owned()
    } else {
        name
    };
    let route = Route::new(
        name,
        source,
        points,
        Vec::new(),
        elevation,
        SurfaceProvenance {
            osm: None,
            from_file: surfaces_from_file > 0,
        },
    )?;
    Ok(ImportedRoute { route, report })
}

impl RouteImportReport {
    /// Record surface values the file gave that name no class.
    pub(crate) fn set_unknown_surfaces(&mut self, n: usize) {
        self.unknown_surfaces = n;
    }
}

/// Fill runs without elevation whose known ends are at most [`ELE_GAP_M`] apart: linearly over
/// distance inside the route, with the nearest value at its start or end (ADR-0005 §2.2.2).
fn fill_elevation(
    points: &mut [RoutePoint],
) -> Result<(ElevationKind, Vec<FilledGap>), RouteImportError> {
    let known: Vec<usize> = (0..points.len())
        .filter(|&i| points[i].ele_m.is_some())
        .collect();
    let (Some(&first), Some(&last)) = (known.first(), known.last()) else {
        return Ok((ElevationKind::None, Vec::new()));
    };
    let end_m = points.last().map_or(0.0, |p| p.dist_m);
    let mut gaps = Vec::new();
    let mut check = |from_m: f64, to_m: f64, n: usize| {
        if to_m - from_m > ELE_GAP_M {
            return Err(RouteImportError::MissingElevation { from_m, to_m });
        }
        gaps.push(FilledGap {
            from_m,
            to_m,
            points: n,
        });
        Ok(())
    };
    if first > 0 {
        check(0.0, points[first].dist_m, first)?;
        let e = points[first].ele_m;
        points[..first].iter_mut().for_each(|p| p.ele_m = e);
    }
    for pair in known.windows(2) {
        let (i, j) = (pair[0], pair[1]);
        if j == i + 1 {
            continue;
        }
        let (a, b) = (&points[i], &points[j]);
        check(a.dist_m, b.dist_m, j - i - 1)?;
        let (d0, e0, d1, e1) = (a.dist_m, a.ele_m, b.dist_m, b.ele_m);
        let (Some(e0), Some(e1)) = (e0, e1) else {
            continue;
        };
        // d1 > d0: distances strictly increase (the merge in `route_from_points`).
        let span = d1 - d0;
        for p in &mut points[i + 1..j] {
            p.ele_m = Some(if span > 0.0 {
                e0 + (e1 - e0) * (p.dist_m - d0) / span
            } else {
                e0
            });
        }
    }
    if last + 1 < points.len() {
        check(points[last].dist_m, end_m, points.len() - last - 1)?;
        let e = points[last].ele_m;
        points[last + 1..].iter_mut().for_each(|p| p.ele_m = e);
    }
    Ok((ElevationKind::Recorded, gaps))
}
