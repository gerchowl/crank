//! `SQLite` persistence (ADR-0001 §2, §5–6; ADR-0002 §5).
//! Owners acquire `ride.lock` before migrating/recovering on one writer thread.
//! Query processes use read-only WAL connections and never recover rides.
//! Coordinates are session seconds; timestamps are UTC Unix seconds.
mod athlete;
mod json;
mod route;
mod workout;
mod writer;
pub use athlete::{
    EstimateOrigin, METHOD_LEGACY, METHOD_PROFILE, NewProposal, Proposal, ProposalClose,
    ProposalStatus, ProposedValue, StaleReason,
};
use crank_domain::athlete::{AthleteError, AthleteParam, AthleteSnapshot, Entry};
pub use json::*;
pub use route::{RouteListing, RouteOriginal, StoredRoute, decode_points, encode_points};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
pub use workout::StoredWorkout;
use writer::WriterHandle;
/// Typed persistence failures (ADR-0001 §7).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// `SQLite` failure.
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    /// Migration failure, including unsupported future versions.
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    /// Filesystem/lock failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Another process owns the ride lock.
    #[error("ride.lock is already held")]
    LockBusy,
    /// Query-only store mutation.
    #[error("store is read-only")]
    ReadOnly,
    /// JSON violated its column contract.
    #[error("invalid JSON in {table}.{column}: {reason}")]
    InvalidJson {
        /// Owning table.
        table: &'static str,
        /// Owning column.
        column: &'static str,
        /// Validation failure.
        reason: String,
    },
    /// Invalid ride, sample, or repository key.
    #[error("invalid store input: {0}")]
    InvalidInput(String),
    /// Invalid persisted binary chunk.
    #[error("corrupt ride chunk: {0}")]
    CorruptChunk(String),
    /// A stored route that does not decode, or breaks an ADR-0005 §2.1 invariant.
    #[error("corrupt route: {0}")]
    CorruptRoute(String),
    /// The database was written by a newer schema than this build supports;
    /// it is never interpreted, read-only or not.
    #[error("database schema v{found} is newer than supported v{supported}")]
    SchemaTooNew {
        /// `user_version` found in the database.
        found: u32,
        /// Newest schema this build understands.
        supported: u32,
    },
    /// A stored `ride.state` is not one of the known values.
    #[error("unknown ride state {0:?}")]
    UnknownRideState(String),
    /// A stored `ride.end_cause` is not one of the known values.
    #[error("unknown ride end cause {0:?}")]
    UnknownEndCause(String),
    /// A stored athlete-model row (`athlete_estimate`, `athlete_proposal`, `athlete_profile`) that does not decode:
    /// the database is corrupt, not the caller's input (#739).
    #[error("corrupt athlete row: {0}")]
    CorruptAthlete(String),
    /// An athlete-model row the domain refuses (`crank_domain::athlete::check_entry`, `check_value`).
    #[error(transparent)]
    Athlete(AthleteError),
    /// The row named does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The row is not in a state that allows the change (a proposal that is no longer open, a taken key).
    #[error("conflict: {0}")]
    Conflict(String),
    /// Writer terminated or reader mutex poisoned.
    #[error("store worker unavailable")]
    WorkerStopped,
}
/// Lifecycle of a ride, mirroring the `ride.state` column.
/// The column is NULL while the ride is live and otherwise `finished` or
/// `aborted` (enforced by a CHECK constraint); `Open` is the NULL case.
/// The serde names are the stored strings, plus `open` for NULL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RideState {
    /// Still being recorded (`state` is NULL).
    Open,
    /// Closed normally by `finish_ride`.
    Finished,
    /// Closed by crash recovery in the next lock owner.
    Aborted,
}
impl RideState {
    /// Decode the nullable `ride.state` column.
    /// # Errors
    /// Returns `UnknownRideState` for any string other than `finished`/`aborted`.
    pub fn from_column(stored: Option<&str>) -> Result<Self, StoreError> {
        match stored {
            None => Ok(Self::Open),
            Some("finished") => Ok(Self::Finished),
            Some("aborted") => Ok(Self::Aborted),
            Some(other) => Err(StoreError::UnknownRideState(other.to_owned())),
        }
    }
    /// The value stored in `ride.state`; `None` (NULL) for `Open`.
    #[must_use]
    pub fn as_column(self) -> Option<&'static str> {
        match self {
            Self::Open => None,
            Self::Finished => Some("finished"),
            Self::Aborted => Some("aborted"),
        }
    }
}
/// Why a ride ended, the `ride.end_cause` column (ADR-0009 §2.3.2 "Ride end cause"). NULL for a
/// ride closed before migration 0005's column was written (`None` in [`Ride::end_cause`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndCause {
    /// The workout ran out.
    Completed,
    /// The rider's End.
    RiderEnd,
    /// Report symptom or a medical stop (DESIGN §5.2). No path writes it yet (#762).
    Safety,
    /// The core ended the ride because its engine could not (#331); written by
    /// [`Store::abort_ride`].
    Forced,
    /// Closed by crash recovery when the next lock owner opened the store.
    Recovered,
}
impl EndCause {
    /// Every cause.
    pub const ALL: [Self; 5] = [
        Self::Completed,
        Self::RiderEnd,
        Self::Safety,
        Self::Forced,
        Self::Recovered,
    ];
    /// The stored string.
    #[must_use]
    pub fn as_column(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::RiderEnd => "rider_end",
            Self::Safety => "safety",
            Self::Forced => "forced",
            Self::Recovered => "recovered",
        }
    }
    /// Decode the nullable `ride.end_cause` column.
    /// # Errors
    /// Returns `UnknownEndCause` for any other string.
    pub fn from_column(stored: Option<&str>) -> Result<Option<Self>, StoreError> {
        stored
            .map(|v| {
                Self::ALL
                    .into_iter()
                    .find(|c| c.as_column() == v)
                    .ok_or_else(|| StoreError::UnknownEndCause(v.to_owned()))
            })
            .transpose()
    }
}
/// One second's per-metric mean (ADR-0001 §5–6).
#[derive(Clone, Debug)]
pub struct Sample {
    /// Engine metric identifier.
    pub metric: String,
    /// Missing or finite value; missing encodes as NaN.
    pub value: Option<f32>,
    /// Accumulated-power gap filling supplied this second.
    pub estimated: bool,
}
/// Validated chunk of up to 60 session seconds.
#[derive(Debug)]
pub struct Chunk {
    /// First session second, aligned to 60 seconds.
    pub start_s: u32,
    /// Values; NaNs decode to missing.
    pub values: Vec<Option<f32>>,
    /// Estimated flags; least significant bit represents the first second.
    pub estimated: Vec<bool>,
}
/// Immutable start snapshot; metric set stays fixed during a ride.
#[derive(Debug)]
pub struct NewRide {
    /// Stable identifier.
    pub id: String,
    /// UTC Unix seconds.
    pub started_at: i64,
    /// Workout snapshot.
    pub workout: RideWorkoutJson,
    /// Compiled snapshot.
    pub compiled: CompiledJson,
    /// The athlete model the ride was loaded with (ADR-0009 §2.1.4).
    pub athlete: RideAthleteJson,
    /// Paired trainer id, if any.
    pub trainer_device_id: Option<String>,
    /// Paired HR id, if any.
    pub hr_device_id: Option<String>,
    /// Telemetry provenance, e.g. `sim` or `fec`.
    pub telemetry_source: String,
    /// Engine metric ids, including `workout_s` when provided.
    pub metrics: Vec<String>,
    /// Which metrics the record holds (`ride.record_version`, ADR-0009 §2.3.2); at least 1.
    pub record_version: u32,
}
/// A ride's whole record ([`Store::record`]).
#[derive(Debug)]
pub struct StoredRecord {
    /// The ride's `record_version`.
    pub record_version: u32,
    /// Each stored metric's values by session second, from 0; NaN decodes to `None`.
    pub metrics: BTreeMap<String, Vec<Option<f32>>>,
}
/// Ride metadata with validated snapshots.
#[derive(Debug)]
pub struct Ride {
    /// Stable identifier.
    pub id: String,
    /// UTC Unix seconds at start.
    pub started_at: i64,
    /// UTC Unix seconds at finish/recovery; absent while live.
    pub ended_at: Option<i64>,
    /// Lifecycle state; `Open` while live.
    pub state: RideState,
    /// Workout snapshot.
    pub workout: RideWorkoutJson,
    /// Compiled snapshot.
    pub compiled: CompiledJson,
    /// The athlete model the ride was loaded with; `None` for a ride recorded before it was stored
    /// (ADR-0009 §2.1.4: such a ride falls back to the FTP inside its workout).
    pub athlete: Option<RideAthleteJson>,
    /// Optional summary.
    pub summary: Option<RideSummaryJson>,
    /// Why it ended; `None` while live and for rides closed before it was recorded.
    pub end_cause: Option<EndCause>,
    /// Which metrics its record holds (ADR-0009 §2.3.2): 1 for every ride recorded before
    /// version 2.
    pub record_version: u32,
}
/// One row of the ride list ([`Store::rides`]).
#[derive(Debug)]
pub struct RideListing {
    /// Stable identifier.
    pub id: String,
    /// UTC Unix seconds at start.
    pub started_at: i64,
    /// UTC Unix seconds at finish/recovery; absent while live.
    pub ended_at: Option<i64>,
    /// Lifecycle state; `Open` while live.
    pub state: RideState,
    /// Id of the ridden workout.
    pub workout_id: String,
    /// Name of the ridden workout.
    pub workout_name: String,
    /// Optional summary.
    pub summary: Option<RideSummaryJson>,
}
/// Pairing memory (ADR-0002 §5): one row per (`platform_id`, `kind`) role. Matching policy lives in `crank-ble`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Stable database id.
    pub id: String,
    /// Device role.
    pub kind: String,
    /// Advertised name.
    pub name: Option<String>,
    /// Platform peripheral id (`CoreBluetooth` UUID on macOS).
    pub platform_id: String,
    /// DIS serial number.
    pub serial: Option<String>,
    /// DIS model number (2A24).
    pub model: Option<String>,
    /// UTC Unix seconds at last sighting.
    pub last_seen: i64,
}
/// Two read-only WAL readers and, for an owner, one writer thread.
/// Drop drains queued work before releasing the writer's advisory lock.
pub struct Store {
    readers: [Mutex<Connection>; 2],
    next_reader: AtomicUsize,
    writer: Option<WriterHandle>,
}
impl Store {
    /// Acquire `ride.lock`, migrate and recover before accepting work.
    /// # Errors
    /// Returns `LockBusy`, I/O, migration, SQL or worker errors.
    pub fn open_owner(data_dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir)?;
        Self::readers(data_dir, Some(WriterHandle::open(data_dir)?))
    }
    /// Open an existing database without migrations, writes or recovery.
    /// A database whose `user_version` exceeds this build's schema is refused:
    /// a read-only process must never interpret a schema it does not know. The check runs at
    /// open only: a long-lived read-only `Store` keeps its connections after a newer owner
    /// migrates the database, so a process that outlives upgrades must reopen.
    /// # Errors
    /// Returns SQL errors for an absent/unreadable database, or
    /// `SchemaTooNew` for a newer schema.
    pub fn open_read_only(data_dir: &Path) -> Result<Self, StoreError> {
        Self::readers(data_dir, None)
    }
    fn readers(data_dir: &Path, writer: Option<WriterHandle>) -> Result<Self, StoreError> {
        let open = || -> Result<_, StoreError> {
            let db = Connection::open_with_flags(
                data_dir.join("crank.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let found: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if found > writer::SCHEMA_VERSION {
                return Err(StoreError::SchemaTooNew {
                    found,
                    supported: writer::SCHEMA_VERSION,
                });
            }
            db.pragma_update(None, "foreign_keys", true)?;
            Ok(Mutex::new(db))
        };
        Ok(Self {
            readers: [open()?, open()?],
            next_reader: AtomicUsize::new(0),
            writer,
        })
    }
    fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let i = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        let db = self.readers[i]
            .lock()
            .map_err(|_| StoreError::WorkerStopped)?;
        f(&db)
    }
    fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut writer::Writer) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        self.writer.as_ref().ok_or(StoreError::ReadOnly)?.call(f)
    }
    /// Current migration version.
    /// # Errors
    /// Returns connection/worker errors.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        self.read(|db| Ok(db.pragma_query_value(None, "user_version", |r| r.get(0))?))
    }
    /// Whether reader foreign-key enforcement is enabled.
    /// # Errors
    /// Returns connection/worker errors.
    pub fn foreign_keys_enabled(&self) -> Result<bool, StoreError> {
        self.read(|db| Ok(db.pragma_query_value(None, "foreign_keys", |r| r.get(0))?))
    }
    /// Read a named column by its ordered primary-key values. SQL stays here.
    /// # Errors
    /// Returns `InvalidJson`, `InvalidInput` for wrong key length, or read errors.
    pub fn load_json<T: JsonColumn>(&self, key: &[&str]) -> Result<Option<T>, StoreError> {
        if key.len() != T::KEYS.len() {
            return Err(StoreError::InvalidInput("incorrect JSON key length".into()));
        }
        let predicate = T::KEYS
            .iter()
            .map(|k| format!("{k} = ?"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("SELECT {} FROM {} WHERE {predicate}", T::COLUMN, T::TABLE);
        self.read(|db| {
            let text: Option<Option<String>> = db
                .query_row(&sql, rusqlite::params_from_iter(key), |r| r.get(0))
                .optional()?;
            text.flatten().map(|s| T::parse(&s)).transpose()
        })
    }
    /// Insert/update a validated setting.
    /// # Errors
    /// Returns read-only, SQL or worker errors.
    pub fn put_setting(&self, key: &str, value: SettingJson) -> Result<(), StoreError> {
        let key = key.to_owned();
        self.write(move |w| {
            w.db.execute(
                "INSERT INTO setting VALUES (?1,?2)
                 ON CONFLICT(key) DO UPDATE SET json=excluded.json",
                params![key, value.as_str()],
            )?;
            Ok(())
        })
    }
    /// Persist a (peripheral, role) pairing: platform id, advertised name and DIS
    /// serial + model. A row is unique per `(platform_id, kind)`, so one
    /// peripheral can be both trainer and strap and roles never overwrite each
    /// other. Re-remembering a role updates that row and **keeps its existing
    /// id**, which is returned (the caller's `id` is used only for a new row).
    /// An existing `id` of the same kind with a new platform id
    /// (`CoreBluetooth` re-assigned it) is updated in place.
    /// # Errors
    /// `InvalidInput` if `id` already names a row of a different kind; read-only,
    /// SQL or worker errors.
    pub fn remember_device(&self, device: Device) -> Result<String, StoreError> {
        self.write(move |w| {
            let tx = w.db.transaction()?;
            let Device { id, kind, name, platform_id, serial, model, last_seen } = device;
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM device WHERE platform_id=?1 AND kind=?2",
                    params![platform_id, kind],
                    |r| r.get(0),
                )
                .optional()?;
            let id = if let Some(existing) = existing {
                tx.execute(
                    "UPDATE device SET name=?1,serial=?2,model=?3,last_seen=?4 WHERE id=?5",
                    params![name, serial, model, last_seen, existing],
                )?;
                existing
            } else {
                let taken: Option<String> = tx
                    .query_row("SELECT kind FROM device WHERE id=?1", params![id], |r| r.get(0))
                    .optional()?;
                match taken {
                    Some(other) if other != kind => {
                        return Err(StoreError::InvalidInput(format!(
                            "device id {id:?} already names a {other} pairing"
                        )));
                    }
                    Some(_) => {
                        tx.execute(
                            "UPDATE device SET name=?1,serial=?2,model=?3,last_seen=?4,platform_id=?5
                             WHERE id=?6",
                            params![name, serial, model, last_seen, platform_id, id],
                        )?;
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO device (id,kind,name,platform_id,serial,model,last_seen)
                             VALUES (?1,?2,?3,?4,?5,?6,?7)",
                            params![id, kind, name, platform_id, serial, model, last_seen],
                        )?;
                    }
                }
                id
            };
            tx.commit()?;
            Ok(id)
        })
    }
    /// Read pairing memory ordered by id.
    /// # Errors
    /// Returns SQL or worker errors.
    pub fn devices(&self) -> Result<Vec<Device>, StoreError> {
        self.read(|db| {
            Ok(db
                .prepare(
                    "SELECT id,kind,name,platform_id,serial,model,last_seen FROM device ORDER BY id",
                )?
                .query_map([], |r| {
                    Ok(Device {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        name: r.get(2)?,
                        platform_id: r.get(3)?,
                        serial: r.get(4)?,
                        model: r.get(5)?,
                        last_seen: r.get(6)?,
                    })
                })?
                .collect::<Result<_, _>>()?)
        })
    }
    /// Start a ride; finish it before starting another.
    /// # Errors
    /// Returns `InvalidInput` for invalid metrics/live ride, or write errors.
    pub fn start_ride(&self, ride: NewRide) -> Result<(), StoreError> {
        self.write(move |w| w.start(ride))
    }
    /// Append all metrics at the next session second; persist every fifth second.
    /// Missing values are explicit `None`; pauses still consume session seconds.
    /// # Errors
    /// Returns `InvalidInput` for wrong ride/order/metrics/nonfinite values, or
    /// write errors. Failure leaves the buffered record unchanged.
    pub fn append_second(
        &self,
        ride_id: &str,
        session_s: u32,
        samples: Vec<Sample>,
    ) -> Result<(), StoreError> {
        let id = ride_id.to_owned();
        self.write(move |w| w.append(&id, session_s, samples))
    }
    /// Replace one metric's value of an already appended second of the live ride (the engine's
    /// `block_end`, known only after the block's last second, ADR-0009 §2.3.2). It is written
    /// at once, also when that second's chunk is already stored.
    /// # Errors
    /// Returns `InvalidInput` if the ride is not live, the second not appended yet, the metric
    /// not one of the ride's or the value not finite, or write errors.
    pub fn amend_second(
        &self,
        ride_id: &str,
        session_s: u32,
        metric: &str,
        value: Option<f32>,
    ) -> Result<(), StoreError> {
        let (id, metric) = (ride_id.to_owned(), metric.to_owned());
        self.write(move |w| w.amend(&id, session_s, &metric, value))
    }
    /// Atomically flush final partial chunks and close a ride as finished, ended by `cause`
    /// (`Completed`, `RiderEnd` or `Safety`; the other two belong to [`Store::abort_ride`] and
    /// crash recovery).
    /// # Errors
    /// Returns `InvalidInput` if not live, end precedes start, or `cause` is `Forced` or
    /// `Recovered`, or write errors.
    pub fn finish_ride(
        &self,
        ride_id: &str,
        ended_at: i64,
        summary: RideSummaryJson,
        cause: EndCause,
    ) -> Result<(), StoreError> {
        if matches!(cause, EndCause::Forced | EndCause::Recovered) {
            return Err(StoreError::InvalidInput(format!(
                "a finished ride cannot end as {}",
                cause.as_column()
            )));
        }
        let id = ride_id.to_owned();
        self.write(move |w| w.finish(&id, ended_at, &summary, cause))
    }
    /// Close the live ride as aborted at `ended_at`, keeping what was recorded (the buffered
    /// seconds are flushed) and writing no summary; its end cause is [`EndCause::Forced`]. For a ride whose engine can no longer
    /// finish it; the next ride may start afterwards.
    /// # Errors
    /// Returns `InvalidInput` if the ride is not live, or write errors.
    pub fn abort_ride(&self, ride_id: &str, ended_at: i64) -> Result<(), StoreError> {
        let id = ride_id.to_owned();
        self.write(move |w| w.abort(&id, ended_at))
    }
    /// Read metadata, validating every JSON snapshot.
    /// # Errors
    /// Returns `InvalidJson`, SQL or worker errors.
    pub fn ride(&self, id: &str) -> Result<Option<Ride>, StoreError> {
        self.read(|db| {
            db.query_row(&format!("{RIDE_SELECT} WHERE id=?1"), [id], ride_row)
                .optional()?
                .map(Ride::try_from_row)
                .transpose()
        })
    }
    /// Every ride, newest first, as a list projection: the workout's id and name and the
    /// summary, without parsing or returning the workout and compiled snapshots (use
    /// [`Store::ride`] for those). The id and name are read from `workout_json` by `SQLite`;
    /// the summary is validated.
    /// # Errors
    /// Returns `InvalidJson`, SQL or worker errors.
    pub fn rides(&self) -> Result<Vec<RideListing>, StoreError> {
        self.read(|db| {
            db.prepare(
                "SELECT id,started_at,ended_at,state,
                        json_extract(workout_json,'$.id'),json_extract(workout_json,'$.name'),
                        summary_json
                 FROM ride ORDER BY started_at DESC, id DESC",
            )?
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })?
            .map(|row| {
                let (id, started_at, ended_at, state, workout_id, workout_name, summary) = row?;
                let missing = || StoreError::InvalidJson {
                    table: "ride",
                    column: "workout_json",
                    reason: "missing id or name".into(),
                };
                Ok(RideListing {
                    id,
                    started_at,
                    ended_at,
                    state: RideState::from_column(state.as_deref())?,
                    workout_id: workout_id.ok_or_else(missing)?,
                    workout_name: workout_name.ok_or_else(missing)?,
                    summary: summary.map(|s| RideSummaryJson::parse(&s)).transpose()?,
                })
            })
            .collect()
        })
    }
    /// Forget a pairing. Rides that referenced it keep their telemetry but lose the
    /// device link (`trainer_device_id`/`hr_device_id` become NULL), so history never
    /// blocks forgetting. Returns whether a row existed. The caller must not forget a
    /// device during an active ride; `crank-core` guards that, the store does not know.
    /// # Errors
    /// Returns read-only, SQL or worker errors.
    pub fn forget_device(&self, id: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.write(move |w| {
            let tx = w.db.transaction()?;
            tx.execute(
                "UPDATE ride SET trainer_device_id=NULL WHERE trainer_device_id=?1",
                [&id],
            )?;
            tx.execute(
                "UPDATE ride SET hr_device_id=NULL WHERE hr_device_id=?1",
                [&id],
            )?;
            let deleted = tx.execute("DELETE FROM device WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(deleted > 0)
        })
    }
    /// Store a new route under `id` with the file it was imported from, if any (ADR-0005 §2.1).
    /// The original's SHA-256 is computed here, so it always matches its bytes.
    /// # Errors
    /// `InvalidInput` for a taken id or an original over 32 MiB; read-only, SQL or worker errors.
    pub fn put_route(
        &self,
        id: String,
        route: crank_domain::route::Route,
        original: Option<RouteOriginal>,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.write(move |w| route::insert(&w.db, &id, &route, original.as_ref(), created_at))
    }
    /// One route, bit for bit as stored, every ADR-0005 §2.1 invariant checked on reading.
    /// # Errors
    /// `CorruptRoute` or `InvalidJson` for a row that does not decode; SQL or worker errors.
    pub fn route(&self, id: &str) -> Result<Option<StoredRoute>, StoreError> {
        self.read(|db| route::select(db, id))
    }
    /// The bytes of the file a route was imported from; `None` for an unknown id or a route
    /// without a file (a grade profile).
    /// # Errors
    /// SQL or worker errors.
    pub fn route_original(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.read(|db| {
            Ok(db
                .query_row("SELECT original FROM route WHERE id=?1", [id], |r| {
                    r.get::<_, Option<Vec<u8>>>(0)
                })
                .optional()?
                .flatten())
        })
    }
    /// Every route, newest first, without points or file.
    /// # Errors
    /// `CorruptRoute` for a row that does not decode; SQL or worker errors.
    pub fn routes(&self) -> Result<Vec<RouteListing>, StoreError> {
        self.read(route::list)
    }
    /// Delete a route. Rides ridden on it stay readable: they keep `route_json` and lose only
    /// the link (ADR-0005 §2.1). Returns whether a row existed.
    /// # Errors
    /// Read-only, SQL or worker errors.
    pub fn delete_route(&self, id: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.write(move |w| route::delete(&w.db, &id))
    }
    /// The rider's workouts, oldest first (by first save, then id), each tree validated.
    /// # Errors
    /// `InvalidJson` for a row whose tree does not parse or names another id; SQL or worker errors.
    pub fn workouts(&self) -> Result<Vec<StoredWorkout>, StoreError> {
        self.read(workout::list)
    }
    /// One of the rider's workouts; `None` for an unknown id.
    /// # Errors
    /// `InvalidJson` for a row whose tree does not parse or names another id; SQL or worker errors.
    pub fn workout(&self, id: &str) -> Result<Option<StoredWorkout>, StoreError> {
        self.read(|db| workout::select(db, id))
    }
    /// Store a new workout under its tree's id, with its canonical DSL text.
    /// # Errors
    /// `InvalidInput` for a taken id; read-only, SQL or worker errors.
    pub fn insert_workout(
        &self,
        tree: WorkoutTreeJson,
        dsl: String,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.write(move |w| workout::insert(&w.db, &tree, &dsl, created_at))
    }
    /// Replace the workout with its tree's id (name, FTP, tree and text), keeping when it was
    /// first saved. Returns whether a row existed; nothing is written when none did.
    /// # Errors
    /// Read-only, SQL or worker errors.
    pub fn update_workout(&self, tree: WorkoutTreeJson, dsl: String) -> Result<bool, StoreError> {
        self.write(move |w| workout::update(&w.db, &tree, &dsl))
    }
    /// Delete one of the rider's workouts. Rides of it stay readable: each keeps its own copy
    /// of the workout (`ride.workout_json`). Returns whether a row existed.
    /// # Errors
    /// Read-only, SQL or worker errors.
    pub fn delete_workout(&self, id: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.write(move |w| workout::delete(&w.db, &id))
    }
    /// Append athlete-model rows through `origin`, the only way to write an estimate (ADR-0009 §2.2.4 "Never
    /// silent"), and return them with their ids: one row, or one per value of an accepted proposal. Every row passes
    /// `crank_domain::athlete::check_entry` against the stored model; in the same transaction, a path other than
    /// accept marks the parameter's open proposals `superseded`, and accept marks its proposal `accepted`.
    /// # Errors
    /// `Athlete` for a row the domain refuses; `InvalidInput` for a revert to a prior or cleared row, an onboarding
    /// prior of a parameter with none, an import without a method, or accepted values that do not match the
    /// proposal; `NotFound` for an unknown estimate or proposal; `Conflict` for a proposal that is not open or
    /// was raised before the current row of one of its parameters;
    /// read-only, SQL or worker errors.
    pub fn append_athlete(
        &self,
        origin: EstimateOrigin,
        now: i64,
    ) -> Result<Vec<Entry>, StoreError> {
        self.write(move |w| {
            let tx = w.db.transaction()?;
            let entries = athlete::append(&tx, origin, now)?;
            tx.commit()?;
            Ok(entries)
        })
    }
    /// The newest athlete-model row of each parameter that has one (ADR-0009 §2.1.1), in
    /// `AthleteParam::ALL`'s order.
    /// # Errors
    /// `CorruptAthlete` for a row that does not decode; SQL or worker errors.
    pub fn athlete_current(&self) -> Result<Vec<Entry>, StoreError> {
        self.read(athlete::current)
    }
    /// The athlete model as stored: each parameter's newest row, else its prior (HR max's from
    /// `athlete_profile.age`), else unset (`AthleteSnapshot::resolve`).
    /// # Errors
    /// As [`Store::athlete_current`], and `CorruptAthlete` for a negative stored age.
    pub fn athlete_snapshot(&self) -> Result<AthleteSnapshot, StoreError> {
        self.read(athlete::snapshot)
    }
    /// Every row of `param`, newest first; a cleared row is an `Entry::Cleared`.
    /// # Errors
    /// As [`Store::athlete_current`].
    pub fn athlete_history(&self, param: AthleteParam) -> Result<Vec<Entry>, StoreError> {
        self.read(|db| athlete::history(db, param))
    }
    /// One athlete-model row; `None` for an unknown id.
    /// # Errors
    /// As [`Store::athlete_current`].
    pub fn athlete_entry(&self, id: i64) -> Result<Option<Entry>, StoreError> {
        self.read(|db| athlete::entry(db, id))
    }
    /// Raise a proposal (stored `open`) and return its id. The raise rules (ADR-0009 §2.2.4) are the caller's.
    /// # Errors
    /// `InvalidInput` for an empty method, generation 0, or values that are not one or two of different
    /// parameters; `Athlete` for a value the domain refuses (`check_value`); `NotFound` for an unknown ride;
    /// `Conflict` when the ride already has that generation of the method; read-only, SQL or worker errors.
    pub fn insert_proposal(&self, proposal: NewProposal) -> Result<i64, StoreError> {
        self.write(move |w| {
            let tx = w.db.transaction()?;
            let id = athlete::insert_proposal(&tx, &proposal)?;
            tx.commit()?;
            Ok(id)
        })
    }
    /// One proposal, whatever its status; `None` for an unknown id.
    /// # Errors
    /// `InvalidJson` or `CorruptAthlete` for a row that does not decode; SQL or worker errors.
    pub fn proposal(&self, id: i64) -> Result<Option<Proposal>, StoreError> {
        self.read(|db| athlete::proposal(db, id))
    }
    /// Every open proposal, oldest first.
    /// # Errors
    /// As [`Store::proposal`].
    pub fn open_proposals(&self) -> Result<Vec<Proposal>, StoreError> {
        self.read(athlete::open_proposals)
    }
    /// The open proposals with a value for `param`, oldest first; an indexed query (ADR-0009 §2.5).
    /// # Errors
    /// As [`Store::proposal`].
    pub fn open_proposals_for(&self, param: AthleteParam) -> Result<Vec<Proposal>, StoreError> {
        self.read(|db| athlete::open_proposals_for(db, param))
    }
    /// Close an open proposal at `now` without accepting it (accepting is
    /// [`EstimateOrigin::ProposalAccept`]).
    /// # Errors
    /// `NotFound` for an unknown id; `Conflict` when it is not open; read-only, SQL or worker errors.
    pub fn close_proposal(
        &self,
        id: i64,
        close: ProposalClose,
        now: i64,
    ) -> Result<(), StoreError> {
        self.write(move |w| {
            let tx = w.db.transaction()?;
            athlete::close_proposal(&tx, id, close, now)?;
            tx.commit()?;
            Ok(())
        })
    }
    /// The whole record of ride `id`: its version and every metric's values by session second;
    /// `None` without such a ride.
    /// # Errors
    /// Returns `CorruptChunk` for invalid binary data or chunks that do not follow each other
    /// from second 0 (a full 60 s chunk before each next one), or read errors.
    pub fn record(&self, id: &str) -> Result<Option<StoredRecord>, StoreError> {
        self.read(|db| {
            // One read transaction: a live ride's writer cannot move the chunks between reads.
            let tx = db.unchecked_transaction()?;
            let Some(record_version) = tx
                .query_row("SELECT record_version FROM ride WHERE id=?1", [id], |r| {
                    r.get::<_, u32>(0)
                })
                .optional()?
            else {
                return Ok(None);
            };
            let mut stmt = tx.prepare(
                "SELECT metric,start_s,n,data,estimated FROM ride_chunk
                 WHERE ride_id=?1 ORDER BY metric,start_s",
            )?;
            let rows = stmt.query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u8>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            })?;
            let mut metrics: BTreeMap<String, Vec<Option<f32>>> = BTreeMap::new();
            for row in rows {
                let (name, start, n, data, estimated) = row?;
                let chunk = writer::decode(start, usize::from(n), &data, &estimated)?;
                let values = metrics.entry(name.clone()).or_default();
                if usize::try_from(chunk.start_s).ok() != Some(values.len()) {
                    return Err(StoreError::CorruptChunk(format!(
                        "{name}: chunk at {} s after {} s",
                        chunk.start_s,
                        values.len()
                    )));
                }
                values.extend(chunk.values);
            }
            Ok(Some(StoredRecord {
                record_version,
                metrics,
            }))
        })
    }
    /// The chunks of one metric of ride `ride_id`, in order.
    /// # Errors
    /// Returns `CorruptChunk` for invalid binary data, or read errors.
    pub fn chunks(&self, ride_id: &str, metric: &str) -> Result<Vec<Chunk>, StoreError> {
        self.read(|db| {
            let mut stmt = db.prepare(
                "SELECT start_s,n,data,estimated FROM ride_chunk
                 WHERE ride_id=?1 AND metric=?2 ORDER BY start_s",
            )?;
            let rows = stmt.query_map(params![ride_id, metric], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u8>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            })?;
            rows.map(|row| {
                let (start, n, data, estimated) = row?;
                writer::decode(start, usize::from(n), &data, &estimated)
            })
            .collect()
        })
    }
}

const RIDE_SELECT: &str = "SELECT id,started_at,ended_at,state,workout_json,compiled_json,summary_json,athlete_json,end_cause,record_version FROM ride";
type RideRow = (
    String,
    i64,
    Option<i64>,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    u32,
);
fn ride_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RideRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
    ))
}
impl Ride {
    fn try_from_row(
        (
            id,
            started_at,
            ended_at,
            state,
            workout,
            compiled,
            summary,
            athlete,
            end_cause,
            record_version,
        ): RideRow,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            id,
            started_at,
            ended_at,
            state: RideState::from_column(state.as_deref())?,
            workout: RideWorkoutJson::parse(&workout)?,
            compiled: CompiledJson::parse(&compiled)?,
            athlete: athlete.map(|a| RideAthleteJson::parse(&a)).transpose()?,
            summary: summary.map(|s| RideSummaryJson::parse(&s)).transpose()?,
            end_cause: EndCause::from_column(end_cause.as_deref())?,
            record_version,
        })
    }
}
