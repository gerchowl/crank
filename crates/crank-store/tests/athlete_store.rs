//! ADR-0009 §2.8 "Store" (#714): migration 0005 on a v4 database with rows, the estimate history appended only
//! through `EstimateOrigin` and checked by the domain, cleared rows, proposals and their ON DELETE behaviour.
use crank_domain::athlete::{
    AthleteError, AthleteParam, Entry, METHOD_MANUAL, METHOD_PRIOR, Source, prior,
};
use crank_store::*;
use rusqlite::{Connection, params};
use std::path::Path;

const WORKOUT_JSON: &str = r#"{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[]}"#;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("test step");
    let store = Store::open_owner(dir.path()).expect("test step");
    (dir, store)
}

/// A finished ride with no chunks (so a raw `DELETE FROM ride` is allowed by the chunk tables' plain foreign keys).
fn ride(store: &Store, id: &str, started_at: i64) {
    store
        .start_ride(NewRide {
            id: id.into(),
            started_at,
            workout: RideWorkoutJson::parse(WORKOUT_JSON).expect("test step"),
            compiled: CompiledJson::parse(r#"{"v":1}"#).expect("test step"),
            trainer_device_id: None,
            hr_device_id: None,
            telemetry_source: "sim".into(),
            metrics: vec!["power".into()],
        })
        .expect("test step");
    store
        .finish_ride(
            id,
            started_at + 60,
            RideSummaryJson::parse(r#"{"v":1}"#).expect("test step"),
        )
        .expect("test step");
}

fn raw(dir: &Path) -> Connection {
    let db = Connection::open(dir.join("crank.db")).expect("test step");
    // As the store's writer does: the ON DELETE actions run only with foreign keys on.
    db.pragma_update(None, "foreign_keys", true)
        .expect("test step");
    db
}

fn value(param: AthleteParam, value: f64, sd: f64) -> ProposedValue {
    ProposedValue { param, value, sd }
}

fn proposal(ride_id: &str, method: &str, values: Vec<ProposedValue>) -> NewProposal {
    NewProposal {
        ride_id: ride_id.into(),
        method: method.into(),
        generation: 1,
        values,
        evidence: ProposalEvidenceJson::parse(r#"{"v":1,"rides":["r"]}"#).expect("test step"),
        analysis_version: 1,
        created_at: 1_000,
    }
}

fn cp_pair() -> Vec<ProposedValue> {
    vec![
        value(AthleteParam::Cp, 270., 6.),
        value(AthleteParam::WPrime, 18_000., 1_500.),
    ]
}

fn estimate(entry: &Entry) -> &crank_domain::athlete::Estimate {
    match entry {
        Entry::Estimate(e) => e,
        Entry::Cleared(c) => panic!("expected an estimate, got {c:?}"),
    }
}

#[test]
fn migration_five_carries_a_v4_database_over() {
    let dir = tempfile::tempdir().expect("test step");
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("fixtures/v4.db"),
    )
    .expect("test step");
    let store = Store::open_owner(dir.path()).expect("test step");
    assert_eq!(store.schema_version().expect("test step"), 5);

    // Estimates kept, as method `legacy`; the profile's weight is the first weight row.
    let ftp = store.athlete_history(AthleteParam::Ftp).expect("test step");
    assert_eq!(ftp.len(), 1);
    let ftp = estimate(&ftp[0]);
    assert_eq!(
        (
            ftp.value,
            ftp.sd,
            ftp.source,
            ftp.method.as_str(),
            ftp.n,
            ftp.updated_at
        ),
        (
            260.,
            20.,
            Source::SelfReport,
            METHOD_LEGACY,
            1,
            1_700_000_000
        )
    );
    assert_eq!((ftp.ride_id.as_deref(), ftp.proposal_id), (None, None));
    let hr_max = store
        .athlete_history(AthleteParam::HrMax)
        .expect("test step");
    let hr_max = estimate(&hr_max[0]);
    assert_eq!(
        (hr_max.value, hr_max.source, hr_max.method.as_str()),
        (185., Source::Test, METHOD_LEGACY)
    );
    let weight = store
        .athlete_history(AthleteParam::Weight)
        .expect("test step");
    assert_eq!(weight.len(), 1);
    let weight = estimate(&weight[0]);
    assert_eq!(
        (
            weight.value,
            weight.sd,
            weight.source,
            weight.method.as_str(),
            weight.n
        ),
        (72.5, 72.5 * 0.20, Source::SelfReport, METHOD_PROFILE, 1)
    );
    assert!(
        weight.updated_at > 1_700_000_000,
        "written at migration time"
    );
    let snapshot = store.athlete_snapshot().expect("test step");
    assert_eq!(snapshot.weight.map(|w| w.value), Some(72.5));
    assert_eq!(snapshot.ftp.map(|r| r.estimate_id), Some(Some(ftp.id)));
    drop(store);
    migrated_profile_ride_and_checkin_columns(dir.path());
}

type RideColumns = (String, String, i64, Option<String>, Option<String>);

/// The rest of migration 0005 on the v4 fixture: the profile, the ride columns and the check-in column.
fn migrated_profile_ride_and_checkin_columns(dir: &Path) {
    let db = raw(dir);
    assert!(
        db.prepare("SELECT weight FROM athlete_profile").is_err(),
        "athlete_profile.weight is dropped"
    );
    assert_eq!(
        db.query_row("SELECT age FROM athlete_profile WHERE id = 1", [], |r| r
            .get::<_, i64>(0))
            .expect("test step"),
        40
    );
    // Kinds back-filled from what each ride holds; every older ride is record version 1 with no snapshot or end
    // cause.
    let rides: Vec<RideColumns> = db
        .prepare("SELECT id,kind,record_version,athlete_json,end_cause FROM ride ORDER BY id")
        .expect("test step")
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("test step")
        .collect::<Result<_, _>>()
        .expect("test step");
    assert_eq!(
        rides,
        [
            ("free-ride-1".into(), "free".into(), 1, None, None),
            ("route-ride".into(), "route".into(), 1, None, None),
            ("workout-ride".into(), "workout".into(), 1, None, None),
        ]
    );
    let unwell: Option<i64> = db
        .query_row("SELECT unwell FROM checkin WHERE id = 'c1'", [], |r| {
            r.get(0)
        })
        .expect("test step");
    assert_eq!(unwell, None);
    assert!(
        db.execute("UPDATE checkin SET unwell = 2", []).is_err(),
        "unwell is 0 or 1"
    );
    assert!(db.execute("UPDATE ride SET kind = 'spin'", []).is_err());
    assert!(
        db.execute("UPDATE ride SET end_cause = 'bored'", [])
            .is_err()
    );
    let violations = db
        .prepare("PRAGMA foreign_key_check")
        .expect("test step")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("test step")
        .count();
    assert_eq!(violations, 0);
}

/// The v4 fixture copied into a fresh data directory, with `extra` run on it first (raw, as an older build or a
/// hand edit could have left it).
fn v4_with(extra: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("test step");
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("fixtures/v4.db"),
    )
    .expect("test step");
    Connection::open(dir.path().join("crank.db"))
        .expect("test step")
        .execute_batch(extra)
        .expect("test step");
    dir
}

type SkippedRow = (String, String, Option<String>, String);

/// #739 item 1: a v4 ride whose `workout_json` is not JSON never aborts migration 0005 (`json_extract` raises
/// on it): the ride is a `workout`.
#[test]
fn migration_five_reads_a_malformed_workout_json_as_a_workout() {
    let dir = v4_with(
        "INSERT INTO ride (id, started_at, ended_at, state, workout_json, compiled_json, telemetry_source)
             VALUES ('broken-ride', 1700000500, 1700000560, 'finished', '{not json', '{}', 'sim');",
    );
    let store = Store::open_owner(dir.path()).expect("migration 0005 opens the store");
    assert_eq!(store.schema_version().expect("test step"), 5);
    drop(store);
    let kind: String = raw(dir.path())
        .query_row("SELECT kind FROM ride WHERE id = 'broken-ride'", [], |r| {
            r.get(0)
        })
        .expect("test step");
    assert_eq!(kind, "workout");
}

/// #739 item 1: legacy estimate rows outside 0005's lists or types never abort migration 0005: they are kept
/// verbatim in `athlete_estimate_skipped` with their reason, and every other row is carried over as before.
#[test]
fn migration_five_skips_legacy_estimates_it_cannot_hold() {
    let dir = v4_with(
        "INSERT INTO athlete_estimate (param, value, sd, source, n, updated_at) VALUES
             ('vo2max', 55, 3, 'test', 1, 1700000100),
             ('ftp', 250, 10, 'guess', 1, 1700000200),
             ('ftp', 255, 10, 'test', -1, 1700000300),
             ('lthr', 'high', 5, 'test', 1, 1700000400);",
    );
    let store = Store::open_owner(dir.path()).expect("migration 0005 opens the store");
    assert_eq!(store.schema_version().expect("test step"), 5);
    let ftp = store.athlete_history(AthleteParam::Ftp).expect("test step");
    assert_eq!(
        ftp.len(),
        1,
        "only the fixture's in-list FTP row is carried over"
    );
    assert_eq!(
        estimate(&ftp[0]).updated_at,
        1_700_000_000,
        "the fixture's FTP row"
    );
    assert!(
        store
            .athlete_history(AthleteParam::Lthr)
            .expect("test step")
            .is_empty()
    );
    store
        .athlete_snapshot()
        .expect("every carried-over row decodes");
    drop(store);
    let db = raw(dir.path());
    let skipped: Vec<SkippedRow> = db
        .prepare(
            "SELECT from_table, param, source, reason FROM athlete_estimate_skipped ORDER BY from_rowid",
        )
        .expect("test step")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("test step")
        .collect::<Result<_, _>>()
        .expect("test step");
    let row = |param: &str, source: &str, reason: &str| {
        (
            "athlete_estimate".to_owned(),
            param.to_owned(),
            Some(source.to_owned()),
            reason.to_owned(),
        )
    };
    assert_eq!(
        skipped,
        [
            row("vo2max", "test", "param"),
            row("ftp", "guess", "source"),
            row("ftp", "test", "number"),
            row("lthr", "test", "number"),
        ]
    );
    // Verbatim: the value that did not fit is kept as it was stored.
    let lthr: String = db
        .query_row(
            "SELECT value FROM athlete_estimate_skipped WHERE param = 'lthr'",
            [],
            |r| r.get(0),
        )
        .expect("test step");
    assert_eq!(lthr, "high");
}

/// #739 item 1: a profile weight that is not a number is skipped (kept in `athlete_estimate_skipped`), never
/// copied into a row that would not decode.
#[test]
fn migration_five_skips_a_profile_weight_that_is_not_a_number() {
    let dir = v4_with("UPDATE athlete_profile SET weight = 'seventy' WHERE id = 1;");
    let store = Store::open_owner(dir.path()).expect("migration 0005 opens the store");
    assert!(
        store
            .athlete_history(AthleteParam::Weight)
            .expect("test step")
            .is_empty()
    );
    store.athlete_snapshot().expect("test step");
    drop(store);
    let skipped: (String, String, String, String) = raw(dir.path())
        .query_row(
            "SELECT from_table, param, value, reason FROM athlete_estimate_skipped",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("test step");
    assert_eq!(
        skipped,
        (
            "athlete_profile".into(),
            "weight".into(),
            "seventy".into(),
            "number".into()
        )
    );
}

/// Every table's rows (as debug text) and the schema, for comparing a database before and after.
fn dump(db: &Connection) -> Vec<String> {
    let mut out = vec![format!(
        "user_version {}",
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .expect("test step")
    )];
    let tables: Vec<(String, Option<String>)> = db
        .prepare("SELECT name, sql FROM sqlite_master ORDER BY type, name")
        .expect("test step")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("test step")
        .collect::<Result<_, _>>()
        .expect("test step");
    for (name, sql) in tables {
        out.push(format!("{name}: {sql:?}"));
        let Ok(mut rows) = db.prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid")) else {
            continue; // an index
        };
        let width = rows.column_count();
        let rows: Vec<String> = rows
            .query_map([], |r| {
                (0..width)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
                    .map(|v| format!("{name} {v:?}"))
            })
            .expect("test step")
            .collect::<Result<_, _>>()
            .expect("test step");
        out.extend(rows);
    }
    out
}

/// #739 item 2: a migration 0005 that fails at its last statement (here a v4 database that already has
/// `checkin.unwell`) leaves the database exactly as it was: the schema, every row and `user_version` 4.
#[test]
fn a_failed_migration_five_leaves_the_v4_database_unchanged() {
    let dir = v4_with("ALTER TABLE checkin ADD COLUMN unwell INTEGER;");
    let before = dump(&raw(dir.path()));
    assert!(before.contains(&"user_version 4".to_owned()));
    let failed = Store::open_owner(dir.path());
    assert!(
        matches!(failed, Err(StoreError::Migration(_))),
        "{:?}",
        failed.err()
    );
    assert_eq!(dump(&raw(dir.path())), before);
}

#[test]
fn each_origin_appends_its_row_and_the_newest_row_is_current() {
    let (_dir, store) = store();
    let set = store
        .append_athlete(
            EstimateOrigin::Set {
                param: AthleteParam::Ftp,
                value: 280.,
                sd: 20.,
            },
            100,
        )
        .expect("test step");
    let set_row = estimate(&set[0]).clone();
    assert_eq!(
        (
            set_row.source,
            set_row.method.as_str(),
            set_row.n,
            set_row.updated_at
        ),
        (Source::SelfReport, METHOD_MANUAL, 1, 100)
    );
    store
        .append_athlete(
            EstimateOrigin::Clear {
                param: AthleteParam::Ftp,
            },
            200,
        )
        .expect("test step");
    // Clear of FTP is its prior's row.
    let current = store
        .athlete_snapshot()
        .expect("test step")
        .ftp
        .expect("test step");
    assert_eq!(
        (current.value, current.source),
        (prior::FTP_W, Source::Prior)
    );
    // Revert copies the old row with source and method kept and a new time; reverting to the prior is refused.
    let reverted = store
        .append_athlete(
            EstimateOrigin::Revert {
                estimate_id: set_row.id,
            },
            300,
        )
        .expect("test step");
    let r = estimate(&reverted[0]);
    assert_ne!(r.id, set_row.id);
    assert_eq!(
        (r.value, r.sd, r.source, r.method.as_str(), r.updated_at),
        (280., 20., Source::SelfReport, METHOD_MANUAL, 300)
    );
    let prior_id = store.athlete_history(AthleteParam::Ftp).expect("test step")[1].id();
    assert!(matches!(
        store.append_athlete(
            EstimateOrigin::Revert {
                estimate_id: prior_id
            },
            400
        ),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store.append_athlete(EstimateOrigin::Revert { estimate_id: 999 }, 400),
        Err(StoreError::NotFound(_))
    ));
    // History newest first; the newest row of each parameter is current.
    let history: Vec<i64> = store
        .athlete_history(AthleteParam::Ftp)
        .expect("test step")
        .iter()
        .map(Entry::updated_at)
        .collect();
    assert_eq!(history, [300, 200, 100]);
    let current: Vec<AthleteParam> = store
        .athlete_current()
        .expect("test step")
        .iter()
        .map(Entry::param)
        .collect();
    assert_eq!(current, [AthleteParam::Ftp]);
    assert_eq!(
        store
            .athlete_snapshot()
            .expect("test step")
            .ftp
            .map(|r| r.value),
        Some(280.)
    );
    assert_eq!(
        store.athlete_entry(r.id).expect("test step").as_ref(),
        Some(&reverted[0])
    );
}

#[test]
fn onboarding_and_import_append_their_sources() {
    let (_dir, store) = store();
    // Onboarding and import.
    let p = store
        .append_athlete(
            EstimateOrigin::OnboardingPrior {
                param: AthleteParam::WPrime,
            },
            400,
        )
        .expect("test step");
    assert_eq!(
        (estimate(&p[0]).source, estimate(&p[0]).method.as_str()),
        (Source::Prior, METHOD_PRIOR)
    );
    assert!(matches!(
        store.append_athlete(
            EstimateOrigin::OnboardingPrior {
                param: AthleteParam::Weight
            },
            400
        ),
        Err(StoreError::InvalidInput(_))
    ));
    store
        .append_athlete(
            EstimateOrigin::OnboardingSelfReport {
                param: AthleteParam::Weight,
                value: 70.,
                sd: 7.,
            },
            400,
        )
        .expect("test step");
    let imported = store
        .append_athlete(
            EstimateOrigin::Import {
                param: AthleteParam::HrRest,
                value: 48.,
                sd: 3.,
                method: "intervals_icu".into(),
                n: 30,
            },
            400,
        )
        .expect("test step");
    assert_eq!(
        (estimate(&imported[0]).source, estimate(&imported[0]).n),
        (Source::Imported, 30)
    );
    let current: Vec<AthleteParam> = store
        .athlete_current()
        .expect("test step")
        .iter()
        .map(Entry::param)
        .collect();
    assert_eq!(
        current,
        [
            AthleteParam::WPrime,
            AthleteParam::HrRest,
            AthleteParam::Weight
        ]
    );
}

#[test]
fn a_row_written_with_a_clock_that_went_back_is_still_current() {
    let (_dir, store) = store();
    for (v, now) in [(250., 1_000), (260., 900)] {
        store
            .append_athlete(
                EstimateOrigin::Set {
                    param: AthleteParam::Ftp,
                    value: v,
                    sd: 20.,
                },
                now,
            )
            .expect("test step");
    }
    let ftp = store
        .athlete_snapshot()
        .expect("test step")
        .ftp
        .expect("test step");
    assert_eq!(Some(ftp.value), Some(260.));
    assert_eq!(
        store.athlete_history(AthleteParam::Ftp).expect("test step")[0].updated_at(),
        1_000
    );
}

#[test]
fn the_store_refuses_every_row_the_domain_refuses() {
    let (_dir, store) = store();
    let set = |param, value, sd| store.append_athlete(EstimateOrigin::Set { param, value, sd }, 10);
    assert!(matches!(
        set(AthleteParam::Weight, 0., 1.),
        Err(StoreError::Athlete(AthleteError::OutOfRange { .. }))
    ));
    assert!(matches!(
        set(AthleteParam::Ftp, f64::NAN, 1.),
        Err(StoreError::Athlete(AthleteError::NonFinite { .. }))
    ));
    assert!(matches!(
        set(AthleteParam::Ftp, 250., 200.),
        Err(StoreError::Athlete(AthleteError::SdOutOfRange { .. }))
    ));
    // The cross check runs against the stored model.
    set(AthleteParam::HrMax, 180., 5.).expect("test step");
    assert!(matches!(
        set(AthleteParam::Lthr, 185., 5.),
        Err(StoreError::Athlete(AthleteError::LthrNotBelowHrMax { .. }))
    ));
    // An import without a method.
    assert!(matches!(
        store.append_athlete(
            EstimateOrigin::Import {
                param: AthleteParam::Ftp,
                value: 250.,
                sd: 20.,
                method: String::new(),
                n: 1,
            },
            10,
        ),
        Err(StoreError::InvalidInput(_))
    ));
    // Nothing refused was written.
    assert_eq!(store.athlete_current().expect("test step").len(), 1);
}

#[test]
fn cleared_rows_round_trip_and_are_not_reverted_to() {
    let (dir, store) = store();
    store
        .append_athlete(
            EstimateOrigin::Set {
                param: AthleteParam::Lthr,
                value: 170.,
                sd: 5.,
            },
            10,
        )
        .expect("test step");
    let cleared = store
        .append_athlete(
            EstimateOrigin::Clear {
                param: AthleteParam::Lthr,
            },
            20,
        )
        .expect("test step");
    let Entry::Cleared(c) = cleared[0] else {
        panic!("LTHR has no prior: Clear appends a cleared row, got {cleared:?}");
    };
    assert_eq!((c.param, c.updated_at), (AthleteParam::Lthr, 20));
    assert_eq!(
        store.athlete_entry(c.id).expect("test step"),
        Some(Entry::Cleared(c))
    );
    assert_eq!(
        store
            .athlete_history(AthleteParam::Lthr)
            .expect("test step")[0],
        Entry::Cleared(c)
    );
    assert_eq!(
        store.athlete_snapshot().expect("test step").lthr,
        None,
        "cleared is unset"
    );
    assert!(matches!(
        store.append_athlete(EstimateOrigin::Revert { estimate_id: c.id }, 30),
        Err(StoreError::InvalidInput(_))
    ));
    // The stored row has its value columns NULL, and the table refuses a cleared row with a value.
    let db = raw(dir.path());
    let nulls: (Option<f64>, Option<String>, Option<i64>, String) = db
        .query_row(
            "SELECT value, method, n, kind FROM athlete_estimate WHERE id = ?1",
            [c.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("test step");
    assert_eq!(nulls, (None, None, None, "cleared".into()));
    assert!(
        db.execute(
            "INSERT INTO athlete_estimate (param,kind,value,updated_at) VALUES ('lthr','cleared',170,1)",
            [],
        )
        .is_err()
    );
    assert!(
        db.execute(
            "INSERT INTO athlete_estimate (param,kind,value,sd,source,updated_at) \
             VALUES ('lthr','estimate',170,5,'self_report',1)",
            [],
        )
        .is_err(),
        "an estimate row needs its method and n"
    );
    // HR max without an age has no prior: cleared; once the age is known its prior resolves.
    store
        .append_athlete(
            EstimateOrigin::Clear {
                param: AthleteParam::HrMax,
            },
            40,
        )
        .expect("test step");
    assert_eq!(store.athlete_snapshot().expect("test step").hr_max, None);
    db.execute("INSERT INTO athlete_profile (id, age) VALUES (1, 40)", [])
        .expect("test step");
    let hr_max = store
        .athlete_snapshot()
        .expect("test step")
        .hr_max
        .expect("test step");
    assert_eq!((hr_max.value, hr_max.estimate_id), (208. - 28., None));
}

#[test]
fn manual_entries_supersede_open_proposals_of_their_parameter() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let hr = store
        .insert_proposal(proposal(
            "r",
            "peak_hr_v1",
            vec![value(AthleteParam::HrMax, 190., 2.)],
        ))
        .expect("test step");
    let cp = store
        .insert_proposal(proposal(
            "r",
            "cp_ols_v1",
            vec![
                value(AthleteParam::Cp, 270., 6.),
                value(AthleteParam::WPrime, 18_000., 1_500.),
            ],
        ))
        .expect("test step");
    assert_eq!(store.open_proposals().expect("test step").len(), 2);
    // A manual W′ supersedes the CP/W′ pair, not the HR max proposal.
    store
        .append_athlete(
            EstimateOrigin::Set {
                param: AthleteParam::WPrime,
                value: 21_000.,
                sd: 2_000.,
            },
            2_000,
        )
        .expect("test step");
    let pair = store.proposal(cp).expect("test step").expect("test step");
    assert_eq!(
        (pair.status, pair.decided_at),
        (ProposalStatus::Superseded, Some(2_000))
    );
    assert!(
        store
            .open_proposals_for(AthleteParam::Cp)
            .expect("test step")
            .is_empty()
    );
    let open: Vec<i64> = store
        .open_proposals()
        .expect("test step")
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(open, [hr]);
}

#[test]
fn clear_onboarding_import_and_revert_supersede_too() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let hr = store
        .insert_proposal(proposal(
            "r",
            "peak_hr_v1",
            vec![value(AthleteParam::HrMax, 190., 2.)],
        ))
        .expect("test step");
    let mut generation = 1;
    let mut supersedes = |origin: EstimateOrigin| {
        if generation > 1 {
            store
                .insert_proposal(NewProposal {
                    generation,
                    ..proposal(
                        "r",
                        "peak_hr_v1",
                        vec![value(AthleteParam::HrMax, 195., 2.)],
                    )
                })
                .expect("test step");
        }
        generation += 1;
        assert_eq!(
            store
                .open_proposals_for(AthleteParam::HrMax)
                .expect("test step")
                .len(),
            1
        );
        store.append_athlete(origin, 3_000).expect("test step");
        assert!(
            store
                .open_proposals_for(AthleteParam::HrMax)
                .expect("test step")
                .is_empty()
        );
    };
    supersedes(EstimateOrigin::Clear {
        param: AthleteParam::HrMax,
    });
    supersedes(EstimateOrigin::OnboardingSelfReport {
        param: AthleteParam::HrMax,
        value: 188.,
        sd: 5.,
    });
    supersedes(EstimateOrigin::Import {
        param: AthleteParam::HrMax,
        value: 187.,
        sd: 4.,
        method: "intervals_icu".into(),
        n: 1,
    });
    let onboarded = store
        .athlete_history(AthleteParam::HrMax)
        .expect("test step")[1]
        .id();
    supersedes(EstimateOrigin::Revert {
        estimate_id: onboarded,
    });
    assert_eq!(
        store
            .proposal(hr)
            .expect("test step")
            .expect("test step")
            .status,
        ProposalStatus::Superseded
    );
}

#[test]
fn a_refused_accept_writes_nothing_and_leaves_the_proposal_open() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let pair = store
        .insert_proposal(proposal(
            "r",
            "cp_ols_v1",
            vec![
                value(AthleteParam::Cp, 270., 6.),
                value(AthleteParam::WPrime, 18_000., 1_500.),
            ],
        ))
        .expect("test step");
    // Values that do not match the proposal, or a source no proposal has, are refused and write nothing.
    for (source, values) in [
        (Source::Rides, vec![value(AthleteParam::Cp, 270., 6.)]),
        (
            Source::SelfReport,
            vec![
                value(AthleteParam::Cp, 270., 6.),
                value(AthleteParam::WPrime, 18_000., 1_500.),
            ],
        ),
    ] {
        assert!(matches!(
            store.append_athlete(
                EstimateOrigin::ProposalAccept {
                    proposal_id: pair,
                    source,
                    values,
                    n: 4
                },
                2_000,
            ),
            Err(StoreError::InvalidInput(_))
        ));
    }
    // A value the domain refuses fails the whole accept: no row, still open.
    assert!(matches!(
        store.append_athlete(
            EstimateOrigin::ProposalAccept {
                proposal_id: pair,
                source: Source::Rides,
                values: vec![
                    value(AthleteParam::Cp, 270., 6.),
                    value(AthleteParam::WPrime, 90_000., 1_500.)
                ],
                n: 4,
            },
            2_000,
        ),
        Err(StoreError::Athlete(AthleteError::OutOfRange { .. }))
    ));
    assert!(store.athlete_current().expect("test step").is_empty());
    assert_eq!(
        store
            .proposal(pair)
            .expect("test step")
            .expect("test step")
            .status,
        ProposalStatus::Open
    );
}

