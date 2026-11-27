-- ADR-0009 §2.5 (#714): the athlete model's history and proposals, and what a ride records for its analysis.
--
-- Proposals first: an estimate names the accepted proposal that wrote it. A proposal belongs to its triggering
-- ride and goes with it (CASCADE, whatever its status, §2.2.4 "When the triggering ride is deleted"); its params
-- are their own rows so "open proposals touching param X" is an index search. `values_json` holds the one or two
-- `{param, value, sd}` (CP and W′ together for P5/P6) in a `{"v":1,"values":[…]}` envelope; the raise rules of
-- §2.2.4 "Never raised twice" are the core's, not a constraint.
CREATE TABLE athlete_proposal (
    id INTEGER PRIMARY KEY,
    ride_id TEXT NOT NULL REFERENCES ride(id) ON DELETE CASCADE,
    method TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    values_json TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    analysis_version INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('open', 'accepted', 'dismissed', 'superseded', 'stale')),
    stale_reason TEXT CHECK (stale_reason IN ('version', 'ride_deleted', 'cp_replaced', 'displaced')),
    created_at INTEGER NOT NULL,
    decided_at INTEGER,
    -- A reason exactly for a stale proposal; a decision time exactly once it left `open`.
    CHECK ((status = 'stale') = (stale_reason IS NOT NULL)),
    CHECK ((status = 'open') = (decided_at IS NULL)),
    UNIQUE (ride_id, method, generation)
);
CREATE INDEX athlete_proposal_open ON athlete_proposal(id) WHERE status = 'open';
CREATE TABLE athlete_proposal_param (
    proposal_id INTEGER NOT NULL REFERENCES athlete_proposal(id) ON DELETE CASCADE,
    param TEXT NOT NULL
        CHECK (param IN ('ftp', 'cp', 'w_prime', 'pmax', 'hr_max', 'lthr', 'hr_rest', 'weight')),
    PRIMARY KEY (proposal_id, param)
);
CREATE INDEX athlete_proposal_param_by_param ON athlete_proposal_param(param, proposal_id);

