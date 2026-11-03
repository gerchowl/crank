//! Runtime configuration of the desktop process: simulated or real devices, and where
//! the data lives.
//!
//! * `--sim` or `CRANK_SIM` (any value but empty, `0` or `false`): drive the seeded
//!   simulator instead of Bluetooth devices (ADR-0002 §3). The e2e build runs this way.
//! * `CRANK_DATA_DIR`: directory of `crank.db` and `ride.lock` (ADR-0001 §9.2); the
//!   app-data directory of the bundle identifier when unset.
//! * `CRANK_CAPTURE_DIR`: directory for the raw `.fecap` capture of each trainer connection
//!   and for `crank.log` at TRACE for `crank_ble` (the HIL riding session, #411); see
//!   [`crate::logging`]. Nothing is captured when unset.
use std::path::PathBuf;

/// How this process was asked to run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Use the device simulator.
    pub sim: bool,
    /// Data directory override.
    pub data_dir: Option<PathBuf>,
    /// Capture and log directory override.
    pub capture_dir: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Read the arguments and environment of this process.
    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(
            std::env::args().skip(1),
            std::env::var("CRANK_SIM").ok().as_deref(),
            std::env::var_os("CRANK_DATA_DIR"),
            std::env::var_os("CRANK_CAPTURE_DIR"),
        )
    }

    /// The pure form of [`Self::from_env`]. Arguments it does not know are ignored: the
    /// platform and Tauri's own tooling may add some.
    pub fn parse(
        args: impl IntoIterator<Item = String>,
        crank_sim: Option<&str>,
        data_dir: Option<std::ffi::OsString>,
        capture_dir: Option<std::ffi::OsString>,
    ) -> Self {
        let dir = |v: Option<std::ffi::OsString>| v.filter(|d| !d.is_empty()).map(PathBuf::from);
        let flag = args.into_iter().any(|a| a == "--sim");
        let env = crank_sim.is_some_and(|v| !matches!(v, "" | "0" | "false"));
        Self {
            sim: flag || env,
            data_dir: dir(data_dir),
            capture_dir: dir(capture_dir),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn defaults_to_real_devices_in_the_app_data_dir() {
        assert_eq!(
            RuntimeConfig::parse(args(&[]), None, None, None),
            RuntimeConfig::default()
        );
    }

    #[test]
    fn the_flag_or_the_variable_selects_the_simulator() {
        assert!(RuntimeConfig::parse(args(&["--sim"]), None, None, None).sim);
        assert!(RuntimeConfig::parse(args(&[]), Some("1"), None, None).sim);
        assert!(RuntimeConfig::parse(args(&["--other", "--sim"]), Some("0"), None, None).sim);
        for off in ["", "0", "false"] {
            assert!(
                !RuntimeConfig::parse(args(&[]), Some(off), None, None).sim,
                "{off:?}"
            );
        }
    }

    #[test]
    fn the_data_dir_comes_from_the_variable() {
        let c = RuntimeConfig::parse(args(&[]), None, Some("/tmp/crank".into()), None);
        assert_eq!(c.data_dir, Some(PathBuf::from("/tmp/crank")));
        assert_eq!(
            RuntimeConfig::parse(args(&[]), None, Some("".into()), None).data_dir,
            None
        );
    }

    #[test]
    fn the_capture_dir_comes_from_the_variable_and_is_off_when_unset_or_empty() {
        let c = RuntimeConfig::parse(args(&[]), None, None, Some("/tmp/cap".into()));
        assert_eq!(c.capture_dir, Some(PathBuf::from("/tmp/cap")));
        for off in [None, Some(std::ffi::OsString::new())] {
            assert_eq!(
                RuntimeConfig::parse(args(&[]), None, None, off).capture_dir,
                None
            );
        }
    }

    #[test]
    fn the_capture_dir_reaches_the_ble_device_mode_and_not_the_simulator() {
        use crank_core::DeviceMode;
        let c = RuntimeConfig::parse(args(&[]), None, None, Some("/tmp/cap".into()));
        assert!(matches!(
            crate::device_mode(&c),
            DeviceMode::Ble { capture_dir: Some(d) } if d == std::path::Path::new("/tmp/cap")
        ));
        let none = RuntimeConfig::parse(args(&[]), None, None, None);
        assert!(matches!(
            crate::device_mode(&none),
            DeviceMode::Ble { capture_dir: None }
        ));
        let sim = RuntimeConfig::parse(args(&["--sim"]), None, None, Some("/tmp/cap".into()));
        assert!(matches!(crate::device_mode(&sim), DeviceMode::Sim(_)));
    }
}
