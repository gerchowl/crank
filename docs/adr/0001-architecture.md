# ADR-0001: Architecture, crate layout, Rust↔UI contract, persistence, testing

- **Status:** Proposed
- **Date:** 2026-10-13
- **Issue:** #26
- **Related:** ADR-0002 (BLE + `Trainer`), #24 (first-principles UX), #28 (scaffold), #2 #4 #12–#19

## 1. Context

crank is an indoor-cycling app for the owner's Tacx NEO (gen 1, BLE only, FE-C over BLE; see
`spec/TRAINER-BLE.md`) with an AI coach: an in-ride System-1 judge (`ease_off | hold | push | replan`
with `strong | lean | uncertain` bands) and an LLM planner (Claude API) before and after rides. v1 also
ships an MCP server, an integrated CLI and intervals.icu import. Already decided: **Rust core + Tauri v2 +
React 19 with the React Compiler** (`spec/SPIKE-RESULTS.md`), BLE only, v1 = everything in the spec.

The UI spikes ran against `shared/`, a TypeScript mock of the future core (`shared/src/contract.ts`,
~3.7k LOC of metrics, filters, multi-resolution history, W′bal, HRV, HR-recovery fit, DSL, compiler,
simulator, planner/coach mocks; 1.1k LOC of tests). The spike reviews found architecture-level defects
that a production design must make impossible rather than fix one by one:

