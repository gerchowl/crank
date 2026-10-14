//! ADR-0001 §6 persistence acceptance and crash-process regression tests.
use crank_store::*;
use rusqlite::{Connection, params};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const WORKOUT_JSON: &str = r#"{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[{"type":"step","id":"s","label":null,"end":{"kind":"time","s":60},"target":{"kind":"pct_ftp","value":75},"cadence":null}]}"#;

// Frozen shared MetricId catalog, plus the workout mapping in ADR-0001 §5.
const METRICS: &[&str] = &[
    "power",
    "power_3s",
    "power_10s",
    "power_30s",
    "power_lap",
    "np",
    "if",
    "tss",
    "kj",
    "w_per_kg",
    "pct_ftp",
    "target_power",
    "delta_target",
    "power_zone",
    "wbal_pct",
    "hr",
    "hr_zone",
    "hr_pct_max",
    "hr_vs_typical",
    "hr_drift",
    "decoupling",
    "rr",
    "rmssd",
    "dfa_a1",
    "cadence",
    "target_cadence",
    "speed",
    "distance",
    "elapsed",
    "remaining",
    "block_remaining",
    "block_index",
    "compliance",
    "workout_s",
];

fn ride() -> NewRide {
    NewRide {
        id: "ride".into(),
        started_at: 1_700_000_000,
        workout: RideWorkoutJson::parse(WORKOUT_JSON).expect("valid test setup"),
        compiled: CompiledJson::parse(r#"{"v":1}"#).expect("valid test setup"),
        trainer_device_id: None,
        hr_device_id: None,
        telemetry_source: "sim".into(),
        metrics: METRICS.iter().map(|name| (*name).to_owned()).collect(),
    }
}
fn samples(second: u32) -> Vec<Sample> {
    METRICS
        .iter()
        .zip(0_u32..)
        .map(|(name, n)| Sample {
            metric: (*name).to_owned(),
            value: if second.is_multiple_of(13) {
                None
            } else {
                Some(f32::from_bits(0x4000_0000 + second * 30 + n))
            },
            estimated: second.is_multiple_of(7),
        })
        .collect()
}

#[test]
fn migrate_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        tables,
        [
            "athlete_estimate",
            "athlete_profile",
            "checkin",
            "coach_decision",
            "device",
            "import_activity",
            "lap",
            "layout",
            "plan_item",
            "recovery",
            "ride",
            "ride_chunk",
            "ride_rr",
            "setting",
            "snippet",
            "workout"
        ]
    );
    assert_eq!(
        db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert!(store.foreign_keys_enabled().unwrap());
}

#[test]
fn migrate_every_released_fixture() {
    const FIXTURES: &[&[u8]] = &[
        include_bytes!("fixtures/v1.db"),
        include_bytes!("fixtures/v2.db"),
    ];
    for fixture in FIXTURES {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("crank.db"), fixture).unwrap();
        let store = Store::open_owner(dir.path()).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
        assert_eq!(
            store
                .load_json::<SettingJson>(&["fixture"])
                .unwrap()
                .unwrap()
                .as_str(),
            r#"{"v":1,"preserved":true}"#
        );
    }
}

#[test]
fn two_hour_ride_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    for second in 0..7200 {
        store
            .append_second("ride", second, samples(second))
            .unwrap();
    }
    store
        .finish_ride(
            "ride",
            1_700_007_200,
            RideSummaryJson::parse(r#"{"v":1}"#).unwrap(),
        )
        .unwrap();
    drop(store);
    let reader = Store::open_read_only(dir.path()).unwrap();
    assert_eq!(
        reader.ride("ride").unwrap().unwrap().state,
        RideState::Finished
    );
    for (metric, name) in METRICS.iter().enumerate() {
        let chunks = reader.chunks("ride", name).unwrap();
        assert_eq!(chunks.len(), 120);
        for (idx, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.start_s as usize, idx * 60);
            assert_eq!(chunk.values.len(), 60);
            for (offset, value) in chunk.values.iter().enumerate() {
                let second = u32::try_from(idx * 60 + offset).unwrap();
                let sample = samples(second).remove(metric);
                assert_eq!(value.map(f32::to_bits), sample.value.map(f32::to_bits));
                assert_eq!(chunk.estimated[offset], sample.estimated);
            }
        }
    }
}

