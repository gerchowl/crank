//! Exact `.fecap` format from crank-fec/tests/fixtures/README.md (#141).
use crank_devices::MonoTime;
use std::{
    fmt::Write as _,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
const HEADER: &str = "# elapsed_us direction complete_frame_hex\n";
/// Per-ride raw capture, preserving complete payloads including malformed bytes.
pub struct Capture {
    state: Mutex<(File, MonoTime)>,
    path: PathBuf,
    warned: AtomicBool,
}
impl Capture {
    /// Create a new capture without overwriting an existing ride.
    /// # Errors
    /// Returns filesystem errors.
    pub fn create(path: &Path, start: MonoTime) -> io::Result<Self> {
        let mut file = File::create_new(path)?;
        file.write_all(HEADER.as_bytes())?;
        Ok(Self {
            state: Mutex::new((file, start)),
            path: path.into(),
            warned: AtomicBool::new(false),
        })
    }
    /// Append a complete payload in processing order. RX arrival stamps can precede a TX
    /// processed earlier. Replay stably sorts by stamp, retaining file order for ties,
    /// without altering arrival times.
    ///
    /// The record is formatted first and written with one unbuffered `write` (the file is
    /// not flushed separately: there is no user-space buffer, so a crash loses nothing the
    /// kernel has accepted). That write is a blocking syscall on the caller's task, about
    /// 50 bytes into the page cache; on a stalled disk it blocks the BLE reader task.
    /// # Errors
    /// Rejects timestamps before the capture epoch or poisoned/file errors.
    pub fn record(&self, at: MonoTime, rx: bool, bytes: &[u8]) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture lock poisoned"))?;
        append(&mut state, at, rx, bytes)
    }
    /// Append a notification of another characteristic as a comment line
    /// `# <tag> <elapsed_us> <hex bytes>`, in processing order with the frames (#412: the Cycling
    /// Power measurement beside FE-C in the `hil_fec` check 5). Readers that skip comments (the
    /// replay and `crank-fec`'s fixtures) read the file exactly as before.
    /// # Errors
    /// As [`Capture::record`].
    pub fn record_aux(&self, at: MonoTime, tag: &str, bytes: &[u8]) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("capture lock poisoned"))?;
        let elapsed =
            at.0.checked_sub(state.1.0)
                .ok_or_else(|| io::Error::other("capture time before epoch"))?;
        let mut line = format!("# {tag} {elapsed}");
        for byte in bytes {
            // Writing to a String cannot fail.
            let _ = write!(line, " {byte:02X}");
        }
        line.push('\n');
        state.0.write_all(line.as_bytes())
    }
    /// Log the first write failure of this capture (and only the first: a full disk would
    /// fail every frame). That is once per capture, hence once per trainer connection (each
    /// connect, reconnects included, has its own file), not once per process. The ride goes on without the capture; nothing else tells the
    /// rider that the file stopped growing.
    pub(crate) fn warn_failed(&self, error: &dyn std::fmt::Display) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(path = %self.path.display(), %error, "trainer capture write failed; this capture stops growing");
        }
    }
}
fn append(state: &mut (File, MonoTime), at: MonoTime, rx: bool, bytes: &[u8]) -> io::Result<()> {
    let elapsed =
        at.0.checked_sub(state.1.0)
            .ok_or_else(|| io::Error::other("capture time before epoch"))?;
    let mut line = format!("{elapsed} {}", if rx { "rx" } else { "tx" });
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(line, " {byte:02X}");
    }
    line.push('\n');
    state.0.write_all(line.as_bytes())
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
    /// Remove a capture that holds only its header: a connection that failed before any
    /// frame leaves no evidence worth a file, and the app's own file is the app's to remove.
    /// A capture with frames is kept (a connection that then fails verification is exactly
    /// what the rider wants to read). A connection dropped mid-setup is not covered.
    pub fn discard_if_empty(path: &Path) {
        let empty = std::fs::metadata(path).is_ok_and(|m| m.len() == HEADER.len() as u64);
        if empty && let Err(error) = std::fs::remove_file(path) {
            tracing::warn!(path = %path.display(), %error, "empty trainer capture not removed");
        }
    }
}
