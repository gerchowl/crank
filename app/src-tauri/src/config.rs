//! Runtime configuration of the desktop process: simulated or real devices, and where
//! the data lives.
//!
//! * `--sim` or `CRANK_SIM` (any value but empty, `0` or `false`): drive the seeded
//!   simulator instead of Bluetooth devices (ADR-0002 §3). The e2e build runs this way.
//! * `CRANK_DATA_DIR`: directory of `crank.db` and `ride.lock` (ADR-0001 §9.2); the
//!   app-data directory of the bundle identifier when unset.
use std::path::PathBuf;

/// How this process was asked to run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Use the device simulator.
    pub sim: bool,
    /// Data directory override.
    pub data_dir: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Read the arguments and environment of this process.
    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(
            std::env::args().skip(1),
            std::env::var("CRANK_SIM").ok().as_deref(),
            std::env::var_os("CRANK_DATA_DIR"),
        )
    }

    /// The pure form of [`Self::from_env`]. Arguments it does not know are ignored: the
    /// platform and Tauri's own tooling may add some.
    pub fn parse(
        args: impl IntoIterator<Item = String>,
        crank_sim: Option<&str>,
        data_dir: Option<std::ffi::OsString>,
    ) -> Self {
        let flag = args.into_iter().any(|a| a == "--sim");
        let env = crank_sim.is_some_and(|v| !matches!(v, "" | "0" | "false"));
        Self {
            sim: flag || env,
            data_dir: data_dir.filter(|d| !d.is_empty()).map(PathBuf::from),
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
            RuntimeConfig::parse(args(&[]), None, None),
            RuntimeConfig::default()
        );
    }

    #[test]
    fn the_flag_or_the_variable_selects_the_simulator() {
        assert!(RuntimeConfig::parse(args(&["--sim"]), None, None).sim);
        assert!(RuntimeConfig::parse(args(&[]), Some("1"), None).sim);
        assert!(RuntimeConfig::parse(args(&["--other", "--sim"]), Some("0"), None).sim);
        for off in ["", "0", "false"] {
            assert!(
                !RuntimeConfig::parse(args(&[]), Some(off), None).sim,
                "{off:?}"
            );
        }
    }

    #[test]
    fn the_data_dir_comes_from_the_variable() {
        let c = RuntimeConfig::parse(args(&[]), None, Some("/tmp/crank".into()));
        assert_eq!(c.data_dir, Some(PathBuf::from("/tmp/crank")));
        assert_eq!(
            RuntimeConfig::parse(args(&[]), None, Some("".into())).data_dir,
            None
        );
    }
}