#[test]
fn crash_child() {
    let Some(path) = std::env::var_os("CRANK_STORE_CRASH_DIR") else {
        return;
    };
    let store = Store::open_owner(std::path::Path::new(&path)).unwrap();
    store.start_ride(ride()).unwrap();
    for second in 0..77 {
        store
            .append_second("ride", second, samples(second))
            .unwrap();
    }
    println!("persisted-75-buffered-2");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    panic!("parent must SIGKILL this child");
}
fn child(dir: &std::path::Path) -> std::process::Child {
    let mut child = Command::new(std::env::current_exe().expect("valid test setup"))
        .args(["--exact", "crash_child", "--nocapture"])
        .env("CRANK_STORE_CRASH_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("valid test setup");
    let mut stdout = BufReader::new(child.stdout.take().expect("valid test setup"));
    loop {
        let mut line = String::new();
        assert!(
            stdout.read_line(&mut line).expect("valid test setup") > 0,
            "child died before handshake"
        );
        if line.trim() == "persisted-75-buffered-2" {
            break;
        }
    }
    child
}
#[test]
fn kill_during_ride_recovers_only_with_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = child(dir.path());
    let reader = Store::open_read_only(dir.path()).unwrap();
    assert!(reader.ride("ride").unwrap().unwrap().ended_at.is_none());
    assert!(matches!(
        Store::open_owner(dir.path()),
        Err(StoreError::LockBusy)
    ));
    child.kill().unwrap(); // std::process::Child::kill is SIGKILL on Unix.
    assert!(!child.wait().unwrap().success());
    assert!(reader.ride("ride").unwrap().unwrap().ended_at.is_none());
    let owner = Store::open_owner(dir.path()).unwrap();
    assert_eq!(
        owner.ride("ride").unwrap().unwrap().state,
        RideState::Aborted
    );
    for (metric, name) in METRICS.iter().enumerate() {
        let chunks = owner.chunks("ride", name).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].values.len(), 60);
        assert_eq!(chunks[1].values.len(), 15);
        for second in 0..75 {
            let expected = samples(u32::try_from(second).unwrap()).remove(metric);
            assert_eq!(
                chunks[second / 60].values[second % 60].map(f32::to_bits),
                expected.value.map(f32::to_bits)
            );
            assert_eq!(
                chunks[second / 60].estimated[second % 60],
                expected.estimated
            );
        }
    }
}
#[test]
fn lockless_process_never_recovers_or_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = child(dir.path());
    child.kill().unwrap();
    child.wait().unwrap();
    let reader = Store::open_read_only(dir.path()).unwrap();
    assert!(reader.ride("ride").unwrap().unwrap().ended_at.is_none());
    assert!(matches!(
        reader.start_ride(ride()),
        Err(StoreError::ReadOnly)
    ));
    assert!(matches!(
        reader.put_setting("x", SettingJson::parse(r#"{"v":1}"#).unwrap()),
        Err(StoreError::ReadOnly)
    ));
}

fn invalid_column<T: JsonColumn>(store: &Store, db: &Connection, insert: &str, key: &[&str]) {
    let valid = WORKOUT_JSON;
    db.execute(insert, [valid])
        .expect("write valid persisted envelope");
    assert_eq!(
        store
            .load_json::<T>(key)
            .expect("read envelope")
            .expect("existing row")
            .as_str(),
        valid
    );
    for invalid in ["[]", "null", "3", "{}", r#"{"v":1.5}"#, r#"{"v":"1"}"#, "{"] {
        db.execute(insert, [invalid]).expect("valid test setup");
        let err = store
            .load_json::<T>(key)
            .expect_err("invalid column must fail");
        assert!(
            matches!(err, StoreError::InvalidJson { table, column, .. } if table == T::TABLE && column == T::COLUMN)
        );
    }
}
#[test]
fn every_json_column_validated_on_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    macro_rules! column {
        ($t:ty, $sql:expr, $key:expr) => {
            invalid_column::<$t>(&store, &db, $sql, $key);
        };
    }
    column!(
        WorkoutTreeJson,
        "INSERT OR REPLACE INTO workout(id,name,dsl,tree_json,created_at) VALUES('w','name','',?1,0)",
        &["w"]
    );
    column!(
        SnippetJson,
        "INSERT OR REPLACE INTO snippet VALUES('s','name',?1,0)",
        &["s"]
    );
    column!(
        LayoutJson,
        "INSERT OR REPLACE INTO layout VALUES('l','name',?1,0)",
        &["l"]
    );
    column!(
        SettingJson,
        "INSERT OR REPLACE INTO setting VALUES('k',?1)",
        &["k"]
    );
    column!(
        RideWorkoutJson,
        "UPDATE ride SET workout_json=?1 WHERE id='ride'",
        &["ride"]
    );
    column!(
        CompiledJson,
        "UPDATE ride SET compiled_json=?1 WHERE id='ride'",
        &["ride"]
    );
    column!(
        RideSummaryJson,
        "UPDATE ride SET summary_json=?1 WHERE id='ride'",
        &["ride"]
    );
    column!(
        LapStatsJson,
        "INSERT OR REPLACE INTO lap VALUES('ride',0,'node','/',0,1,0,1,?1)",
        &["ride", "0"]
    );
    column!(
        RecoveryFitJson,
        "INSERT OR REPLACE INTO recovery VALUES('ride','node',0,?1,1,X'')",
        &["ride", "node", "0"]
    );
    column!(
        CoachFeaturesJson,
        "INSERT OR REPLACE INTO coach_decision(call_id,ride_id,t,verdict,band,features_json) VALUES('call','ride',0,'hold','strong',?1)",
        &["call"]
    );
    assert!(matches!(
        store.ride("ride"),
        Err(StoreError::InvalidJson { .. })
    ));
}

