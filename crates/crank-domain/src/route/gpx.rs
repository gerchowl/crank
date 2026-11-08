//! GPX 1.0 and 1.1 import (ADR-0005 §2.2.1): a streaming reader that expands no entity, refuses a
//! DOCTYPE and stops at a depth of [`MAX_XML_DEPTH`]. Every element of the GPX namespace must
//! stand where its schema's content model puts it, from the single root down ([`child_type`]);
//! other namespaces only where the schema allows them ([`foreign_allowed`]). Everything inside a
//! GPX 1.1 `<extensions>` is extension data, skipped whatever its namespace ([`Content::Foreign`]).
use super::{
    DEFAULT_ROUTE_NAME, MAX_FILE_BYTES, MAX_TRACKS, MAX_XML_DEPTH, RouteSource, Surface,
    haversine_m,
    import::{ImportedRoute, RawPoint, RouteImportError, RouteImportOptions, route_from_points},
};
use quick_xml::{
    XmlVersion,
    escape::resolve_predefined_entity,
    events::{BytesStart, Event},
    name::ResolveResult,
    reader::NsReader,
};
use serde::{Deserialize, Serialize};

/// Whether a GPX candidate is a `trk` or an `rte`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// A `trk`; its `trkseg`s are joined in document order.
    Track,
    /// An `rte`.
    Route,
}

/// One track or route of a file with several, for the rider to pick (ADR-0005 §2.2.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TrackChoice {
    /// Its index in document order, for [`RouteImportOptions::track`].
    pub index: usize,
    /// `trk` or `rte`.
    pub kind: TrackKind,
    /// Its `<name>`, if it has one.
    pub name: Option<String>,
    /// Haversine length over its points, metres, before any limit is applied.
    pub distance_m: f64,
}

/// The result of reading a GPX file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GpxImport {
    /// The file's single track or route, or the one [`RouteImportOptions::track`] named.
    Imported(Box<ImportedRoute>),
    /// The file has several and none was named: the rider picks one.
    ChooseTrack {
        /// Every track and route, in document order.
        tracks: Vec<TrackChoice>,
    },
}

struct Candidate {
    kind: TrackKind,
    name: Option<String>,
    points: Vec<RawPoint>,
    unknown_surfaces: usize,
}

/// A value the importer reads, as a bit of its owner's [`Frame::seen`]: each is read once.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum Field {
    DocName = 1,
    TrackName = 2,
    Ele = 4,
    Surface = 8,
    /// Not a value but the element holding the document name: GPX 1.1 `metadata` occurs once,
    /// so a second one cannot replace the first's name.
    Metadata = 16,
}

#[derive(Default)]
struct Doc {
    name: Option<String>,
    candidates: Vec<Candidate>,
}

/// Read a GPX file and import its track or route. `file_name` names the route when the file
/// names neither the track nor itself.
///
/// # Errors
/// [`RouteImportError::TooLarge`] above [`MAX_FILE_BYTES`]; `Malformed` for XML that is not
/// well-formed, not UTF-8, not GPX, has a DOCTYPE, an entity other than XML's five, nesting
/// deeper than [`MAX_XML_DEPTH`] or a point out of range; `NoTrack` without a track or route
/// (or none at the requested index); and every refusal of [`route_from_points`].
pub fn import_gpx(
    bytes: &[u8],
    file_name: &str,
    options: &RouteImportOptions,
) -> Result<GpxImport, RouteImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(RouteImportError::TooLarge { bytes: bytes.len() });
    }
    let doc = read(bytes)?;
    let index = match (options.track, doc.candidates.len()) {
        (_, 0) => return Err(RouteImportError::NoTrack),
        (Some(i), _) => i,
        (None, 1) => 0,
        (None, _) => {
            let tracks = doc
                .candidates
                .iter()
                .enumerate()
                .map(|(index, c)| TrackChoice {
                    index,
                    kind: c.kind,
                    name: c.name.clone(),
                    distance_m: listed_distance(&c.points),
                })
                .collect();
            return Ok(GpxImport::ChooseTrack { tracks });
        }
    };
    let Some(c) = doc.candidates.into_iter().nth(index) else {
        return Err(RouteImportError::NoTrack);
    };
    let name = c.name.or(doc.name).unwrap_or_else(|| stem(file_name));
    let source = match c.kind {
        TrackKind::Track => RouteSource::GpxTrack,
        TrackKind::Route => RouteSource::GpxRoute,
    };
    let mut imported = route_from_points(name, source, &c.points)?;
    imported.report.set_unknown_surfaces(c.unknown_surfaces);
    Ok(GpxImport::Imported(Box::new(imported)))
}

