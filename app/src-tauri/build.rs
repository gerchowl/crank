//! Tauri build integration, and the rev half of the build's version string (#552, #659).

#[path = "src/build_rev.rs"]
mod build_rev;

use std::env::{self, VarError};

fn main() {
    // `version::BUILD_LABEL` is `concat!` over `CARGO_PKG_VERSION` and this variable, so the binary carries
    // the whole `<version> · <rev>` (the flake's `stamped` packages grep for it). A rev that is not UTF-8
    // cannot be shown; it fails the build rather than reading `dev`.
    println!("cargo:rerun-if-env-changed=CRANK_BUILD_REV");
    let rev = match env::var("CRANK_BUILD_REV") {
        Ok(rev) => Some(rev),
        Err(VarError::NotPresent) => None,
        Err(VarError::NotUnicode(rev)) => panic!("CRANK_BUILD_REV is not UTF-8: {}", rev.display()),
    };
    println!(
        "cargo:rustc-env=CRANK_BUILD_REV_SHOWN={}",
        build_rev::shown_rev(rev.as_deref())
    );
    tauri_build::build();
}
