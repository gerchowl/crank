# FE-C decoder fuzzing

The default stable toolchain and `nix flake check` run bounded proptests for
arbitrary byte slices and arbitrary checksum-valid envelopes. The separate
`fuzz` dev shell uses the latest compatible nightly from the **locked**
rust-overlay input, including rust-src for cargo-fuzz's build-std. It leaves the
default production toolchain unchanged. From the repository root:

```sh
export CARGO_BUILD_JOBS=<jobs>              # optional; defaults to all cores
export CARGO_TARGET_DIR=<task-specific-target-dir>
nix develop .#fuzz -c cargo fuzz build --fuzz-dir crates/crank-fec/fuzz --target-dir "$CARGO_TARGET_DIR" decode
nix develop .#fuzz -c cargo fuzz run --fuzz-dir crates/crank-fec/fuzz --target-dir "$CARGO_TARGET_DIR" decode -- -max_total_time=60 -max_len=512
```

Choose a task-specific target directory and, on shared hosts, a job count
(the `fuzz` devShell does not set `CARGO_BUILD_JOBS`). Wrap long commands in
watcher-s1 as AGENTS.md requires. AddressSanitizer, debug assertions and overflow
checks use cargo-fuzz defaults. The target tests raw input and also wraps 8-byte
inputs in a checksum-valid envelope to reach every page decoder. Committed
`corpus/decode` seeds cover the specified telemetry/control pages, unspecified
17/19, and the Tacx worked example. Retain crash reproducers as regression tests.
The fuzz workspace has a separate committed Cargo.lock and is excluded from
production flake sources/dependency checks. Its dependency addition is
libfuzzer-sys; the production crate gains only a proptest dev dependency with
fork/timeout features disabled. A scheduled fuzz CI job/artifact retention is
tracked in [#128](https://github.com/gerchowl/crank/issues/128).

Capture replay format and synthetic provenance are documented in
[the fixture README](../tests/fixtures/README.md); real hardware captures are #44.
