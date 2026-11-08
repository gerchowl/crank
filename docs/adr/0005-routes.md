# ADR-0005: Routes: GPX, FIT and grade-profile import, SIM-mode ride, speed model, OSM surface

- **Status:** Accepted (owner, 2026-11-06)
- **Date:** 2026-11-05
- **Issue:** #523
- **Related:** ADR-0001 (crates, clock, pauses, store), ADR-0002 (FE-C pages 50/51/55, `ControlMode::Sim`, §4.4.1
  following, §7.4–§7.9), `spec/TRAINER-BLE.md`, `docs/DSL.md`, `docs/safety/FMEA.md`, #257 (FIT), #44 (HIL),
  #96 (ride anatomy), DESIGN §18.3, #524 (check 8 air term)

## 1. Context

The owner wants to ride **routes** in the trainer's SIM mode (decisions of 2026-11-05 in #523):

- **Sources:** GPX tracks with elevation, FIT course files (one FIT decoder shared with #257), and
  grade-profile workouts (distance and grade segments written like a workout, no file).
- **Speed:** the rider chooses (a) crank's own physics model, which takes the road surface from
  OpenStreetMap so that Crr follows tarmac, gravel or cobbles, or (b) the speed the trainer reports (page 16).
- **Timing:** designed now, in parallel with the MVP. The ride side cannot be verified on the NEO before
  Part B check 8 (SIM grades) of #44 has run.

Facts this design rests on:

- **The wire.** Page 51 carries grade in 0.01 % with a −200 % offset and Crr in 5×10⁻⁵; page 50 carries
  the wind resistance coefficient in 0.01 kg/m, wind speed in km/h and drafting in 0.01; page 55
  carries rider and bike mass and wheel size (ADR-0002 §4.2). `encode` refuses grade outside ±200 % and
  Crr at or above 0.012725, and `crank-devices` refuses Crr above `CRR_MAX` = 0.0127 (ADR-0002 §4.3).
- **The SIM writer.** `ControlMode::Sim { grade_pct, crr, wind }` goes out as pages 50 then 51, at most
  once a second on change, with 1 s alternating keep-alives and page-71 polls (ADR-0002 §4.4, §7.9).
  Following has two components, `track` and `wind`, each answered every 2 s (§4.4.1).
- **Speed telemetry.** Page 16 bytes 4–5 are speed in 0.001 m/s and byte 3 is distance in metres,
  rolling over at 256 (ADR-0002 §4.2). On the NEO page 16 arrives about once a second at the median and
  5.0–5.5 s apart at the 99th percentile, so speed is held for 6 s (ADR-0001 §5, #437).
- **The FE-C road model.** The ANT+ FE profile, which defines page 50, needs an adopter login and was not
  retrieved (ADR-0002 §4.1). Garmin's Connect IQ documentation of the same FE-C settings, `SimulationSettings`
  (read 2026-11-05), defines the field: "Wind Resistance Coefficient [kg/m] = Frontal Surface Area [m2] x Drag
  Coefficient x Air Density [kg/m3]", range 0–1.86 kg/m, invalid 0xFF. These are page 50's range and invalid
  value in ADR-0002 §4.2, so it describes the same field. The trainer's air force is therefore ½·Kw·(v + w)²·drafting:
  FortiusANT, an open FE-C trainer implementation, computes exactly that, and factors the default
  0.51 kg/m as ρ 1.275 × A 0.4 × Cd 1.0. The same documentation gives sample rolling resistances: wooden track
  0.001, smooth concrete 0.002, asphalt 0.004, rough road 0.008. The check 8 analyser omits the ½;
  that is #524.
- **The NEO.** The NEO (T2800) simulates up to 25 % and descents down to −5 %, and drives the flywheel forward on
  descents when it is on mains power (DC Rainmaker's in-depth review, 2015, and trainer guide, 2019). The rider's
  freehub decouples the pedals, so the motor never turns the cranks.
- **Privacy.** Route coordinates can reveal a home. crank keeps everything on the machine (ADR-0001 §7,
  ADR-0003). A per-route OSM lookup would be the first feature to send ride-related data to a third party, which
  is why surface comes from a downloaded regional extract instead (§2.6.1, owner input of 2026-11-05).

## 2. Decision

### 2.1 Route model and store

**Types** (`crank-domain::route`, pure: no I/O, no clock, ADR-0001 §2):

```rust
pub struct Route {
    pub name: String,                       // from the file (<name>, FIT course name) or the profile's first label
    pub source: RouteSource,
    pub points: Vec<RoutePoint>,            // ≥ 2, dist_m strictly increasing from 0; a grade profile's are its segment ends
    pub segments: Vec<RouteSegment>,        // labelled stretches (FIT course points, profile labels); a grade profile's carry its grades
    pub elevation: ElevationKind,           // Recorded | None (§2.2.2) | Exact (grade profile)
    pub surface: SurfaceProvenance,         // where each point's surface came from (§2.6)
}
pub enum RouteSource { GpxTrack, GpxRoute, FitCourse, GradeProfile { dsl: String } }
pub struct RoutePoint {
    pub dist_m: f64,                        // along-route distance (haversine, §2.3.1)
    pub ele_m: Option<f64>,                 // None only when the whole route has no elevation
    pub lat_lon: Option<(f64, f64)>,        // degrees, WGS 84; None for a grade profile
    pub surface: Option<Surface>,           // from the file, else from OSM, else None (= default)
}
pub struct RouteSegment { pub start_m: f64, pub end_m: f64, pub label: Option<String>, pub grade: SegmentGrade, pub surface: Option<Surface> }
pub enum SegmentGrade { FromPoints, Constant { pct: f64 }, Ramp { from_pct: f64, to_pct: f64 } }
pub enum Surface { SmoothPaved, RoughPaved, Gravel, Grass, Sand, Cobbles }   // §2.6.2
pub struct SurfaceProvenance { pub osm: Option<OsmMatch>, pub from_file: bool }
pub struct OsmMatch { pub region: String, pub extract_at: UtcTime, pub matched_at: UtcTime, pub matched_share: f32 }
```

The ride does not read points directly. `Route::profile(&RouteSettings) -> Profile` resamples and smooths
(§2.3) into nodes every 10 m: distance, smoothed elevation, route grade and surface class. It is a pure function
of the stored points, so it is recomputed at every load and never stored. `PROFILE_VERSION` names the algorithm
and is recorded with each ride (§2.7.5), so an old ride says which profile it was ridden on.

**Store** (`crank-store`, migration `NNNN_route.sql`):

| Table | Columns | Notes |
|---|---|---|
| `route` | `id, name, source, elevation, points BLOB, segments_json, surface_json, dsl, original_name, original_sha256, original BLOB, created_at` | `points`: versioned little-endian columns (`dist_m` f64, `ele_m` f64 with NaN for none, `lat`/`lon` f64 with NaN for none, `surface` u8 with 0 for none), so a route round-trips bit for bit; `original` is the imported file's bytes (≤ 32 MiB, §2.2.4), the "link to the original file" that survives the file being moved or deleted |
| `ride` (new columns) | `route_id, speed_source, route_json` | `route_json`: the `RouteSettings`, masses, CdA, `PROFILE_VERSION` and the OSM attribution as ridden (§2.7.5) |

**Round trip.** `store.put_route(r)` then `store.get_route(id)` equals `r` exactly (bit for bit, f64 included).
A grade profile also round-trips through its text: `parse_route_dsl(print_route_dsl(r))` equals `r`, and
printing that again gives the same bytes (the workout DSL's rule, `docs/DSL.md`). Deleting a route keeps every
ride ridden on it: `ride.route_json` carries what the ride needs to show itself.

### 2.2 Import

One import call per file: `Core::route_import(bytes, file_name, options) -> RouteImportOutcome`, where `options`
holds the track index of a multi-track file (§2.2.1); surface matching against the local index needs no option
(§2.6.1). The outcome is the route plus an **import report**, which the UI
shows before the route is saved: points kept, points dropped and why, gaps filled, elevation kind, distance,
ascent, and the surface sources. Nothing is dropped or filled silently.

#### 2.2.1 GPX

- GPX 1.0 and 1.1. Parsed with a streaming XML reader (`quick-xml`) that expands no entities and stops at a depth
  of 32. A file with a `<!DOCTYPE` is refused as `Malformed` (GPX needs none), so a hostile file cannot inflate (an
  XML bomb) or recurse.
- `trk/trkseg/trkpt` and `rte/rtept` are read; `wpt` and `extensions` are ignored, except a surface extension
  (`<surface>` in any namespace, mapped by §2.6.2), which sets the point's surface.
- **One route per import.** A file with exactly one `trk` or one `rte` imports it. A file with several lists them
  (name, distance) and the rider picks one; the import call takes the index. The segments of a `trk` are joined in
  document order, and the join is ridden as a straight line (its distance counts).
- Time stamps, heart rate, cadence and power in the file are ignored. A route is a road, not an activity.

#### 2.2.2 Missing elevation

- **Gaps inside a route:** a run of points without `ele` is filled linearly over distance when its endpoints
  are at most `ELE_GAP_M` = 500 m apart, and the report counts it. A longer run refuses the import with
  `MissingElevation { from_m, to_m }`. A missing start or end is filled with the nearest value under the same
  500 m rule.
- **No elevation at all:** the route is imported with `elevation: None` and is ridden **flat** (grade 0 %). The
  import report, the route library and the ride's Ready check all say "No elevation in this file: rides flat". It
  is never filled from a terrain model, because that would be another network call.
- Rationale for 500 m: with the 100 m smoothing of §2.3.2, a straight fill over 500 m changes no grade the rider
  could otherwise feel. Over a longer gap the linear fill would be invented terrain.

#### 2.2.3 FIT course

- One FIT decoder, crate `crank-fit` (pure, no I/O), shared with #257's activity export and import. It wraps
  `fitparser` 0.12.0 (MIT, crates.io, released 2026-11-03), subject to `cargo deny` like every dependency.
  The FIT SDK itself is not used, so its licence does not apply.
- Accepted when `file_id.type` = course (6). Read: `course.name`; `record.position_lat/position_long`
  (semicircles × 180 / 2³¹ → degrees); `record.enhanced_altitude`, else `record.altitude`, as elevation; and
  `course_point` messages, which become segment labels at their distance. Missing elevation follows §2.2.2.
- Distance is recomputed with the haversine of §2.3.1, as for GPX, so every route has the same distance rule.
  The file's `record.distance` is shown in the report beside it, and a difference above 2 % is flagged.
- Any other FIT file type refuses the import with `NotACourse { file_type }`. FIT activities are #257's import.

#### 2.2.4 Limits and malformed files

| Limit | Value | Rationale |
|---|---|---|
| File size | 32 MiB | A 1 Hz recording of 200 000 points is about 25 MiB of GPX |
| Points (after dropping) | 200 000 | 55 h at 1 Hz, or 1 000 km at 5 m spacing |
| Route length | 100 m to 1 000 km | Shorter than one smoothing kernel is not a route; 1 000 km covers multi-day courses |
| Latitude, longitude | finite, within ±90°, ±180° | Else the point is malformed |
| Point at exactly (0, 0) | dropped, counted | The usual "no fix yet" value of GPS loggers |
| Elevation | −500 m to 9 000 m | Below the Dead Sea shore (−430 m) or above Everest (8 849 m) is a sensor fault |
| Repeated point (distance 0 to the previous) | merged, counted | Keeps distance strictly increasing |
| Distance between consecutive points | ≤ 10 km | A larger jump is a corrupt file or a teleport; the import is refused and the report names the position |
| Repeated position, by accumulated distance | merged, counted | A move too small to change the along-route distance in f64 (sub-micrometre near the poles) is a repeat too, so `dist_m` strictly increases (#539) |
| Tracks and routes per GPX file | 1 000 | A year of daily tracks is 366; the cap keeps 32 MiB of empty `<trk/>` from listing millions (#539) |
| Grade-profile text | 256 KiB | 1 000 km at 100 m per segment, written without repeats, is about 150 KB (#539) |
| Grade-profile label | 100 characters | A name for the ride screen, a few words (#539) |
| Grade-profile labels after expanding repeats | 1 MiB, checked before expanding | Each expanded segment carries its label; without it `100000x("<10 000 chars>" 0.01km 0%)`, 10 KB of text, would allocate about 1 GB (#539). 10 000 labelled segments of 100 bytes fit |

Every refusal is a typed `RouteImportError` (`TooLarge`, `TooManyPoints`, `TooShort`, `TooLong`, `Malformed { at,
what }`, `MissingElevation { from_m, to_m }`, `Jump { at_m, len_m }`, `NotACourse`, `NoTrack`, `TooManyTracks`), mapped to
`ApiError::InvalidInput` with the position (ADR-0001 §4.4). A truncated file (an element still open at the end) and
text outside the root element are `Malformed`. `Invariant { what }` is not a property of the file: it reports a route that
breaks §2.1, which `Route::new` refuses, and is a bug (#539).

#### 2.2.5 Grade-profile text

A grade profile is written like a workout (`docs/DSL.md`): the same tokens, labels, separators, comments,
`N x(...)` repeats, error type (`RouteDslError { kind, message, offset, length }`, offsets in UTF-16) and
canonical printing. Its **end** is a distance and its **target** is a grade:

```text
"Flat start" 2km 0%
"Col" 5km 6.5%, 3km ramp 6.5%..9%
3x(0.4km 12% cobbles, 0.6km 2%)
"Descent" 4km -5% gravel
```

```ebnf
profile   = sep* [ item { sep+ item } ] sep* ;
item      = [ label ] ( repeat | segment ) ;
repeat    = COUNT "x" "(" profile ")" ;                       (* no progression options *)
segment   = DIST grade [ surface ] ;
DIST      = NUMBER "km" ;                                    (* 0.001 km = 1 m resolution *)
grade     = SIGNED_OR_NUMBER "%"                             (* constant grade *)
          | "ramp" SIGNED_OR_NUMBER "%" ".." SIGNED_OR_NUMBER "%" ;  (* linear from → to *)
surface   = "smooth_paved" | "rough_paved" | "gravel" | "grass" | "sand" | "cobbles" ;
```

- **Kilometres only.** In a workout, `m` means minutes (`4m`), so a route never accepts `m`: `500m` is refused
  with "routes are measured in km: write 0.5km ('m' is minutes in a workout)". Durations and power targets
  are refused the same way, naming the workout form, so text pasted from a workout fails on its first token.
- Grade in −25 % to +30 % per segment (beyond that is refused as implausible, like `900%` in a workout);
  the ride's cap (§2.3.3) applies on top. Segment length 0.01 km to 100 km; 1 000 km and 32 nesting levels in all,
  the workout DSL's `MAX_NESTING_DEPTH`.
- Elevation starts at 0 m and is the integral of the grade (`ElevationKind::Exact`): no smoothing, because there
  is no noise. The slew limit of §2.3.5 still softens steps between segments.

### 2.3 Elevation to grade

The pipeline, from stored points to what page 51 carries:

```text
points ─haversine─▶ distance ─resample 10 m─▶ elevation ─Gaussian σ 100 m─▶ smoothed elevation ─Δ/10 m─▶ route grade
route grade ─× difficulty (§2.3.4)─▶ ─cap −15…+20 % (§2.3.3)─▶ target grade
target grade ─slew 1 %/s, 0.01 % grid, ≤ 1 change/s (§2.3.5)─▶ page 51
```

| Parameter | Value | Source or rationale |
|---|---|---|
| `STEP_M` resampling step | 10 m | The median point spacing of the sample tracks is 10–11 m; at 30 km/h 10 m is 1.2 s, about the page-51 send period |
| `SIGMA_M` Gaussian sigma | 100 m | §2.3.2: the smallest candidate that holds a flat road under 3 %, and the knee of the climbs' ascent |
| `FLAT_MAX_PCT` spurious grade allowed on a flat road | 3 | A 3 % step is about 25 N at 84 kg, which the rider feels; below it the residual is noise the rider rides through |
| `GRADE_MAX_PCT` cap, up | 20 | Below the NEO's 25 % with a margin; real roads seldom stay steeper than 20 % over a 100 m kernel |
| `GRADE_MIN_PCT` cap, down | -15 | The NEO acts only down to −5 %; the cap bounds garbage from a file and keeps model (a)'s descents finite |
| `DESCENT_SCALE` difficulty on descents, default | 1.0 | §2.3.4 |
| `SLEW_PCT_PER_S` largest change of the sent grade per second | 1.0 | §2.3.5 |
| `SEND_PERIOD_S` at most one page-51 change per | 1.0 | ADR-0002 §7.9 ("≤ 1 Hz on change") |
| `GRADE_GRID_PCT` grade resolution crank sends | 0.01 | §2.3.5: the wire grid |

These rows are checked against the executable model, `docs/adr/models/route_grade.py` (`check_constants`).

#### 2.3.1 Distance and resampling

Distance between points is the haversine distance on a sphere of radius 6 371 008.8 m (IUGG mean radius; the
error against the ellipsoid is under 0.5 %, the same for every route and for OSM matching). Elevation is resampled
every `STEP_M` = 10 m along the route, linearly between points. Points closer than 10 m are averaged by the
smoothing that follows, points farther apart are interpolated.

#### 2.3.2 Smoothing (justified on real noisy elevation)

GPS elevation is noisy: recorded on a flat lakeshore, it climbs and drops by metres between fixes. Differentiated
at 10 m, that noise becomes grades of ±40 %, which a trainer would make the rider feel.

**Method:** a Gaussian kernel over distance, truncated at 3σ and renormalised at the ends of the route, on the
10 m samples. It weights by distance, so it does not depend on how often the device recorded. Unlike a moving
average, it adds no ripple of its own.

**Data:** four segments of two public tracks with recorded elevation, from gpxpy's test files (Apache-2.0),
extracted with source, commit and checksums into `docs/adr/models/fixtures/gpx-elevation-samples.csv` (40 KB).
`cerknica-1` runs along the shore of a lake, so its true grade is close to 0. The three `korita` segments
climb and descend in hills. Both were recorded on foot, so their real grades are steeper than a road's; what
matters here is the receiver's elevation noise, which does not depend on how the receiver was carried. Output of `python3 docs/adr/models/route_grade.py`:

```text
constants: ADR-0005 §2.3 == model (step, sigma, flat bound, cap, slew, send period, grid, scaling), ok
korita-1: 358 points, 8.64 km, resampled every 10 m
  sigma     0 m: ascent   511 m, p99 |grade| 131.2 %, max |grade| 280.0 %, rms change  0.0 m, sign reversals 10.0/km
  sigma    10 m: ascent   437 m, p99 |grade|  96.0 %, max |grade| 140.0 %, rms change  0.9 m, sign reversals  8.2/km
  sigma    25 m: ascent   373 m, p99 |grade|  52.3 %, max |grade|  75.8 %, rms change  2.0 m, sign reversals  4.4/km
  sigma    50 m: ascent   345 m, p99 |grade|  36.0 %, max |grade|  44.6 %, rms change  3.1 m, sign reversals  1.5/km
  sigma   100 m: ascent   330 m, p99 |grade|  28.8 %, max |grade|  29.7 %, rms change  4.4 m, sign reversals  0.8/km  <- decided
  sigma   200 m: ascent   310 m, p99 |grade|  23.3 %, max |grade|  23.5 %, rms change  6.7 m, sign reversals  0.3/km
korita-2: 176 points, 2.29 km, resampled every 10 m
  sigma     0 m: ascent   230 m, p99 |grade|  69.2 %, max |grade|  83.2 %, rms change  0.0 m, sign reversals 14.4/km
  sigma    10 m: ascent   210 m, p99 |grade|  48.7 %, max |grade|  57.3 %, rms change  0.5 m, sign reversals 10.5/km
  sigma    25 m: ascent   191 m, p99 |grade|  38.1 %, max |grade|  40.3 %, rms change  1.3 m, sign reversals  6.1/km
  sigma    50 m: ascent   180 m, p99 |grade|  28.2 %, max |grade|  28.5 %, rms change  2.2 m, sign reversals  1.7/km
  sigma   100 m: ascent   170 m, p99 |grade|  21.7 %, max |grade|  21.8 %, rms change  3.9 m, sign reversals  0.9/km  <- decided
  sigma   200 m: ascent   148 m, p99 |grade|  13.5 %, max |grade|  13.5 %, rms change  8.8 m, sign reversals  0.0/km
korita-3: 337 points, 3.98 km, resampled every 10 m
  sigma     0 m: ascent    97 m, p99 |grade|  43.2 %, max |grade|  48.6 %, rms change  0.0 m, sign reversals 10.3/km
  sigma    10 m: ascent    81 m, p99 |grade|  32.2 %, max |grade|  38.3 %, rms change  0.3 m, sign reversals  6.3/km
  sigma    25 m: ascent    70 m, p99 |grade|  27.7 %, max |grade|  30.2 %, rms change  0.8 m, sign reversals  2.3/km
  sigma    50 m: ascent    61 m, p99 |grade|  21.4 %, max |grade|  21.9 %, rms change  1.5 m, sign reversals  1.3/km
  sigma   100 m: ascent    46 m, p99 |grade|  16.5 %, max |grade|  16.7 %, rms change  2.9 m, sign reversals  0.8/km  <- decided
  sigma   200 m: ascent    26 m, p99 |grade|  12.9 %, max |grade|  12.9 %, rms change  6.2 m, sign reversals  0.3/km
cerknica-1: 173 points, 1.92 km, resampled every 10 m
  sigma     0 m: ascent    61 m, p99 |grade|  40.5 %, max |grade|  73.0 %, rms change  0.0 m, sign reversals 24.0/km
  sigma    10 m: ascent    43 m, p99 |grade|  33.4 %, max |grade|  34.8 %, rms change  0.4 m, sign reversals 17.2/km
  sigma    25 m: ascent    24 m, p99 |grade|  10.6 %, max |grade|  11.2 %, rms change  0.9 m, sign reversals 10.9/km
  sigma    50 m: ascent    13 m, p99 |grade|   6.1 %, max |grade|   6.1 %, rms change  1.4 m, sign reversals  4.7/km
  sigma   100 m: ascent     8 m, p99 |grade|   2.6 %, max |grade|   2.6 %, rms change  1.8 m, sign reversals  2.6/km  <- decided
  sigma   200 m: ascent     3 m, p99 |grade|   1.2 %, max |grade|   1.2 %, rms change  2.4 m, sign reversals  2.1/km
sigma: 100 m is the smallest candidate with the flat segment under 3.0 % and the knee of the climbs' ascent, ok
pipeline: a 0 -> 25 % data step is capped at 20.0 % and reaches it 20.0 s later in 22 page-51 changes (<= 1.0 %/s, >= 1 s apart), ok
all checks passed
```

**Decision: σ = 100 m.**

- **The flat-road bound decides.** On the flat shore, σ = 50 m still leaves a 6.1 % spurious grade. σ = 100 m
  brings it to 2.6 %, under the 3 % bound, and no smaller candidate does. The shore's spurious ascent falls from
  61 m (raw) to 8 m.
- **On the climbs, 100 m is where a larger kernel starts to cost more than it gains.** Ascent lost:

  | Segment | 50 → 100 m | 100 → 200 m |
  |---|---|---|
  | `korita-1` | 4 % (345 → 330 m) | 6 % (330 → 310 m) |
  | `korita-2` | 6 % (180 → 170 m) | 13 % (170 → 148 m) |
  | `korita-3` | 25 % (61 → 46 m) | 43 % (46 → 26 m) |

  Every segment loses more from 100 to 200 m than from 50 to 100 m.
- **`korita-3` shows the cost.** Its relief is small: 46 m over 4 km, in bumps close to the noise in size. So it
  already loses a quarter of its ascent at 100 m. Short undulations of that kind are flattened (the cost paragraph
  below), and the flat-road bound is what justifies paying it.
- The model asserts the flat bound and the ratio test on all three climbs (`check_sigma`).

Cost: a real ramp shorter than about 200 m (a 50 m kicker at 15 %) is ridden as a longer, gentler slope. This is
the price of not letting fix-to-fix noise move resistance. FIT courses and planner GPX files whose elevation came
from a terrain model are smoothed with the same kernel. They are already smooth, so it changes little, and one
rule keeps every source predictable. A grade profile is exact and is not smoothed (§2.2.5).

#### 2.3.3 Grade cap and the trainer's range

The wire carries ±200 % (ADR-0002 §4.2). The NEO simulates up to 25 % and down to −5 % (§1). crank caps the
target grade at **−15 % to +20 %**, after difficulty scaling. Model (a) uses the same cap on the unscaled route
grade, so at 100 % difficulty the trainer and the model see the same grade. Sending below −5 % is harmless (the NEO
acts only down to −5 %), and model (a) still rides the descent at its real grade down to −15 %.
`TrainerCaps` gains no grade range: FE-C reports none (page 54 reports only modes and maximum resistance).
A trainer with a narrower range clamps on its own, which is the FE-C behaviour.

#### 2.3.4 Difficulty scaling

**Decided: two settings, both defaulting to 100 %.**

- `scale_all`: 0–100 % in 10 % steps, applied to every grade.
- `descent_scale`: 0–100 %, applied after it to negative grades only.

Scaling changes **only what the trainer makes the rider feel**. Model (a) always uses the unscaled route grade,
so the speed and the time of a ride stay those of the real road, which is the usual convention for a "trainer
difficulty" setting. Source (b) cannot follow that convention: the trainer computes its speed from the grade it
was sent, so at 50 % a climb is ridden faster. The Ready check says so when (b) and a scale below 100 % are
both chosen, and the ride records both settings (§2.7.5).

The default is 100 % because a route should ride like the road unless the rider chooses otherwise. On the NEO a
descent below −5 % feels the same at any scale, so a lower descent default would buy nothing.

#### 2.3.5 Grade updates: slew, grid and cadence

- The engine computes the target grade **every tick** (250 ms, ADR-0001 §5), at the current position (§2.7.2),
  linearly between the 10 m nodes.
- **Slew:** the commanded grade moves towards the target by at most `SLEW_PCT_PER_S` = 1 % per second (0.25 % per
  tick). At 30 km/h that is 0.12 % per metre: a road that goes from 0 to 10 % within 20 m reaches the trainer over
  10 s (83 m). This gives the rider time to shift, and it bounds the torque step at low cadence (§2.8). It is
  also the SIM "soft start" at a ride start and after a pause (§2.7.3).
- **Grid:** the commanded grade is rounded to the wire grid, 0.01 % (ADR-0002 §4.4.1 `grid_key`). A coarser grid
  would save nothing: while the grade moves, the 1 s send period already bounds the writes, and a 0.1 % grid would
  step the force by 0.8 N at 84 kg (7 W at 30 km/h), which a rider can feel.
- **Cadence:** a changed value goes out at most once per `SEND_PERIOD_S` = 1 s, the ADR-0002 §7.9 rule, through the
  coalescing `control` slot (§4.4, the newest value wins). Keep-alives and polls are unchanged (§4.4.1).
- **Following:** a route's grade changes are the `Continuous` class of ADR-0002 §4.4.1 (like an ERG ramp), never
  `Transition`, so settle is not triggered. With a track value that changes up to once a second, every 2 s echo
  is of a value at most `L` = 2 s old, which agrees by the §4.4.1 lag rule. The ride-mode issue (§6, R7) adds the
  `crank-engine` scenario "a route ride whose grade changes every second stays `following`", plus a run of the
  following model's SIM walk with a 1 s grade change.

### 2.4 Speed model (a)

#### 2.4.1 Equations

The ride's speed `v ≥ 0` (m/s) obeys the road-cycling power balance (the model form of Martin et al. 1998:
propulsion against gravity, rolling resistance, aerodynamic drag and the inertia of rider, bike and wheels):

```text
m_eff · dv/dt = η·P / max(v, V_MIN) − F_grav − F_roll − F_aero
θ      = atan(G / 100)                       G = route grade, %, capped −15…+20 % (§2.3.3)
F_grav = m · g · sin θ                       negative downhill
F_roll = Crr · m · g · cos θ
F_aero = ½ · ρ · CdA · d · (v + w) · |v + w| d = drafting, w = head wind
m      = rider kg + bike kg                  the page-55 values (§2.4.5)
m_eff  = m + M_rot
```

`P` is the trainer's measured power for the tick (ADR-0002 §5). `η` is 1 because that power is measured at the
trainer's hub, after the chain (§2.4.5). Position advances by the distance covered: `x += ∫ v dt`.

#### 2.4.2 Parameters

| Parameter | Value | Source or rationale |
|---|---|---|
| `g` standard gravity | 9.80665 m/s² | 3rd CGPM (1901), Resolution 2 |
| `rho` air density | 1.225 kg/m³ | ISA at sea level, 15 °C (ISO 2533:1975). Constant per ride: varying it with route elevation would change page 50 continuously (§3) |
| `M_rot` rotational equivalent mass of both wheels | 1.4 kg | Thin-hoop approximation: I/r² is the rim, tyre and tube mass, about 0.7 kg per 700c road wheel. It changes accelerations by under 2 % and steady speeds not at all |
| `eta_hub` drivetrain efficiency | 1.0 | Power is measured at the hub (§2.4.5). A chain loses 1.4–19 % depending on tension (Spicer et al. 2001); applying a loss to hub power would count it twice |
| `CdA` drag area, default | 0.40 m² | The FE-C default 0.51 kg/m is A·Cd = 0.4 at ρ 1.275 (FortiusANT), so the model's default is what an FE-C trainer assumes by default. Rider setting (§2.4.5), 0.15–0.80 m² |
| `drafting` | 1.0 | FE-C: 1.0 = no drafting (Garmin `SimulationSettings`); a route ride is solo |
| `Crr_default` rolling resistance, no surface known | 0.004 | FE-C sample "asphalt" (Garmin `SimulationSettings`), the default of ADR-0002 §7.9 |
| `Crr_max` | 0.0127 | Page 51's maximum, `CRR_MAX` (ADR-0002 §4.3) |
| Wind speed | 0 | Still air: there is no weather in a route; page 50 wind speed 0 km/h |
| Rider and bike mass | profile, equipment | The athlete profile and equipment settings, the same values page 55 carries (ADR-0002 §7.9) |

Wheel bearing friction, a term of Martin et al.'s model, is omitted: the trainer has no road wheel, and model (a)
uses only what the trainer is told (§2.4.5), so the two keep the same terms.

#### 2.4.3 Numerical integration

| Constant | Value | Why |
|---|---|---|
| `H` step | 0.025 s | 10 explicit-midpoint (RK2) steps per 250 ms tick. Checked against a 1 ms reference over flat, coast, pause, and a 400 W start on 8 %: the speed differs by at most 0.002 m/s and the position by 0.008 m. At 50 ms the first tick after a standing start differed by 0.017 m/s |
| `V_MIN` | 0.5 m/s | Regularises P/v at rest. A 30 s standing start at 200 W covers within 0.44 m of the V_MIN → 0 limit (V_MIN = 1 m/s: 1.8 m short) |
| `V_STOP` | 0.05 m/s | Below it, if rolling resistance can hold the bike, it is at rest (v = 0 exactly) |

Power and grade are constant over a tick. The integrator is deterministic and has no clock of its own: it runs
inside the engine tick, so the same ride inputs give the same distance (ADR-0001 §5).

#### 2.4.4 Zero power, descents, stops and pauses

- **Zero power** (`P` = 0 or no power value in the tick, e.g. the ≤ 2 s loss window of ADR-0001 §5): the bike
  coasts. On the flat it slows down slowly (worked example 4).
- **Descents:** gravity drives the bike to its terminal speed, where `F_grav + F_roll + F_aero = 0`, also from
  rest. Nothing caps the speed beyond the grade cap (§2.3.3): at −15 % the terminal speed is 79 km/h with the
  default parameters, at −6 % 49 km/h (worked example 3). On a descent gentler than 0.4 % (Crr 0.004), gravity
  cannot overcome rolling resistance, so a bike at rest there stays at rest.
- **Climbs at zero power:** the bike decelerates to v = 0 and **stays there**. It never rolls back (a trainer
  cannot), and position never decreases.
- **Pauses** (user, `stopped_pedalling`, `trainer_lost`, ADR-0001 §5): position freezes and **every resume starts
  from rest** (v = 0). One rule for all three. Carrying the pre-pause speed into the resume would hand out free
  distance, and the trainer was at 0 % resistance (ADR-0002 §7.5) while paused.
- **`stopped_pedalling` in a route ride** (amends ADR-0001 §5 for route rides only): it enters when cadence = 0
  and power = 0 for 3 s **and** the selected source's speed is below 0.5 m/s (a `null` trainer speed counts as
  below). Coasting down a descent at
  cadence 0 is riding, not stopping. On the flat, a coast from 32 km/h falls below 0.5 m/s after about 100 s
  (model (a), default parameters), and that is a stop.

#### 2.4.5 What crank sends and what it computes

| Quantity | Sent to the trainer | Used by model (a) | Why they agree, and nothing counts twice |
|---|---|---|---|
| Grade | page 51: scaled, capped, slewed (§2.3) | the unscaled route grade, capped | The difficulty setting acts on feel only (§2.3.4). The slew is a control bound, not a road |
| Crr | page 51: the surface's Crr (§2.6.2), on the 5×10⁻⁵ grid | the same wire value, decoded | The rider feels gravel and model (a) slows on gravel. The two are separate systems with the same input, never added |
| Air | page 50: Kw = CdA × ρ (0.40 × 1.225 = 0.49 kg/m by default), wind 0 km/h, drafting 1.0 | the same CdA, ρ, wind, drafting | FE-C and model (a) both use ½·Kw·v² (§1) |
| Mass | page 55: rider and bike mass | the same masses | The trainer's inertia and gravity use them, and so does model (a) |
| Drivetrain | — | η = 1 | The NEO measures power at its hub, after the chain. Neither side applies a chain loss |
| Inertia | the trainer's own flywheel simulation | `m_eff` | Each applies its own inertia to its own speed; inertia changes no steady speed |
| Speed, position | — | computed | Only crank computes model (a) |

Every value sent is on the wire grid, and the model integrates the **decoded** wire values (`check_wire`), so
model (a) and the trainer see the same numbers to the last unit. In a route ride page 50 carries the rider's CdA
instead of ADR-0002 §7.9's flat-road default. `Free` and the free-ride control keep that default.

#### 2.4.6 Worked examples

`python3 docs/adr/models/route_speed.py` (rider 75 kg, bike 9 kg, defaults of §2.4.2; example 6 is the relief of §2.8.1):

```text
constants: ADR-0005 §2.4.2, §2.4.3, §2.6.2, §2.8.1 == model (parameters, integration, surface Crr, relief), ok
steady state: integrator == analytic power balance within 0.01 m/s on 5 grades, ok
step: H = 25 ms vs 1 ms reference: max speed error 0.0018 m/s, distance error 0.008 m over 120 s, ok
V_MIN: a 30 s standing start at 200 W covers -0.44 m against the V_MIN -> 0 limit, ok
wire: grade, Crr and wind coefficient round-trip the FE-C grid; table Crr <= 0.0127, ok
invariants: v >= 0, position never decreases, no rolling back, resume from rest, ok
relief: arming grade >= 2.0 % for every FTP (floor at 10 W); never released coasting on a descent or the flat; arms on a stall; no chatter at the arming grade; ends 1 point below it, also at cadence 0, ok
worked examples (rider + bike 84 kg, CdA 0.4 m², Crr 0.004, rho 1.225 kg/m³, hub power, still air):
  1. flat 0 %, 200 W from rest: steady 31.9 km/h, 95 % of it after 32.2 s, 2.58 km in 5 min
  2. climb 8 %, 200 W, entered at the flat speed: steady 10.2 km/h (after 60 s 10.2 km/h), 0.88 km in 5 min, 70 m climbed
  3. descent −6 %, 0 W, entered at the flat speed: terminal 49.4 km/h (after 30 s 47.4 km/h, after 120 s 49.4 km/h)
  4. stop/start, flat: 200 W for 120 s (31.9 km/h), coast 30 s at 0 W (15.4 km/h), pause (resume from rest), 200 W again: 90 % of 31.9 km/h after 23.8 s; 1.62 km in all
  5. flat 0 %, 200 W on gravel (Crr 0.012): steady 28.5 km/h (tarmac 31.9 km/h)
  6. relief arming grade, 30 rpm in 39x25 on 700x25c = 5.9 km/h: FTP 150 W: 10.6 %, FTP 200 W: 14.4 %, FTP 280 W: 20.6 % (at 10 % that speed needs 141 W, at 20 % 272 W)
all checks passed
```

#### 2.4.7 Calibration against the trainer (HIL check 13, added to Part B)

Model (a) and the trainer's page-16 speed (b) should agree in steady state when they use the same inputs. The
new Part B check measures it:

- **Procedure** (`hil_fec check13`, owner, about 4 min): SIM with page 50 at Kw 0.49 kg/m, wind 0, drafting 1.0,
  and page 55 with the rider's masses. Steady effort for 45 s each at grade 0 %, 4 % and 8 %, then a 20 s coast at
  0 % (no pedalling) and 30 s at −4 % coasting (only on mains power: the NEO drives descents only then).
- **Result** per state held ≥ 15 s (means from 10 s after the write): mean power, mean page-16 speed, and
  model (a)'s steady speed at that power (`steady_speed`, the same equations). From the coast, the deceleration
  from page 16 against the model's.
- **Pass:** steady speeds agree within 5 % or 1 km/h, whichever is larger, on 0, 4 and 8 %. Rationale, from
  model (a) at 200 W: the NEO's stated ±2 % power accuracy (DC Rainmaker 2015) alone moves the steady speed by
  0.7 % on the flat and 1.9 % on 8 %, and a 15 % error in the trainer's effective Kw moves the flat speed by 4.3 %.
  A disagreement above 5 % is therefore a modelling difference worth an amendment, not measurement noise. A failed check amends this ADR (the trainer's
  effective Kw or Crr) before the ride-mode issue (R7) closes. The −4 % coast reports the NEO's descent drive and is
  informative only.
- Check 13 depends on #524 (the shared road model with the ½) and #516 (one validated capture timeline), as
  check 8 does.

### 2.5 Speed source (b): the trainer's speed

- **Field:** FE-C page 16 bytes 4–5, speed in 0.001 m/s, and byte 3, distance in metres modulo 256
  (ADR-0002 §4.2). On the NEO in SIM, page 16 speed is the trainer's virtual road speed: hub rotation times the
  wheel size of page 55. A wrong wheel size scales it, which is why page 55 carries the equipment's wheel size.
- **Position from (b)** advances by page-16 **distance** deltas (modulo 256), not by speed × time, so a missed
  page 16 loses no distance (the accumulated-power idea of ADR-0002 §5).
  - A delta is accepted when the time between the two page-16 samples is at most 10 s. 256 m in 10 s is
    92 km/h, so the rollover is unambiguous. It must also be at most `25 m/s × Δt + 1 m`.
  - A longer gap, or an implausible delta, starts a new baseline. The gap's distance is then estimated as the mean
    of the held speeds × time and those seconds are flagged `estimated` in the record, like power gap filling.
- **Displayed speed:** page 16 speed with the 6 s hold of ADR-0001 §5. After that it is `null` ("— no trainer
  speed"), and position holds until page 16 returns. Model (a) keeps running in the background and is recorded,
  but is never substituted for (b) within a ride.
- **Quirks to confirm in HIL (check 13):** page 16's arrival is irregular (§1). Whether its distance field counts
  virtual distance in SIM is measured by check 13's coast: (b)'s distance against ∫ speed dt.
- **Trainers without speed:** `TrainerCaps` gains `speed: bool`, true for `FecTrainer` (it reads page 16 on every
  FE-C link, ADR-0002 §5) and the simulator. FTMS's instantaneous speed is optional in Indoor Bike Data, so `FtmsTrainer` (#45) sets `speed` from
  the Indoor Bike Data flags. When the trainer has no speed, (b) is unavailable. The Ready check says "This trainer
  reports no speed: riding with crank's model", and the ride records `model`. This is shown and recorded, never
  silent.
- **The rider's choice** is the setting `route.speed_source = model | trainer`, default **model** (§5, Q4). It is
  fixed per ride, recorded in `ride.speed_source`, and both speeds are recorded when available (§2.7.5).

### 2.6 OpenStreetMap surface, from a local regional extract

#### 2.6.1 Decision: a local surface index per region

Surface comes from OpenStreetMap **without the route ever leaving the machine**. The rider downloads a whole
region once, crank keeps a compact surface index of it, and every route is matched against that index locally.

- **Choosing a region (once).** Settings → Map data lists the regions of Geofabrik's download server (its
  `index-v1-nogeom.json`, 0.5 MB, fetched when the list is opened), for example a country or a state. The rider
  picks one or more. crank downloads `<region>-latest.osm.pbf` over HTTPS and checks it against Geofabrik's
  published `.md5`.
- **What the network sees:** the region's name and the rider's IP address, in a request every rider of that region
  makes for the same bytes. Never a route, a point, a time stamp or a file name. Downloads happen only when the
  rider starts one in Settings, never during a ride and never on their own. The downloader is in `crank-core`; the
  index builder and matcher are a new pure crate, `crank-osm`. The `deps-edges` check fails if `crank-engine` or
  `crank-domain` reaches either, or `reqwest` (ADR-0001 §2 rule 1).
- **Building the index** (`crank-osm`, reading the PBF with `osmpbf` 0.3.8, MIT OR Apache-2.0):
  - one streaming pass keeps the ways with a ridable `highway` value and their `surface`, `tracktype` and
    `smoothness` tags, reduced to a surface class (§2.6.2) and a highway code;
  - a second pass resolves only those ways' node coordinates, **in bounded memory** (below: builder memory);
  - the result is stored in crank-store as `surface_region` (Geofabrik id, name, extract time from the PBF
    header, md5, built_at, way count) and `surface_way` (region, class, highway code, geometry as zigzag-varint
    deltas of 1e-7° coordinates), with a 0.01° grid table from cell to ways for matching.
  - Non-ridable values are not kept: `motorway`, `motorway_link`, `construction`, `proposed`, `abandoned`,
    `razed`, `planned`, `steps`, `elevator`, `platform`, `raceway`, `bus_guideway`, `busway`, `corridor`, `escape`
    and `via_ferrata`.
  - The downloaded `.pbf` is deleted once the index is built. Only the index is kept.
- **Lookup.** `route_import` matches every route that a stored region covers (§2.6.3). No opt-in is needed,
  because nothing leaves the machine. The report says "surface: OpenStreetMap, <region> extract of <date>, <n> %
  matched". `route_resurface(id)` re-matches a stored route, for example after a refresh. Points outside every
  stored region fall back as below, and the report names the gap and offers to add the region.
- **Fallback, offline or outside every region:** the surface from the file if present, else the default class
  `smooth_paved` (Crr 0.004). The ride only ever reads the surface stored in the route.
- **Update cadence and staleness.** Geofabrik normally updates its extracts every day. crank never refreshes on
  its own. Map data shows each region's extract date and marks a region older than 180 days as outdated; refreshing
  downloads it again and rebuilds the index. A route keeps the surface it was matched with, and its provenance
  records the region and extract date, so a stale match is visible rather than silent. Road surfaces change
  slowly (resurfacing, new gravel), so a season-old extract is still mostly right; 180 days is that rationale.
- **Attribution and licence (ODbL 1.0).** The extracts are "created by OpenStreetMap contributors", licensed under
  ODbL 1.0 (Geofabrik). The index is a derivative database used on the owner's machine. ODbL's share-alike applies
  only to a derivative database that is publicly used (ODbL §4.4), and crank never shares the index. Wherever
  OSM-derived surface is shown (Map data, import report, route library, ride summary) crank shows "Surface data ©
  OpenStreetMap contributors", linked to https://www.openstreetmap.org/copyright (OSMF attribution guidelines). An
  export that carries surface data (GPX/FIT) carries the same attribution and the ODbL notice.
- **A later use, not decided here:** the same index holds road geometry, so a later ADR could draw a route map
  from it with no tile server. This ADR adds no map (§2.6.4) and defers nothing to that idea.

**Costs, measured on 2026-11-05.**

Download sizes (Geofabrik `Content-Length` of `<region>-latest.osm.pbf`):

| Region | PBF |
|---|---|
| Luxembourg | 48 MB |
| Slovenia | 314 MB |
| Switzerland | 550 MB |
| Austria | 815 MB |
| Bavaria | 857 MB |
| California | 1 335 MB |
| Netherlands | 1 410 MB |
| Great Britain | 2 197 MB |
| Germany | 4 876 MB |
| France | 5 110 MB |

Processing Slovenia (314 MB, extract of 2026-11-04) on sage (Mac Studio, Apple silicon):

| Step | Measured | Notes |
|---|---|---|
| Download | 314 MB in 21 s | md5 matches Geofabrik's `.md5` |
| Filter to `highway` ways and their nodes (`osmium tags-filter`, 1.18.0) | 1.8 s wall, 10.2 s CPU, 2.3 GB peak memory | 53 MB filtered PBF |
| Resolve way geometry (`osmium export`, linestrings) | 3.9 s wall, 0.37 GB peak memory | 499 263 ridable ways (10 201 non-ridable dropped), 6 952 101 nodes, 13.9 nodes per way |
| Tags | — | 34 % of the ways carry `surface` (asphalt 106 k, gravel 19 k, unpaved 10 k, ground 8 k, compacted 5 k); 72 k carry `tracktype`, 19 k `smoothness`; the other 66 % take the highway defaults of §2.6.2 |
| **Index size** (crank's format, computed from those ways) | **≈ 35 MB**: 32.3 MB of geometry (zigzag-varint deltas of 1e-7°) + 2.6 MB of grid (637 936 cell-way pairs over 24 944 cells of 0.01°, 4 B each) | A tenth of the PBF |

Reproduce with `docs/adr/models/osm_extract_cost.sh <region.osm.pbf> <work dir>` (osmium-tool and python3; it runs
`docs/adr/models/osm_index_size.py`, stdlib only, which computes the index size, tag coverage and node count).

**Builder memory.** osmium stands in for crank's builder here because it is a native PBF reader. Its 2.3 GB peak
comes from holding every node location of the extract. A builder that keeps only the kept ways' nodes in memory
(id and coordinates, 16 B each) needs 111 MB for Slovenia's 6 952 101 nodes. Scaled by PBF size, that is about
1.7 GB for Germany before hash-map overhead, which typically doubles it. That is too much for a desktop app
running beside other software on an 8–16 GB laptop, and the cost grows with the region the rider picks.

**Decision: a streaming builder with bounded memory** (§5, Q9):

- Pass 1 writes each kept way's node references, as (node id, way, position) records, into runs of a fixed size.
  Each run is sorted by node id and spilled to a temporary file in the app's cache directory.
- Pass 2 merges the runs with the node stream. A sorted PBF delivers nodes in id order: Geofabrik's extracts are
  sorted by type, then id (Slovenia's declares `pbf_optional_feature_0=Sort.Type_then_ID`, read with
  `osmium fileinfo`). The builder checks the PBF header's `Sort.Type_then_ID` feature and refuses an unsorted
  file, naming why.
- The resolved coordinates are sorted back into way order the same way, and each way's geometry is then written
  in sequence.
- Peak memory is the run buffers plus the merge heap: under 256 MB for any region.
- The cost moves to temporary disk, about 2 × 20 B per node reference: 0.3 GB for Slovenia and about 4 GB for
  Germany while it builds. That is on top of the download, and freed afterwards.

The build time for this design is not measured. The R6 acceptance measures it on Slovenia and pins a peak-memory
bound with a run size small enough to force spills on its fixture.

Filtering and the index are both linear in the PBF's size, so a region's figure is Slovenia's scaled by its PBF
size. These are estimates, not measurements, because the mix of road types varies by region:

| Region | Index | Temporary disk while building |
|---|---|---|
| Switzerland | about 60 MB | about 0.5 GB |
| Bavaria | about 95 MB | about 0.8 GB |
| Germany | about 540 MB | about 4 GB |

The download is the dominant cost, and it happens once per region and refresh.

#### 2.6.2 Surface → Crr

Surface classes, their OSM tags and Crr (checked against `route_speed.py`):

| Class | OSM `surface` values | Crr | Source or rationale |
|---|---|---|---|
| `smooth_paved` | `asphalt`, `concrete`, `paved`, `chipseal`, `concrete:lanes` (with `smoothness` absent, `excellent`, `good` or `intermediate`) | 0.004 | FE-C sample "asphalt" (Garmin `SimulationSettings`); ADR-0002 §7.9's default |
| `rough_paved` | `paving_stones`, `concrete:plates`, `metal`, `wood`; any paved value with `smoothness` `bad` or worse | 0.008 | FE-C sample "rough road" (Garmin `SimulationSettings`) |
| `gravel` | `compacted`, `fine_gravel`, `gravel`, `pebblestone`, `unpaved`, `dirt`, `earth`, `ground`, `rock` | 0.012 | Steyn & Warnich 2014: gravel 0.006 against bituminous 0.002, a ratio of 3, applied to crank's 0.004 |
| `grass` | `grass`, `grass_paver` | 0.0127 | Steyn & Warnich 2014: grass 0.007, a ratio of 3.5 (0.014), clamped to `Crr_max` |
| `sand` | `sand`, `mud` | 0.0127 | Steyn & Warnich 2014: sand 0.030, a ratio of 15 (0.060), clamped to `Crr_max` |
| `cobbles` | `sett`, `cobblestone`, `unhewn_cobblestone`, `cobblestone:flattened` | 0.0127 | No bicycle measurement on cobbles was found. They are rougher than "rough road", so the FE-C ceiling is used (Q5, decided in §5) |

- **No `surface` tag:** `highway=track` with `tracktype=grade1` is `rough_paved`; any other `track`, and `path` or
  `bridleway`, is `gravel`; every other `highway` value is `smooth_paved` (OSM's default assumption for roads).
- **Crr above the wire maximum** never reaches the wire: every class is at or below `Crr_max` by construction,
  checked by the model. The real Crr of sand and grass is higher, so the ride is easier there than the road would
  be; that is the trainer's limit, and both sides use the same capped value (§2.4.5).
- **Why ratios:** Steyn & Warnich measured mountain-bike tyres by coast-down. Their absolute values (0.002 on
  bitumen) are lower than the FE-C asphalt sample, but the surface ratios are what transfers between tyres.
  Their surface explained most of the variance (r = 0.81), and tyre pressure, mass and wheel size were
  second-order.

#### 2.6.3 Matching

- Each 10 m node takes the nearest indexed way segment within 20 m (a GPS track is within a few metres of the
  road, and a 20 m limit avoids snapping to a parallel road beyond that).
- When two ways are within 2 m of the same distance, the node keeps the previous node's way, so a junction
  does not flicker.
- A surface run shorter than 50 m between two equal classes is absorbed into them. A short bridge or a mapping
  gap then does not change the Crr for 2 s.
- Unmatched nodes fall back as in §2.6.1. `matched_share` (the share of nodes matched) is stored and shown.

#### 2.6.4 Map display: decided out of scope

The ride view shows **no map** in v1. Tiles are another network call, with their own privacy, usage policy and
attribution. The ride shows the elevation profile with a position marker and a surface band (§2.7.6). The route
library may draw the route's outline from its own coordinates, as a plain SVG line with no tiles; that is local
data, no network.

#### 2.6.5 Rejected for privacy: what a per-route lookup cannot hide

The owner's options of 2026-11-05 (random sampling, other providers, a local copy) and the per-route lookup that
the first draft proposed, judged by **what the provider can learn about the route**:

| Alternative | Why rejected |
|---|---|
| **Random sampling of the route across several requests** | Every sampled point still lies on the route. The requests come from one IP within a short time, so the provider can join them back into the route. Decoy points help only when they are numerous, and then they load a volunteer-run service against its usage policy |
| **Other providers** (another Overpass instance, a commercial API) | Changes whom we trust, not what we disclose: the route still leaves the machine |
| **Caching tiles or cells along the route** | Fetching the tiles that cover the route's bounding box or corridor still traces the route at tile resolution (zoom 14 is 2.4 km at the equator, 1.7 km at 45°N). Only a uniform download of a whole region hides the route, which is the extract above |
| **Per-route Overpass lookup** (the first draft: cells of the corridor, start and end trimmed, opt-in per route) | It still discloses a corridor of the route and the IP. It is **not offered**, not even as a fallback: the extract covers the same need with no disclosure, and a route outside every stored region falls back to the file or the default, and the report offers the missing region |

### 2.7 Ride integration

#### 2.7.1 A route ride is a plan the engine runs

`EngineCmd::Load` takes a `RidePlan`: `Workout(Workout)` (today's) or `Route(RouteRide)`. `RouteRide` holds:

- the `Profile` of §2.1;
- the `RouteSettings` (scales, cap);
- the speed source and the model parameters (masses, CdA).

A route ride is **SIM from start to end**. It has no blocks, no ERG targets and no compliance: ADR-0002 §7.8
compliance is ERG-only. The judge (ADR-0001 §1) gets no route features in v1. ERG segments inside a route are not
part of this design (§5, Q7).

#### 2.7.2 Each tick

1. Read the tick's power and cadence (ADR-0001 §5 holds).
2. Advance the position from the selected source: model (a) integrates (§2.4), and (b) adds page-16 distance
   (§2.5). The other source is computed or held for the record.
3. Look up the route grade and surface at the position; apply the pipeline of §2.3 (scale, cap, slew, grid).
4. Emit `ControlMode::Sim { grade_pct, crr, wind }` when the grid value changed and the 1 s send period allows
   (§2.3.5). Page 50's values are constant for the ride, so only page 51 changes.
5. No lead: the grade is the one at the current position, like ERG's lead of 0 (ADR-0002 §7.2). A lead may be
   added only by an amendment backed by check 3's end-to-end latency.

#### 2.7.3 Start, pause, release, end

- **Start.** The first mode is `Sim` with the commanded grade starting at 0 % and slewing towards the route grade
  (1 %/s), Crr and page 50 as decided. A first steep metre never arrives as a step. ERG's 0 W hold (ADR-0002
  §7.5) has no SIM counterpart. In SIM the brake force depends on the grade, not on a power target, so a
  standing start in SIM is a standing start on a road.
- **User pause:** `Resistance { pct: 0 }`, as in every ride (ADR-0002 §7.5, DESIGN §5.2). Position freezes; the
  resume starts from rest (§2.4.4) and slews the grade from 0 % again.
- **Low-cadence relief on climbs (§2.8.1):** only on a grade at or above the rider's arming grade (≥ 2 %), never
  on a flat or a descent. A stall there switches to `Resistance { pct: 0 }`. The relief ends on spinning up, or as
  soon as the target grade eases 1 point below the arming grade, so a coasting descent is always simulated.
- **End of route:** when the position reaches the route's length, the ride finishes as after a workout's last
  block (ADR-0001 §5): the end mode `Resistance { pct: 0 }` with its delivery hold (ADR-0002 §7.9).
- **ERG ↔ SIM:** there is no switch inside a route ride. A route ride's end mode is the ADR-0002 §7.9 end mode, and
  a following ride's engagement rules apply as decided there.
- **Trainer lost:** as for every ride (ADR-0001 §5, ADR-0002 §6). Position freezes from the detecting tick.
  The ≤ 2 s window coasts (power `null` counts as 0 W). Reconnect re-applies `Sim`, slewing from 0 %.

#### 2.7.4 Skip, distance left, finish

`SkipBlock` has no meaning in a route ride and returns `InvalidInput`. The status gains `routePositionM`,
`routeLengthM`, `routeGradePct` (unscaled at the position), `commandedGradePct`, `surface` and `speedSource`.
They are `null` outside route rides, and the UI never derives position from time (ADR-0001 §5's live-cursor rule).

#### 2.7.5 Recording and export

- **`ride`:** `route_id`, `speed_source` (`model` | `trainer`), and `route_json`: `RouteSettings`, masses,
  CdA, ρ, `PROFILE_VERSION` and the OSM provenance with its attribution.
- **Per-second metrics** (`ride_chunk`, ADR-0001 §6):
  - `route_distance_m`: the selected source's position;
  - `speed_mps`: the selected source;
  - `speed_model_mps`, `speed_trainer_mps`: both, when available, for calibration and review;
  - `route_grade_pct`, `commanded_grade_pct`;
  - `route_elevation_m`: smoothed, at the position;
  - `surface`: class code;
  - `estimated`: set for source (b)'s gap seconds (§2.5).
- **Export through #257:** a FIT activity with `sport` cycling and `sub_sport` virtual_activity. `record`
  messages carry `distance`, `speed` (the selected source), `enhanced_altitude` (the route elevation at the
  position) and `position_lat/long` when the route has coordinates. That last field is the rider's choice at
  export, because it reveals the route (#257 owns the export dialog). Surface data in an export carries the ODbL
  notice (§2.6.1).

#### 2.7.6 The ride screen (UI issue R9, a DESIGN amendment)

DESIGN §18.3's anatomy keeps its geometry. In a route ride:

- the metric column's countdown shows distance left (T1) with "km left";
- the power row shows no target band (SIM has none);
- a grade row shows the grade now (unscaled, with the commanded grade in the sub-line when they differ) and the
  surface;
- speed shows with its source ("speed · crank model" / "speed · trainer");
- the block timeline becomes the **elevation profile**: done part dimmed, a "now" marker, the next 10 km at
  readable scale and the whole route thin below it (DESIGN §16.1's two-scale road), with a surface band under it.

A state banner says "Grade released · spin up to 50 rpm" during the low-cadence relief. Fixed slots (P4) apply as
everywhere. DESIGN is amended by R9 before the screen is built.

### 2.8 Safety and FMEA

| Hazard | Guard | FMEA |
|---|---|---|
| A sudden steep grade (a step in the data, a grade-profile segment boundary, resuming on a climb, a reconnect) | Slew 1 %/s from what was last commanded, and from 0 % after start, pause, release or reconnect (§2.3.5, §2.7.3) | FM-ROUTE-01 (data), -05 (start), -04 (pause), -11 (reconnect); the release's slew is FM-ROUTE-03 |
| A grade beyond what the rider can push | Cap +20 % after scaling (§2.3.3) | FM-ROUTE-02 |
| A stall on a climb: low cadence under a grade the rider cannot push (strain; the flywheel stops and the rider cannot restart) | Low-cadence relief, armed only at or above the rider's arming grade (§2.8.1) | FM-ROUTE-03 |
| The relief fires, or stays on, off a climb: coasting on a descent or the flat at cadence 0 drops the simulated road | The arming grade is ≥ 2 %; the relief ends at once when the target grade falls below it (§2.8.1) | FM-ROUTE-12 |
| Descents | The NEO drives the flywheel, never the pedals (freehub, §1). Model (a) is bounded by the grade cap. Coasting does not auto-pause (§2.4.4) | FM-ROUTE-06 |
| The end of the route leaves resistance applied | End mode `Resistance { 0 }` with the delivery hold (ADR-0002 §7.9) | FM-ROUTE-07 |
| A Crr above the wire maximum refuses the mode and stalls control | Every class ≤ `Crr_max` by construction (§2.6.2) | FM-ROUTE-08 |
| Model (a) produces a non-finite or negative speed and the position jumps | Speed is clamped ≥ 0, and position never decreases (`check_invariants`); a non-finite input (power `null`) counts as 0 W | FM-ROUTE-09 |
| The trainer's speed stops under source (b) | Position holds, "— no trainer speed", recorded `null` (§2.5) | FM-ROUTE-10 |

#### 2.8.1 Low-cadence relief on climbs

**The hazard, and why cadence alone does not show it.** On a climb in SIM the trainer brakes with the road's
force, `m·g·(sin θ + Crr·cos θ)` plus a little air. The rider must deliver that force at the wheel through the
gears. If the grade asks more than the rider can sustain in their lowest gear, cadence falls until the
flywheel stops. Restarting from rest against the full climb force means a near-standstill pedal stroke at the
highest torque of the ride (the strain hazard), and check 9 measures the same stall in ERG. Low cadence alone is
not this hazard. A rider coasting a descent or a flat has cadence 0 by choice, with almost no force on the pedals.
The first draft armed the relief on cadence below 30 rpm with any positive grade, and ended it only on spinning up.
So a relief at the top of a climb stayed on down the descent and dropped the simulated road (owner report of
2026-11-06). The trigger is therefore defined from the hazard: **force the rider cannot sustain at low cadence**.

**Signal.** The NEO reports no torque or resistance on FE-C. In the #44 Part A captures it sends pages 25
(power, cadence) and 16 (speed), Tacx-specific pages `0xF0`, `0xFA` and `0xFB`, and the replies crank requests
(54, 55, 71, 80, 81). There is no page 26 (trainer torque data). Torque derived as power / cadence is not usable
either: it degenerates exactly at the stall, where both go to 0. The relief therefore arms on the **commanded
grade**. crank chose that grade, so it is known exactly.

**Arming grade (derived from the stall physics with model (a)).**

- At the relief cadence `RELIEF_RPM` in the rider's lowest gear, the road speed is
  `v_r = RELIEF_RPM / 60 × LOWEST_GEAR × WHEEL_CIRC_M`.
- The arming grade `G_arm` is the grade at which holding `v_r` needs the rider's FTP:
  `F(G_arm, v_r)·v_r = FTP`, with `F` the resistive force of §2.4.1 (`arming_grade` in `route_speed.py`).
- Above `G_arm`, a rider who has fallen below the relief cadence needs more than their FTP just to hold it. Unless
  they produce it, they cannot pedal back up, so they are stalling. Below `G_arm`, the same cadence costs less than
  FTP. That is a slow grind or a gear choice, and the rider can shift: it is not a stall.
- `G_arm` is computed at ride load from the rider's mass and FTP (athlete profile, ADR-0001 §6) and the equipment's
  lowest gear and wheel. It is floored at `GRADE_ARM_MIN` = 2 %, so the relief can never arm on a flat or a
  descent, whatever the inputs.

| Constant | Value | Source or rationale |
|---|---|---|
| `RELIEF_RPM` relief cadence | 30 | Below it the crank turns at most once in 2 s: the pedal stroke is a standing push, not pedalling. ERG's release uses 40 rpm against the ERG spiral (ADR-0002 §7.4); SIM has no spiral, because the force does not rise as cadence falls |
| `RELIEF_HOLD_S` | 3 | Longer than one slow pedal stroke at 30 rpm (2 s), so a single dead spot never arms it; the same 3 s as `stopped_pedalling` (ADR-0001 §5) |
| `RETURN_RPM` return cadence | 50 | Clearly pedalling again. The 20 rpm gap to `RELIEF_RPM` is the cadence hysteresis |
| `RETURN_HOLD_S` | 2 | ERG's re-engagement hold (ADR-0002 §7.4) |
| `RELIEF_HYST_PCT` grade hysteresis | 1.0 | The relief ends on grade only when the target eases below `G_arm − 1` point. A stalled rider's target that hovers at `G_arm` (the grade wobbles by a few tenths over a few metres, and at walking pace those metres take seconds) would otherwise toggle the relief every few seconds. `check_relief` shows the same hovering trace toggling ten times without it, and once with it. One point is 13 W at the default relief speed, at the arming grade of a 200 W rider (7 % of that FTP): eased by that much, the climb is materially easier than the stall grade. It is twice the ±0.5 % hover that `check_relief` models |
| `LOWEST_GEAR` default | 1.56 | 39 × 25, the largest lowest gear of a common road groupset: Shimano 105 R7000 offers 53/39, 52/36 and 50/34 chainrings and 12-25, 11-28, 11-30 and 11-32 cassettes (§7). A bigger gear gives a higher speed at the relief cadence and so a lower arming grade. With 39 × 25 as the default, every combination of that range arms at or below its true grade (table below), so a rider of that range who has not set their gear is relieved early, never late. Older or racier gearing is bigger still and arms late: 39 × 23 (a standard double with a 12-23 cassette) arms 1.2 points late for a 200 W rider (13.2 % against the computed 14.4 %), which is why the rider sets their real gear in Settings. The rider sets their real lowest gear (chainring and largest cog) in the equipment settings |
| `WHEEL_CIRC_M` | 2.111 | 700×25c, ISO 5775 25-622: π × (622 + 2 × 25) mm. The equipment's wheel size (page 55) when set |
| `GRADE_ARM_MIN` | 2.0 | Floor. At 2 % the relief speed needs 26 W, at which no rider stalls, and it keeps every flat and descent out of the relief by construction |

These rows are checked against `route_speed.py` (`check_constants`). Worked example 6 (§2.4.6) gives, with the
default gear, `v_r` = 5.9 km/h and arming grades of 10.6 % (FTP 150 W), 14.4 % (200 W) and 20.6 % (280 W, the
reference athlete) for 84 kg.

The same rider (84 kg) in the lowest gears of that range (`arming_grade` with each gear):

| Lowest gear | Ratio | Arming grade, FTP 150 W | FTP 200 W |
|---|---|---|---|
| 39 × 25 (default) | 1.560 | 10.6 % | 14.4 % |
| 39 × 28 | 1.393 | 12.0 % | 16.3 % |
| 36 × 28 | 1.286 | 13.1 % | 17.7 % |
| 34 × 28 | 1.214 | 13.9 % | 18.8 % |
| 36 × 32 | 1.125 | 15.1 % | 20.4 % |
| 34 × 32 | 1.062 | 16.0 % | 21.7 % |

The default is the lowest row, so without a gear setting the relief arms at or before the rider's true arming
grade. Once the rider sets their real gear, a strong rider in a compact may never reach it under the 20 % cap;
for that rider the cap is the guard.

**What the relief does.**

- **Arms:** cadence below `RELIEF_RPM` for `RELIEF_HOLD_S`, while the target grade (route grade after scaling and
  the cap, §2.3) is at or above `G_arm`. The cadence is the held trainer cadence of ADR-0001 §5; an invalid cadence
  counts as 0.
- **Relief:** `Resistance { pct: 0 }` at once, the same single-page, echo-verified mode as a pause (ADR-0002 §7.5).
  It does not slew the grade down, because a stall needs the force gone now, not in 1 %/s steps. Position keeps
  advancing under the selected source (model (a) coasts and stops on the climb), and the banner says "Grade released
  · spin up to 50 rpm".
- **Ends** when either:
  - cadence is at or above `RETURN_RPM` for `RETURN_HOLD_S`; or
  - **the target grade eases below `G_arm − RELIEF_HYST_PCT`** (1 point below the arming grade), at once and at
    any cadence. A target that hovers around `G_arm` keeps the relief on rather than toggling it.

  Either way, `Sim` returns with the grade slewed from 0 % towards the target (§2.3.5). If the climb goes on and the
  rider stalls again, the relief arms again: shown each time, never hidden.
- **Never mid-descent:** arming needs a target at or above `G_arm` ≥ 2 %, and a relief that is on ends as soon as
  the target eases below `G_arm − 1` point (at least 1 %). So a descent and a coasting flat are always simulated.
- `check_relief` runs, at cadence 0:
  - a minute at −15, −6, −0.5, 0 and 1.9 %;
  - a relief at the top of a climb that tips into −6 %;
  - a target hovering ±0.5 % around `G_arm` (released once, no chatter);
  - the hysteresis edge;
  - a 10 W FTP, where the 2 % floor applies.
- `stopped_pedalling` (§2.4.4) still applies on top: a rider who stops on a climb (speed below 0.5 m/s) is paused,
  and the pause's `Resistance { 0 }` takes over.

Every row is in `docs/safety/fmea.toml` as `missing` with #545 (R7, §6), whose acceptance adds the engine tests
and flips the rows to `guarded`. The `free_sim` phase covers a route ride. Track-page loss in a route ride is FM-FREE-04's
case, unchanged.

### 2.9 Tests and acceptance per phase

| Phase (issue, §6) | Acceptance |
|---|---|
| R1 route types, GPX import, grade-profile text | Property tests: every grade-profile text round-trips (parse → print → parse equal, print idempotent). Every §2.2.4 limit and refusal has a test that fails without it (the oversize file, too many points, (0, 0) dropped and counted, a 10 km jump, an XML bomb (a DOCTYPE with nested entities) refused, depth 33 refused). Missing elevation: a 400 m gap filled and counted, a 600 m gap refused, no elevation at all → `ElevationKind::None`. `500m` refused with the minutes message. Multi-track files list their tracks. The fixture of `route_grade.py` imports to the same distance as the model (±0.1 %) |
| R2 FIT course | A course file round-trips its points; a FIT activity refuses with `NotACourse`; semicircle conversion tested at ±180°; `enhanced_altitude` preferred |
| R3 store | `put_route`/`get_route` bit-exact (f64 NaN included) by property test; the migration is forward-only; deleting a route keeps its rides readable |
| R4 elevation → grade | A refinement fixture `route_grade.json` emitted by `route_grade.py` (smoothed profile of every fixture segment, the pipeline trace of the 0 → 25 % step); the Rust `Profile` and pipeline match it within 1e-9; the flake check `route-grade-model` runs the model |
| R5 speed model | A refinement fixture `route_speed.json` from `route_speed.py` (the worked examples' per-tick traces); the Rust integrator matches within 1e-9 m/s; the flake check `route-speed-model` runs the model; both models are also run by `just model` |
| R6 local surface index | No network call outside the Map data download, and none from `route_import`, `route_resurface` or a ride (a mock transport that fails the test on any other request). The md5 is checked and a mismatch refuses the region. A filtered fixture extract (a few km², committed under the size limit) builds an index whose way count, classes and matched surfaces are pinned. The `.pbf` is deleted after the build, and the builder's peak memory stays under its bound with a run size that forces spills on the fixture (§2.6.1, builder memory); an unsorted PBF is refused. A route outside every region falls back and names the gap. A stale region (> 180 days) is marked. `deps-edges` fails when `crank-engine` or `crank-domain` depends on `crank-osm` or `reqwest`. Attribution is present in Map data, the import report and the summary DTO |
| R7 ride mode | `crank-engine` scenarios with `SimTrainer`: the slew bound on every emitted grade; the cap; start, pause, reconnect and release slew from 0 %; the low-cadence relief arms only at or above the arming grade, ends on spinning up or at once when the target eases 1 point below it, does not chatter on a target hovering at it, and never engages coasting a descent or a flat (FM-ROUTE-12); the end of the route sends the end mode; coasting a descent does not auto-pause; a stop at the bottom of a climb does; source (b)'s distance across the 256 rollover and a 12 s gap (estimated); a trainer without speed refuses (b) at load with the notice; "a grade changing every second stays `following`". FMEA rows FM-ROUTE-01..12 flip to `guarded` |
| R8 HIL check 13 | §2.4.7 runs in the owner's Part B session; results in `docs/hil/<date>-fec.md`; a failed pass criterion amends this ADR |
| R9 UI | The DESIGN amendment first; then Vitest + mockIPC for the import dialog and its report, Map data (region list, download progress, extract date, outdated mark, attribution), the library, and the ride template; design-harness fixtures for the route ride states; the mocks model every refusal of `route_import` and `ride_load` for routes |
| R10 FIT export | A route ride exported through #257's FIT writer has `sub_sport` virtual_activity and, per `record`, `distance`, `speed` (the selected source) and `enhanced_altitude` (the route elevation at the position). `position_lat/long` are present only when the rider opts in at export and the route has coordinates. Surface data in the export carries the ODbL notice. #257's FIT import reads the file back with the same distance and speed per second |

## 3. Alternatives considered

| Alternative | Why rejected |
|---|---|
| Send the surface Crr above 0.0127 as extra grade (Crr − 0.0127 ≈ a grade offset) | It turns surface into slope, which the grade cap, the slew and the shown grade would then have to explain; indoors the difference above 0.0127 is not a safety or training matter. Both sides use the capped value instead |
| Model (a) uses the scaled grade | Then a 50 % difficulty would shorten every climb's time, and (a) would be (b) with extra steps. The convention is feel only |
| Air density from route elevation (ρ falls about 18 % at 2 000 m) | Page 50 would change continuously, adding a wind write to most grade changes and churn to the `wind` component, for a realism gain indoors that the rider never feels at the pedals. ρ stays the ISA constant |
| Query Overpass with the route line (`around:` a polyline) | It sends the exact path. The per-route alternatives are in §2.6.5 |
| Match surface during a ride, or on every load | The match is stored with the route, so a ride reads a fixed surface and is the same offline; a refresh re-matches only when the rider asks (`route_resurface`) |
| Elevation from a terrain model (DEM) when the file has none | Another third-party call with the whole route. Riding flat with a label is honest and needs no network |
| Savitzky–Golay or a moving average for smoothing | A moving average ripples, and Savitzky–Golay keeps short spikes (it preserves peaks by design). Both depend on the sample spacing unless resampled first; the Gaussian over 10 m samples depends on distance only |
| Kalman filtering elevation with time stamps | Routes from planners have no time stamps, and one rule should serve every source |
| One `trk` per import chosen automatically (the longest) | Silent choice; multi-day files would ride the wrong day |
| A route as a workout DSL extension (distance ends inside `parse_dsl`) | `m` is minutes there, and `%` is % FTP; one grammar with two meanings per token would be a trap. A sibling grammar with the same lexer, labels, repeats and printing is consistent without the ambiguity |
| Applying a drivetrain loss (η = 0.976 or Spicer's range) to trainer power | The NEO measures at the hub; the loss is already outside the number |
| Switching the speed source mid-ride | One source per ride keeps the record and its distance interpretable (ADR-0002 §5's "one telemetry source per ride, never mixed") |
| A map in the ride view | Tile requests during a ride, which the privacy rule excludes; the profile answers "how far, how steep" |

## 4. Consequences

- Routes are a second kind of ride plan. The engine, store, contract and UI each gain a route path, and the
  ERG-only rules (compliance, soft start, ERG release) stay as they are, because a route is SIM throughout.
- crank downloads public OSM extracts, but sends no route anywhere: the network sees only which region the rider
  chose. The price is disk and a one-off download per region (§2.6.1: from 48 MB for Luxembourg to 5.1 GB for
  France), and surfaces as old as the extract.
- Two executable models (`route_speed.py`, `route_grade.py`) join `trainer_following.py`. Their refinement
  fixtures tie the Rust code to them (R4, R5).
- Model (a) and the trainer use the same inputs, so check 13 tests both the trainer and crank's model. A
  disagreement amends a number here rather than adding a correction factor.
- Short real ramps under about 200 m are softened by the 100 m kernel; grade profiles are exact.
- Sand, grass and cobbles are capped at the trainer's maximum Crr, so they ride easier than they are.
- Check 8's analyser needs #524 before Part B: its air term is twice too large.

## 5. Owner decisions

Decided by the owner on 2026-11-06, when ADR-0005 was accepted. Q1–Q5 and Q7–Q9 are as recommended. Q6 was
reworked after the owner's report that the first draft fired on descents (§2.8.1); the reworked recommendation is
what was accepted. This ADR implements every row as decided.

| # | Question | Recommended | Cost of the recommendation | Alternative and its cost | Decision |
|---|---|---|---|---|---|
| Q1 | Grade cap | −15 % to +20 % | Roads steeper than 20 % over 100 m ride as 20 % | +25 % (the NEO's maximum): more stall exposure at low cadence | Decided: as recommended |
| Q2 | Difficulty defaults | 100 % overall, 100 % on descents | A weak day on a hilly route is the road's difficulty until the rider lowers it | 50 % overall by default (common in other apps): the route no longer feels like itself unless changed | Decided: as recommended |
| Q3 | When a region's extract counts as outdated | 180 days, shown in Map data; refresh only on the rider's request | Surfaces up to half a year old | 30 days: a 5 GB France download monthly for little change | Decided: as recommended |
| Q4 | Default speed source | (a) crank's model | Speed differs from what the trainer would report until check 13 calibrates | (b) the trainer: no surface effect on speed unless sent as Crr, and nothing on trainers without speed | Decided: as recommended |
| Q5 | Cobbles' Crr | 0.0127 (the FE-C ceiling) | Unmeasured; may be harder than real cobbles on wide tyres | 0.008 (FE-C "rough road", sourced): cobbles feel like rough tarmac | Decided: as recommended |
| Q6 | When the low-cadence relief arms (owner, 2026-11-06: it must never fire on a descent) | Only at or above the rider's arming grade: the grade at which holding 30 rpm in the lowest gear needs more than their FTP, floored at 2 %. The default gear, 39 × 25, is the largest lowest gear of the Shimano 105 R7000 range, so for any gear of that range it arms early, never late, until the rider sets their own. Older or racier gearing is bigger still and arms late: 39 × 23 (a standard double with a 12-23 cassette) arms 1.2 points late for a 200 W rider (13.2 % against the computed 14.4 %), which is why the rider sets their real gear in Settings. Below 30 rpm for 3 s there → `Resistance { 0 }`. It ends at 50 rpm for 2 s, or at once when the target eases below the arming grade minus 1 point, the hysteresis that stops it toggling on a target hovering at the arming grade (§2.8.1) | Needs a current FTP and the rider's real lowest gear (an equipment setting). Until it is set, a rider with a smaller gear is relieved on grades they could still push (by up to 7 points at 200 W with 34 × 32). A rider grinding at 25 rpm above the arming grade is released | Cadence below 30 rpm on any positive grade (the first draft): fires on rolling terrain, and without the climb-end rule it dropped every descent after a relief | Decided: as reworked (§2.8.1) |
| Q7 | ERG blocks inside a route (a structured workout on a route) | Not in v1 | A route ride has no targets | A combined plan: compliance, soft start and SIM↔ERG switching inside one ride; a design of its own | Decided: as recommended |
| Q8 | Per-route Overpass lookup as a fallback for routes outside every stored region | Not offered (§2.6.5) | Such a route rides on the file's surface or tarmac until the rider adds its region | An opt-in per-route lookup: discloses the route's corridor and the IP to a volunteer-run service | Decided: as recommended |
| Q9 | How the index builder resolves node coordinates (§2.6.1) | A streaming external-sort builder: under 256 MB peak for any region | More code than a hash map; about 4 GB of temporary disk and an unmeasured, longer build for Germany | An in-memory map: about 110 MB for Slovenia but about 1.7 GB (before hash overhead) for Germany, so regions would need a PBF cap of about 1 GB (a state rather than a large country) | Decided: as recommended |

## 6. Implementation issues (filed 2026-11-06)

In order. Each issue cites the section it builds and the acceptance of §2.9.

| # | Issue | Title | Scope | Depends on |
|---|---|---|---|---|
| R1 | #539 | Route model and GPX + grade-profile import (ADR-0005 §2.1, §2.2.1–2.2.2, §2.2.4–2.2.5) | `crank-domain::route` types, `Profile`; GPX parser (`quick-xml`, hardened), missing-elevation rules, limits, import report; grade-profile grammar, parser, printer | — |
| R2 | #540 | FIT course import with the shared `crank-fit` decoder (ADR-0005 §2.2.3) | `crank-fit` crate over `fitparser`, course messages → `Route`; the decoder #257 reuses | R1 |
| R3 | #541 | Route store and `route_import` / `route_list` / `route_get` / `route_delete` commands (ADR-0005 §2.1, §2.2) | migration, `route` table, `ride` columns, bit-exact round trip, `Core` facade + Tauri commands + bindings | R1 |
| R4 | #542 | Elevation → grade profile and the send pipeline (ADR-0005 §2.3) | resampling, Gaussian smoothing, cap, scaling, slew/grid/period as pure functions; `route_grade.json` refinement fixture; flake check `route-grade-model` | R1 |
| R5 | #543 | Speed model (a) (ADR-0005 §2.4) | integrator in `crank-domain`, `steady_speed`; `route_speed.json` refinement fixture; flake check `route-speed-model`; `just model` runs all three models | R4 |
| R6 | #544 | Local OSM surface index: region download, index build, matching (ADR-0005 §2.6) | Geofabrik region list and download with md5 check (`crank-core`); `crank-osm` (PBF filter over `osmpbf`, surface classes, index, matching); `surface_region`/`surface_way` tables; `route_resurface`; fallback, attribution, `deps-edges` rule | R3, R4 |
| R7 | #545 | Route ride mode in the engine (ADR-0005 §2.5, §2.7, §2.8) | `RidePlan::Route`, position, SIM emission, start/pause/relief/end (the relief's arming grade from FTP, mass and the equipment's new lowest-gear setting, §2.8.1), `stopped_pedalling` amendment, source (b) with `TrainerCaps::speed`, recording, status fields; FMEA rows to `guarded` | R4, R5 (R6 optional: surfaces default without it) |
| R8 | #546 | HIL check 13: speed model against trainer speed (ADR-0005 §2.4.7) | `hil_fec check13` and its analysis; added to `docs/hil/part-b.md` and the owner's runbook | R5, #524, #516 |
| R9 | #547 | Route UI: import dialog, Map data settings, route library, route ride screen (ADR-0005 §2.6.1, §2.7.6) | DESIGN amendment first, then the screens and design-harness fixtures; the equipment screen gains the lowest gear (chainring, largest cog; default 39 × 25) | R3, R6, R7 |
| R10 | #548 | Route rides in FIT export (ADR-0005 §2.7.5) | the `virtual_activity` fields, position opt-in, ODbL notice in the export | #257, R7 |

## 7. References

Checked 2026-11-05.

- Martin, J. C., Milliken, D. L., Cobb, J. E., McFadden, K. L., Coggan, A. R. (1998). Validation of a mathematical
  model for road cycling power. *Journal of Applied Biomechanics* 14(3), 276–291. doi:10.1123/jab.14.3.276 (the model
  form: aerodynamic drag, rolling resistance, bearing friction, wheel inertia, potential and kinetic energy; its
  constants are not used here because they could not be checked against the paper's text).
- Steyn, W. J. vdM., Warnich, J. (2014). Comparison of tyre rolling resistance for different mountain bike tyre
  diameters and surface conditions. *South African Journal for Research in Sport, Physical Education and
  Recreation* 36(2), 179–193. https://repository.up.ac.za/handle/2263/42518 (Table 2: bituminous 0.002, gravel
  0.006, grass 0.007, sand 0.030).
- Spicer, J. B., Richardson, C. J. K., Ehrlich, M. J., Bernstein, J. R., Fukuda, M., Terada, M. (2001). Effects of
  frictional loss on bicycle chain drive efficiency. *Journal of Mechanical Design* 123(4), 598–605.
  doi:10.1115/1.1412848 (80.9–98.6 % depending on chain tension).
- ISO 2533:1975, Standard Atmosphere (sea-level density 1.225 kg/m³).
- BIPM, 3rd CGPM (1901), Resolution 2: standard acceleration of gravity 980.665 cm/s².
  https://www.bipm.org/en/committees/cg/cgpm/3-1901/resolution-2
- Moritz, H. (2000). Geodetic Reference System 1980. *Journal of Geodesy* 74, 128–133 (mean Earth radius
  R₁ = 6 371 008.8 m).
- Garmin Connect IQ API, `Toybox.AntPlus.SimulationSettings`, read 2026-11-05: `windResistance` "Wind Resistance
  Coefficient [kg/m] = Frontal Surface Area [m2] x Drag Coefficient x Air Density [kg/m3]", 0.0–1.86 kg/m, invalid
  0xFF; `draftFactor` 0–1 (1.0 = no drafting); `surfaceResistance` 0–0.0127 with the samples wooden track 0.001,
  smooth concrete 0.002, asphalt road 0.004, rough road 0.008. It stands in for the ANT+ FE profile, which was not
  retrieved (ADR-0002 §4.1).
  https://developer.garmin.com/connect-iq/api-docs/Toybox/AntPlus/SimulationSettings.html
- FortiusANT, `pythoncode/usbTrainer.py` (FE-C defaults Crr 0.004, wind 0.51 kg/m = 1.275 × 0.4 × 1.0; air power
  `0.5 · Kw · (v+w)·|v+w| · d · v`). https://github.com/WouterJD/FortiusANT
- DC Rainmaker, Tacx NEO in-depth review (2015-09-18): 25 % maximum, descent simulation to −5 %.
  https://www.dcrainmaker.com/2015/09/trainer-everything-wanted.html ; trainer recommendations (2019): the NEO drives
  the flywheel on descents when on mains power.
- Geofabrik download server: extracts "normally updated every day", ODbL 1.0, per-region `.osm.pbf` and `.md5`,
  `index-v1-nogeom.json`. https://download.geofabrik.de/ (sizes read from `Content-Length` on 2026-11-05)
- `osmpbf` 0.3.8 (MIT OR Apache-2.0). https://crates.io/crates/osmpbf
- Overpass API commons and usage policy (§2.6.5, the rejected per-route lookup).
  https://dev.overpass-api.de/overpass-doc/en/preface/commons.html
- OpenStreetMap copyright and licence. https://www.openstreetmap.org/copyright ; OSMF attribution guidelines,
  https://osmfoundation.org/wiki/Licence/Attribution_Guidelines ; ODbL 1.0 (§4.4 share-alike on public use), https://opendatacommons.org/licenses/odbl/1-0/
- OSM wiki, `Key:surface`, `Key:smoothness`, `Key:tracktype`. https://wiki.openstreetmap.org/wiki/Key:surface
- gpxpy test files (Apache-2.0), commit 056cdfc34677f85b0185abffb855d2de01520da7: `korita-zbevnica.gpx`,
  `cerknicko-jezero.gpx`. https://github.com/tkrajina/gpxpy/tree/dev/test_files
- `fitparser` 0.12.0 (MIT). https://crates.io/crates/fitparser
- Shimano, FC-R7000 exploded view (EV-FC-R7000-4331A): "Chainring teeth combinations: 50-34T/52-36T/53-39T".
  Mirror read 2026-11-06: https://velosiped.com/pdf/shimano_tech_docs/cranks/ev-fc-r7000-4331a.pdf
- Shimano CS-R7000 cassette: 12-25T, 11-28T, 11-30T and 11-32T, read on 2026-11-06 from a retailer listing of the
  part (Shimano's own product pages refuse scripted fetches with HTTP 403):
  https://www.rosebikes.com/shimano-105-cs-r7000-11-speed-cassette-2668808
- In-repo: ADR-0001 §2, §4.4, §5, §6, §7; ADR-0002 §4.2–§4.4.1, §5, §7.2–§7.9, §9; `spec/TRAINER-BLE.md`;
  `docs/DSL.md`; `docs/hil/part-b.md` (check 8); `docs/safety/FMEA.md`; `docs/adr/models/route_speed.py`,
  `route_grade.py`, `fixtures/gpx-elevation-samples.csv`.
