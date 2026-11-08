//! Routes: the route model, GPX import and grade-profile text (ADR-0005 §2.1, §2.2).
//!
//! Pure: no I/O, no clock (ADR-0001 §2). Import takes the file's bytes; storing a route is
//! `crank-store`'s (#541), resampling and smoothing into a [`Profile`] is #542.
use serde::{Deserialize, Serialize};

mod dsl;
mod gpx;
mod import;

pub use dsl::{
    GradeProfileItem, GradeProfileRepeat, GradeProfileSegment, MAX_EXPANDED_LABEL_BYTES,
    MAX_GRADE_PCT, MAX_LABEL_CHARS, MAX_PROFILE_SEGMENTS, MAX_PROFILE_TEXT_BYTES, MAX_SEGMENT_M,
    MIN_GRADE_PCT, MIN_SEGMENT_M, ProfileGrade, RouteDslError, RouteDslErrorKind, UNLABELLED_NAME,
    parse_grade_profile, parse_route_dsl, print_grade_profile, print_route_dsl,
};
pub use gpx::{GpxImport, TrackChoice, TrackKind, import_gpx};
pub use import::{
    FilledGap, ImportedRoute, RawPoint, RouteImportError, RouteImportOptions, RouteImportReport,
    route_from_points,
};

/// Mean Earth radius (IUGG, Moritz 2000) of the haversine distance (ADR-0005 §2.3.1).
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;
/// Largest imported file (ADR-0005 §2.2.4).
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// Most points a route keeps after dropping and merging (ADR-0005 §2.2.4).
pub const MAX_POINTS: usize = 200_000;
/// Shortest imported route, metres (ADR-0005 §2.2.4).
pub const MIN_ROUTE_M: f64 = 100.0;
/// Longest route, metres, for files and grade profiles (ADR-0005 §2.2.4, §2.2.5).
pub const MAX_ROUTE_M: f64 = 1_000_000.0;
/// Longest distance between consecutive points, metres; a longer one is a [`RouteImportError::Jump`].
pub const MAX_JUMP_M: f64 = 10_000.0;
/// Lowest plausible elevation, metres; below it a value is a sensor fault (ADR-0005 §2.2.4).
pub const ELE_MIN_M: f64 = -500.0;
/// Highest plausible elevation, metres; above it a value is a sensor fault (ADR-0005 §2.2.4).
pub const ELE_MAX_M: f64 = 9_000.0;
/// Longest run without elevation that is filled, metres (ADR-0005 §2.2.2 `ELE_GAP_M`).
pub const ELE_GAP_M: f64 = 500.0;
/// Deepest XML element nesting a GPX file may use (ADR-0005 §2.2.1).
pub const MAX_XML_DEPTH: usize = 32;
/// Most tracks and routes one GPX file may hold, so a file of empty `<trk/>`s cannot build a
/// list of millions (ADR-0005 §2.2.4): a year of daily tracks is 366, a multi-day course a few.
pub const MAX_TRACKS: usize = 1_000;
/// The name of a route whose file names neither it nor itself, and has no usable file name.
pub const DEFAULT_ROUTE_NAME: &str = "Route";

/// A road to ride: points along it plus labelled stretches (ADR-0005 §2.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// From the file (`<name>`, the FIT course name) or the grade profile's first label.
    pub name: String,
    /// Where the route came from.
    pub source: RouteSource,
    /// At least two; `dist_m` strictly increasing from 0. A grade profile's are its segment ends.
    pub points: Vec<RoutePoint>,
    /// Labelled stretches (FIT course points, profile labels); a grade profile's carry its grades.
    pub segments: Vec<RouteSegment>,
    /// Whether the points carry elevation and how it was obtained.
    pub elevation: ElevationKind,
    /// Where the points' surfaces came from.
    pub surface: SurfaceProvenance,
}

/// The kind of input a route was imported from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteSource {
    /// A GPX `trk`.
    GpxTrack,
    /// A GPX `rte`.
    GpxRoute,
    /// A FIT course file (#540).
    FitCourse,
    /// Grade-profile text (ADR-0005 §2.2.5).
    GradeProfile {
        /// The canonical text ([`print_grade_profile`]) the route was built from.
        dsl: String,
    },
}

/// One point of a route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RoutePoint {
    /// Along-route distance from the first point, metres (haversine, ADR-0005 §2.3.1).
    pub dist_m: f64,
    /// Elevation, metres; `None` only when the whole route has none ([`ElevationKind::None`]).
    pub ele_m: Option<f64>,
    /// Latitude and longitude, degrees, WGS 84; `None` for a grade profile.
    pub lat_lon: Option<(f64, f64)>,
    /// From the file, else from OSM, else `None` (the default surface).
    pub surface: Option<Surface>,
}

