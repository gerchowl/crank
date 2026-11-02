//! Exact `.fecap` format from crank-fec/tests/fixtures/README.md (#141).
use crank_devices::MonoTime;
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
/// Per-ride raw capture, preserving complete payloads including malformed bytes.
pub struct Capture(Mutex<(File, MonoTime)>);
impl Capture {
    /// Create a new capture without overwriting an existing ride.
    /// # Errors
    /// Returns filesystem errors.
    pub fn create(path: &Path, start: MonoTime) -> io::Result<Self> {
        let mut file = File::create_new(path)?;
        writeln!(file, "# elapsed_us direction complete_frame_hex")?;
        Ok(Self(Mutex::new((file, start))))
    }
    /// Append a complete payload in processing order; flush for crash diagnostics.
    /// RX arrival stamps can precede a TX processed earlier. Replay stably sorts
    /// by stamp, retaining file order for ties, without altering arrival times.
    /// # Errors
    /// Rejects timestamps before the capture epoch or poisoned/file errors.
    pub fn record(&self, at: MonoTime, rx: bool, bytes: &[u8]) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("capture lock poisoned"))?;
        append(&mut state, at, rx, bytes)
    }
}
fn append(state: &mut (File, MonoTime), at: MonoTime, rx: bool, bytes: &[u8]) -> io::Result<()> {
    let elapsed =
        at.0.checked_sub(state.1.0)
            .ok_or_else(|| io::Error::other("capture time before epoch"))?;
    write!(state.0, "{elapsed} {}", if rx { "rx" } else { "tx" })?;
    for byte in bytes {
        write!(state.0, " {byte:02X}")?;
    }
    writeln!(state.0)?;
    state.0.flush()?;
    Ok(())
}

/// A directory that receives one `.fecap` per trainer connection (#411), named
/// `trainer-<unix seconds>-<n>.fecap`. The files hold raw device frames: see
/// [`Capture`] and AGENTS.md for what they identify.
#[derive(Clone, Debug)]
pub struct CaptureDir(PathBuf);
impl CaptureDir {
    /// Captures go into `dir`, which is created on the first capture.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }
    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
    /// Start the capture of one connection, with `start` as its epoch. Never overwrites
    /// a file: a name taken by an earlier connection (or process) moves on to the next `n`.
    /// # Errors
    /// Returns filesystem errors.
    pub fn create(&self, start: MonoTime) -> io::Result<(PathBuf, Capture)> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::fs::create_dir_all(&self.0)?;
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = self.0.join(format!("trainer-{secs}-{n}.fecap"));
            match Capture::create(&path, start) {
                Ok(capture) => return Ok((path, capture)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }
}
