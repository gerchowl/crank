-- ADR-0001 §6. Applied migrations are immutable; add a new numbered file.
CREATE TABLE athlete_estimate (
    param TEXT NOT NULL, value REAL NOT NULL, sd REAL NOT NULL,
    source TEXT NOT NULL, n INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX athlete_estimate_current ON athlete_estimate(param, updated_at DESC);
CREATE TABLE athlete_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1), weight REAL, age INTEGER,
    sex TEXT, crank_length REAL, par_q_result TEXT
);
CREATE TABLE workout (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, dsl TEXT NOT NULL,
    tree_json TEXT NOT NULL, ftp_at_creation REAL, source TEXT, created_at INTEGER NOT NULL
);
CREATE TABLE snippet (id TEXT PRIMARY KEY, name TEXT NOT NULL, json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE layout (id TEXT PRIMARY KEY, name TEXT NOT NULL, json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE device (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT, platform_id TEXT NOT NULL,
    serial TEXT, last_seen INTEGER NOT NULL
);
CREATE TABLE ride (
    id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, ended_at INTEGER,
    state TEXT CHECK (state IN ('finished', 'aborted')),
    workout_json TEXT NOT NULL, compiled_json TEXT NOT NULL,
    trainer_device_id TEXT REFERENCES device(id), hr_device_id TEXT REFERENCES device(id),
    telemetry_source TEXT NOT NULL, summary_json TEXT,
    CHECK ((ended_at IS NULL AND state IS NULL) OR (ended_at IS NOT NULL AND state IS NOT NULL))
);
CREATE TABLE ride_chunk (
    ride_id TEXT NOT NULL REFERENCES ride(id), metric TEXT NOT NULL,
    start_s INTEGER NOT NULL CHECK (start_s >= 0 AND start_s % 60 = 0),
    n INTEGER NOT NULL CHECK (n BETWEEN 1 AND 60),
    data BLOB NOT NULL CHECK (length(data) = 4 * n),
    estimated BLOB NOT NULL CHECK (length(estimated) = (n + 7) / 8),
    PRIMARY KEY (ride_id, metric, start_s)
);
CREATE TABLE ride_rr (
    ride_id TEXT NOT NULL REFERENCES ride(id), start_ms INTEGER NOT NULL,
    data BLOB NOT NULL CHECK (length(data) % 2 = 0), PRIMARY KEY (ride_id, start_ms)
);
CREATE TABLE lap (
    ride_id TEXT NOT NULL REFERENCES ride(id), idx INTEGER NOT NULL,
    node_id TEXT NOT NULL, path TEXT NOT NULL, start_s REAL NOT NULL, end_s REAL NOT NULL,
    workout_start_s REAL NOT NULL, workout_end_s REAL NOT NULL, stats_json TEXT NOT NULL,
    PRIMARY KEY (ride_id, idx)
);
CREATE TABLE recovery (
    ride_id TEXT NOT NULL REFERENCES ride(id), node_id TEXT NOT NULL, iteration INTEGER NOT NULL,
    fit_json TEXT NOT NULL, tau_s REAL, observed BLOB NOT NULL, PRIMARY KEY (ride_id, node_id, iteration)
);
CREATE TABLE coach_decision (
    call_id TEXT PRIMARY KEY, ride_id TEXT NOT NULL REFERENCES ride(id), t REAL NOT NULL,
    verdict TEXT NOT NULL, band TEXT NOT NULL, p REAL, message TEXT, proposed_pct REAL,
    status TEXT, rating INTEGER, useful INTEGER, features_json TEXT NOT NULL
);
CREATE TABLE checkin (
    id TEXT PRIMARY KEY, at INTEGER NOT NULL, sleep REAL, legs REAL, stress REAL,
    motivation REAL, minutes INTEGER, note TEXT, proposal_workout_id TEXT REFERENCES workout(id)
);
CREATE TABLE plan_item (date TEXT PRIMARY KEY, workout_id TEXT REFERENCES workout(id), status TEXT, rationale TEXT);
CREATE TABLE setting (key TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE import_activity (
    source TEXT NOT NULL, external_id TEXT NOT NULL, ride_id TEXT NOT NULL REFERENCES ride(id),
    imported_at INTEGER NOT NULL, PRIMARY KEY (source, external_id)
);