#[test]
fn workout_columns_validate_the_domain_owner_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    db.execute(
        "INSERT INTO workout(id,name,dsl,tree_json,created_at) VALUES('w','Ride','',?1,0)",
        [WORKOUT_JSON],
    )
    .unwrap();
    let tree = store.load_json::<WorkoutTreeJson>(&["w"]).unwrap().unwrap();
    assert_eq!(tree.workout().nodes.len(), 1);
    assert_eq!(
        store.ride("ride").unwrap().unwrap().workout.workout(),
        tree.workout()
    );
    for invalid in [
        r#"{"v":1}"#,
        r#"{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[{"type":"unknown"}]}"#,
    ] {
        db.execute("UPDATE workout SET tree_json=?1", [invalid])
            .unwrap();
        db.execute("UPDATE ride SET workout_json=?1", [invalid])
            .unwrap();
        assert!(matches!(
            store.load_json::<WorkoutTreeJson>(&["w"]),
            Err(StoreError::InvalidJson {
                table: "workout",
                column: "tree_json",
                ..
            })
        ));
        assert!(matches!(
            store.ride("ride"),
            Err(StoreError::InvalidJson {
                table: "ride",
                column: "workout_json",
                ..
            })
        ));
    }
}

#[test]
fn boundaries_final_flush_and_invalid_samples() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    assert!(store.append_second("ride", 1, samples(1)).is_err());
    let mut invalid = samples(0);
    invalid[0].value = Some(f32::INFINITY);
    assert!(store.append_second("ride", 0, invalid).is_err());
    for second in 0..62 {
        store
            .append_second("ride", second, samples(second))
            .unwrap();
    }
    assert_eq!(store.chunks("ride", METRICS[0]).unwrap().len(), 1);
    store
        .finish_ride(
            "ride",
            1_700_000_100,
            RideSummaryJson::parse(r#"{"v":1}"#).unwrap(),
        )
        .unwrap();
    assert_eq!(store.chunks("ride", METRICS[0]).unwrap()[1].values.len(), 2);
    assert!(store.append_second("ride", 62, samples(62)).is_err());
}

