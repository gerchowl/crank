//! Exact `.fecap` format from crank-fec/tests/fixtures/README.md (#141).
use crank_devices::MonoTime;
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    sync::Mutex,
};
/// Per-ride raw capture, preserving complete payloads including malformed bytes.
pub struct Capture(Mutex<(File, MonoTime, u64)>);
impl Capture {
    /// Create a new capture without overwriting an existing ride.
    /// # Errors
    /// Returns filesystem errors.
    pub fn create(path: &Path, start: MonoTime) -> io::Result<Self> {
        let mut file = File::create_new(path)?;
        writeln!(file, "# elapsed_us direction complete_frame_hex")?;
        Ok(Self(Mutex::new((file, start, 0))))
    }
    /// Append a complete payload in arrival order; flush for crash diagnostics.
    /// # Errors
    /// Rejects timestamps before the previous record or poisoned/file errors.
    pub fn record(&self, at: MonoTime, rx: bool, bytes: &[u8]) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("capture lock poisoned"))?;
        append(&mut state, at, rx, bytes)
    }
    pub(crate) fn record_current(
        &self,
        clock: &dyn crate::Clock,
        rx: bool,
        bytes: &[u8],
    ) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("capture lock poisoned"))?;
        append(&mut state, clock.now(), rx, bytes)
    }
}
fn append(
    state: &mut (File, MonoTime, u64),
    at: MonoTime,
    rx: bool,
    bytes: &[u8],
) -> io::Result<()> {
    let elapsed =
        at.0.checked_sub(state.1.0)
            .filter(|t| *t >= state.2)
            .ok_or_else(|| io::Error::other("capture time out of order"))?;
    write!(state.0, "{elapsed} {}", if rx { "rx" } else { "tx" })?;
    for byte in bytes {
        write!(state.0, " {byte:02X}")?;
    }
    writeln!(state.0)?;
    state.0.flush()?;
    state.2 = elapsed;
    Ok(())
}
