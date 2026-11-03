//! The desktop process's log and raw-frame capture (ADR-0001 §7.1, #411, #414).
//!
//! A Finder launch has no stderr, so the reconnect events of `crank-ble` (and everything
//! else the process logs) would be lost. The pipeline is vigil's, installed here, in the
//! desktop entry point, never in a library crate: OTLP/JSON Lines in [`LOG_FILE`] and
//! [`TRACE_FILE`], size-rotated in [`SEGMENT_BYTES`] segments.
//!
//! * Unset `CRANK_CAPTURE_DIR`: the platform log directory (`~/Library/Logs/<identifier>/` on
//!   macOS), filtered by [`FILTER_ENV`] (default `info`), within [`Limits::NORMAL`]. No capture.
//! * `CRANK_CAPTURE_DIR` set: the capture directory, `crank_ble` always at TRACE ([`capture_filter`]),
//!   [`Limits::CAPTURE`] (vigil never deletes anything there), and the BLE central writes a
//!   `.fecap` per trainer connection into it (the format `hil_fec analyse` and `redact` read).
//!
//! The plain-text `crank.log` and `crank.log.1` of #411 are never deleted (ADR-0001 §7.1, Migration).
//! The `ride` and `device` spans of §7.1 are the core's (`crank-core`, #465); the `coach` span follows with the first coach call (#474).
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tracing_subscriber::EnvFilter;
use vigil::{Config, Guard, Signal, rotate::SignalRotation};

/// vigil's live log file in the log directory.
pub const LOG_FILE: &str = "logs.jsonl";
/// vigil's live span file in the log directory.
pub const TRACE_FILE: &str = "traces.jsonl";
/// The filter variable of a normal launch (an `EnvFilter`, default `info`).
pub const FILTER_ENV: &str = "CRANK_LOG";
/// Reserved: the filter variable vigil is pointed at in capture mode, so it cannot apply
/// `CRANK_LOG` over the resolved capture filter. A capture run refuses to start when it is set (#432).
pub const RESERVED_FILTER_ENV: &str = "CRANK_LOG_CAPTURE_RESERVED";
/// Size at which a live file moves to a rotated segment.
pub const SEGMENT_BYTES: u64 = 8 * MIB;
/// Age after which a normal launch's rotated segments are pruned.
pub const RETENTION: Duration = Duration::from_hours(7 * 24);
const MIB: u64 = 1024 * 1024;
/// The target the capture always records at TRACE.
const CAPTURE_TARGET: &str = "crank_ble";

/// What vigil may prune; `None` turns a limit off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Age of rotated segments, for every signal.
    pub retention: Option<Duration>,
    /// All telemetry files in the directory together.
    pub max_total_bytes: Option<u64>,
    /// Rotated segments of `logs`.
    pub log_segments: Option<usize>,
    /// Rotated segments of `traces`.
    pub trace_segments: Option<usize>,
}

impl Limits {
    /// A normal launch: 7 days, at most 6 log and 2 trace segments, under 96 MiB in all.
    pub const NORMAL: Self = Self {
        retention: Some(RETENTION),
        max_total_bytes: Some(96 * MIB),
        log_segments: Some(6),
        trace_segments: Some(2),
    };
    /// A capture directory is evidence the owner curates: nothing is ever pruned.
    pub const CAPTURE: Self = Self {
        retention: None,
        max_total_bytes: None,
        log_segments: None,
        trace_segments: None,
    };
}

/// The filter vigil is given: the variable it reads and the directives it falls back to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// The environment variable vigil reads its directives from.
    pub env: &'static str,
    /// The directives when that variable is unset or empty.
    pub default: String,
    /// What the resolution changed, logged as WARN records once vigil runs.
    pub warnings: Vec<String>,
}

/// The capture filter (ADR-0001 §7.1): `crank_log`'s directives (`info` when unset, empty or
/// invalid), without any directive for `crank_ble` or `crank_ble::…`, then `crank_ble=trace`.
/// Removing those directives keeps the capture at TRACE: the most specific directive wins.
/// `crank_desktop::logging=warn` closes it, so the WARN records reporting the changes are kept.
#[must_use]
pub fn capture_filter(crank_log: Option<&str>) -> Filter {
    let mut warnings = Vec::new();
    let given = match crank_log.filter(|v| !v.is_empty()) {
        Some(v) if EnvFilter::try_new(v).is_ok() => v,
        Some(v) => {
            warnings.push(format!(
                "{FILTER_ENV}={v:?} is invalid; the capture uses info"
            ));
            "info"
        }
        None => "info",
    };
    let mut kept: Vec<&str> = Vec::new();
    for directive in directives(given) {
        if is_capture_target(directive) {
            warnings.push(format!(
                "{FILTER_ENV} directive {directive:?} dropped: the capture records {CAPTURE_TARGET} at TRACE"
            ));
        } else {
            kept.push(directive);
        }
    }
    kept.push("crank_ble=trace");
    // The resolution's own WARN records, whatever `crank_log` turned off.
    kept.push("crank_desktop::logging=warn");
    Filter {
        env: RESERVED_FILTER_ENV,
        default: kept.join(","),
        warnings,
    }
}