#[test]
fn pairing_memory_and_typed_json_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    let dev = Device {
        id: "neo".into(),
        kind: "trainer".into(),
        name: Some("NEO".into()),
        platform_id: "uuid".into(),
        serial: Some("55624".into()),
        model: Some("Neo".into()),
        last_seen: 1,
    };
    store.remember_device(dev.clone()).unwrap();
    assert_eq!(store.devices().unwrap(), vec![dev]);
    store
        .put_setting("key", SettingJson::parse(r#"{"v":1,"x":true}"#).unwrap())
        .unwrap();
    assert_eq!(
        store
            .load_json::<SettingJson>(&["key"])
            .unwrap()
            .unwrap()
            .as_str(),
        r#"{"v":1,"x":true}"#
    );
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    db.execute(
        "UPDATE setting SET json=?1 WHERE key=?2",
        params![r#"{"v":false}"#, "key"],
    )
    .unwrap();
    assert!(matches!(
        store.load_json::<SettingJson>(&["key"]),
        Err(StoreError::InvalidJson { .. })
    ));
}

fn device(id: &str, platform_id: &str, last_seen: i64) -> Device {
    Device {
        id: id.into(),
        kind: "trainer".into(),
        name: Some("NEO".into()),
        platform_id: platform_id.into(),
        serial: Some("55624".into()),
        model: Some("Neo".into()),
        last_seen,
    }
}

#[test]
fn remembering_a_role_again_updates_one_row_and_returns_its_existing_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    assert_eq!(
        store.remember_device(device("neo", "uuid", 1)).unwrap(),
        "neo"
    );
    // Same peripheral and role under another caller id: one row, original id returned.
    assert_eq!(
        store.remember_device(device("other", "uuid", 2)).unwrap(),
        "neo"
    );
    assert_eq!(store.devices().unwrap(), vec![device("neo", "uuid", 2)]);
    // CoreBluetooth re-assigned the platform id of a known device: updated in place.
    assert_eq!(
        store.remember_device(device("neo", "uuid-2", 3)).unwrap(),
        "neo"
    );
    assert_eq!(store.devices().unwrap(), vec![device("neo", "uuid-2", 3)]);
    // An id that already names another role is rejected, never silently reused.
    let mut strap = device("neo", "uuid-3", 4);
    strap.kind = "hr".into();
    assert!(matches!(
        store.remember_device(strap),
        Err(StoreError::InvalidInput(_))
    ));
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    let duplicate = db.execute(
        "INSERT INTO device (id,kind,platform_id,last_seen) VALUES ('x','trainer','uuid-2',0)",
        [],
    );
    assert!(
        duplicate.is_err(),
        "the schema itself enforces UNIQUE(platform_id, kind)"
    );
}

#[test]
fn migration_three_renames_par_q_result_and_preserves_the_value() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("fixtures/v2.db"),
    )
    .unwrap();
    drop(Store::open_owner(dir.path()).unwrap());
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    let value: String = db
        .query_row(
            "SELECT health_check_result FROM athlete_profile WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, r#"{"v":1,"cleared":true}"#);
    assert!(
        db.prepare("SELECT par_q_result FROM athlete_profile")
            .is_err(),
        "the old column name is gone"
    );
}

