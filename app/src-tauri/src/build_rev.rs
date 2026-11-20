//! The rev half of the build's version string (#552, #659). `build.rs` includes this file to resolve
//! `CRANK_BUILD_REV` at build time and `version.rs` tests it, so it uses `std` only.

/// What a build without `CRANK_BUILD_REV` shows in place of the rev.
pub const NO_REV: &str = "dev";

/// The rev a build shows: `CRANK_BUILD_REV` verbatim (a `-dirty` suffix included), or [`NO_REV`]
/// when it is absent or empty.
#[must_use]
pub fn shown_rev(rev: Option<&str>) -> &str {
    rev.filter(|r| !r.is_empty()).unwrap_or(NO_REV)
}