/// The comma-separated directives of a valid `filter` (span field lists, which could hold a
/// comma, do not parse in this build's `EnvFilter`).
fn directives(filter: &str) -> impl Iterator<Item = &str> {
    filter.split(',').map(str::trim).filter(|d| !d.is_empty())
}

/// Whether `directive` names `crank_ble` or a module under it (a bare target means TRACE).
fn is_capture_target(directive: &str) -> bool {
    let target = directive
        .split(['[', '='])
        .next()
        .unwrap_or_default()
        .trim();
    target == CAPTURE_TARGET
        || target
            .strip_prefix(CAPTURE_TARGET)
            .is_some_and(|rest| rest.starts_with("::"))
}

/// Why logging did not start.
#[derive(Debug)]
pub enum LogError {
    /// [`RESERVED_FILTER_ENV`] is set in capture mode: vigil would apply it over the capture filter.
    Reserved,
    /// vigil refused the configuration.
    Init(vigil::InitError),
    /// vigil fell back to stderr: the directory cannot be written.
    NoDirectory(PathBuf),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved => write!(f, "{RESERVED_FILTER_ENV} is reserved; unset it"),
            Self::Init(e) => write!(f, "{e}"),
            Self::NoDirectory(dir) => write!(f, "{} cannot be written", dir.display()),
        }
    }
}

impl std::error::Error for LogError {}

/// Where this process logs and what it captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPlan {
    /// Directory of [`LOG_FILE`] and [`TRACE_FILE`].
    pub log_dir: PathBuf,
    /// Whether `crank_ble` is recorded at TRACE and trainer frames are captured into `log_dir`.
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
    /// What vigil may prune in [`Self::log_dir`].
    #[must_use]
    pub fn limits(&self) -> Limits {
        if self.capture {
            Limits::CAPTURE
        } else {
            Limits::NORMAL
        }
    }
    /// The filter: [`capture_filter`] of `crank_log` in capture mode, else vigil reads [`FILTER_ENV`].
    #[must_use]
    pub fn filter(&self, crank_log: Option<&str>) -> Filter {
        if self.capture {
            capture_filter(crank_log)
        } else {
            Filter {
                env: FILTER_ENV,
                default: "info".into(),
                warnings: Vec::new(),
            }
        }
    }
    /// The vigil configuration. Every limit is set explicitly, so the `VIGIL_*` environment
    /// cannot change it.
    #[must_use]
    pub fn config(&self, filter: &Filter) -> Config {
        let limits = self.limits();
        let signal = |segments| {
            SignalRotation::new()
                .max_bytes(SEGMENT_BYTES)
                .max_segments(segments)
                .retention(limits.retention)
        };
        let config = Config::new("crank")
            .dir(&self.log_dir)
            .version(env!("CARGO_PKG_VERSION"))
            // `revision` comes with the build provenance the bug report shares (#445, ADR-0003 §2.6).
            .filter_env(filter.env)
            .default_filter(filter.default.clone())
            .max_bytes(SEGMENT_BYTES)
            .retention(limits.retention)
            .max_total_bytes(limits.max_total_bytes)
            .signal_rotation(Signal::Logs, signal(limits.log_segments))
            .signal_rotation(Signal::Traces, signal(limits.trace_segments));
        if cfg!(debug_assertions) {
            config.console(vigil::Console::Stderr)
        } else {
            config
        }
    }
    /// Refuse a capture while [`RESERVED_FILTER_ENV`] is set (`reserved` is its value): vigil would apply it.
    ///
    /// # Errors
    /// [`LogError::Reserved`].
    pub fn check_reserved(&self, reserved: Option<&std::ffi::OsStr>) -> Result<(), LogError> {
        if self.capture && reserved.is_some() {
            Err(LogError::Reserved)
        } else {
            Ok(())
        }
    }
    /// Install the pipeline process-wide. `crank_log` is [`FILTER_ENV`]'s value, which only a
    /// capture resolves itself ([`capture_filter`]); a normal launch lets vigil read the variable.
    ///
    /// # Errors
    /// [`LogError`]. A started pipeline that cannot be used is shut down first.
    pub fn install(&self, crank_log: Option<&str>) -> Result<Guard, LogError> {
        self.check_reserved(std::env::var_os(RESERVED_FILTER_ENV).as_deref())?;
        let filter = self.filter(crank_log);
        let guard = self.config(&filter).init().map_err(LogError::Init)?;
        if guard.dir().is_none() {
            // A shutdown report (stderr) is all there is to say; the caller reports the error.
            let _ = guard.shutdown_with_timeout(Duration::ZERO);
            return Err(LogError::NoDirectory(self.log_dir.clone()));
        }
        for warning in &filter.warnings {
            tracing::warn!("{warning}");
        }
        Ok(guard)
    }
}