#[test]
fn migration_two_dedupes_within_a_kind_keeps_cross_kind_rows_and_fks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("crank.db"),
        include_bytes!("fixtures/v1.db"),
    )
    .unwrap();
    {
        let db = Connection::open(dir.path().join("crank.db")).unwrap();
        db.execute_batch(
            "INSERT INTO device VALUES ('old','trainer','NEO','uuid','1',10);
             INSERT INTO device VALUES ('new','trainer','NEO','uuid','1',20);
             INSERT INTO device VALUES ('strap','hr','Strap','uuid',NULL,15);
             INSERT INTO device VALUES ('strap-old','hr','Strap','uuid',NULL,5);
             INSERT INTO ride (id,started_at,workout_json,compiled_json,trainer_device_id,
                               hr_device_id,telemetry_source)
             VALUES ('r','1','{}','{}','old','strap-old','fec');",
        )
        .unwrap();
    }
    let store = Store::open_owner(dir.path()).unwrap();
    let ids: Vec<_> = store.devices().unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(
        ids,
        ["new", "strap"],
        "cross-kind rows are kept, same-kind deduped"
    );
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    let (trainer, hr): (String, String) = db
        .query_row(
            "SELECT trainer_device_id, hr_device_id FROM ride WHERE id='r'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((trainer.as_str(), hr.as_str()), ("new", "strap"));
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let violations = db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .count();
    assert_eq!(violations, 0);
}

#[test]
fn trainer_and_strap_on_one_platform_id_coexist_per_role() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    let mut strap = device("strap", "uuid", 2);
    strap.kind = "hr".into();
    strap.name = Some("Strap".into());
    store.remember_device(device("neo", "uuid", 1)).unwrap();
    assert_eq!(store.remember_device(strap.clone()).unwrap(), "strap");
    let devices = store.devices().unwrap();
    assert_eq!(devices.len(), 2, "{devices:?}");
    assert!(devices.contains(&device("neo", "uuid", 1)));
    assert!(devices.contains(&strap));
    // Same role again: updates that role's name/serial only.
    let mut renamed = device("neo", "uuid", 5);
    renamed.name = Some("NEO 2".into());
    store.remember_device(renamed.clone()).unwrap();
    let devices = store.devices().unwrap();
    assert!(
        devices.contains(&renamed) && devices.contains(&strap),
        "{devices:?}"
    );
}

#[test]
fn pairing_memory_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    Store::open_owner(dir.path())
        .unwrap()
        .remember_device(device("neo", "uuid", 7))
        .unwrap();
    let store = Store::open_read_only(dir.path()).unwrap();
    assert_eq!(store.devices().unwrap(), vec![device("neo", "uuid", 7)]);
}