- physiology computed in the UI (#2), so formulas diverged between UI and backend;
- unvalidated persisted data crashing the app (#4);
- per-ride state leaking across rides (#3), a live cursor that drifted from actual time (#14);
- laps, thresholds and flags missing from the contract, so the UI invented them (#12, #13).

Forces: the core must run **headless** (CLI, MCP, tests, CI) with no Tauri and no Bluetooth; only one
BLE central can be connected to the NEO at a time; macOS TCC gates Bluetooth per signed identity; the
ride must survive BLE dropouts without corrupting the workout timeline; the host CI has no Bluetooth.

## 2. Decision: workspace layout and crate boundaries

One Cargo workspace (edition 2024, toolchain pinned in `rust-toolchain.toml` by #28). Crates live in
`crates/`, the Tauri shell in `app/src-tauri`, the React app in `app/`.

| Crate | Kind | Owns | May depend on |
|---|---|---|---|
| `crank-domain` | lib, **pure** (no I/O, no async, no clock) | units, metric catalog and zones, filters, multi-resolution history, W′bal, rMSSD/DFA-α1, HR-recovery fit, histogram/heatmap/MMP, athlete model (`Estimate`), lap stats, workout model + DSL + compiler + deterministic rewrite, `Judge` trait + coach feature/verdict types | `serde`, `thiserror`, `specta` (feature `specta`) |
| `crank-devices` | lib | `TrainerDriver` / `HeartRateDriver` traits and their types (ADR-0002), `sim` module: simulated rider + trainer + HR strap (port of `sim.ts`) | `crank-domain`, `tokio` (sync, time) |
| `crank-fec` | lib, pure | ANT message framing + FE-C page codec (ADR-0002 §4) | none |
| `crank-ble` | lib | btleplug discovery, FE-C trainer driver, HRS driver, reconnect, raw-frame capture (ADR-0002) | `crank-devices`, `crank-fec`, `btleplug` |
| `crank-engine` | lib | ride engine actor: tick clock, ride state machine, metric bus, subscriptions + filter chains, ride record, laps, status, ERG policy, judge hook | `crank-domain`, `crank-devices` |
| `crank-store` | lib | SQLite schema, migrations, repositories | `crank-domain`, `rusqlite`, `rusqlite_migration` |
| `crank-coach` | lib | `RulesJudge` (knowledge-pack rules, implements `crank_domain::Judge`), `Planner` trait + Claude API client | `crank-domain`, `reqwest` |
| `crank-intervals` | lib | intervals.icu client + mapping into domain types | `crank-domain`, `reqwest` |
| `crank-core` | lib | `Core` facade wiring everything; **the API** (async methods + DTOs); config/data dirs, secrets, ride lock | all of the above |
| `crank-mcp` | lib | MCP tools over `Core` (`rmcp`) | `crank-core` |
| `crank-cli` | bin `crank` | `clap` CLI: `ride --sim`, `workout parse/compile/print`, `devices scan`, `import intervals`, `mcp` (stdio server), `devserver` | `crank-core`, `crank-mcp` |
| `crank-desktop` | bin (Tauri v2) | thin `#[tauri::command]` wrappers over `Core`, bindings export, window/menu | `crank-core`, `tauri`, `tauri-specta` |

Rules:

1. **Dependency direction** is `domain ← devices ← engine ← core ← {mcp, cli, desktop}`; `fec ← ble ← core`;
   `store, coach, intervals ← core`. `crank-engine` never depends on `crank-ble`, so engine tests and CI need
   no Bluetooth. `crank-domain` never depends on tokio. #28 adds a CI check (`cargo metadata` script) that
   fails on a forbidden edge.
2. **Headless first.** Everything the desktop app can do goes through `Core`; `crank-desktop` contains
   no logic beyond argument mapping. The CLI rides headless with `crank ride --sim` or a real trainer.
3. **One engine owner.** The process that holds `ride.lock` (an advisory `flock` on
   `<data_dir>/ride.lock`) owns BLE devices and the engine. Other processes (`crank mcp`, CLI queries)
   use the SQLite store only. This follows from the NEO's single-central limit and avoids two processes
   fighting over a trainer. Live-ride access across processes is open: #40.
4. **Async runtime:** tokio everywhere (Tauri and btleplug both run on it). The engine is a single task
   that owns its state and receives commands over an `mpsc` channel (actor); no `Mutex` around ride
   state.

Rejected alternatives:

- **One `crank-core` crate with modules.** Rejected: the purity rule for domain code and the "engine
  without BLE" rule cannot be enforced inside one crate, and compile times for the domain tests grow
  with every dependency.
- **BLE in the UI (`tauri-plugin-blec`, Web Bluetooth).** Rejected in ADR-0002: CLI/MCP could not ride.
- **A separate daemon that the app, CLI and MCP all talk to.** Rejected for v1: a second process to
  install, sign (TCC is per identity) and supervise, for a single-user desktop app. The ride lock gives
  the same safety; #40 revisits cross-process live access.

## 3. Decision: porting `shared/`

Everything that computes moves to Rust; TypeScript keeps only presentation.

| `shared/src` | Goes to | Notes |
|---|---|---|
| `contract.ts` | `crank-core::api` DTOs (+ domain types) | generated back to TS (§4); hand-written TS contract deleted |
| `workout/{model,dsl,compile,rewrite}.ts`, `library.ts` | `crank-domain::workout` | DSL grammar in `shared/DSL.md` is the spec; moves to `docs/` with the port |
| `filters.ts`, `history.ts`, `wbal.ts`, `hrv.ts`, `hrRecovery.ts`, `distributions.ts`, `athlete.ts` | `crank-domain` | |
| `bus.ts`, `clock.ts`, `backend.ts` (ride parts) | `crank-engine` | |
| `sim.ts`, `rng.ts` | `crank-devices::sim` | the simulated rider stays seedable and deterministic |
| `coach.ts`, `planner.ts` | `crank-coach` | mock planner becomes the deterministic library planner used in tests |
| `layouts.ts` | `crank-domain::layout` (types + defaults) + `crank-store` | validated on read (#4) |
| `scales.ts` (`timeAgoToU` etc.) | **stays in TS** (app) | pure axis maths needed per animation frame; parity with Rust bucketing asserted by vectors |

How the tests move (#31): before porting, a script in `shared/` exports **golden vectors** (JSON) from the
TS implementation for every tested function, including all DSL error cases. Each `shared/test/*.test.ts`
gets a Rust equivalent that must reproduce the vectors (exact for integers/strings, a stated tolerance for
floats), plus proptest properties (`parse(print(t)) == t`, compiled blocks tile `[0, totalS)`, filter
output bounded by input range). Lifecycle of `shared/`: frozen now (no new logic; #24's prototype
still uses it), deleted when the React app runs on the generated bindings. No physiology, aggregation or
thresholds remain in TS after the port (#2, #13).

Rejected: **keep the TS logic and run it in the webview** (two implementations or a UI-only core, CLI/MCP
could not use it); **compile the Rust domain to WASM for the UI** (a second runtime to ship for no gain:
the UI asks the core over IPC at ≤ 10 Hz, see §4).

## 4. Decision: the Rust↔UI contract

### 4.1 Single source of truth

Rust types are the contract. DTOs derive `serde::{Serialize, Deserialize}` and `specta::Type`; commands are
annotated `#[tauri::command] #[specta::specta]`; **tauri-specta** generates `app/src/bindings.ts` (types,
typed command wrappers, typed `Channel<T>` parameters). The file is committed; a Rust test regenerates it
in CI and fails on any diff.

Versions (checked on crates.io 2026-10-13): `tauri` 2.12.3 stable (3.0 is alpha; stay on 2.x),
`tauri-specta` 2.0.0-rc.25, `specta` 2.0.0-rc.25, `specta-typescript` 0.0.12. tauri-specta v2 has been a
release candidate since 2024, so the three specta crates are **pinned with `=`** and upgraded together
deliberately. tauri-specta's own example app generates bindings for commands that take
`tauri::ipc::Channel<T>` (`withChannel: (channel: Channel<number>) => …` in
`examples/app/src/bindings.ts`), which §4.3 depends on.

Serialization conventions: `#[serde(rename_all = "camelCase")]` on structs, enums as tagged unions
`#[serde(tag = "kind", rename_all = "snake_case")]` (matches the existing `Filter`, `PowerTarget`,
`StepEnd` shapes), `MetricId` as a `snake_case` string enum. All times at the boundary are seconds as
`f64`; internal time is integer ticks (§5). `Option<T>` maps to `T | null`, never `undefined`.

Rejected:

- **ts-rs** (12.0.1, stable): generates types only; command names, argument shapes and channel payloads
  would be hand-written in TS and drift. Kept as the documented exit if tauri-specta stalls (types are
  plain serde either way, so the switch only touches the generated layer).
- **TauRPC**: also specta-based, adds its own router and procedure model on top of Tauri; more surface
  than we need.
- **Hand-written TS contract kept as the source** (the spike setup): it is how #2/#13 happened.

### 4.2 Commands vs channels vs events

Tauri's docs: the event system "is not designed for low latency or high throughput situations", event
payloads are always JSON strings, and channels "are designed to be fast and deliver ordered data" and
are what Tauri itself uses for download progress and child-process output
([Calling the frontend from Rust](https://v2.tauri.app/develop/calling-frontend/)).

| Mechanism | Used for |
|---|---|
| **Commands** (request/response, async) | every query and mutation: `history`, `histogram`, `heatmap`, `mmp`, `hr_recovery`, `streams`, `recoveries`, `laps`, `compile_workout`, `parse_dsl`, `print_dsl`, ride control (`ride_load/start/pause/resume/stop/adjust/skip_block`), layouts, snippets, workouts, athlete model, coach respond/rate, planner propose/rewrite, devices, settings |
| **One `Channel<StreamFrame>` per window** | all live data: subscribed samples, ride status, nudge changes, device link changes |
| **Events** | only low-rate, app-wide invalidation hints for multiple windows: `store_changed { table }` |

Queries become async (they were synchronous in the mock). The polling budget in `shared/README.md`
stays (history ≤ 1 Hz per chart, distributions ≤ 0.5 Hz, MMP ≤ 0.2 Hz); the UI animates between polls.
Spec text update: #36.

### 4.3 Streaming

The mock's per-widget subscription semantics stay (rate 0.1–10 Hz per subscription, a filter chain per
subscription running on every source sample, delivery only when a new source sample arrived). The
transport changes: instead of one callback/channel per widget, the window opens **one multiplexed
channel** and registers subscriptions on it.

```rust
#[tauri::command] #[specta::specta]
async fn open_stream(core: State<'_, Core>, ch: Channel<StreamFrame>) -> Result<StreamId, ApiError>;
#[tauri::command] #[specta::specta]
async fn subscribe(core: State<'_, Core>, stream: StreamId, metric: MetricId, opts: SubscribeOpts)
    -> Result<SubId, ApiError>;
#[tauri::command] #[specta::specta]
async fn unsubscribe(core: State<'_, Core>, stream: StreamId, sub: SubId) -> Result<(), ApiError>;

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    pub seq: u64,                        // gap ⇒ the UI requests a status snapshot
    pub samples: Vec<SubSample>,         // only subscriptions that are due in this frame
    pub status: Option<RideStatus>,      // on change, and every 500 ms while riding
    pub nudges: Vec<CoachNudge>,         // created or status-changed since last frame
    pub links: Vec<LinkChange>,          // trainer / HR link state changes
}
pub struct SubSample { pub sub: SubId, pub t: f64, pub v: Option<f64> }
```

Rates:

| Stream | Rate | Why |
|---|---|---|
| Engine tick / trainer source | **4 Hz** (250 ms) | FE-C native rate; Tacx recommends 4 Hz in both directions (ADR-0002 §4) |
| HR source | per notification (~1 Hz), RR per beat | Heart Rate Service |
| Frame emission | **≤ 10 Hz**, only when something is due | 10 Hz is the maximum display rate; bounds IPC to ≤ 10 messages/s per window regardless of widget count |
| Status heartbeat | 2 Hz while riding, immediately on change | same as mock |
| Ride record | 1 Hz per-second means | `streams()` and persistence |

At 10× / 60× simulated speed the source rates scale with ride time, frame emission does not.
Ordering guarantee: everything in one frame happened before everything in the next; a status change
and the samples that follow it can't be reordered, which events could not promise.

Rejected: **a channel per subscription** (30 widgets × 10 Hz = 300 IPC messages/s and no cross-widget
ordering); **events for telemetry** (JSON-only, unordered under load, per the Tauri docs above);
**fixed-rate full snapshots** (sends everything to every widget; per-widget rates are a product
requirement).

### 4.4 Errors across the boundary

Every command returns `Result<T, ApiError>`:

```rust
#[derive(Serialize, specta::Type, thiserror::Error, Debug)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct ApiError { pub code: ErrorCode, pub message: String }

#[derive(Serialize, specta::Type, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput, NotFound, Conflict,          // e.g. ride already running, ride lock held
    DeviceUnavailable, PermissionDenied,       // BLE adapter off, TCC denied
    Upstream,                                  // Claude / intervals.icu failure
    Storage, Internal,
}
```

DSL parse errors are **values**, not errors (`ParseResult = { nodes } | { error: DslError }` as today),
because invalid drafts are a normal editor state.

## 5. Decision: time and the ride clock

- **Tick source.** The engine advances in fixed **250 ms ticks** (`SIM_DT` in the mock, FE-C's 4 Hz).
  Ride time is `ticks × 250 ms` as an integer (`RideTicks(u64)`), so ride time is exact and the same
  inputs give the same ride.
- **Clock trait.** `trait Clock { fn now(&self) -> MonoTime; async fn sleep_until(&self, t: MonoTime); }`
  with `MonotonicClock` (wraps `tokio::time::Instant`, never wall-clock) and `VirtualClock` (tests,
  `--clock virtual`, 10×/60× simulation). Wall-clock UTC is used only for `ride.started_at` and
  calendar data.
- **Tick scheduling.** The real clock ticks on a `tokio::time::interval` with
  `MissedTickBehavior::Burst`: if the process stalls, missed ticks are processed back-to-back with the
  device samples that arrived, so ride time never drifts from monotonic time.
- **Device samples** are timestamped with `MonoTime` on arrival and consumed at the next tick
  (latest sample wins; FE-C accumulated power and event count give exact average power across dropped
  notifications, ADR-0002 §5).
- **Speed** 10×/60× is only available with the simulated trainer. With a real trainer `setSpeed` returns
  `InvalidInput`.
- **States:** `idle → riding ⇄ paused(reason) → finished`. Ride time and the current block's elapsed time
  advance only in `riding`.

| Pause reason | Enters when | Leaves when |
|---|---|---|
| `user` | pause command / key | resume command |
| `stopped_pedalling` | cadence = 0 **and** power = 0 for 3 s (12 ticks) | cadence > 0 on any tick |
| `trainer_lost` | ADR-0002 link-lost event, or no trainer sample for 2.0 s (8 ticks) | trainer link back **and** cadence > 0 |

Behaviour when BLE drops mid-interval: the block timer freezes at the last good tick; the record gets
`null` for the gap (no interpolation); the ERG target is re-sent on reconnect (ADR-0002 §6); the interval
continues with its remaining time, so the rider never loses or gains work. HR loss does not pause: HR
metrics go `null`, `hr_recovery` blocks fall back to their `maxS` with a visible notice, and the judge
receives no HR features (and therefore cannot `push`, knowledge-pack rule R-GEN-02). The 3 s and 2 s
thresholds are initial values verified in HIL session 1 (#44).

- **Live cursor** (#14): `RideStatus` carries the current block's actual start (ride seconds) and elapsed
  time, so the UI never derives position from planned durations.
- **Per-ride state reset** (#3): a ride is a `RideSession` value created by `ride_load`; nothing ride-scoped
  lives outside it, so a new workout cannot see the previous ride.

Engine sketch:

```rust
pub enum EngineCmd {
    Load(Workout), Start, Pause, Resume, Stop, Adjust { pct: f64 }, SkipBlock,
    SetSpeed(SimSpeed), Subscribe { stream: StreamId, metric: MetricId, opts: SubscribeOpts, reply: oneshot::Sender<SubId> },
    Unsubscribe { stream: StreamId, sub: SubId }, RespondNudge { call_id: CallId, accept: bool },
}
pub struct Engine<C: Clock> { clock: C, session: Option<RideSession>, trainer: Option<TrainerLink>, hr: Option<HeartRateLink>, judge: Box<dyn Judge> }
impl<C: Clock> Engine<C> { pub async fn run(self, cmds: mpsc::Receiver<EngineCmd>, out: EngineOutputs); }
```

## 6. Decision: persistence

- **Engine:** SQLite via **`rusqlite`** (0.40, feature `bundled`, so the SQLite version is ours, not the
  OS's), **WAL** mode, `foreign_keys = ON`. One writer connection on a dedicated blocking thread fed
  by an `mpsc`; readers use a small pool. Other processes (CLI, MCP) open the same file; WAL allows
  concurrent readers with one writer.
- **Migrations:** **`rusqlite_migration`** (2.6) with SQL files `crates/crank-store/migrations/NNNN_name.sql`,
  applied at open, tracked in `PRAGMA user_version`. Forward-only; an applied migration is never edited.
  The store crate owns the schema; no other crate issues SQL.
- **File:** `<data_dir>/crank.db` (§7).

Schema v1 (#33):

| Table | Key columns | Notes |
|---|---|---|
| `athlete_estimate` | `param, value, sd, source, n, updated_at` | append-only history; current = latest per `param` (ONBOARDING.md §3) |
| `athlete_profile` | singleton: weight, age, sex, crank length, PAR-Q result | |
| `workout` | `id, name, dsl, tree_json, ftp_at_creation, source, created_at` | `dsl` canonical text; `tree_json` validated on read |
| `snippet`, `layout` | `id, name, json, updated_at` | validated on read (#4); built-in defaults live in code |
| `ride` | `id, started_at (UTC), ended_at, state (finished/aborted), workout_json, compiled_json, trainer_device_id, hr_device_id, telemetry_source, summary_json` | snapshot of the workout as ridden, incl. coach changes (#16) |
| `ride_chunk` | `(ride_id, metric, start_s)`, `n, data BLOB` | 1 Hz per-second means, 60 s per chunk, `f32` LE, NaN = no data |
| `ride_rr` | `(ride_id, start_ms)`, `data BLOB` | RR intervals (`u16` ms) for DFA-α1/rMSSD recomputation |
| `lap` | `(ride_id, idx)`, `node_id, path, start_s, end_s, stats_json` | per-lap stats by the backend's definition (#12) |
| `recovery` | `(ride_id, node_id, iteration)`, `fit_json, tau_s, observed BLOB` | finished `hr_recovery` blocks |
| `coach_decision` | `call_id, ride_id, t, verdict, band, p, message, proposed_pct, status, rating, useful, features_json` | ratings included (#16) |
| `checkin` | `id, at, sleep, legs, stress, motivation, minutes, note, proposal_workout_id` | |
| `plan_item` | `date, workout_id, status, rationale` | planner week plan (V7) |
| `device` | `id, kind, name, platform_id, serial, last_seen` | pairing memory (ADR-0002 §5) |
| `setting` | `key, json` | settings live here, not in files |
| `import_activity` | `source, external_id, ride_id, imported_at` | intervals.icu dedup |

Crash safety: the open chunk of each metric is upserted every 5 s, so a crash loses at most 5 s of the
ride record. On startup a `ride` without `ended_at` is closed as `aborted` with its data intact and
shown in review.

Rejected: **sqlx** (async API we don't need on a single writer thread; compile-time checked queries need a
live database or an offline query cache in the build); **an embedded KV store (redb, sled)** (the review and
analytics views are relational queries over rides, laps and decisions); **one row per sample** (7 200 rows
per metric per 2 h ride × ~30 metrics; chunks keep the database small and the 1 Hz record cheap to read);
**files per ride (FIT/JSON)** (no transactional updates, no cross-ride queries; FIT export can be added
on top).

## 7. Decision: errors, logging, config, secrets

- **Errors:** `thiserror` enums per crate (`DslError`, `StoreError`, `DeviceError`, `CoachError`, …);
  `crank-core` maps them to `ApiError` (§4.4). `anyhow` only in `crank-cli` `main`. Panics are bugs: the
  workspace sets `clippy::unwrap_used = "deny"` outside tests (#28).
- **Logging:** `tracing` + `tracing-subscriber` (JSON lines) + `tracing-appender` (daily rolling files in
  `<log_dir>`, 7 days kept). Spans: `ride{id}`, `device{kind,id}`, `coach{call_id}`. Raw BLE frames at
  `TRACE` go to a per-ride capture file (`<data_dir>/captures/<ride_id>.fecap`) that the replay tests read
  (ADR-0002 §9). Default level `info`; `CRANK_LOG` overrides (`EnvFilter` syntax).
- **Locations:** the `directories` crate (6.0) with bundle id `io.github.gerchowl.crank`; on macOS
  `~/Library/Application Support/io.github.gerchowl.crank/` (db, captures, ride.lock) and
  `~/Library/Logs/io.github.gerchowl.crank/`. `CRANK_DATA_DIR` overrides both (tests, dev, CLI against a
  scratch profile).
- **Secrets** (#38): the `keyring` crate (4.2) stores the Claude API key and the intervals.icu key in the
  macOS Keychain (service `io.github.gerchowl.crank`). Keys are never written to SQLite, logs, captures or
  the IPC contract (commands return `has_key: bool`). Debug builds only accept `CRANK_ANTHROPIC_API_KEY` /
  `CRANK_INTERVALS_API_KEY` from the environment; release builds compile that path out.

## 8. Decision: testing strategy

| Layer | Tool | What |
|---|---|---|
| Unit | `cargo test` (+ `cargo nextest` in CI) | every crate; domain vectors from `shared/` (§3) |
| Property | `proptest` | DSL round trip, compiler invariants, filters, FE-C encode/decode round trip, chunk codec |
| Fuzz | `cargo-fuzz` | `parse_dsl`, `crank_fec::decode` never panic |
| Snapshot | `insta` | compiled workouts, `bindings.ts`, store migrations' resulting schema |
| Engine integration | `SimTrainer` + `SimHeart` + `VirtualClock` | 45-min workout in milliseconds; scenarios: dropout mid-interval, reconnect, HR loss, skip, manual and auto pause, low-cadence ERG release, coach auto-apply |
| BLE replay | captured `.fecap` files through `crank-fec` + FE-C driver over a fake transport | real NEO behaviour in CI without Bluetooth |
| Store | temp DB | migrate empty, migrate fixtures of every released schema, kill-during-ride recovery |
| UI unit | Vitest + Testing Library | editor tree ops, undo/redo, DSL↔tree sync incl. invalid drafts (#9), stores; `@tauri-apps/api/mocks` `mockIPC` for command calls |
| UI E2E + screenshots | WebdriverIO `@wdio/tauri-service`, embedded provider (`tauri-plugin-wdio-webdriver`, debug builds only) against the real app launched with `--sim --seed N --clock virtual` | view × state matrix into `docs/design/screens/` (`just screens`); validation is #35 |
| Hardware-in-the-loop | owner-run, batched, scripted | ADR-0002 §9 |

Why not Playwright against the real app: `tauri-driver` supports Windows and Linux only, "as macOS has no
WKWebView driver tool available" ([Tauri WebDriver docs](https://v2.tauri.app/develop/tests/webdriver/)).
WebdriverIO's Tauri service has an embedded provider for macOS
([platform support](https://webdriver.io/docs/desktop-testing/tauri/platform-support)); the plugin opens an
HTTP automation endpoint, so it is registered only under `debug_assertions`. Until #35 lands, #24's
Playwright harness against the `shared/` mock remains the design-review tool.

Hardware-in-the-loop protocol: test scripts live in `hil/` and run in a terminal that has the Bluetooth
grant (on sage: an Alacritty window opened via `alacritty msg create-window -e <wrapper>`, see the crank
traps). Sessions are batched into one owner request with a checklist, a time estimate and the exact pedalling
needed; every run writes a capture file and a result note to `docs/hil/<date>-<topic>.md`. No agent
assumes a hardware result it did not see.

## 9. Decision: packaging

- macOS first: Tauri v2 bundle, bundle id `io.github.gerchowl.crank`, minimum macOS 13.
- **Signing for dev** (#37): Apple Development identity, so the Bluetooth TCC grant survives rebuilds
  (ad-hoc signed builds lose it, `spec/TRAINER-BLE.md`).
- `app/src-tauri/Info.plist` adds `NSBluetoothAlwaysUsageDescription`; Tauri merges it with the generated
  plist ([macOS application bundle](https://v2.tauri.app/distribute/macos-application-bundle/)). The
  same key is required for any process using CoreBluetooth on macOS, per the btleplug README.
- Hardened runtime on; **no App Sandbox in v1** (direct distribution; the sandbox would also need the
  `com.apple.security.device.bluetooth` and network-client entitlements and a container for the data dir).
- The `crank` CLI is shipped inside the bundle (`Contents/MacOS/crank`) and on PATH via a documented
  symlink; when run from a terminal its Bluetooth grant belongs to the terminal (TCC responsible process).
- Distribution beyond the owner (Developer ID, notarization, updater) and Windows/Linux bundles: #41.

## 10. Alternatives considered (summary)

| Decision | Alternative | Why rejected |
|---|---|---|
| Rust core in workspace crates | single crate | can't enforce purity / BLE-free engine |
| Engine + BLE in the app process, ride lock | separate daemon | extra signed process for a single-user app; revisit in #40 |
| tauri-specta generated bindings | ts-rs; TauRPC; hand-written TS | types-only drift; extra router; caused #2 |
| One multiplexed `Channel<StreamFrame>` | channel per widget; events | IPC volume, no ordering; events are JSON and unordered |
| Integer 250 ms ticks on a monotonic clock | wall-clock timers; float time | drift and non-determinism |
| rusqlite + rusqlite_migration, chunked 1 Hz | sqlx; KV store; row per sample; files | see §6 |
| WebdriverIO embedded provider | tauri-driver; Playwright on the web build only | no macOS driver; would test the mock, not the core |

## 11. Consequences

- The UI cannot compute physiology: there is no TS module to compute it with. New UI needs that require
  data become contract additions (Rust types, regenerated bindings), which is the intended friction.
- All queries are async. UI components need loading states; the 1 Hz/0.5 Hz polling budget makes this
  cheap.
- tauri-specta stays an RC dependency pinned exactly; upgrading it is a deliberate PR with the bindings
  diff reviewed. ts-rs is the documented exit.
- Real-trainer rides run at 1× only; fast-forward exists only in simulation and tests.
- The desktop app and CLI cannot ride at the same time (ride lock), mirroring the trainer's own
  single-central limit.
- `shared/` lives on until the React app consumes the bindings, and receives no new logic.
- CI runs everything except BLE hardware; hardware truth comes from captures replayed in CI.

## 12. Open questions (filed)

| Question | Issue | Milestone |
|---|---|---|
| Port plan with golden vectors (implementation) | #31 | M1 |
| Engine implementation (clock, pause, dropout, frames) | #32 | M1 |
| Store schema v1 + crash recovery (implementation) | #33 | M1 |
| Core + desktop commands and bindings (implementation) | #34 | M1 |
| Does `@wdio/tauri-service`'s embedded provider work for E2E/screenshots on macOS? | #35 | M1 |
| Spec/contract text alignment with this ADR | #36 | M1 |
| Apple Development signing identity (owner action) | #37 | M1 |
| Keychain secrets (implementation) | #38 | M2 |
| Judge backend: local rules vs a System One model; latency and offline behaviour | #39 | M2 |
| MCP/CLI access to a live ride owned by the desktop app | #40 | M3 |
| Distribution (Developer ID, notarization, updater) and Windows/Linux | #41 | M3 |

Scaffold (#28) acceptance gains: crate list of §2, pinned specta trio, bindings drift test, dependency-edge
check, `clippy::unwrap_used` deny outside tests.

## 13. References

- Tauri v2, calling the frontend (channels vs events): https://v2.tauri.app/develop/calling-frontend/
- Tauri v2, WebDriver (platform support): https://v2.tauri.app/develop/tests/webdriver/
- Tauri v2, mocking (`mockIPC`): https://v2.tauri.app/develop/tests/mocking/
- Tauri v2, macOS application bundle (Info.plist merge, entitlements): https://v2.tauri.app/distribute/macos-application-bundle/
- tauri-specta (v2 RC; Channel support in `examples/app`): https://github.com/specta-rs/tauri-specta
- WebdriverIO Tauri service, platform support: https://webdriver.io/docs/desktop-testing/tauri/platform-support
- btleplug README (platforms, macOS usage description): https://github.com/deviceplug/btleplug
- crates.io versions as of 2026-10-13: tauri 2.12.3, tauri-specta/specta 2.0.0-rc.25, specta-typescript
  0.0.12, ts-rs 12.0.1, rusqlite 0.40.2, rusqlite_migration 2.6.0, keyring 4.2.0, directories 6.0.0,
  rmcp 3.5.1, btleplug 0.13.5, tauri-plugin-wdio-webdriver 1.5.0
- In-repo: `spec/UX-SPEC.md`, `spec/ONBOARDING.md`, `spec/TRAINER-BLE.md`, `spec/SPIKE-RESULTS.md`,
  `shared/README.md`, `shared/DSL.md`, `shared/src/contract.ts`, `spec/knowledge-pack/rules.md`