fn stem(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s).trim();
    if stem.is_empty() {
        DEFAULT_ROUTE_NAME
    } else {
        stem
    }
    .to_owned()
}

fn listed_distance(points: &[RawPoint]) -> f64 {
    points
        .iter()
        .filter(|p| !(p.lat == 0.0 && p.lon == 0.0))
        .map(|p| (p.lat, p.lon))
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| haversine_m(w[0], w[1]))
        .sum()
}

fn malformed(at: u64, what: impl Into<String>) -> RouteImportError {
    RouteImportError::Malformed {
        at,
        what: what.into(),
    }
}

/// A decimal number as GPX writes one (xsd:decimal, plus an exponent); never `NaN` or `inf`.
fn decimal(text: &str) -> Option<f64> {
    let t = text.trim();
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (mantissa, exponent) = body
        .split_once(['e', 'E'])
        .map_or((body, None), |(m, e)| (m, Some(e)));
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits(e)
    });
    let ok = !(int.is_empty() && frac.is_empty()) && digits(int) && digits(frac) && exponent_ok;
    ok.then(|| t.parse().ok()).flatten()
}

/// The GPX 1.0 and 1.1 namespaces; the root must be `gpx` in one of them, and the GPX
/// elements below it in the root's.
const GPX_NAMESPACES: [&str; 2] = [
    "http://www.topografix.com/GPX/1/0",
    "http://www.topografix.com/GPX/1/1",
];

/// The schema type of an element of the document's GPX namespace: GPX 1.1's complex types
/// (`gpx.xsd` 1.1), and GPX 1.0's anonymous types named after their element (`gpx.xsd` 1.0).
#[derive(Clone, Copy, PartialEq)]
enum Ty {
    /// `gpxType`, the root.
    Gpx,
    /// 1.1 `metadataType`.
    Metadata,
    /// 1.1 `personType` (`author`).
    Person,
    /// 1.1 `copyrightType`.
    Copyright,
    /// 1.1 `linkType`.
    Link,
    /// `wptType` as a waypoint: read by no route.
    Wpt,
    /// `rteType`.
    Rte,
    /// `wptType` as a route point.
    Rtept,
    /// `trkType`.
    Trk,
    /// `trksegType`.
    Trkseg,
    /// `wptType` as a track point (1.0 adds `course` and `speed`).
    Trkpt,
    /// 1.1 `extensionsType`: `<xsd:any namespace="##other">` in the schema; its content is
    /// extension data whatever its namespace (ADR-0005 §2.2.1).
    Extensions,
    /// A simple type (`xsd:string`, `xsd:decimal`, `xsd:dateTime`, …): text, no child element.
    Text,
    /// `boundsType`, 1.1 `emailType`: attributes only.
    Empty,
}

impl Ty {
    fn xsd(self) -> &'static str {
        match self {
            Self::Gpx => "gpxType",
            Self::Metadata => "metadataType",
            Self::Person => "personType",
            Self::Copyright => "copyrightType",
            Self::Link => "linkType",
            Self::Wpt | Self::Rtept | Self::Trkpt => "wptType",
            Self::Rte => "rteType",
            Self::Trk => "trkType",
            Self::Trkseg => "trksegType",
            Self::Extensions => "extensionsType",
            Self::Text => "simple type, no child elements",
            Self::Empty => "empty type",
        }
    }
}

