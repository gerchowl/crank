//! The build's version string (#552): `<version> · <rev>`, e.g. `0.2.0-rc.1 · 7516045`.
//!
//! One source, fixed at build time, never `git` at run time: the version is the workspace's
//! `CARGO_PKG_VERSION`, the rev is `CRANK_BUILD_REV`, which the flake sets on the
//! `crank-desktop` package to its `self.shortRev` (`self.dirtyShortRev`, with its `-dirty`
//! suffix, for a dirty tree). The release pipeline builds `.#crank-app` from the same flake,
//! so an rc zip carries the same string. A build that is not the flake package (`just dev`,
//! `cargo test`) has no rev and says `dev`. The start screen shows the string, keys its
//! once-per-version spin-up on it, and the window title carries it (DESIGN §5.9).

/// What a build without `CRANK_BUILD_REV` shows in place of the rev.
pub const NO_REV: &str = "dev";

/// `<version> · <rev>`; an absent or empty rev reads [`NO_REV`].
#[must_use]
pub fn label(version: &str, rev: Option<&str>) -> String {
    let rev = rev.filter(|r| !r.is_empty()).unwrap_or(NO_REV);
    format!("{version} · {rev}")
}

/// This build's string, from the inputs compiled in.
#[must_use]
pub fn build_label() -> String {
    label(env!("CARGO_PKG_VERSION"), option_env!("CRANK_BUILD_REV"))
}

/// The main window's title: the app's name and this build's string.
#[must_use]
pub fn window_title(label: &str) -> String {
    format!("crank — {label}")
}

/// This build's version string, for the start screen.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn app_build() -> String {
    build_label()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_version_dot_rev() {
        assert_eq!(label("0.2.0-rc.1", Some("7516045")), "0.2.0-rc.1 · 7516045");
        assert_eq!(
            label("0.2.0-rc.1", Some("7516045-dirty")),
            "0.2.0-rc.1 · 7516045-dirty"
        );
        assert_eq!(label("0.1.0", None), "0.1.0 · dev");
        assert_eq!(label("0.1.0", Some("")), "0.1.0 · dev");
    }

    /// The compiled-in string is the build inputs: the workspace version, and the
    /// `CRANK_BUILD_REV` this test binary was built with (cargo builds and runs it in one
    /// environment; the flake's checks set none, its `crank-desktop` package sets the rev).
    #[test]
    fn build_label_matches_the_build_inputs() {
        let workspace = include_str!("../../../Cargo.toml");
        let version = workspace
            .lines()
            .skip_while(|l| l.trim() != "[workspace.package]")
            .find_map(|l| l.strip_prefix("version = "))
            .map(|v| v.trim_matches('"'))
            .expect("the workspace declares its version");
        let rev = std::env::var("CRANK_BUILD_REV").ok();
        let got = build_label();
        assert_eq!(got, label(version, rev.as_deref()));
        assert!(got.starts_with(&format!("{version} · ")), "{got}");
    }

    #[test]
    fn window_title_carries_the_label() {
        assert_eq!(window_title("0.1.0 · 7516045"), "crank — 0.1.0 · 7516045");
    }
}