-- The estimate history, rebuilt (no code has ever written a row, §1.1): a stable rowid alias, the method, the ride
-- and proposal behind a value (both kept as NULL when that ride or proposal is deleted: the value stays), and a row
-- kind. A `cleared` row (crank-domain `Entry::Cleared`, a Clear of a parameter with no prior) has no value, sd,
-- source, method, ride, proposal or count; an `estimate` row has every one but the ride and proposal.
CREATE TABLE athlete_estimate_v5 (
    id INTEGER PRIMARY KEY,
    param TEXT NOT NULL
        CHECK (param IN ('ftp', 'cp', 'w_prime', 'pmax', 'hr_max', 'lthr', 'hr_rest', 'weight')),
    kind TEXT NOT NULL CHECK (kind IN ('estimate', 'cleared')),
    value REAL,
    sd REAL,
    source TEXT CHECK (source IN ('prior', 'self_report', 'test', 'rides', 'imported')),
    method TEXT,
    ride_id TEXT REFERENCES ride(id) ON DELETE SET NULL,
    proposal_id INTEGER REFERENCES athlete_proposal(id) ON DELETE SET NULL,
    n INTEGER CHECK (n >= 0),
    updated_at INTEGER NOT NULL,
    CHECK (CASE kind
        WHEN 'estimate' THEN value IS NOT NULL AND sd IS NOT NULL AND source IS NOT NULL
            AND method IS NOT NULL AND n IS NOT NULL
        ELSE value IS NULL AND sd IS NULL AND source IS NULL AND method IS NULL AND n IS NULL
            AND ride_id IS NULL AND proposal_id IS NULL
    END)
);
-- A legacy row this table cannot hold, or whose numbers would not decode, must never abort the migration (a
-- failed migration fails every launch, #739): it is skipped and kept here verbatim (untyped columns, as stored)
-- with its origin and the first reason that applies: `param` (not one of the eight names), `source` (not one of
-- the five), `number` (value or sd not a number, n not a non-negative integer, updated_at not an integer; or a
-- profile weight that is not a number). Nothing reads or writes this table after 0005; it is the record of what
-- the migration left out (ADR-0009 §2.5).
CREATE TABLE athlete_estimate_skipped (
    id INTEGER PRIMARY KEY,
    from_table TEXT NOT NULL CHECK (from_table IN ('athlete_estimate', 'athlete_profile')),
    from_rowid INTEGER NOT NULL,
    param, value, sd, source, n, updated_at,
    reason TEXT NOT NULL CHECK (reason IN ('param', 'source', 'number')),
    UNIQUE (from_table, from_rowid)
);
INSERT INTO athlete_estimate_skipped (from_table, from_rowid, param, value, sd, source, n, updated_at, reason)
SELECT 'athlete_estimate', legacy_rowid, param, value, sd, source, n, updated_at, reason FROM (
    SELECT rowid AS legacy_rowid, *, CASE
        WHEN param NOT IN ('ftp', 'cp', 'w_prime', 'pmax', 'hr_max', 'lthr', 'hr_rest', 'weight') THEN 'param'
        WHEN source NOT IN ('prior', 'self_report', 'test', 'rides', 'imported') THEN 'source'
        WHEN typeof(value) NOT IN ('integer', 'real') OR typeof(sd) NOT IN ('integer', 'real')
            OR typeof(n) != 'integer' OR n < 0 OR typeof(updated_at) != 'integer' THEN 'number'
    END AS reason
    FROM athlete_estimate
) WHERE reason IS NOT NULL
ORDER BY legacy_rowid;
INSERT INTO athlete_estimate_v5 (param, kind, value, sd, source, method, n, updated_at)
SELECT param, 'estimate', value, sd, source, 'legacy', n, updated_at
FROM athlete_estimate
WHERE rowid NOT IN (SELECT from_rowid FROM athlete_estimate_skipped WHERE from_table = 'athlete_estimate')
ORDER BY updated_at, rowid;
-- The profile's weight becomes the first weight row: typed in (`self_report`, method `profile`), with the
-- self-report sd of a typed value whose origin is unknown (ADR-0009 §2.1.3: the "guess" sd, 20 %), n 1 as for
-- any typed-in value, written now (the profile kept no time). A weight that is not a number is skipped (above).
INSERT INTO athlete_estimate_skipped (from_table, from_rowid, param, value, reason)
SELECT 'athlete_profile', rowid, 'weight', weight, 'number'
FROM athlete_profile WHERE weight IS NOT NULL AND typeof(weight) NOT IN ('integer', 'real');
INSERT INTO athlete_estimate_v5 (param, kind, value, sd, source, method, n, updated_at)
SELECT 'weight', 'estimate', weight, weight * 0.20, 'self_report', 'profile', 1, CAST(strftime('%s', 'now') AS INTEGER)
FROM athlete_profile WHERE typeof(weight) IN ('integer', 'real');
DROP TABLE athlete_estimate;
ALTER TABLE athlete_estimate_v5 RENAME TO athlete_estimate;
CREATE INDEX athlete_estimate_current ON athlete_estimate(param, updated_at DESC);
ALTER TABLE athlete_profile DROP COLUMN weight;

-- What a ride records for its analysis (§2.1.4, §2.3.2): the athlete snapshot it was loaded with (NULL before it
-- existed), its kind (back-filled from what each row holds: a route ride has `route_json`, the core's free ride has
-- workout id `free-ride`, anything else is a workout; #705's RideKind writes it for new rides), how it ended (written
-- by #717; NULL for older rides) and which metrics its record holds (every existing ride is version 1).
-- `json_extract` raises on text that is not JSON, which would abort the migration (#739): only valid JSON is read
-- (a CASE evaluates only the branch it takes; AND promises no order), and a `workout_json` that is not is a workout.
ALTER TABLE ride ADD COLUMN athlete_json TEXT;
ALTER TABLE ride ADD COLUMN kind TEXT CHECK (kind IN ('workout', 'free', 'assessment', 'route'));
UPDATE ride SET kind = CASE
    WHEN route_json IS NOT NULL THEN 'route'
    WHEN CASE WHEN json_valid(workout_json) THEN json_extract(workout_json, '$.id') END = 'free-ride' THEN 'free'
    ELSE 'workout'
END;
ALTER TABLE ride ADD COLUMN end_cause TEXT
    CHECK (end_cause IN ('completed', 'rider_end', 'safety', 'forced', 'recovered'));
ALTER TABLE ride ADD COLUMN record_version INTEGER NOT NULL DEFAULT 1;

-- "Unwell today" (DESIGN §5.8), written by the daily check-in (#94); a day is unwell if any of its rows says so.
ALTER TABLE checkin ADD COLUMN unwell INTEGER CHECK (unwell IN (0, 1));