/// What [`Logging`] needs of the pipeline: vigil's [`Guard`], as a trait so a test can stall a flush.
pub trait Pipeline: Send + Sync + 'static {
    /// Write queued records now; may block (vigil waits up to 5 s).
    fn flush(&self);
    /// Shut down, waiting at most `budget`.
    fn shut_down(self: Box<Self>, budget: Duration);
}

impl Pipeline for Guard {
    fn flush(&self) {
        flush(self);
    }
    fn shut_down(self: Box<Self>, budget: Duration) {
        // Nothing can log the outcome any more: vigil reports a loss on stderr itself.
        let _ = self.shutdown_with_timeout(budget);
    }
}

type Shared = Arc<Box<dyn Pipeline>>;

/// How often [`Logging::shut_down`] looks whether an in-flight flush has finished.
const POLL: Duration = Duration::from_millis(5);

/// The pipeline as Tauri state, always managed (empty when logging did not start). The quit
/// takes it to shut vigil down explicitly, so the guard's Drop (which would wait 5 s) never runs.
///
/// The lock is held only to clone or take the handle, never across a flush: a flush runs on its
/// own clone, so the quit never waits behind the lock (#414 review).
#[derive(Default)]
pub struct Logging(Mutex<Option<Shared>>);

impl Logging {
    /// Hold `pipeline` until the quit.
    #[must_use]
    pub fn new(pipeline: impl Pipeline) -> Self {
        Self(Mutex::new(Some(Arc::new(Box::new(pipeline)))))
    }
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Shared>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn handle(&self) -> Option<Shared> {
        self.slot().clone()
    }
    /// [`flush`] on this thread, unless the pipeline is shut down already.
    pub fn flush(&self) {
        if let Some(pipeline) = self.handle() {
            pipeline.flush();
        }
    }
    /// [`flush`] on a blocking thread, so a stalled disk holds up neither the caller nor a
    /// runtime worker; `None` when the pipeline is shut down already. A quit that arrives
    /// meanwhile waits for it only within its own budget ([`Self::shut_down`]).
    pub fn flush_in_background(&self) -> Option<tauri::async_runtime::JoinHandle<()>> {
        let pipeline = self.handle()?;
        Some(tauri::async_runtime::spawn_blocking(move || {
            pipeline.flush();
        }))
    }
    /// Shut vigil down within `budget`; later calls do nothing. Records still queued when
    /// `budget` runs out are lost (vigil says how many on stderr): at most what was logged
    /// since the last export (~1 s batches) or [`flush`], typically the quit's own lines.
    ///
    /// A flush in flight (a ride just ended) holds the pipeline: the shutdown waits for it
    /// until `budget` runs out, then shuts down with what is left. If the flush outlasts the
    /// budget, vigil is not shut down at all: the flush keeps its thread and the process exits
    /// under it, losing what that flush had not written yet, and the guard's Drop never runs.
    pub fn shut_down(&self, budget: Duration) {
        let deadline = Instant::now() + budget;
        let Some(mut pipeline) = self.slot().take() else {
            return;
        };
        loop {
            match Arc::try_unwrap(pipeline) {
                Ok(owned) => {
                    return owned.shut_down(deadline.saturating_duration_since(Instant::now()));
                }
                Err(shared) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        // The last handle is the flush's: forgetting this one keeps its Drop
                        // (a 5 s shutdown) from running when that flush returns.
                        std::mem::forget(shared);
                        return;
                    }
                    pipeline = shared;
                    std::thread::sleep(left.min(POLL));
                }
            }
        }
    }
}

