# FE-C decoder fuzzing

The normal stable-toolchain test suite exercises arbitrary byte slices and
arbitrary checksum-valid page envelopes with proptest. `just fuzz` runs the same
pure decoder through libFuzzer, including valid envelopes built from 8-byte inputs.

Run inside `nix develop` with a nightly compiler selected using `RUSTC`:

```sh
RUSTC=/absolute/path/to/nightly/rustc just fuzz
```

The pinned production Rust 1.96 toolchain is stable; libFuzzer sanitizer flags
need nightly. A pinned nightly CI job and retained crash artifacts are tracked
in [#128](https://github.com/gerchowl/crank/issues/128). The nested fuzz workspace
has its own lockfile and is explicitly excluded from production flake sources.
It has no effect on the production dependency graph or dependency checks.

Replay fixtures use `.fecap` text: one complete ANT frame per line as
space-separated hexadecimal bytes; blank lines and `#` comments are permitted.
The fixture harness loads every `.fecap` in `tests/fixtures`. The initial fixture
is the Tacx FE-C over BLE v1.0.0 worked example, not a hardware capture. NEO
captures and hardware acceptance of the codec's layout/checksum are #44.
