//! The build's version string (#552): `<version> · <rev>`, e.g. `0.2.0-rc.1 · 7516045`.
//!
//! One source, fixed at build time, never `git` at run time: the version is the workspace's
//! `CARGO_PKG_VERSION`, the rev is `CRANK_BUILD_REV`, which the flake sets on its desktop
//! packages to its `self.shortRev` (`self.dirtyShortRev`, with its `-dirty` suffix, for a dirty
//! tree). The release pipeline builds `.#crank-app` from the same flake, so an rc zip carries the
//! same string. A build that is not the flake package (`just dev`, `cargo test`) has no rev and
//! says `dev`. The start screen shows the string, keys its once-per-version spin-up on it, and the
//! window title carries it (DESIGN §5.9).
//!
//! The string is one compile-time constant (#659): `build.rs` resolves the rev (or `dev`) into
//! `CRANK_BUILD_REV_SHOWN` and [`BUILD_LABEL`] is `concat!` over it, so the binary carries the whole
//! string and the flake's `stamped` packages can check it there.

pub use crate::build_rev::{NO_REV, shown_rev};

/// This build's string, `<version> · <rev>`, compiled in whole.
pub const BUILD_LABEL: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " \u{b7} ",
    env!("CRANK_BUILD_REV_SHOWN")
);

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
    BUILD_LABEL.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rev is shown verbatim, clean or with its `-dirty`.
    #[test]
    fn a_rev_is_shown_verbatim() {
        assert_eq!(shown_rev(Some("7516045")), "7516045");
        assert_eq!(shown_rev(Some("7516045-dirty")), "7516045-dirty");
    }

    #[test]
    fn no_rev_reads_dev() {
        assert_eq!(shown_rev(None), "dev");
        assert_eq!(shown_rev(Some("")), "dev");
    }

    /// The compiled-in string is the build inputs, joined by one space each side of U+00B7: the
    /// workspace version, and the `CRANK_BUILD_REV` this test binary was built with (cargo builds
    /// and runs it in one environment; the flake's checks set none). The path with a real rev is
    /// checked on the binary itself: the flake's `build-stamped` check and `stamped` packages grep
    /// it for `<version> · <rev>`.
    #[test]
    fn build_label_is_the_build_inputs() {
        let workspace = include_str!("../../../Cargo.toml");
        let version = workspace
            .lines()
            .skip_while(|l| l.trim() != "[workspace.package]")
            .find_map(|l| l.strip_prefix("version = "))
            .map(|v| v.trim_matches('"'))
            .expect("the workspace declares its version");
        let rev = std::env::var("CRANK_BUILD_REV").ok();
        let mut expected = version.as_bytes().to_vec();
        expected.extend_from_slice(b" \xc2\xb7 ");
        expected.extend_from_slice(shown_rev(rev.as_deref()).as_bytes());
        assert_eq!(BUILD_LABEL.as_bytes(), expected, "{BUILD_LABEL}");
        assert_eq!(app_build(), BUILD_LABEL);
    }

    #[test]
    fn window_title_carries_the_label() {
        assert_eq!(window_title("0.1.0 · 7516045"), "crank — 0.1.0 · 7516045");
    }
}
