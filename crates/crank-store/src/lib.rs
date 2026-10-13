//! `SQLite` persistence (ADR-0001 §2, §5–6; ADR-0002 §5).
//! Owners acquire `ride.lock` before migrating/recovering on one writer thread.
//! Query processes use read-only WAL connections and never recover rides.
//! Coordinates are session seconds; timestamps are UTC Unix seconds.
mod json;
mod writer;
pub use json::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
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
    /// Writer terminated or reader mutex poisoned.
    #[error("store worker unavailable")]
    WorkerStopped,
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
    /// Paired trainer id, if any.
    pub trainer_device_id: Option<String>,
    /// Paired HR id, if any.
    pub hr_device_id: Option<String>,
    /// Telemetry provenance, e.g. `sim` or `fec`.
    pub telemetry_source: String,
    /// Engine metric ids, including `workout_s` when provided.
    pub metrics: Vec<String>,
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
    /// `finished` or `aborted`; absent while live.
    pub state: Option<String>,
    /// Workout snapshot.
    pub workout: RideWorkoutJson,
    /// Compiled snapshot.
    pub compiled: CompiledJson,
    /// Optional summary.
    pub summary: Option<RideSummaryJson>,
}
/// Pairing memory only; discovery and pairing policy belong to #147.
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
    /// # Errors
    /// Returns SQL errors for an absent/unreadable database.
    pub fn open_read_only(data_dir: &Path) -> Result<Self, StoreError> {
        Self::readers(data_dir, None)
    }
    fn readers(data_dir: &Path, writer: Option<WriterHandle>) -> Result<Self, StoreError> {
        let open = || -> Result<_, StoreError> {
            let db = Connection::open_with_flags(
                data_dir.join("crank.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
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
    /// Persist platform id, advertised name and DIS serial.
    /// # Errors
    /// Returns read-only, SQL or worker errors.
    pub fn remember_device(&self, device: Device) -> Result<(), StoreError> {
        self.write(move |w| {
            w.db.execute(
                "INSERT INTO device VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,name=excluded.name,
                 platform_id=excluded.platform_id,serial=excluded.serial,last_seen=excluded.last_seen",
                params![device.id, device.kind, device.name, device.platform_id,
                    device.serial, device.last_seen],
            )?;
            Ok(())
        })
    }
    /// Read pairing memory ordered by id.
    /// # Errors
    /// Returns SQL or worker errors.
    pub fn devices(&self) -> Result<Vec<Device>, StoreError> {
        self.read(|db| {
            Ok(db
                .prepare(
                    "SELECT id,kind,name,platform_id,serial,last_seen FROM device ORDER BY id",
                )?
                .query_map([], |r| {
                    Ok(Device {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        name: r.get(2)?,
                        platform_id: r.get(3)?,
                        serial: r.get(4)?,
                        last_seen: r.get(5)?,
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
    /// Atomically flush final partial chunks and close a ride as finished.
    /// # Errors
    /// Returns `InvalidInput` if not live or end precedes start, or write errors.
    pub fn finish_ride(
        &self,
        ride_id: &str,
        ended_at: i64,
        summary: RideSummaryJson,
    ) -> Result<(), StoreError> {
        let id = ride_id.to_owned();
        self.write(move |w| w.finish(&id, ended_at, &summary))
    }
    /// Read metadata, validating every JSON snapshot.
    /// # Errors
    /// Returns `InvalidJson`, SQL or worker errors.
    pub fn ride(&self, id: &str) -> Result<Option<Ride>, StoreError> {
        self.read(|db| {
            let row = db
                .query_row(
                    "SELECT id,started_at,ended_at,state,workout_json,compiled_json,summary_json
                 FROM ride WHERE id=?1",
                    [id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                            r.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .optional()?;
            row.map(
                |(id, started_at, ended_at, state, workout, compiled, summary)| {
                    Ok(Ride {
                        id,
                        started_at,
                        ended_at,
                        state,
                        workout: RideWorkoutJson::parse(&workout)?,
                        compiled: CompiledJson::parse(&compiled)?,
                        summary: summary.map(|s| RideSummaryJson::parse(&s)).transpose()?,
                    })
                },
            )
            .transpose()
        })
    }
    /// Decode persisted metric chunks ordered by session second.
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