/// GPX 1.1 `wptType`'s simple-typed children: every child but `link` and `extensions`.
const WPT_TEXT_1_1: [&str; 17] = [
    "ele",
    "time",
    "magvar",
    "geoidheight",
    "name",
    "cmt",
    "desc",
    "src",
    "sym",
    "type",
    "fix",
    "sat",
    "hdop",
    "vdop",
    "pdop",
    "ageofdgpsdata",
    "dgpsid",
];
/// GPX 1.0 `wpt`'s and `rtept`'s children, all simple-typed; `trkpt` adds `course` and `speed`.
const WPT_TEXT_1_0: [&str; 19] = [
    "ele",
    "time",
    "magvar",
    "geoidheight",
    "name",
    "cmt",
    "desc",
    "src",
    "url",
    "urlname",
    "sym",
    "type",
    "fix",
    "sat",
    "hdop",
    "vdop",
    "pdop",
    "ageofdgpsdata",
    "dgpsid",
];
/// GPX 1.1 `rteType`'s and `trkType`'s simple-typed children.
const TRK_TEXT_1_1: [&str; 6] = ["name", "cmt", "desc", "src", "number", "type"];
/// GPX 1.0 `rte`'s and `trk`'s simple-typed children.
const TRK_TEXT_1_0: [&str; 7] = ["name", "cmt", "desc", "src", "url", "urlname", "number"];
/// GPX 1.0 `gpx`'s simple-typed children.
const GPX_TEXT_1_0: [&str; 8] = [
    "name", "desc", "author", "email", "url", "urlname", "time", "keywords",
];
/// GPX 1.1 `metadataType`'s simple-typed children.
const METADATA_TEXT_1_1: [&str; 4] = ["name", "desc", "time", "keywords"];

/// The type of the GPX-namespace element `name` inside one of type `parent`, `None` if the
/// schema's content model does not allow it there. Sequence order and `maxOccurs` are not
/// checked, except for the values the importer reads ([`Field`]), which may occur once.
fn child_type(v1_0: bool, parent: Ty, name: &str) -> Option<Ty> {
    let text = |list: &[&str]| list.contains(&name).then_some(Ty::Text);
    if v1_0 {
        return match (parent, name) {
            (Ty::Gpx, "bounds") => Some(Ty::Empty),
            (Ty::Gpx, "wpt") => Some(Ty::Wpt),
            (Ty::Gpx, "rte") => Some(Ty::Rte),
            (Ty::Gpx, "trk") => Some(Ty::Trk),
            (Ty::Gpx, _) => text(&GPX_TEXT_1_0),
            (Ty::Rte, "rtept") => Some(Ty::Rtept),
            (Ty::Trk, "trkseg") => Some(Ty::Trkseg),
            (Ty::Rte | Ty::Trk, _) => text(&TRK_TEXT_1_0),
            (Ty::Trkseg, "trkpt") => Some(Ty::Trkpt),
            (Ty::Trkpt, "course" | "speed") => Some(Ty::Text),
            (Ty::Wpt | Ty::Rtept | Ty::Trkpt, _) => text(&WPT_TEXT_1_0),
            _ => None,
        };
    }
    match (parent, name) {
        (
            Ty::Gpx
            | Ty::Metadata
            | Ty::Wpt
            | Ty::Rte
            | Ty::Rtept
            | Ty::Trk
            | Ty::Trkseg
            | Ty::Trkpt,
            "extensions",
        ) => Some(Ty::Extensions),
        (Ty::Gpx, "metadata") => Some(Ty::Metadata),
        (Ty::Gpx, "wpt") => Some(Ty::Wpt),
        (Ty::Gpx, "rte") => Some(Ty::Rte),
        (Ty::Gpx, "trk") => Some(Ty::Trk),
        (
            Ty::Metadata | Ty::Person | Ty::Wpt | Ty::Rte | Ty::Rtept | Ty::Trk | Ty::Trkpt,
            "link",
        ) => Some(Ty::Link),
        (Ty::Metadata, "author") => Some(Ty::Person),
        (Ty::Metadata, "copyright") => Some(Ty::Copyright),
        (Ty::Metadata, "bounds") | (Ty::Person, "email") => Some(Ty::Empty),
        (Ty::Metadata, _) => text(&METADATA_TEXT_1_1),
        (Ty::Person, "name")
        | (Ty::Copyright, "year" | "license")
        | (Ty::Link, "text" | "type") => Some(Ty::Text),
        (Ty::Rte, "rtept") => Some(Ty::Rtept),
        (Ty::Trk, "trkseg") => Some(Ty::Trkseg),
        (Ty::Rte | Ty::Trk, _) => text(&TRK_TEXT_1_1),
        (Ty::Trkseg, "trkpt") => Some(Ty::Trkpt),
        (Ty::Wpt | Ty::Rtept | Ty::Trkpt, _) => text(&WPT_TEXT_1_1),
        _ => None,
    }
}

