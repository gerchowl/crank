//! GPX 1.0 and 1.1 import (ADR-0005 §2.2.1): a streaming reader that expands no entity, refuses a
//! DOCTYPE and stops at a depth of [`MAX_XML_DEPTH`].
use super::{
    MAX_FILE_BYTES, MAX_XML_DEPTH, RouteSource, Surface, haversine_m,
    import::{ImportedRoute, RawPoint, RouteImportError, RouteImportOptions, route_from_points},
};
use quick_xml::{
    XmlVersion,
    escape::resolve_predefined_entity,
    events::{BytesStart, Event},
    reader::Reader,
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
    if stem.is_empty() { "Route" } else { stem }.to_owned()
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

struct Walk {
    /// Local names of the open elements, outermost first.
    stack: Vec<String>,
    doc: Doc,
    point: Option<RawPoint>,
    /// Depth of the point's element; 0 outside a point.
    point_depth: usize,
    /// The field being read, the depth of its element and its text so far.
    capture: Option<(Field, usize, String)>,
}

impl Walk {
    fn parent_is(&self, name: &str) -> bool {
        self.stack.len() >= 2 && self.stack[self.stack.len() - 2] == name
    }

    fn open(&mut self, e: &BytesStart<'_>, at: u64) -> Result<(), RouteImportError> {
        if self.stack.len() == MAX_XML_DEPTH {
            return Err(malformed(
                at,
                format!("elements nested deeper than {MAX_XML_DEPTH} levels"),
            ));
        }
        let name = e.local_name().as_ref().to_owned();
        if self.stack.is_empty() && name != "gpx" {
            return Err(malformed(at, "the root element is not <gpx>"));
        }
        self.stack.push(name);
        let depth = self.stack.len();
        let name = self.stack[depth - 1].as_str();
        let in_point = self.point.is_some();
        let field = match name {
            "trk" | "rte" if depth == 2 => {
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
                None
            }
            "trkpt" if depth == 4 && self.parent_is("trkseg") && self.stack[1] == "trk" => {
                self.point = Some(point(e, at)?);
                self.point_depth = depth;
                None
            }
            "rtept" if depth == 3 && self.parent_is("rte") => {
                self.point = Some(point(e, at)?);
                self.point_depth = depth;
                None
            }
            "name" if depth == 3 && (self.parent_is("trk") || self.parent_is("rte")) => {
                Some(Field::TrackName)
            }
            "name" if depth == 2 || (depth == 3 && self.parent_is("metadata")) => {
                Some(Field::DocName)
            }
            "ele" if in_point && (self.parent_is("trkpt") || self.parent_is("rtept")) => {
                Some(Field::Ele)
            }
            "surface" if in_point => Some(Field::Surface),
            _ => None,
        };
        if let Some(field) = field
            && self.capture.is_none()
        {
            self.capture = Some((field, depth, String::new()));
        }
        Ok(())
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

    fn text(&mut self, s: &str) {
        if let Some((_, _, t)) = &mut self.capture {
            t.push_str(s);
        }
    }
}

fn point(e: &BytesStart<'_>, at: u64) -> Result<RawPoint, RouteImportError> {
    let (mut lat, mut lon) = (None, None);
    for attr in e.attributes() {
        let attr = attr.map_err(|err| malformed(at, err.to_string()))?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| malformed(at, err.to_string()))?;
        let slot = match attr.key.local_name().as_ref() {
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
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| {
        malformed(
            u64::try_from(e.valid_up_to()).unwrap_or(u64::MAX),
            "the file is not UTF-8",
        )
    })?;
    let mut reader = Reader::from_str(text);
    let mut walk = Walk {
        stack: Vec::new(),
        doc: Doc::default(),
        point: None,
        point_depth: 0,
        capture: None,
    };
    let mut root_seen = false;
    loop {
        let at = reader.buffer_position();
        let event = reader
            .read_event()
            .map_err(|e| malformed(reader.error_position(), e.to_string()))?;
        match event {
            Event::DocType(_) => {
                return Err(malformed(at, "a DOCTYPE is not allowed in GPX"));
            }
            Event::Start(e) => {
                if root_seen && walk.stack.is_empty() {
                    return Err(malformed(at, "content after the root element"));
                }
                root_seen = true;
                walk.open(&e, at)?;
            }
            Event::Empty(e) => {
                if root_seen && walk.stack.is_empty() {
                    return Err(malformed(at, "content after the root element"));
                }
                root_seen = true;
                walk.open(&e, at)?;
                walk.close(at)?;
            }
            Event::End(_) => walk.close(at)?,
            Event::Text(t) => walk.text(&t.xml10_content()),
            Event::CData(t) => walk.text(&t.xml10_content()),
            Event::GeneralRef(r) => {
                if let Some(c) = r
                    .resolve_char_ref()
                    .map_err(|e| malformed(at, e.to_string()))?
                {
                    walk.text(c.encode_utf8(&mut [0; 4]));
                } else if let Some(s) = resolve_predefined_entity(&r) {
                    walk.text(s);
                } else {
                    return Err(malformed(at, format!("entity '&{};' is not expanded", &*r)));
                }
            }
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    if !root_seen {
        return Err(malformed(0, "no <gpx> element"));
    }
    Ok(walk.doc)
}
