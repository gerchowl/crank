# Frozen TypeScript parity fixtures

ADR-0001 §3 and #31 require exporting the reference results before porting.
Run `nix develop -c just vectors` from the repository root to regenerate every
JSON file. `nix develop -c just vectors-check` re-exports in memory and compares
every byte, without writing files. Both `shared-test` and `vectors-drift` run
this comparison in the flake gate on Linux and macOS. No new runtime dependency
is needed: Vitest executes the exporter in `shared/scripts/`.

Each file has `schemaVersion: 1` and a `cases` array. Each case has a diagnostic
`name`, the actual function `input`, and its reference `output`. Case names may
repeat; array position identifies the case. Inputs include expanded sample
arrays so Rust tests do not need to reimplement the TS random generator to
exercise filters, history, or fitting. Simulator inputs instead specify seed,
fixed tick count, athlete, workout, timestep, and observed metrics: the seeded
generator is part of the simulator parity contract.

JSON strings `"NaN"`, `"Infinity"`, and `"-Infinity"` encode non-finite numbers.
`null` remains a missing sample or a function's absent result. Optional object
fields follow the mock's JSON shape (undefined fields are absent). Exported
numbers retain JavaScript's full JSON precision; there is no rounding or
tolerance in the reproducibility check. Rust parity assertions must be exact
for integers and strings and document any necessary floating tolerance at the
assertion (#66, #68, #69).

DSL trees omit volatile ids; offsets and lengths retain TS UTF-16 code-unit
semantics (including Unicode labels). Compiler and simulator workouts use
stable `n:<dot-separated tree path>` ids so block and issue references remain
testable. The exporter calls the frozen implementation directly; it neither
changes mock logic nor replaces its existing tests.

Coverage:

| File | Reference operations and cases |
| --- | --- |
| `dsl.json` | parse/print, library, all existing error cases, remaining parser failure branches, Unicode offsets, canonical durations, labels/options |
| `compile.json` | nested flattening/paths, progression, phases, NP/IF/TSS/zones, kJ/lap/HR estimates, validation, custom athlete |
| `filters.json` | identity, SMA, EMA at both rates, median, one-euro, hold, nulls, composition, backwards/duplicate timestamps |
| `history.json` | HistoryStore bucketing for log/fisheye, ring wrap, gaps, subsecond envelope, reset/isolation, scale edges/mapping |
| `wbal.json` | depletion, exponential refill, both step sizes, CP equality, negative reserve |
| `hrv.json` | rMSSD/DFA-alpha1, scarce/constant/noisy/trending RR, artefact rejection |
| `recovery.json` | mono-exponential fit and prediction, scarce data, several time constants, delayed peaks/nulls, ready/min/max/unreachable states |
| `distributions.json` | histogram/typical, heatmap, MMP/synthetic MMP, empty/non-finite/negative/large inputs |
| `athlete.json` | physiology, HR/power conversion, zone boundaries for two athletes |
| `simulator.json` | seeded 4 Hz trainer/rider and RR/HR output, transitions, coach evaluation, recovery hooks and state snapshots |
| `rewrite.json` | four deterministic rewrite intents, fixed seed, ids removed |

Rust loaders and assertions belong to the corresponding ports (#66–#69).