#[test]
fn failed_chunk_transaction_preserves_prior_commit_and_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    for second in 0..9 {
        store
            .append_second("ride", second, samples(second))
            .unwrap();
    }
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    // Abort after earlier metrics have been upserted within the transaction.
    db.execute_batch("CREATE TRIGGER fail_chunk BEFORE UPDATE ON ride_chunk WHEN NEW.metric='workout_s' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(matches!(
        store.append_second("ride", 9, samples(9)),
        Err(StoreError::Sql(_))
    ));
    for metric in METRICS {
        assert_eq!(store.chunks("ride", metric).unwrap()[0].values.len(), 5);
    }
    db.execute_batch("DROP TRIGGER fail_chunk").unwrap();
    store.append_second("ride", 9, samples(9)).unwrap();
    for metric in METRICS {
        assert_eq!(store.chunks("ride", metric).unwrap()[0].values.len(), 10);
    }
}
#[test]
fn lock_release_new_ride_reset_and_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    let mut invalid = ride();
    invalid.trainer_device_id = Some("missing".into());
    assert!(matches!(store.start_ride(invalid), Err(StoreError::Sql(_))));
    store.start_ride(ride()).unwrap();
    store.append_second("ride", 0, samples(0)).unwrap();
    assert!(store.start_ride(ride()).is_err());
    store
        .finish_ride(
            "ride",
            1_700_000_100,
            RideSummaryJson::parse(r#"{"v":1}"#).unwrap(),
        )
        .unwrap();
    let mut next = ride();
    next.id = "next".into();
    next.metrics = vec!["only".into()];
    store.start_ride(next).unwrap();
    store
        .append_second(
            "next",
            0,
            vec![Sample {
                metric: "only".into(),
                value: Some(-0.0),
                estimated: false,
            }],
        )
        .unwrap();
    store
        .finish_ride(
            "next",
            1_700_000_100,
            RideSummaryJson::parse(r#"{"v":1}"#).unwrap(),
        )
        .unwrap();
    assert_eq!(
        store.chunks("next", "only").unwrap()[0].values[0]
            .unwrap()
            .to_bits(),
        (-0_f32).to_bits()
    );
    assert!(store.chunks("next", METRICS[0]).unwrap().is_empty());
    drop(store);
    assert_eq!(
        Store::open_owner(dir.path())
            .unwrap()
            .ride("ride")
            .unwrap()
            .unwrap()
            .state,
        RideState::Finished
    );
}
#[test]
fn future_schema_rejected_by_owner_without_downgrade() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open_owner(dir.path()).unwrap());
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    assert!(matches!(
        Store::open_owner(dir.path()),
        Err(StoreError::Migration(_))
    ));
    // The owner refusal must not have downgraded the database.
    let version: u32 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);
}
#[test]
fn read_only_refuses_a_newer_schema() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open_owner(dir.path()).unwrap());
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    assert!(matches!(
        Store::open_read_only(dir.path()),
        Err(StoreError::SchemaTooNew {
            found: 999,
            supported: 3
        })
    ));
}
#[test]
fn ride_state_is_typed_and_unknown_values_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    store.start_ride(ride()).unwrap();
    assert_eq!(store.ride("ride").unwrap().unwrap().state, RideState::Open);
    let db = Connection::open(dir.path().join("crank.db")).unwrap();
    // The CHECK constraint forbids unknown strings; bypass it to model a
    // database written by other software.
    db.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    db.execute("UPDATE ride SET state='paused', ended_at=1", [])
        .unwrap();
    assert!(matches!(
        store.ride("ride"),
        Err(StoreError::UnknownRideState(s)) if s == "paused"
    ));
    for (state, text) in [
        (RideState::Open, "\"open\""),
        (RideState::Finished, "\"finished\""),
        (RideState::Aborted, "\"aborted\""),
    ] {
        assert_eq!(serde_json::to_string(&state).unwrap(), text);
        assert_eq!(RideState::from_column(state.as_column()).unwrap(), state);
    }
}
#[test]
fn workout_constructor_adds_the_version_envelope() {
    let workout: crank_domain::workout::Workout = serde_json::from_str(WORKOUT_JSON).unwrap();
    let json = serde_json::to_value(&workout).unwrap();
    assert!(json.get("v").is_none(), "domain type has no v of its own");
    let tree = WorkoutTreeJson::new(&workout).unwrap();
    assert_eq!(tree.workout(), &workout);
    let stored: serde_json::Value = serde_json::from_str(tree.as_str()).unwrap();
    assert_eq!(stored["v"], 1);
    assert_eq!(RideWorkoutJson::new(&workout).unwrap().workout(), &workout);
}

#[test]
fn rides_lists_newest_first_and_forget_device_unlinks_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_owner(dir.path()).unwrap();
    let trainer = device("t", "p-t", 1);
    store.remember_device(trainer).unwrap();
    for (id, started_at) in [("a", 10), ("b", 20)] {
        let mut r = ride();
        r.id = id.into();
        r.started_at = started_at;
        r.trainer_device_id = Some("t".into());
        store.start_ride(r).unwrap();
        store.append_second(id, 0, samples(0)).unwrap();
        store
            .finish_ride(
                id,
                started_at + 5,
                RideSummaryJson::parse(r#"{"v":1}"#).unwrap(),
            )
            .unwrap();
    }
    let ids: Vec<_> = store.rides().unwrap().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, ["b", "a"]);
    assert!(store.forget_device("t").unwrap());
    assert!(!store.forget_device("t").unwrap());
    assert!(store.devices().unwrap().is_empty());
    assert_eq!(
        store.rides().unwrap().len(),
        2,
        "history survives the pairing"
    );
}
