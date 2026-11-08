# GPX fixtures (#539)

Small synthetic files in the layout of each app's real exports: header, namespaces, metadata
and per-point extensions as the app writes them, with made-up coordinates and values. No
third-party file is copied. `strava-power.gpx` follows a public Strava `StravaGPX` export of a
virtual ride, which writes an unprefixed `<power>` inside `<extensions>` (so in the GPX
namespace); `osmand-old.gpx` follows older OsmAnd, which writes unprefixed `<speed>` and
`<color>` there. `route_r5.rs` imports every `.gpx` here and round-trips it through JSON.
