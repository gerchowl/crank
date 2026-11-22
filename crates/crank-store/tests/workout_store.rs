//! The `workout` table (#110): insert, update, list, delete, a reopen, and corrupt rows.
use crank_domain::workout::Workout;
use crank_store::{JsonColumn, Store, StoreError, StoredWorkout, WorkoutTreeJson};
use rusqlite::Connection;

fn workout(id: &str, name: &str, minutes: f64) -> Workout {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": name, "ftp": 250.0,
        "nodes": [{"type": "step", "id": format!("{id}-s"), "label": null,
                   "end": {"kind": "time", "s": minutes * 60.0},
                   "target": {"kind": "pct_ftp", "value": 70.0}, "cadence": null}],
    }))
    .expect("a workout")
}

fn tree(w: &Workout) -> WorkoutTreeJson {
    WorkoutTreeJson::new(w).expect("serialises")
}

fn names(store: &Store) -> Vec<(String, String, i64)> {
    store
        .workouts()
        .expect("lists")
        .into_iter()
        .map(|s: StoredWorkout| {
            (
                s.tree.workout().id.clone(),
                s.tree.workout().name.clone(),
                s.created_at,
            )
        })
        .collect()
}

#[test]
fn insert_update_list_delete_and_reopen() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open_owner(dir.path()).expect("opens");
    assert!(store.workouts().expect("lists").is_empty());

    // Listed by first save, then id, whatever the insert order.
    store
        .insert_workout(tree(&workout("b", "Bee", 1.0)), "1m 70%".into(), 20)
        .expect("insert");
    store
        .insert_workout(tree(&workout("a", "Ay", 1.0)), "1m 70%".into(), 20)
        .expect("insert");
    store
        .insert_workout(tree(&workout("c", "Sea", 1.0)), "1m 70%".into(), 10)
        .expect("insert");
    assert_eq!(
        names(&store),
        [
            ("c".into(), "Sea".into(), 10),
            ("a".into(), "Ay".into(), 20),
            ("b".into(), "Bee".into(), 20)
        ]
    );

    // A taken id is refused, never overwritten.
    let taken = store
        .insert_workout(tree(&workout("a", "Other", 2.0)), "2m 70%".into(), 30)
        .expect_err("taken");
    assert!(matches!(taken, StoreError::InvalidInput(_)), "{taken}");
    assert_eq!(
        store
            .workout("a")
            .expect("reads")
            .expect("stored")
            .tree
            .workout()
            .name,
        "Ay"
    );

    // An update replaces name, tree and text and keeps when it was first saved.
    let renamed = workout("a", "Ay two", 3.0);
    assert!(
        store
            .update_workout(tree(&renamed), "3m 70%".into())
            .expect("update")
    );
    let got = store.workout("a").expect("reads").expect("stored");
    assert_eq!((got.tree.workout(), got.created_at), (&renamed, 20));
    // An update of no row writes nothing.
    assert!(
        !store
            .update_workout(tree(&workout("zz", "Zed", 1.0)), "1m 70%".into())
            .expect("update")
    );
    assert!(store.workout("zz").expect("reads").is_none());

    // The row's own columns repeat the tree, for a plain SELECT.
    let db = Connection::open(dir.path().join("crank.db")).expect("db");
    let (name, dsl, ftp, source): (String, String, f64, String) = db
        .query_row(
            "SELECT name,dsl,ftp_at_creation,source FROM workout WHERE id='a'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("row");
    assert_eq!(
        (name.as_str(), dsl.as_str(), ftp, source.as_str()),
        ("Ay two", "3m 70%", 250.0, "user")
    );

    assert!(store.delete_workout("b").expect("delete"));
    assert!(!store.delete_workout("b").expect("delete"));

    // Reopen: what was saved is there.
    drop(store);
    let store = Store::open_owner(dir.path()).expect("reopens");
    assert_eq!(
        names(&store),
        [
            ("c".into(), "Sea".into(), 10),
            ("a".into(), "Ay two".into(), 20)
        ]
    );
    assert_eq!(
        store
            .workout("a")
            .expect("reads")
            .expect("stored")
            .tree
            .workout(),
        &renamed
    );
    // A read-only process reads them and cannot write.
    let ro = Store::open_read_only(dir.path()).expect("read-only");
    assert_eq!(ro.workouts().expect("lists").len(), 2);
    assert!(matches!(ro.delete_workout("a"), Err(StoreError::ReadOnly)));
}

#[test]
fn a_corrupt_row_is_refused_not_misread() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open_owner(dir.path()).expect("opens");
    store
        .insert_workout(tree(&workout("a", "Ay", 1.0)), "1m 70%".into(), 1)
        .expect("insert");
    let db = Connection::open(dir.path().join("crank.db")).expect("db");
    // A tree that names another id than its row.
    db.execute(
        "UPDATE workout SET tree_json=?1 WHERE id='a'",
        [tree(&workout("b", "Bee", 1.0)).as_str()],
    )
    .expect("update");
    for read in [store.workouts().map(|_| ()), store.workout("a").map(|_| ())] {
        assert!(
            matches!(read, Err(StoreError::InvalidJson { table: "workout", column: "tree_json", ref reason }) if reason.contains("not the row's")),
            "{read:?}"
        );
    }
    // A tree that does not parse.
    db.execute("UPDATE workout SET tree_json='{\"v\":1}' WHERE id='a'", [])
        .expect("update");
    assert!(matches!(
        store.workouts(),
        Err(StoreError::InvalidJson {
            table: "workout",
            ..
        })
    ));
}
