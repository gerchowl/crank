//! GPX 1.0 and 1.1 import (ADR-0005 §2.2.1): a streaming reader that expands no entity, refuses a
//! DOCTYPE and stops at a depth of [`MAX_XML_DEPTH`].
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

#[derive(Clone, Copy, PartialEq)]
enum Field {
    DocName,
    TrackName,
    Ele,
    Surface,
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

/// The GPX 1.0 and 1.1 namespaces; the root must be `gpx` in one of them, and structural
/// elements must be in the root's.
const GPX_NAMESPACES: [&str; 2] = [
    "http://www.topografix.com/GPX/1/0",
    "http://www.topografix.com/GPX/1/1",
];

/// Parents under which the GPX 1.0 schema allows elements of other namespaces
/// (`<xsd:any namespace="##other">` at the end of `gpx`, `wpt`, `rte`, `rtept`, `trk`, `trkpt`;
/// not `trkseg`). GPX 1.1 allows them only inside `<extensions>` (`extensionsType`).
const GPX10_OPEN_PARENTS: [&str; 6] = ["gpx", "wpt", "rte", "rtept", "trk", "trkpt"];

/// How an open element is read.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// An element of the document's GPX namespace.
    Gpx,
    /// A GPX 1.1 `<extensions>`: its children are extensions.
    Extensions,
    /// An element of another namespace where the schema allows one, or anything inside it:
    /// skipped, except a `<surface>` inside a point (ADR-0005 §2.2.1).
    Extension,
}

struct Frame {
    name: String,
    role: Role,
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
    /// Whether the open point already had its `<ele>`.
    ele_seen: bool,
    /// The field being read, the depth of its element and its text so far.
    capture: Option<(Field, usize, String)>,
}

impl Walk {
    fn parent(&self) -> Option<&Frame> {
        self.stack.last()
    }

    fn parent_is(&self, name: &str) -> bool {
        self.parent()
            .is_some_and(|f| f.role == Role::Gpx && f.name == name)
    }

    fn v1_0(&self) -> bool {
        self.gpx_ns == GPX_NAMESPACES[0]
    }

    /// Refuse a structural element whose parent is not the one the schema gives it.
    fn placed(&self, name: &str, parent: &str, xsd: &str, at: u64) -> Result<(), RouteImportError> {
        if self.parent_is(parent) {
            Ok(())
        } else {
            Err(malformed(
                at,
                format!("<{name}> outside a <{parent}> (GPX {xsd})"),
            ))
        }
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
        let Some(parent) = self.parent().map(|f| f.role) else {
            let root_ns = uri.and_then(|u| GPX_NAMESPACES.iter().find(|g| **g == u));
            return match (name.as_str(), root_ns) {
                ("gpx", Some(g)) => {
                    self.gpx_ns = g;
                    self.stack.push(Frame {
                        name,
                        role: Role::Gpx,
                    });
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
        if parent != Role::Gpx {
            let surface = name == "surface" && self.point.is_some();
            self.stack.push(Frame {
                name,
                role: Role::Extension,
            });
            if surface {
                self.start_capture(Field::Surface);
            }
            return Ok(());
        }
        if uri != Some(self.gpx_ns) {
            let open_parent = self
                .parent()
                .is_some_and(|f| GPX10_OPEN_PARENTS.contains(&f.name.as_str()));
            if self.v1_0() && open_parent {
                let surface = name == "surface" && self.point.is_some();
                self.stack.push(Frame {
                    name,
                    role: Role::Extension,
                });
                if surface {
                    self.start_capture(Field::Surface);
                }
                return Ok(());
            }
            let rule = if self.v1_0() {
                "GPX 1.0 allows other namespaces only at the end of gpx, wpt, rte, rtept, trk and trkpt"
            } else {
                "GPX 1.1 allows other namespaces only inside <extensions>, extensionsType"
            };
            return Err(malformed(
                at,
                format!("<{name}> in namespace '{}': {rule}", uri.unwrap_or("")),
            ));
        }
        self.structural(name, e, at)
    }

    /// An element of the document's GPX namespace, below the root.
    fn structural(
        &mut self,
        name: String,
        e: &BytesStart<'_>,
        at: u64,
    ) -> Result<(), RouteImportError> {
        let mut role = Role::Gpx;
        let mut field = None;
        match name.as_str() {
            "extensions" if !self.v1_0() => role = Role::Extensions,
            "trk" | "rte" => {
                self.placed(&name, "gpx", "gpxType", at)?;
                if self.doc.candidates.len() == MAX_TRACKS {
                    return Err(RouteImportError::TooManyTracks { max: MAX_TRACKS });
                }
                let kind = if name == "trk" {
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
            "trkseg" => self.placed(&name, "trk", "trkType", at)?,
            "trkpt" | "rtept" => {
                let (parent, xsd) = if name == "trkpt" {
                    ("trkseg", "trksegType")
                } else {
                    ("rte", "rteType")
                };
                self.placed(&name, parent, xsd, at)?;
                self.point = Some(point(e, at)?);
                self.point_depth = self.stack.len() + 1;
                self.ele_seen = false;
            }
            "ele" if self.parent_is("trkpt") || self.parent_is("rtept") => {
                if self.ele_seen {
                    return Err(malformed(
                        at,
                        "a second <ele> in one point (GPX wptType: ele maxOccurs 1)",
                    ));
                }
                self.ele_seen = true;
                field = Some(Field::Ele);
            }
            "name" if self.parent_is("trk") || self.parent_is("rte") => {
                field = Some(Field::TrackName);
            }
            "name" if self.parent_is("metadata") || (self.v1_0() && self.parent_is("gpx")) => {
                field = Some(Field::DocName);
            }
            _ => {}
        }
        self.stack.push(Frame { name, role });
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
            Field::DocName => self.doc.name = self.doc.name.take().or(text),
            Field::TrackName => {
                if let Some(c) = self.doc.candidates.last_mut() {
                    c.name = c.name.take().or(text);
                }
            }
            Field::Ele => {
                if let (Some(p), Some(t)) = (&mut self.point, text) {
                    p.ele_m = Some(decimal(&t).ok_or_else(|| {
                        malformed(at, format!("elevation '{t}' is not a number"))
                    })?);
                }
            }
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

    /// Character data; outside the root element only whitespace is allowed.
    fn text(&mut self, s: &str, at: u64) -> Result<(), RouteImportError> {
        if self.stack.is_empty() && !s.chars().all(char::is_whitespace) {
            return Err(malformed(at, "text outside the <gpx> element"));
        }
        if let Some((_, _, t)) = &mut self.capture {
            t.push_str(s);
        }
        Ok(())
    }
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
        ele_seen: false,
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
                walk.open(&ns, &e, at)?;
            }
            Event::Empty(e) => {
                if root_seen && walk.stack.is_empty() {
                    return Err(malformed(at, "content after the root element"));
                }
                root_seen = true;
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