/// Whether an element of another namespace may stand in an element of type `parent` outside an
/// `<extensions>`: GPX 1.0's `<xsd:any namespace="##other">` at the end of `gpx`, `wpt`, `rte`,
/// `rtept`, `trk` and `trkpt` (not `trkseg`). GPX 1.1 allows other namespaces only inside
/// `extensionsType`, whose content [`Walk::open`] skips whatever its namespace.
fn foreign_allowed(v1_0: bool, parent: Ty) -> bool {
    v1_0 && matches!(
        parent,
        Ty::Gpx | Ty::Wpt | Ty::Rte | Ty::Rtept | Ty::Trk | Ty::Trkpt
    )
}

/// How an open element is read.
#[derive(Clone, Copy, PartialEq)]
enum Content {
    /// An element of the document's GPX namespace, of this type.
    Gpx(Ty),
    /// Extension data: an element of another namespace where GPX 1.0 allows one, any element
    /// inside a GPX 1.1 `<extensions>`, or anything inside either. Skipped, except a `<surface>`
    /// of a namespace other than GPX's inside a point (ADR-0005 §2.2.1).
    Foreign,
}

struct Frame {
    name: String,
    content: Content,
    /// The [`Field`]s this element already had.
    seen: u8,
}

/// An element's namespace, resolved by quick-xml's namespace reader.
enum Ns {
    None,
    Uri(String),
    Undeclared(String),
}

impl Ns {
    fn of(r: &ResolveResult<'_>) -> Self {
        match r {
            ResolveResult::Unbound => Self::None,
            ResolveResult::Bound(n) => Self::Uri(n.as_ref().to_owned()),
            ResolveResult::Unknown(p) => Self::Undeclared(p.clone()),
        }
    }
}

struct Walk {
    /// The open elements, outermost first.
    stack: Vec<Frame>,
    /// The root's namespace, one of [`GPX_NAMESPACES`].
    gpx_ns: &'static str,
    doc: Doc,
    point: Option<RawPoint>,
    /// Depth of the point's element; 0 outside a point.
    point_depth: usize,
    /// The field being read, the depth of its element and its text so far.
    capture: Option<(Field, usize, String)>,
}

impl Walk {
    fn v1_0(&self) -> bool {
        self.gpx_ns == GPX_NAMESPACES[0]
    }

