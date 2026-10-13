# Frozen TypeScript parity fixtures

ADR-0001 §3 and #31 require exporting the reference results before porting.
Run `nix develop -c just vectors` from the repository root to regenerate every
JSON file. `nix develop -c just vectors-check` re-exports in memory and compares
JSON semantically, without writing files. Both `shared-test` and `vectors-drift` run
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
numbers retain JavaScript's full JSON precision; the export never rounds.

ADR-0001 §3's float parity tolerance applies to the reproducibility comparator:
integer fields, strings, booleans, nulls, object keys, structure and array lengths
must match exactly (object key order is immaterial). Float fields compare equal
only when `|a−b| ≤ max(1e-12·max(|a|,|b|), 1e-15)`, including whole-valued
floats such as `250` versus `250.00000000000003`. Non-finite values use the exact
string encoding above. The comparator reports the first mismatching JSON path.

`schema.json` is the portable classification schema for every vector file,
exported from the explicit declarations in `shared/scripts/vector-schema.ts`.
The schema's version is independent of the vector envelope version. It is a
comparison type map, not a JSON Schema document or a full domain validator:

- `float` declares a physical quantity, duration, timestamp, or continuous
  calculation result. No classification depends on whether its value is whole.
- `integer` declares counts, indices, seeds, ticks, parser UTF-16 offsets/lengths,
  zone codes, the simulator speed enum, and the 1 Hz sample counts reported as
  histogram/heatmap seconds or compiler time-in-zone seconds. Integer values
  must also be integral.
- `exact` declares nonnumeric discrete data (strings, booleans, null).
- `fields` enumerates allowed object keys; `items` declares array elements;
  `ref` resolves through `definitions` for recursive workout nodes.
- `number` allows a float scalar at a node also used for object/array variants.
  `indexed` declares decimal index keys in typed-array JSON objects (W′bal).
  Optional fields and operation variants share declarations; the compared
  documents must still have identical keys, types, and array lengths.

Null and encoded non-finite strings compare exactly wherever they occur.
Both complete documents are checked for schema coverage before comparison,
even if they are equal or an earlier field differs. An unknown field, missing
numeric classification, or unresolved reference throws a comparator error.
Adding a field requires an explicit schema declaration and regeneration with
`just vectors`; `vectors-drift` checks the exported schema byte for byte as well
as checking reference values semantically. Rust ports can load `schema.json`
without interpreting TypeScript. This implements #74's schema-based exactness
under ADR-0001 §3 without changing the tolerance.

JS transcendental functions are implementation-approximated rather than
correctly rounded, so platform/libm differences make bit-exact float parity
invalid. For example, the same generated history sample was
`305.0239643501903` on aarch64-darwin and `305.0239643501902` on x86_64-linux.
Rounding exports would merely move that problem to rounding boundaries.
The comparator tests accept this one-ulp difference and reject real changes
such as `305.02` versus `305.03`, including exact integer/structure drift and
both float tolerance boundaries. Both flake gates run these tests on Linux and
macOS. Downstream Rust parity assertions (#66–#69) use the same tolerance
unless they justify exactness; document it beside each assertion.

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
