-- ADR-0005 §2.1 (#541): imported routes, and the route a ride was ridden on.
-- `points` is the versioned little-endian column encoding of crank-store's `route.rs`
-- (`encode_points`); `dsl` is the canonical grade-profile text, present exactly for a grade profile;
-- `original` is the imported file (at most 32 MiB), with its name and SHA-256, present for a file.
CREATE TABLE route (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('gpx_track', 'gpx_route', 'fit_course', 'grade_profile')),
    elevation TEXT NOT NULL CHECK (elevation IN ('recorded', 'none', 'exact')),
    points BLOB NOT NULL,
    segments_json TEXT NOT NULL,
    surface_json TEXT NOT NULL,
    dsl TEXT,
    original_name TEXT,
    original_sha256 TEXT CHECK (original_sha256 IS NULL OR length(original_sha256) = 64),
    original BLOB CHECK (original IS NULL OR length(original) <= 33554432),
    created_at INTEGER NOT NULL,
    CHECK ((source = 'grade_profile') = (dsl IS NOT NULL)),
    CHECK ((original IS NULL) = (original_sha256 IS NULL) AND (original IS NULL) = (original_name IS NULL))
);
-- Deleting a route keeps every ride ridden on it: the link is cleared, `route_json` (what the ride
-- needs to show itself, ADR-0005 §2.7.5) stays. Workout rides leave all three NULL.
ALTER TABLE ride ADD COLUMN route_id TEXT REFERENCES route(id) ON DELETE SET NULL;
ALTER TABLE ride ADD COLUMN speed_source TEXT CHECK (speed_source IN ('model', 'trainer'));
-- A route ride has both `speed_source` and `route_json`; `route_id` only while its route exists.
ALTER TABLE ride ADD COLUMN route_json TEXT
    CHECK ((route_json IS NULL) = (speed_source IS NULL) AND (route_id IS NULL OR route_json IS NOT NULL));