/// A stretch of a route between two distances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RouteSegment {
    /// Start distance, metres.
    pub start_m: f64,
    /// End distance, metres.
    pub end_m: f64,
    /// Its label, if any.
    pub label: Option<String>,
    /// How its grade is obtained.
    pub grade: SegmentGrade,
    /// Its surface, if the source gives one.
    pub surface: Option<Surface>,
}

/// How a segment's grade is obtained.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SegmentGrade {
    /// From the points' elevation (files).
    FromPoints,
    /// A constant grade (grade profiles).
    Constant {
        /// Grade, percent.
        pct: f64,
    },
    /// A grade changing linearly over distance (grade profiles).
    Ramp {
        /// Grade at the start, percent.
        from_pct: f64,
        /// Grade at the end, percent.
        to_pct: f64,
    },
}

/// Road surface class (ADR-0005 §2.6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Asphalt, concrete and other smooth paving.
    SmoothPaved,
    /// Paving stones, plates, wood and badly kept paving.
    RoughPaved,
    /// Compacted, gravel, dirt and other unpaved roads.
    Gravel,
    /// Grass.
    Grass,
    /// Sand and mud.
    Sand,
    /// Setts and cobblestones.
    Cobbles,
}

impl Surface {
    /// Every class, in the order of ADR-0005 §2.6.2.
    pub const ALL: [Self; 6] = [
        Self::SmoothPaved,
        Self::RoughPaved,
        Self::Gravel,
        Self::Grass,
        Self::Sand,
        Self::Cobbles,
    ];

    /// The class's name in grade-profile text and in a GPX surface extension.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SmoothPaved => "smooth_paved",
            Self::RoughPaved => "rough_paved",
            Self::Gravel => "gravel",
            Self::Grass => "grass",
            Self::Sand => "sand",
            Self::Cobbles => "cobbles",
        }
    }

    /// The class of an OSM `surface` value without a `smoothness` tag, or of a class name
    /// ([`Surface::name`]); `None` for any other value (ADR-0005 §2.6.2).
    #[must_use]
    pub fn from_osm(value: &str) -> Option<Self> {
        Some(match value {
            "asphalt" | "concrete" | "paved" | "chipseal" | "concrete:lanes" | "smooth_paved" => {
                Self::SmoothPaved
            }
            "paving_stones" | "concrete:plates" | "metal" | "wood" | "rough_paved" => {
                Self::RoughPaved
            }
            "compacted" | "fine_gravel" | "gravel" | "pebblestone" | "unpaved" | "dirt"
            | "earth" | "ground" | "rock" => Self::Gravel,
            "grass" | "grass_paver" => Self::Grass,
            "sand" | "mud" => Self::Sand,
            "sett" | "cobblestone" | "unhewn_cobblestone" | "cobblestone:flattened" | "cobbles" => {
                Self::Cobbles
            }
            _ => return None,
        })
    }
}

/// Whether a route's points carry elevation (ADR-0005 §2.2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ElevationKind {
    /// Recorded in the file (gaps of at most [`ELE_GAP_M`] filled).
    Recorded,
    /// The file has none: the route is ridden flat.
    None,
    /// The integral of a grade profile's grades, starting at 0 m.
    Exact,
}

/// Where a route's surfaces came from (ADR-0005 §2.6).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SurfaceProvenance {
    /// The local OSM surface index match, if one was made (#544).
    pub osm: Option<OsmMatch>,
    /// Whether the file or profile gave at least one surface.
    pub from_file: bool,
}

/// A match of a route against a local OSM surface index (ADR-0005 §2.6.1, built by #544).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OsmMatch {
    /// The region's name.
    pub region: String,
    /// When the region's extract was made, UTC Unix seconds.
    pub extract_at: i64,
    /// When the route was matched, UTC Unix seconds.
    pub matched_at: i64,
    /// Share of the profile's nodes that matched a way, 0 to 1.
    pub matched_share: f32,
}

/// A route that breaks an ADR-0005 §2.1 invariant. The importers never build one; seeing this
/// error is a bug in the code that built the route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RouteInvariantError {
    /// Which invariant, and where.
    pub what: String,
}
impl std::fmt::Display for RouteInvariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "route invariant broken: {}", self.what)
    }
}
impl std::error::Error for RouteInvariantError {}

fn broken(what: String) -> RouteInvariantError {
    RouteInvariantError { what }
}