/// What remains of `cap` after `elapsed`: vigil's shutdown gets the rest of the quit budget,
/// nothing when it is used up (ADR-0001 §7.1, Quit budget).
#[must_use]
pub fn remaining(cap: Duration, elapsed: Duration) -> Duration {
    cap.saturating_sub(elapsed)
}

/// Write queued records now (a ride ended). Records dropped since the last flush are
/// reported as a WARN record: a Finder launch has no stderr for vigil's own warning.
pub fn flush(guard: &Guard) {
    if let Err(e) = guard.flush() {
        tracing::warn!(lost = e.lost(Signal::Logs), kind = ?e.kind(), "log flush: {e}");
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
        let filter = plan.filter(None);
        assert_eq!(filter.env, RESERVED_FILTER_ENV);
        assert_eq!(
            filter.default,
            "info,crank_ble=trace,crank_desktop::logging=warn"
        );
    }

    #[test]
    fn without_it_the_platform_log_dir_reads_crank_log_and_no_capture() {
        let plan = LogPlan::new(None, "/logs".into());
        assert_eq!(plan.log_dir, Path::new("/logs"));
        assert!(!plan.capture);
        let filter = plan.filter(Some("crank_ble=trace"));
        assert_eq!((filter.env, filter.default.as_str()), (FILTER_ENV, "info"));
    }

    #[test]
    fn a_capture_prunes_nothing_and_a_normal_launch_is_bounded() {
        let capture = LogPlan::new(Some(Path::new("/cap")), "/logs".into()).limits();
        assert_eq!(capture, Limits::CAPTURE);
        assert_eq!((capture.retention, capture.max_total_bytes), (None, None));
        assert_eq!((capture.log_segments, capture.trace_segments), (None, None));
        let normal = LogPlan::new(None, "/logs".into()).limits();
        assert_eq!(normal.retention, Some(RETENTION));
        assert_eq!(normal.max_total_bytes, Some(96 * MIB));
        // Live and rotated segments of both signals fit the total cap.
        let segments = 2 + normal.log_segments.unwrap_or(0) + normal.trace_segments.unwrap_or(0);
        assert!(segments as u64 * SEGMENT_BYTES <= 96 * MIB);
    }

    #[test]
    fn crank_log_adds_to_the_capture_but_never_lowers_crank_ble() {
        assert_eq!(
            capture_filter(Some("warn")).default,
            "warn,crank_ble=trace,crank_desktop::logging=warn"
        );
        let lowered = capture_filter(Some(
            "debug,crank_ble::driver=warn,crank_ble,crank_blex=info",
        ));
        assert_eq!(
            lowered.default,
            "debug,crank_blex=info,crank_ble=trace,crank_desktop::logging=warn"
        );
        assert_eq!(lowered.warnings.len(), 2, "{:?}", lowered.warnings);
        let span = capture_filter(Some("crank_ble[ride]=warn,[ride]=debug"));
        assert_eq!(
            span.default,
            "[ride]=debug,crank_ble=trace,crank_desktop::logging=warn"
        );
    }

    #[test]
    fn an_invalid_crank_log_is_reported_and_the_capture_default_used() {
        let filter = capture_filter(Some("crank_core=loud"));
        assert_eq!(
            filter.default,
            "info,crank_ble=trace,crank_desktop::logging=warn"
        );
        assert_eq!(filter.warnings.len(), 1);
        assert_eq!(
            capture_filter(Some("")).default,
            "info,crank_ble=trace,crank_desktop::logging=warn"
        );
    }

    #[test]
    fn a_capture_refuses_the_reserved_variable_and_a_normal_launch_ignores_it() {
        let set = Some(std::ffi::OsStr::new(""));
        let capture = LogPlan::new(Some(Path::new("/cap")), "/logs".into());
        assert!(matches!(
            capture.check_reserved(set),
            Err(LogError::Reserved)
        ));
        assert!(capture.check_reserved(None).is_ok());
        assert!(
            LogPlan::new(None, "/logs".into())
                .check_reserved(set)
                .is_ok()
        );
    }

    #[test]
    fn the_flush_gets_what_the_quit_cap_left_and_nothing_once_it_is_used() {
        let cap = Duration::from_secs(3);
        assert_eq!(
            remaining(cap, Duration::from_millis(2500)),
            Duration::from_millis(500)
        );
        assert_eq!(
            remaining(cap, Duration::from_millis(2900)),
            Duration::from_millis(100)
        );
        assert_eq!(remaining(cap, Duration::from_secs(4)), Duration::ZERO);
    }
}
