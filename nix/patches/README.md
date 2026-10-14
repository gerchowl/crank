# Nix patches

## tauri-plugin-wdio-webdriver-writable-build.patch

`tauri-plugin-wdio-webdriver` 1.5.0's `build.rs` runs `tauri_plugin::Builder`, which writes generated
schemas and docs under `./permissions` relative to the crate's manifest directory. A Nix-vendored source
tree is read-only, so the patch runs the builder from a writable copy in `OUT_DIR`. The vendored copy is
patched for the e2e package only (`cargoVendorDirE2e` in `flake.nix`, #233).

Upstream (`webdriverio/desktop-mobile`): searched 2026-10-14 for the build script, `permissions`,
read-only and vendoring; no issue exists. The report is drafted in the PR for #233 and is filed by the
architect; the issue link goes here once filed, and the patch is dropped when a release contains the fix.
