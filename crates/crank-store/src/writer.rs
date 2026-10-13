//! Writer, advisory lock and binary codec (ADR-0001 §6).
use crate::{Chunk, JsonColumn, NewRide, RideSummaryJson, Sample, StoreError};
use rusqlite::{Connection, Transaction, params};
use rusqlite_migration::{M, Migrations};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};
type Job = Box<dyn FnOnce(&mut Writer) + Send>;
pub(crate) struct WriterHandle {
    sender: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
}
impl WriterHandle {
    pub(crate) fn open(dir: &Path) -> Result<Self, StoreError> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("ride.lock"))?;
        lock.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => StoreError::LockBusy,
            TryLockError::Error(e) => StoreError::Io(e),
        })?;
        let path = dir.join("crank.db");
        let (sender, receiver) = mpsc::sync_channel::<Job>(64);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("crank-store-writer".into())
            .spawn(move || {
                let init = || -> Result<Writer, StoreError> {
                    let mut db = Connection::open(path)?;
                    db.pragma_update(None, "journal_mode", "WAL")?;
                    db.pragma_update(None, "synchronous", "FULL")?;
                    db.pragma_update(None, "foreign_keys", true)?;
                    Migrations::new(vec![M::up(include_str!("../migrations/0001_initial.sql"))])
                        .to_latest(&mut db)?;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| StoreError::InvalidInput(e.to_string()))?;
                    let now = i64::try_from(now.as_secs())
                        .map_err(|e| StoreError::InvalidInput(e.to_string()))?;
                    // Only a newly acquired lock owner may close unfinished rides.
                    db.execute(
                        "UPDATE ride SET ended_at=?1,state='aborted' WHERE ended_at IS NULL",
                        [now],
                    )?;
                    Ok(Writer {
                        db,
                        active: None,
                        _lock: lock,
                    })
                };
                match init() {
                    Ok(mut writer) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            for job in receiver {
                                job(&mut writer);
                            }
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })?;
        let handle = Self {
            sender: Some(sender),
            thread: Some(thread),
        };
        ready_rx.recv().map_err(|_| StoreError::WorkerStopped)??;
        Ok(handle)
    }
    pub(crate) fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Writer) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .ok_or(StoreError::WorkerStopped)?
            .send(Box::new(move |w| {
                let _ = sender.send(f(w));
            }))
            .map_err(|_| StoreError::WorkerStopped)?;
        receiver.recv().map_err(|_| StoreError::WorkerStopped)?
    }
}
impl Drop for WriterHandle {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(crate) struct Writer {
    pub(crate) db: Connection,
    active: Option<Active>,
    _lock: File,
}
struct Active {
    id: String,
    started_at: i64,
    next_s: u32,
    metrics: BTreeMap<String, Vec<(Option<f32>, bool)>>,
}
impl Writer {
    pub(crate) fn start(&mut self, ride: NewRide) -> Result<(), StoreError> {
        let metrics: BTreeMap<_, _> = ride
            .metrics
            .iter()
            .map(|m| (m.clone(), Vec::with_capacity(60)))
            .collect();
        if self.active.is_some()
            || ride.id.is_empty()
            || metrics.is_empty()
            || metrics.len() != ride.metrics.len()
            || metrics.keys().any(String::is_empty)
        {
            return Err(StoreError::InvalidInput(
                "live ride, empty id, or invalid metric set".into(),
            ));
        }
        self.db.execute(
            "INSERT INTO ride(id,started_at,workout_json,compiled_json,
             trainer_device_id,hr_device_id,telemetry_source)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                ride.id,
                ride.started_at,
                ride.workout.as_str(),
                ride.compiled.as_str(),
                ride.trainer_device_id,
                ride.hr_device_id,
                ride.telemetry_source
            ],
        )?;
        self.active = Some(Active {
            id: ride.id,
            started_at: ride.started_at,
            next_s: 0,
            metrics,
        });
        Ok(())
    }
    pub(crate) fn append(
        &mut self,
        id: &str,
        second: u32,
        samples: Vec<Sample>,
    ) -> Result<(), StoreError> {
        let active = self
            .active
            .as_mut()
            .filter(|a| a.id == id)
            .ok_or_else(|| StoreError::InvalidInput("ride is not live".into()))?;
        let mut sorted = samples;
        sorted.sort_by(|a, b| a.metric.cmp(&b.metric));
        if second != active.next_s
            || second == u32::MAX
            || sorted.len() != active.metrics.len()
            || !sorted.iter().map(|s| &s.metric).eq(active.metrics.keys())
            || sorted
                .iter()
                .any(|s| s.value.is_some_and(|v| !v.is_finite()))
        {
            return Err(StoreError::InvalidInput(
                "expected next session second and complete, finite metric set".into(),
            ));
        }
        for (sample, values) in sorted.iter().zip(active.metrics.values_mut()) {
            values.push((sample.value, sample.estimated));
        }
        active.next_s += 1;
        if active.next_s.is_multiple_of(5) {
            let result = (|| {
                let tx = self.db.transaction()?;
                flush(&tx, active)?;
                tx.commit()?;
                Ok::<_, StoreError>(())
            })();
            if let Err(e) = result {
                for values in active.metrics.values_mut() {
                    values.pop();
                }
                active.next_s -= 1;
                return Err(e);
            }
        }
        if active.next_s.is_multiple_of(60) {
            for values in active.metrics.values_mut() {
                values.clear();
            }
        }
        Ok(())
    }
    pub(crate) fn finish(
        &mut self,
        id: &str,
        ended: i64,
        summary: &RideSummaryJson,
    ) -> Result<(), StoreError> {
        let active = self
            .active
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or_else(|| StoreError::InvalidInput("ride is not live".into()))?;
        if ended < active.started_at {
            return Err(StoreError::InvalidInput("end precedes start".into()));
        }
        let tx = self.db.transaction()?;
        flush(&tx, active)?;
        tx.execute(
            "UPDATE ride SET ended_at=?1,state='finished',summary_json=?2 WHERE id=?3",
            params![ended, summary.as_str(), id],
        )?;
        tx.commit()?;
        self.active = None;
        Ok(())
    }
}
fn flush(tx: &Transaction<'_>, active: &Active) -> Result<(), StoreError> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO ride_chunk VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(ride_id,metric,start_s) DO UPDATE
         SET n=excluded.n,data=excluded.data,estimated=excluded.estimated",
    )?;
    for (metric, values) in &active.metrics {
        if values.is_empty() {
            continue;
        }
        let mut data = Vec::with_capacity(values.len() * 4);
        let mut estimated = vec![0_u8; values.len().div_ceil(8)];
        for (i, (value, flag)) in values.iter().enumerate() {
            data.extend_from_slice(&value.unwrap_or(f32::NAN).to_le_bytes());
            if *flag {
                estimated[i / 8] |= 1 << (i % 8);
            }
        }
        stmt.execute(params![
            active.id,
            metric,
            (active.next_s - 1) / 60 * 60,
            u8::try_from(values.len()).map_err(|e| StoreError::InvalidInput(e.to_string()))?,
            data,
            estimated
        ])?;
    }
    Ok(())
}
pub(crate) fn decode(
    start_s: u32,
    n: usize,
    data: &[u8],
    estimated: &[u8],
) -> Result<Chunk, StoreError> {
    let invalid = || StoreError::CorruptChunk("invalid alignment, count, payload, or mask".into());
    if !start_s.is_multiple_of(60)
        || !(1..=60).contains(&n)
        || data.len() != n * 4
        || estimated.len() != n.div_ceil(8)
    {
        return Err(invalid());
    }
    if !n.is_multiple_of(8) && estimated[n / 8] >> (n % 8) != 0 {
        return Err(invalid());
    }
    let values = data
        .chunks_exact(4)
        .map(|b| {
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if v.is_infinite() {
                Err(invalid())
            } else {
                Ok((!v.is_nan()).then_some(v))
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(Chunk {
        start_s,
        values,
        estimated: (0..n)
            .map(|i| estimated[i / 8] & (1 << (i % 8)) != 0)
            .collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn arbitrary_float_bits_preserved(bits in proptest::collection::vec(any::<u32>(),1..=60)) {
            let values:Vec<_> = bits.into_iter().map(f32::from_bits).filter(|v| !v.is_infinite()).collect();
            prop_assume!(!values.is_empty());
            let data:Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            let decoded = decode(0,values.len(),&data,&vec![0;values.len().div_ceil(8)]).unwrap();
            for (actual,expected) in decoded.values.iter().zip(values) { prop_assert_eq!(actual.map(f32::to_bits),(!expected.is_nan()).then_some(expected.to_bits())); }
        }
    }
    #[test]
    fn malformed_chunks_are_typed_errors() {
        for (start, n, data, mask) in [
            (1, 1, vec![0; 4], vec![0]),
            (0, 0, vec![], vec![]),
            (0, 61, vec![0; 244], vec![0; 8]),
            (0, 1, vec![0; 3], vec![0]),
            (0, 1, vec![0; 4], vec![2]),
            (0, 1, f32::INFINITY.to_le_bytes().to_vec(), vec![0]),
        ] {
            assert!(matches!(
                decode(start, n, &data, &mask),
                Err(StoreError::CorruptChunk(_))
            ));
        }
    }
}