    fn version(&self) -> &'static str {
        if self.v1_0() { "1.0" } else { "1.1" }
    }

    fn open(&mut self, ns: &Ns, e: &BytesStart<'_>, at: u64) -> Result<(), RouteImportError> {
        if self.stack.len() == MAX_XML_DEPTH {
            return Err(malformed(
                at,
                format!("elements nested deeper than {MAX_XML_DEPTH} levels"),
            ));
        }
        let name = e.local_name().as_ref().to_owned();
        let uri = match ns {
            Ns::Undeclared(p) => {
                return Err(malformed(
                    at,
                    format!("namespace prefix '{p}' is not declared"),
                ));
            }
            Ns::None => None,
            Ns::Uri(u) => Some(u.as_str()),
        };
        let Some(parent) = self.stack.last().map(|f| f.content) else {
            let root_ns = uri.and_then(|u| GPX_NAMESPACES.iter().find(|g| **g == u));
            return match (name.as_str(), root_ns) {
                ("gpx", Some(g)) => {
                    self.gpx_ns = g;
                    self.push(name, Content::Gpx(Ty::Gpx));
                    Ok(())
                }
                _ => Err(malformed(
                    at,
                    format!(
                        "the root is <{name}> in namespace '{}', not <gpx> in GPX 1.0 or 1.1",
                        uri.unwrap_or("")
                    ),
                )),
            };
        };
        let parent = match parent {
            // ADR-0005 §2.2.1: everything inside `<extensions>` is extension data, whatever its
            // namespace. The schema's `##other` refuses the GPX namespace and none there, but real
            // exports write unprefixed elements, which inherit the GPX namespace: Strava's
            // `<power>`, older OsmAnd's `<speed>` and `<color>`. Skipping them is safe, as nothing
            // inside is ever read as GPX structure: a `<trkpt>` or `<gpx>` in there imports nothing.
            Content::Foreign | Content::Gpx(Ty::Extensions) => return self.foreign(name, uri, at),
            Content::Gpx(ty) => ty,
        };
        if uri == Some(self.gpx_ns) {
            let Some(ty) = child_type(self.v1_0(), parent, &name) else {
                return Err(malformed(
                    at,
                    format!(
                        "<{name}> is not allowed in <{}> (GPX {} {})",
                        self.stack.last().map_or("", |f| f.name.as_str()),
                        self.version(),
                        parent.xsd()
                    ),
                ));
            };
            return self.structural(name, parent, ty, e, at);
        }
        if uri.is_some() && foreign_allowed(self.v1_0(), parent) {
            return self.foreign(name, uri, at);
        }
        let rule = if self.v1_0() {
            "GPX 1.0 allows other namespaces only at the end of gpx, wpt, rte, rtept, trk and trkpt"
        } else {
            "GPX 1.1 allows other namespaces only inside <extensions>, extensionsType"
        };
        Err(malformed(
            at,
            format!("<{name}> in namespace '{}': {rule}", uri.unwrap_or("")),
        ))
    }

    fn push(&mut self, name: String, content: Content) {
        self.stack.push(Frame {
            name,
            content,
            seen: 0,
        });
    }

    /// Mark `field` read in the open element at `owner`, refusing a second one.
    fn read_once(&mut self, owner: usize, field: Field, at: u64) -> Result<(), RouteImportError> {
        let bit = field as u8;
        let (what, rule) = match field {
            Field::Surface => ("surface", "one surface per point, ADR-0005 §2.2"),
            Field::Ele => ("ele", "GPX wptType: ele maxOccurs 1"),
            Field::TrackName | Field::DocName => ("name", "GPX: name maxOccurs 1"),
            Field::Metadata => ("metadata", "GPX 1.1 gpxType: metadata maxOccurs 1"),
        };
        let Some(frame) = self.stack.get_mut(owner) else {
            return Ok(());
        };
        if frame.seen & bit != 0 {
            return Err(malformed(
                at,
                format!("a second <{what}> in one <{}> ({rule})", frame.name),
            ));
        }
        frame.seen |= bit;
        Ok(())
    }

    /// Extension data ([`Content::Foreign`]) in namespace `uri`.
    fn foreign(
        &mut self,
        name: String,
        uri: Option<&str>,
        at: u64,
    ) -> Result<(), RouteImportError> {
        let surface =
            name == "surface" && self.point.is_some() && uri.is_some_and(|u| u != self.gpx_ns);
        self.push(name, Content::Foreign);
        if surface {
            self.read_once(self.point_depth - 1, Field::Surface, at)?;
            self.start_capture(Field::Surface);
        }
        Ok(())
    }

    /// An element of the document's GPX namespace, below the root, that its parent allows.
    fn structural(
        &mut self,
        name: String,
        parent: Ty,
        ty: Ty,
        e: &BytesStart<'_>,
        at: u64,
    ) -> Result<(), RouteImportError> {
        match ty {
            Ty::Rte | Ty::Trk => {
                if self.doc.candidates.len() == MAX_TRACKS {
                    return Err(RouteImportError::TooManyTracks { max: MAX_TRACKS });
                }
                let kind = if ty == Ty::Trk {
                    TrackKind::Track
                } else {
                    TrackKind::Route
                };
                self.doc.candidates.push(Candidate {
                    kind,
                    name: None,
                    points: Vec::new(),
                    unknown_surfaces: 0,
                });
            }
            Ty::Rtept | Ty::Trkpt => {
                self.point = Some(point(e, at)?);
                self.point_depth = self.stack.len() + 1;
            }
            Ty::Metadata => self.read_once(self.stack.len() - 1, Field::Metadata, at)?,
            _ => {}
        }
        // The values read; `child_type` allows each only where it is meant (a 1.1 root has no
        // `name`, its document name is the `metadata`'s).
        let field = match (parent, name.as_str()) {
            (Ty::Rtept | Ty::Trkpt, "ele") => Some(Field::Ele),
            (Ty::Rte | Ty::Trk, "name") => Some(Field::TrackName),
            (Ty::Gpx | Ty::Metadata, "name") => Some(Field::DocName),
            _ => None,
        };
        if let Some(field) = field {
            self.read_once(self.stack.len() - 1, field, at)?;
        }
        self.push(name, Content::Gpx(ty));
        if let Some(field) = field {
            self.start_capture(field);
        }
        Ok(())
    }

    fn start_capture(&mut self, field: Field) {
        if self.capture.is_none() {
            self.capture = Some((field, self.stack.len(), String::new()));
        }
    }

    fn close(&mut self, at: u64) -> Result<(), RouteImportError> {
        let depth = self.stack.len();
        if let Some((field, d, _)) = &self.capture
            && *d == depth
        {
            let field = *field;
            let text = self
                .capture
                .take()
                .map(|(_, _, t)| t.trim().to_owned())
                .unwrap_or_default();
            self.finish(field, text, at)?;
        }
        if depth == self.point_depth
            && let (Some(p), Some(c)) = (self.point.take(), self.doc.candidates.last_mut())
        {
            c.points.push(p);
            self.point_depth = 0;
        }
        self.stack.pop();
        Ok(())
    }

    fn finish(&mut self, field: Field, text: String, at: u64) -> Result<(), RouteImportError> {
        let text = (!text.is_empty()).then_some(text);
        match field {
            Field::DocName => self.doc.name = text,
            Field::TrackName => {
                if let Some(c) = self.doc.candidates.last_mut() {
                    c.name = text;
                }
            }
            Field::Ele => {
                if let (Some(p), Some(t)) = (&mut self.point, text) {
                    p.ele_m = Some(decimal(&t).ok_or_else(|| {
                        malformed(at, format!("elevation '{t}' is not a number"))
                    })?);
                }
            }
            // Never captured: it marks the element, not a value.
            Field::Metadata => {}
            Field::Surface => {
                if let (Some(p), Some(t)) = (&mut self.point, text) {
                    match Surface::from_osm(&t.to_ascii_lowercase()) {
                        Some(s) => p.surface = Some(s),
                        None => {
                            if let Some(c) = self.doc.candidates.last_mut() {
                                c.unknown_surfaces += 1;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Character data: outside the root only whitespace, and in a GPX element of a complex
    /// type (element-only content, no `mixed`) only whitespace too. Inside `<extensions>` it is
    /// extension data like the elements there.
    fn text(&mut self, s: &str, at: u64) -> Result<(), RouteImportError> {
        let blank = s.chars().all(char::is_whitespace);
        match self.stack.last() {
            None if !blank => return Err(malformed(at, "text outside the <gpx> element")),
            Some(f)
                if !blank
                    && !matches!(
                        f.content,
                        Content::Foreign | Content::Gpx(Ty::Text | Ty::Extensions)
                    ) =>
            {
                let xsd = match f.content {
                    Content::Gpx(ty) => ty.xsd(),
                    Content::Foreign => "",
                };
                return Err(malformed(
                    at,
                    format!(
                        "text inside <{}> (GPX {} {xsd}: no character content)",
                        f.name,
                        self.version()
                    ),
                ));
            }
            _ => {}
        }
        if let Some((_, _, t)) = &mut self.capture {
            t.push_str(s);
        }
        Ok(())
    }
}

/// Refuse an attribute whose prefix is not declared: such a document is not namespace
/// well-formed (Namespaces in XML 1.0, §5 "Prefix Declared").
fn attributes_declared(
    reader: &NsReader<&[u8]>,
    e: &BytesStart<'_>,
    at: u64,
) -> Result<(), RouteImportError> {
    for attr in e.attributes() {
        let attr = attr.map_err(|err| malformed(at, err.to_string()))?;
        if let (ResolveResult::Unknown(p), _) = reader.resolver().resolve_attribute(attr.key) {
            return Err(malformed(
                at,
                format!("namespace prefix '{p}' is not declared"),
            ));
        }
    }
    Ok(())
}

fn point(e: &BytesStart<'_>, at: u64) -> Result<RawPoint, RouteImportError> {
    let (mut lat, mut lon) = (None, None);
    for attr in e.attributes() {
        let attr = attr.map_err(|err| malformed(at, err.to_string()))?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| malformed(at, err.to_string()))?;
        // Only the unqualified attributes are wptType's lat and lon: `x:lat` is someone else's.
        let slot = match attr.key.as_ref() {
            "lat" => &mut lat,
            "lon" => &mut lon,
            _ => continue,
        };
        *slot = Some(
            decimal(&value)
                .ok_or_else(|| malformed(at, format!("coordinate '{value}' is not a number")))?,
        );
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return Err(malformed(at, "a point needs lat and lon"));
    };
    Ok(RawPoint {
        at,
        lat,
        lon,
        ele_m: None,
        surface: None,
    })
}

fn read(bytes: &[u8]) -> Result<Doc, RouteImportError> {
    // A UTF-8 byte-order mark needs no stripping: quick-xml's reader skips it.
    let text = std::str::from_utf8(bytes).map_err(|e| {
        malformed(
            u64::try_from(e.valid_up_to()).unwrap_or(u64::MAX),
            "the file is not UTF-8",
        )
    })?;
    let mut reader = NsReader::from_str(text);
    let mut walk = Walk {
        stack: Vec::new(),
        gpx_ns: GPX_NAMESPACES[1],
        doc: Doc::default(),
        point: None,
        point_depth: 0,
        capture: None,
    };
    let mut root_seen = false;
    loop {
        let at = reader.buffer_position();
        let step = reader
            .read_resolved_event()
            .map(|(ns, event)| (Ns::of(&ns), event));
        let (ns, event) = step.map_err(|e| malformed(reader.error_position(), e.to_string()))?;
        match event {
            Event::DocType(_) => {
                return Err(malformed(at, "a DOCTYPE is not allowed in GPX"));
            }
            Event::Start(e) => {
                if root_seen && walk.stack.is_empty() {
                    return Err(malformed(at, "content after the root element"));
                }
                root_seen = true;
                attributes_declared(&reader, &e, at)?;
                walk.open(&ns, &e, at)?;
            }
            Event::Empty(e) => {
                if root_seen && walk.stack.is_empty() {
                    return Err(malformed(at, "content after the root element"));
                }
                root_seen = true;
                attributes_declared(&reader, &e, at)?;
                walk.open(&ns, &e, at)?;
                walk.close(at)?;
            }
            Event::End(_) => walk.close(at)?,
            Event::Text(t) => walk.text(&t.xml10_content(), at)?,
            Event::CData(t) => walk.text(&t.xml10_content(), at)?,
            Event::GeneralRef(r) => {
                if let Some(c) = r
                    .resolve_char_ref()
                    .map_err(|e| malformed(at, e.to_string()))?
                {
                    walk.text(c.encode_utf8(&mut [0; 4]), at)?;
                } else if let Some(s) = resolve_predefined_entity(&r) {
                    walk.text(s, at)?;
                } else {
                    return Err(malformed(at, format!("entity '&{};' is not expanded", &*r)));
                }
            }
            Event::Eof => {
                // quick-xml does not report elements left open at the end; a truncated file
                // must not import what it kept (ADR-0005 §2.2: nothing is dropped silently).
                if let Some(open) = walk.stack.last() {
                    return Err(malformed(
                        at,
                        format!("the file ends inside <{}>", open.name),
                    ));
                }
                break;
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    if !root_seen {
        return Err(malformed(0, "no <gpx> element"));
    }
    Ok(walk.doc)
}