#[test]
fn an_accepted_proposal_writes_one_row_per_value_and_is_closed() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let pair = store
        .insert_proposal(proposal("r", "cp_ols_v1", cp_pair()))
        .expect("test step");
    let written = store
        .append_athlete(
            EstimateOrigin::ProposalAccept {
                proposal_id: pair,
                source: Source::Rides,
                values: vec![
                    value(AthleteParam::Cp, 271., 6.),
                    value(AthleteParam::WPrime, 18_100., 1_500.),
                ],
                n: 4,
            },
            2_000,
        )
        .expect("test step");
    assert_eq!(written.len(), 2);
    for (entry, (param, v)) in written
        .iter()
        .zip([(AthleteParam::Cp, 271.), (AthleteParam::WPrime, 18_100.)])
    {
        let e = estimate(entry);
        assert_eq!(
            (
                e.param,
                e.value,
                e.source,
                e.method.as_str(),
                e.ride_id.as_deref(),
                e.proposal_id,
                e.n
            ),
            (
                param,
                v,
                Source::Rides,
                "cp_ols_v1",
                Some("r"),
                Some(pair),
                4
            )
        );
    }
    let accepted = store.proposal(pair).expect("test step").expect("test step");
    assert_eq!(
        (accepted.status, accepted.decided_at),
        (ProposalStatus::Accepted, Some(2_000))
    );
    // Accept refuses a proposal that is not open; an unknown one is not found.
    let again = EstimateOrigin::ProposalAccept {
        proposal_id: pair,
        source: Source::Rides,
        values: vec![
            value(AthleteParam::Cp, 271., 6.),
            value(AthleteParam::WPrime, 18_100., 1_500.),
        ],
        n: 4,
    };
    assert!(matches!(
        store.append_athlete(again, 3_000),
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.append_athlete(
            EstimateOrigin::ProposalAccept {
                proposal_id: 99,
                source: Source::Rides,
                values: vec![],
                n: 1
            },
            3_000,
        ),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn proposals_are_inserted_keyed_and_checked() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let new = proposal(
        "r",
        "peak_hr_v1",
        vec![value(AthleteParam::HrMax, 190., 2.)],
    );
    let id = store.insert_proposal(new.clone()).expect("test step");
    let stored = store.proposal(id).expect("test step").expect("test step");
    assert_eq!(
        (
            stored.ride_id.as_str(),
            stored.method.as_str(),
            stored.generation,
            stored.values.clone(),
            stored.evidence.as_str(),
            stored.analysis_version,
            stored.status,
            stored.created_at,
            stored.decided_at,
        ),
        (
            "r",
            "peak_hr_v1",
            1,
            new.values.clone(),
            new.evidence.as_str(),
            1,
            ProposalStatus::Open,
            1_000,
            None
        )
    );
    // Keyed (ride, method, generation); the next generation is a new row.
    assert!(matches!(
        store.insert_proposal(new.clone()),
        Err(StoreError::Conflict(_))
    ));
    for bad in [
        NewProposal {
            generation: 0,
            ..new.clone()
        },
        NewProposal {
            method: String::new(),
            ..new.clone()
        },
        NewProposal {
            values: vec![],
            ..new.clone()
        },
        NewProposal {
            values: vec![
                value(AthleteParam::HrMax, 190., 2.),
                value(AthleteParam::HrMax, 191., 2.),
            ],
            ..new.clone()
        },
    ] {
        assert!(matches!(
            store.insert_proposal(bad),
            Err(StoreError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        store.insert_proposal(NewProposal {
            values: vec![value(AthleteParam::HrMax, 300., 2.)],
            ..new.clone()
        }),
        Err(StoreError::Athlete(AthleteError::OutOfRange { .. }))
    ));
    assert!(matches!(
        store.insert_proposal(NewProposal {
            ride_id: "nope".into(),
            ..new.clone()
        }),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn a_proposal_leaves_open_once() {
    let (_dir, store) = store();
    ride(&store, "r", 1_000);
    let new = proposal(
        "r",
        "peak_hr_v1",
        vec![value(AthleteParam::HrMax, 190., 2.)],
    );
    let id = store.insert_proposal(new.clone()).expect("test step");
    store
        .close_proposal(id, ProposalClose::Stale(StaleReason::RideDeleted), 2_000)
        .expect("test step");
    let stale = store.proposal(id).expect("test step").expect("test step");
    assert_eq!(
        (stale.status, stale.decided_at),
        (ProposalStatus::Stale(StaleReason::RideDeleted), Some(2_000))
    );
    assert!(matches!(
        store.close_proposal(id, ProposalClose::Dismissed, 3_000),
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.close_proposal(99, ProposalClose::Dismissed, 3_000),
        Err(StoreError::NotFound(_))
    ));
    let next = store
        .insert_proposal(NewProposal {
            generation: 2,
            ..new
        })
        .expect("test step");
    for (close, status) in [
        (ProposalClose::Dismissed, ProposalStatus::Dismissed),
        (ProposalClose::Superseded, ProposalStatus::Superseded),
    ] {
        let p = store
            .insert_proposal(NewProposal {
                generation: 3 + u32::from(close == ProposalClose::Superseded),
                ..proposal(
                    "r",
                    "peak_hr_v1",
                    vec![value(AthleteParam::HrMax, 190., 2.)],
                )
            })
            .expect("test step");
        store.close_proposal(p, close, 4_000).expect("test step");
        assert_eq!(
            store
                .proposal(p)
                .expect("test step")
                .expect("test step")
                .status,
            status
        );
    }
    let open: Vec<i64> = store
        .open_proposals_for(AthleteParam::HrMax)
        .expect("test step")
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(open, [next]);
    assert!(
        store
            .open_proposals_for(AthleteParam::Ftp)
            .expect("test step")
            .is_empty()
    );
}

#[test]
fn deleting_a_ride_keeps_its_estimates_and_deletes_its_proposals() {
    let (dir, store) = store();
    ride(&store, "r", 1_000);
    ride(&store, "other", 2_000);
    let accepted = store
        .insert_proposal(proposal(
            "r",
            "peak_hr_v1",
            vec![value(AthleteParam::HrMax, 190., 2.)],
        ))
        .expect("test step");
    let written = store
        .append_athlete(
            EstimateOrigin::ProposalAccept {
                proposal_id: accepted,
                source: Source::Rides,
                values: vec![value(AthleteParam::HrMax, 190., 2.)],
                n: 1,
            },
            3_000,
        )
        .expect("test step");
    let open = store
        .insert_proposal(proposal(
            "r",
            "lthr_erg_steady_v1",
            vec![value(AthleteParam::Lthr, 170., 5.)],
        ))
        .expect("test step");
    let kept = store
        .insert_proposal(proposal(
            "other",
            "lthr_erg_steady_v1",
            vec![value(AthleteParam::Lthr, 171., 5.)],
        ))
        .expect("test step");

    // No ride-delete command exists yet (ADR-0009 §2.2.4); the schema's ON DELETE does the work.
    let db = raw(dir.path());
    db.execute("DELETE FROM ride WHERE id = 'r'", [])
        .expect("test step");

    // The proposals of the deleted ride are gone, whatever their status, with their param rows.
    assert_eq!(store.proposal(accepted).expect("test step"), None);
    assert_eq!(store.proposal(open).expect("test step"), None);
    let params: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM athlete_proposal_param WHERE proposal_id IN (?1, ?2)",
            params![accepted, open],
            |r| r.get(0),
        )
        .expect("test step");
    assert_eq!(params, 0);
    let others: Vec<i64> = store
        .open_proposals()
        .expect("test step")
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(others, [kept]);
    // The accepted estimate keeps its value, source, method and n, with no ride and no proposal.
    let e = store
        .athlete_entry(written[0].id())
        .expect("test step")
        .expect("test step");
    let e = estimate(&e);
    assert_eq!(
        (
            e.value,
            e.source,
            e.method.as_str(),
            e.n,
            e.ride_id.as_deref(),
            e.proposal_id
        ),
        (190., Source::Rides, "peak_hr_v1", 1, None, None)
    );
    assert_eq!(
        store
            .athlete_snapshot()
            .expect("test step")
            .hr_max
            .map(|r| r.value),
        Some(190.)
    );
}

/// ADR-0009 §2.2.4 "Never silent": `EstimateOrigin` is the only way to append an estimate. The store's only `INSERT`
/// into the table is in `athlete.rs` (behind `append`, which takes an origin), and nothing else in the workspace
/// writes it.
#[test]
fn estimate_origin_is_the_only_way_to_append_an_estimate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut writers = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("app/src-tauri/src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("test step") {
            let path = entry.expect("test step").path();
            let name = path
                .file_name()
                .expect("test step")
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    "target" | "tests" | "migrations" | "fixtures"
                ) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("test step");
                let hits = text.matches("INTO athlete_estimate").count();
                if hits > 0 {
                    writers.push((
                        path.strip_prefix(&root)
                            .expect("test step")
                            .display()
                            .to_string(),
                        hits,
                    ));
                }
            }
        }
    }
    assert_eq!(
        writers,
        [("crates/crank-store/src/athlete.rs".to_owned(), 1)],
        "only athlete::append writes athlete_estimate"
    );
    let source = include_str!("../src/athlete.rs");
    let insert = source
        .find("INSERT INTO athlete_estimate")
        .expect("test step");
    let append = source.find("pub(crate) fn append(").expect("test step");
    let next_fn = source[append + 1..]
        .find("\nfn ")
        .map_or(source.len(), |i| append + 1 + i);
    assert!(
        (append..next_fn).contains(&insert),
        "the INSERT is inside append"
    );
    assert!(source[append..insert].contains("origin: EstimateOrigin"));
}

/// ADR-0009 §2.2.4 "Manual entry wins": accept refuses (`Conflict`) a proposal raised before its parameter's current
/// `updated_at`, here written by accepting another proposal of the same parameter, which supersedes nothing.
#[test]
fn accept_refuses_a_proposal_raised_before_the_current_value() {
    let (_dir, store) = store();
    ride(&store, "r", 500);
    ride(&store, "s", 600);
    let accept = |proposal_id, value_bpm| {
        store.append_athlete(
            EstimateOrigin::ProposalAccept {
                proposal_id,
                source: Source::Rides,
                values: vec![value(AthleteParam::HrMax, value_bpm, 2.)],
                n: 1,
            },
            3_000,
        )
    };
    let old = store
        .insert_proposal(proposal(
            "r",
            "peak_hr_v1",
            vec![value(AthleteParam::HrMax, 190., 2.)],
        ))
        .expect("test step");
    let newer = store
        .insert_proposal(NewProposal {
            created_at: 2_000,
            ..proposal(
                "s",
                "peak_hr_v1",
                vec![value(AthleteParam::HrMax, 192., 2.)],
            )
        })
        .expect("test step");
    accept(newer, 192.).expect("raised after every hr_max row");
    // `old` (raised at 1 000) is still open: accept, not a manual entry, wrote the current hr_max at 3 000.
    assert!(matches!(accept(old, 190.), Err(StoreError::Conflict(_))));
    assert_eq!(
        store
            .proposal(old)
            .expect("test step")
            .expect("test step")
            .status,
        ProposalStatus::Open
    );
    let hr_max = store
        .athlete_history(AthleteParam::HrMax)
        .expect("test step");
    assert_eq!(hr_max.len(), 1, "the refused accept wrote nothing");
    let e = estimate(&hr_max[0]);
    assert_eq!((e.value, e.proposal_id), (192., Some(newer)));
}

/// A ride started after migration 0005 gets its kind by the back-fill rule (ADR-0009 §2.5), never NULL.
#[test]
fn a_new_ride_records_its_kind() {
    let (dir, store) = store();
    ride(&store, "workout", 1_000);
    store
        .start_ride(NewRide {
            id: "free".into(),
            started_at: 2_000,
            workout: RideWorkoutJson::parse(
                r#"{"v":1,"id":"free-ride","name":"Free ride","ftp":0,"nodes":[]}"#,
            )
            .expect("test step"),
            compiled: CompiledJson::parse(r#"{"v":1}"#).expect("test step"),
            trainer_device_id: None,
            hr_device_id: None,
            telemetry_source: "sim".into(),
            metrics: vec!["power".into()],
        })
        .expect("test step");
    let kinds: Vec<(String, Option<String>)> = raw(dir.path())
        .prepare("SELECT id, kind FROM ride ORDER BY id")
        .expect("test step")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("test step")
        .collect::<Result<_, _>>()
        .expect("test step");
    assert_eq!(
        kinds,
        [
            ("free".into(), Some("free".into())),
            ("workout".into(), Some("workout".into())),
        ]
    );
}

/// #739 item 4: a stored athlete row that does not decode is a corrupt store (`CorruptAthlete`, a storage error
/// in the core), never `InvalidInput`, which the core reports as the caller's input.
#[test]
fn an_undecodable_stored_athlete_row_is_corruption() {
    let (dir, store) = store();
    raw(dir.path())
        .execute_batch(
            "INSERT INTO athlete_estimate (param, kind, value, sd, source, method, n, updated_at)
                 VALUES ('ftp', 'estimate', 250, 10, 'test', 'manual', 1099511627776, 1);",
        )
        .expect("test step");
    assert!(matches!(
        store.athlete_history(AthleteParam::Ftp),
        Err(StoreError::CorruptAthlete(_))
    ));
    raw(dir.path())
        .execute_batch(
            "DELETE FROM athlete_estimate;
             INSERT OR REPLACE INTO athlete_profile (id, age) VALUES (1, -1);",
        )
        .expect("test step");
    assert!(matches!(
        store.athlete_snapshot(),
        Err(StoreError::CorruptAthlete(_))
    ));
}
