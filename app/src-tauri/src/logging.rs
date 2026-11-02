//! The desktop process's file log and raw-frame capture (#411).
//!
//! A Finder launch has no stderr, so the reconnect events of `crank-ble` (and everything
//! else the process logs) would be lost. The subscriber is installed here, in the desktop
//! entry point, never in a library crate.
//!
//! * `CRANK_CAPTURE_DIR` set: `<dir>/crank.log` at INFO, TRACE for `crank_ble`, and the
//!   BLE central writes a `.fecap` per trainer connection into `<dir>` (the format
//!   `hil_fec analyse` and `redact` read).
//! * Unset: `crank.log` in the platform log directory (`~/Library/Logs/<identifier>/` on
//!   macOS) at INFO, and no capture.
//!
//! The file rotates at [`MAX_LOG_BYTES`] and keeps one previous generation, so a log never
//! occupies more than twice that.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{Layer, filter::Targets, layer::SubscriberExt};

/// Size at which `crank.log` moves to `crank.log.1`.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
/// The log file's name in its directory.
pub const LOG_FILE: &str = "crank.log";

/// Where this process logs and what it captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPlan {
    /// Directory of [`LOG_FILE`].
    pub log_dir: PathBuf,
    /// Whether `crank_ble` is enabled at TRACE and trainer frames are captured into `log_dir`.
    pub capture: bool,
}

impl LogPlan {
    /// The capture directory when given, else the platform's `default_log_dir`.
    #[must_use]
    pub fn new(capture_dir: Option<&Path>, default_log_dir: PathBuf) -> Self {
        match capture_dir {
            Some(dir) => Self {
                log_dir: dir.into(),
                capture: true,
            },
            None => Self {
                log_dir: default_log_dir,
                capture: false,
            },
        }
    }
    fn filter(&self) -> Targets {
        let targets = Targets::new().with_default(LevelFilter::INFO);
        if self.capture {
            targets.with_target("crank_ble", LevelFilter::TRACE)
        } else {
            targets
        }
    }
    /// Build the subscriber writing to the log file (not yet installed).
    ///
    /// # Errors
    /// The log directory or file cannot be created.
    pub fn subscriber(&self) -> io::Result<impl tracing::Subscriber + Send + Sync> {
        let file = RotatingFile::open(self.log_dir.join(LOG_FILE), MAX_LOG_BYTES)?;
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .with_filter(self.filter());
        Ok(tracing_subscriber::registry().with(layer))
    }
    /// Install the subscriber process-wide.
    ///
    /// # Errors
    /// The log file cannot be created, or a subscriber is already installed.
    pub fn install(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::subscriber::set_global_default(self.subscriber()?)?;
        Ok(())
    }
}

/// An append-only log file that moves to `<path>.1` (replacing it) when a write would
/// pass `max` bytes. If the move fails it keeps appending to the current file (a log that
/// grows is better than a log that stops) and tells `report` once, never silently.
struct RotatingFile {
    path: PathBuf,
    max: u64,
    file: File,
    len: u64,
    report: Box<dyn FnMut(&io::Error) + Send>,
    reported: bool,
}

impl RotatingFile {
    fn open(path: PathBuf, max: u64) -> io::Result<Self> {
        Self::open_reporting(
            path,
            max,
            Box::new(|e| eprintln!("crank: log rotation failed, the log keeps growing: {e}")),
        )
    }
    fn open_reporting(
        path: PathBuf,
        max: u64,
        report: Box<dyn FnMut(&io::Error) + Send>,
    ) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path,
            max,
            file,
            len,
            report,
            reported: false,
        })
    }
    fn rotate(&mut self) -> io::Result<()> {
        let mut old = self.path.clone().into_os_string();
        old.push(".1");
        fs::rename(&self.path, old)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.len = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.len > 0
            && self.len + buf.len() as u64 > self.max
            && let Err(error) = self.rotate()
            && !std::mem::replace(&mut self.reported, true)
        {
            (self.report)(&error);
        }
        let n = self.file.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capture_dir_holds_log_and_captures_and_turns_trace_on() {
        let plan = LogPlan::new(Some(Path::new("/cap")), "/logs".into());
        assert_eq!(plan.log_dir, Path::new("/cap"));
        assert!(plan.capture);
        assert!(
            plan.filter()
                .would_enable("crank_ble::driver", &tracing::Level::TRACE)
        );
        assert!(
            !plan
                .filter()
                .would_enable("crank_core", &tracing::Level::DEBUG)
        );
    }

    #[test]
    fn without_it_the_platform_log_dir_at_info_and_no_capture() {
        let plan = LogPlan::new(None, "/logs".into());
        assert_eq!(plan.log_dir, Path::new("/logs"));
        assert!(!plan.capture);
        assert!(
            plan.filter()
                .would_enable("crank_ble::reconnect", &tracing::Level::INFO)
        );
        assert!(
            !plan
                .filter()
                .would_enable("crank_ble::driver", &tracing::Level::TRACE)
        );
    }

    #[test]
    fn a_rotation_that_fails_is_reported_once_and_the_log_keeps_growing() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `crank.log.1` is a non-empty directory: renaming the log onto it fails.
        let blocker = dir.path().join("crank.log.1");
        fs::create_dir_all(blocker.join("x")).expect("dir");
        let reports = std::sync::Arc::new(Mutex::new(0));
        let counted = reports.clone();
        let mut log = RotatingFile::open_reporting(
            dir.path().join("crank.log"),
            10,
            Box::new(move |_| *counted.lock().expect("lock") += 1),
        )
        .expect("open");
        for line in ["aaaaaa\n", "bbbbbb\n", "cccccc\n"] {
            log.write_all(line.as_bytes()).expect("write");
        }
        let all = fs::read_to_string(dir.path().join("crank.log")).expect("read");
        assert_eq!(all, "aaaaaa\nbbbbbb\ncccccc\n");
        assert_eq!(*reports.lock().expect("lock"), 1);
    }

    #[test]
    fn the_log_rotates_once_and_stays_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = RotatingFile::open(dir.path().join("crank.log"), 10).expect("open");
        for line in ["aaaaaa\n", "bbbbbb\n", "cccccc\n"] {
            log.write_all(line.as_bytes()).expect("write");
        }
        let read = |n: &str| fs::read_to_string(dir.path().join(n)).expect("read");
        assert_eq!(read("crank.log"), "cccccc\n");
        assert_eq!(read("crank.log.1"), "bbbbbb\n");
    }
}