impl Route {
    /// Build a route, checking every invariant of ADR-0005 §2.1 ([`Route::check`]).
    ///
    /// # Errors
    /// The first invariant the parts break.
    pub fn new(
        name: String,
        source: RouteSource,
        points: Vec<RoutePoint>,
        segments: Vec<RouteSegment>,
        elevation: ElevationKind,
        surface: SurfaceProvenance,
    ) -> Result<Self, RouteInvariantError> {
        let route = Self {
            name,
            source,
            points,
            segments,
            elevation,
            surface,
        };
        route.check()?;
        Ok(route)
    }

    /// Check the invariants of ADR-0005 §2.1: a non-blank name; at least two points; `dist_m`
    /// finite, 0 at the first point and strictly increasing; every elevation `None` when
    /// [`ElevationKind::None`], else finite; coordinates on every point of a file route, finite and
    /// within ±90°/±180°, and on none of a grade profile; segments finite, non-empty and within
    /// the route.
    ///
    /// # Errors
    /// The first invariant the route breaks.
    pub fn check(&self) -> Result<(), RouteInvariantError> {
        if self.name.trim().is_empty() {
            return Err(broken("the name is blank".into()));
        }
        if self.points.len() < 2 {
            return Err(broken(format!(
                "{} points, fewer than 2",
                self.points.len()
            )));
        }
        if self.points[0].dist_m.to_bits() != 0f64.to_bits() {
            return Err(broken(format!(
                "the first point is at {} m, not 0",
                self.points[0].dist_m
            )));
        }
        let profile = matches!(self.source, RouteSource::GradeProfile { .. });
        let mut prev = f64::NEG_INFINITY;
        for (i, p) in self.points.iter().enumerate() {
            if !(p.dist_m.is_finite() && p.dist_m > prev) {
                return Err(broken(format!(
                    "point {i}: distance {} m does not increase",
                    p.dist_m
                )));
            }
            prev = p.dist_m;
            let ele_ok = match (self.elevation, p.ele_m) {
                (ElevationKind::None, e) => e.is_none(),
                (_, e) => e.is_some_and(f64::is_finite),
            };
            if !ele_ok {
                return Err(broken(format!(
                    "point {i}: elevation {:?} for {:?}",
                    p.ele_m, self.elevation
                )));
            }
            let position_ok = match p.lat_lon {
                None => profile,
                Some((lat, lon)) => {
                    !profile
                        && lat.is_finite()
                        && lon.is_finite()
                        && (-90.0..=90.0).contains(&lat)
                        && (-180.0..=180.0).contains(&lon)
                }
            };
            if !position_ok {
                return Err(broken(format!("point {i}: position {:?}", p.lat_lon)));
            }
        }
        for (i, s) in self.segments.iter().enumerate() {
            let grades_finite = match s.grade {
                SegmentGrade::FromPoints => true,
                SegmentGrade::Constant { pct } => pct.is_finite(),
                SegmentGrade::Ramp { from_pct, to_pct } => {
                    from_pct.is_finite() && to_pct.is_finite()
                }
            };
            if !(grades_finite && s.start_m >= 0.0 && s.start_m < s.end_m && s.end_m <= prev) {
                return Err(broken(format!(
                    "segment {i}: {} m to {} m, {:?}",
                    s.start_m, s.end_m, s.grade
                )));
            }
        }
        Ok(())
    }
}

/// A route resampled and smoothed for riding (ADR-0005 §2.1, §2.3). It is a pure function of the
/// stored points, recomputed at every load and never stored; `Route::profile` computes it (#542).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Nodes every `STEP_M` (10 m) from 0 to the route's end.
    pub nodes: Vec<ProfileNode>,
}

/// One node of a [`Profile`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProfileNode {
    /// Along-route distance, metres.
    pub dist_m: f64,
    /// Smoothed elevation, metres (exact for a grade profile).
    pub ele_m: f64,
    /// Route grade, percent, before difficulty scaling and the cap.
    pub grade_pct: f64,
    /// Surface class; `None` is the default surface.
    pub surface: Option<Surface>,
}

/// Haversine distance between two points in degrees, metres (ADR-0005 §2.3.1).
#[must_use]
pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (
        a.0.to_radians(),
        a.1.to_radians(),
        b.0.to_radians(),
        b.1.to_radians(),
    );
    let h = libm::sin((la2 - la1) / 2.0).powi(2)
        + libm::cos(la1) * libm::cos(la2) * libm::sin((lo2 - lo1) / 2.0).powi(2);
    2.0 * EARTH_RADIUS_M * libm::asin(h.sqrt())
}
