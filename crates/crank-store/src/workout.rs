//! The `workout` table (ADR-0001 §6; #110): the rider's own workouts. `tree_json` is the truth
//! (the whole domain `Workout`, validated on every read); `dsl` is its canonical text, kept for
//! people reading the database; `name` and `ftp_at_creation` repeat the tree's name and FTP so a
//! plain `SELECT` reads them. Built-in workouts are not stored: they are the core's (`library.rs`).
use crate::{JsonColumn, StoreError, WorkoutTreeJson};
use rusqlite::{Connection, OptionalExtension, params};

/// The `source` every row this module writes carries: a workout the rider saved.
const SOURCE_USER: &str = "user";

/// A stored workout ([`crate::Store::workouts`], [`crate::Store::workout`]).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredWorkout {
    /// The workout, its id the row's id.
    pub tree: WorkoutTreeJson,
    /// UTC Unix seconds when it was first saved; an update keeps it.
    pub created_at: i64,
}

const SELECT: &str = "SELECT id,tree_json,created_at FROM workout";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
}

/// A row as a [`StoredWorkout`]: its tree must parse and carry the row's id.
fn decode((id, tree, created_at): (String, String, i64)) -> Result<StoredWorkout, StoreError> {
    let tree = WorkoutTreeJson::parse(&tree)?;
    if tree.workout().id != id {
        return Err(StoreError::InvalidJson {
            table: "workout",
            column: "tree_json",
            reason: format!("tree id {:?} is not the row's {id:?}", tree.workout().id),
        });
    }
    Ok(StoredWorkout { tree, created_at })
}

pub(crate) fn list(db: &Connection) -> Result<Vec<StoredWorkout>, StoreError> {
    db.prepare(&format!("{SELECT} ORDER BY created_at, id"))?
        .query_map([], row)?
        .map(|r| decode(r?))
        .collect()
}

pub(crate) fn select(db: &Connection, id: &str) -> Result<Option<StoredWorkout>, StoreError> {
    db.query_row(&format!("{SELECT} WHERE id=?1"), [id], row)
        .optional()?
        .map(decode)
        .transpose()
}

pub(crate) fn insert(
    db: &Connection,
    tree: &WorkoutTreeJson,
    dsl: &str,
    created_at: i64,
) -> Result<(), StoreError> {
    let w = tree.workout();
    let taken: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM workout WHERE id=?1)",
        [&w.id],
        |r| r.get(0),
    )?;
    if taken {
        return Err(StoreError::InvalidInput(format!(
            "workout id {:?} is taken",
            w.id
        )));
    }
    db.execute(
        "INSERT INTO workout(id,name,dsl,tree_json,ftp_at_creation,source,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            w.id,
            w.name,
            dsl,
            tree.as_str(),
            w.ftp,
            SOURCE_USER,
            created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn update(
    db: &Connection,
    tree: &WorkoutTreeJson,
    dsl: &str,
) -> Result<bool, StoreError> {
    let w = tree.workout();
    let n = db.execute(
        "UPDATE workout SET name=?2,dsl=?3,tree_json=?4,ftp_at_creation=?5 WHERE id=?1",
        params![w.id, w.name, dsl, tree.as_str(), w.ftp],
    )?;
    Ok(n > 0)
}

pub(crate) fn delete(db: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(db.execute("DELETE FROM workout WHERE id=?1", [id])? > 0)
}
