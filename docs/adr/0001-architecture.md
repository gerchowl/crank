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
Owner decisions of 2026-10-13 (folded in here): build with a Nix flake using crane, `nix flake check` as
the CI gate, ~~anvil's attic as binary cache~~ (superseded, Amendment 2026-10-13: GitHub-hosted CI, no attic); a real, locally signed `crank.app` (`nix-tcc` identity) from day 1;
and the definition of done in §8.1.

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
| `crank-devices` | lib | `TrainerDriver` / `HeartRateDriver` traits and their types (ADR-0002), `sim` module: simulated rider + trainer + HR strap (port of `sim.ts`) | `crank-domain`, `tokio` (sync, time), `futures-util`, `smallvec`, `thiserror` |
| `crank-fec` | lib, pure | ANT message framing + FE-C page codec (ADR-0002 §4) | none |
| `crank-ble` | lib | btleplug discovery, FE-C trainer driver, HRS driver, reconnect, raw-frame capture (ADR-0002) | `crank-devices`, `crank-fec`, `btleplug` |
| `crank-engine` | lib | ride engine actor: tick clock, ride state machine, metric bus, subscriptions + filter chains, ride record, laps, status, ERG policy, judge hook | `crank-domain`, `crank-devices` |
| `crank-store` | lib | SQLite schema, migrations, repositories | `crank-domain`, `rusqlite` (bundled: pinned SQLite engine), `rusqlite_migration` (forward-only schema versions), `serde`/`serde_json` (validated JSON envelopes), `thiserror` (typed store errors) |
| `crank-coach` | lib | `RulesJudge` (knowledge-pack rules, implements `crank_domain::Judge`), `Planner` trait + Claude API client | `crank-domain`, `reqwest` |
| `crank-intervals` | lib | intervals.icu client + mapping into domain types | `crank-domain`, `reqwest` |
| `crank-core` | lib | `Core` facade wiring everything; **the API** (async methods + DTOs); config/data dirs, secrets, ~~ride lock~~ (superseded by §9.4: the lock lives in `crank-store`; core decides when to open as owner) | all of the above |
| `crank-mcp` | lib | MCP tools over `Core` (`rmcp`) | `crank-core` |
| `crank-cli` | bin `crank` | `clap` CLI: `ride --sim`, `workout parse/compile/print`, `devices scan`, `import intervals`, `mcp` (stdio server), `devserver` | `crank-core`, `crank-mcp` |
| `crank-desktop` | bin (Tauri v2) | thin `#[tauri::command]` wrappers over `Core`, bindings export, window/menu | `crank-core`, `tauri`, `tauri-specta` |

`crank-devices` uses `futures-util` for object-safe boxed connection futures and discovery streams,
`smallvec` for inline HR RR intervals, and `thiserror` for transport-neutral device errors (#43).

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

Float parity tolerance: exports retain full precision; semantic comparisons require exact integers, strings, booleans, structure and array lengths, and floats satisfy `|a−b| ≤ max(1e-12·max(|a|,|b|), 1e-15)` because JS transcendental results vary by platform/libm; downstream Rust tests use the same tolerance unless exactness is justified.

Rejected: **keep the TS logic and run it in the webview** (two implementations or a UI-only core, CLI/MCP
could not use it); **compile the Rust domain to WASM for the UI** (a second runtime to ship for no gain:
the UI asks the core over IPC at ≤ 10 Hz, see §4).

## 4. Decision: the Rust↔UI contract

### 4.1 Single source of truth

Rust types are the contract. DTOs derive `serde::{Serialize, Deserialize}` and `specta::Type`; commands are
annotated `#[tauri::command] #[specta::specta]`; **tauri-specta** generates `app/src/bindings.ts` (types,
typed command wrappers, typed `Channel<T>` parameters) through a small export binary
(`export-bindings`, §9.2). The file is committed; the `bindings-drift` flake check regenerates it and fails
on any diff (§9.1).

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
| **Commands** (request/response, async) | every query and mutation: `history`, `histogram`, `heatmap`, `mmp`, `hr_recovery`, `streams`, `recoveries`, `laps`, `compile_workout`, `parse_dsl`, `print_dsl`, ride control (`ride_load/start/pause/resume/stop/adjust/skip_block`), `ride_status`, `coach_decisions`, `device_links`, `stream_ack`, `stream_snapshot`, layouts, snippets, workouts, athlete model, coach respond/rate, planner propose/rewrite, devices, settings |
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
    pub seq: u64,                        // per stream, +1 per frame
    pub resync: bool,                    // frames were coalesced: re-query discrete state (below)
    pub samples: Vec<SubSample>,         // latest value of each subscription that is due
    pub status: Option<RideStatus>,      // on change, and every 500 ms while riding
    pub nudges: Vec<CoachNudge>,         // latest state per callId changed since last frame
    pub links: Vec<LinkChange>,          // latest trainer / HR link state, if changed
}
pub struct SubSample { pub sub: SubId, pub t: f64, pub v: Option<f64> }  // t = session seconds (§5)

#[tauri::command] #[specta::specta]
async fn stream_ack(core: State<'_, Core>, stream: StreamId, seq: u64) -> Result<(), ApiError>;
```

**Flow control.** Tauri's `Channel::send` has no consumer backpressure: payloads under 8 KiB of JSON are
pushed with `webview.eval`, larger ones are parked in a pending map until the page fetches them
(`crates/tauri/src/ipc/channel.rs`, tauri-v2.12.3, `MAX_JSON_DIRECT_EXECUTE_THRESHOLD = 8192`). Throttling
the send rate therefore does not bound queued work if the page stalls (busy main thread, hidden window,
devtools pause). crank bounds it explicitly:

- The UI calls `stream_ack(stream, seq)` after it has applied a frame. The core keeps **at most 3
  unacknowledged frames** per stream. While the window is full it sends nothing and **coalesces** into
  bounded pending state: per subscription only the latest sample, the latest status, the latest state of
  each of the two links, and **at most 8** changed nudges (latest per `callId`).
- **Byte budget, measured.** Every frame is serialized to JSON first and its length **in bytes** is checked
  against a budget of **8 000 bytes** (below Tauri's 8 192-byte direct-eval threshold, leaving headroom for
  the IPC envelope). Counts and character limits are not the guarantee; the measured bytes are.
  1. Fits → send.
  2. Over budget → **split samples first**: discrete state (status, links, nudges) goes in the first frame,
     samples fill that frame and the following frames (consecutive `seq`), each re-measured; split frames
     count against the ack window.
  3. Discrete state alone over budget → send the **fixed-size resync frame** (`{seq, resync: true}` with
     empty payload, < 100 bytes) and drop the pending discrete state; the UI then queries the full state.
- **Resync path.** The next frame after the ack window reopens carries the coalesced state with
  `resync: true`, subject to the same byte budget (so it may itself be the bare resync frame). On any
  `resync` or `seq` gap the UI re-reads the durable state by query: `ride_status()`, `coach_decisions()`,
  `device_links()`, and `stream_snapshot(stream)` (latest sample per subscription). Discrete state lives in
  the engine and store, so a coalesced, bare or missed frame never loses information.
- **Bounded inputs** (for memory and to keep step 3 rare, not as the size proof): ≤ 64 subscriptions per
  stream (`subscribe` returns `Conflict` beyond that), at most one sample per subscription per frame, ≤ 8
  pending nudges (beyond that the pending nudges are dropped and step 3 applies). Nudge `message` is capped
  at **512 UTF-8 bytes, enforced when the nudge is created** (`CoachNudge::new` returns
  `Err(MessageTooLong)`; judge and planner templates must produce shorter text, and a template that doesn't
  fails its unit test).
- **Required test** (#34): a property test feeds the frame builder arbitrary states at and beyond every
  cap (64 subscriptions, 8 nudges of 512 bytes of 4-byte UTF-8 characters, maximal status) and asserts that
  every emitted frame serializes to ≤ 8 000 bytes, that split frames together carry every sample, and that
  the resync frame is the fixed shape.
- **Cleanup:** streams belong to a window label. `open_stream` from a window drops that window's previous
  streams (page reload); the window's `Destroyed` event drops them; a failed `send` drops the stream.
  Subscriptions die with their stream.

Rates:

| Stream | Rate | Why |
|---|---|---|
| Engine tick / trainer source | **4 Hz** (250 ms) | FE-C native rate; Tacx recommends 4 Hz in both directions (ADR-0002 §4) |
| HR source | per notification (~1 Hz), RR per beat | Heart Rate Service |
| Frame emission | **≤ 10 Hz**, only when something is due | 10 Hz is the maximum display rate; bounds IPC to ≤ 10 messages/s per window regardless of widget count |
| Status heartbeat | 2 Hz while riding, immediately on change | same as mock |
| Ride record | 1 Hz per-second means | `streams()` and persistence |

At 10× / 60× simulated speed the source rates scale with simulated session time, frame emission does not.
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

- **Tick source.** The engine advances in fixed **250 ms ticks** (`SIM_DT` in the mock, FE-C's 4 Hz),
  counted as integers, so time is exact and the same inputs give the same ride.
- **Two time axes.**
  - **Session time** (`session_s`, `SessionTicks(u64)`): ticks since `start`, advancing in every state
    except `idle`/`finished`, including pauses. It is the coordinate of the ride record, `streams()`,
    `Sample.t`, `history()` and laps' `start_s/end_s`, so pauses and dropouts appear as gaps (null) in
    charts and storage instead of disappearing.
  - **Workout time** (`workout_s`, `WorkoutTicks(u64)`): ticks spent in `riding`; it drives the block
    timer, `elapsed`/`remaining`, the workout strip cursor and the planned-vs-actual mapping. The record
    stores it as the metric `workout_s` (null while paused), so every session second maps to a workout
    position.
- **Clock trait.** `trait Clock { fn now(&self) -> MonoTime; async fn sleep_until(&self, t: MonoTime); }`
  with `MonotonicClock` (wraps `tokio::time::Instant`, never wall-clock) and `VirtualClock` (tests,
  `--clock virtual`, 10×/60× simulation). Wall-clock UTC is used only for `ride.started_at` and
  calendar data.
- **Tick scheduling and catch-up.** Tick *k* covers the monotonic window `[t0 + k·250 ms, t0 + (k+1)·250 ms)`.
  The real clock ticks on a `tokio::time::interval` with `MissedTickBehavior::Burst`. Device samples are
  timestamped with `MonoTime` on arrival and assigned to the tick whose window contains the timestamp, not
  to whichever tick runs next; after a stall the missed ticks are replayed in order with their own samples,
  so session time never drifts from monotonic time and late processing changes no values.
- **Sample holding.** A tick without a new trainer sample holds the last value for up to 1.0 s (4 ticks;
  page 25 may arrive at only ~2 Hz, ADR-0002 §5), HR for up to 3.0 s; after that the value is `null`.
  Small packet gaps are filled from FE-C accumulated power as an average over the gap, flagged as
  estimated (ADR-0002 §5).
- **Speed** 10×/60× is only available with the simulated trainer. With a real trainer `setSpeed` returns
  `InvalidInput`.
- **States:** `idle → riding ⇄ paused(reason) → finished`. Workout time and the current block's elapsed
  time advance only in `riding`; session time advances in `riding` and `paused`.

| Pause reason | Enters when | Leaves when |
|---|---|---|
| `user` | pause command / key | resume command |
| `stopped_pedalling` | cadence = 0 **and** power = 0 for 3 s (12 ticks) | cadence > 0 on any tick |
| `trainer_lost` | ADR-0002 link-lost event, or no trainer sample for 2.0 s (8 ticks) | trainer `Link(Ready)` and the engine has re-applied the mode **and** cadence > 0 |

Behaviour when BLE drops mid-interval: loss is only known after the 2.0 s detection window. Those ≤ 8
ticks stay `riding` and count as workout time (no rollback: block transitions inside the window stand, and
nothing already emitted is rewritten); their record seconds are `null` because no sample arrived. From the
detecting tick the block timer is frozen and session time keeps running with `null` record values (no
interpolation). On reconnect the engine re-applies the control mode (ADR-0002 §6) and the interval continues
with its remaining time, so apart from the ≤ 2 s window the rider neither loses nor gains work. HR loss does not pause: HR
metrics go `null`, `hr_recovery` blocks fall back to their `maxS` with a visible notice, and the judge
receives no HR features (and therefore cannot `push`, knowledge-pack rule R-GEN-02). The 3 s and 2 s
thresholds are initial values verified in HIL session 1 (#44).

- **Live cursor** (#14): `RideStatus` carries `session_s`, `workout_s`, and the current block's actual
  start (session and workout seconds) and elapsed time, so the UI never derives position from planned
  durations.
- **Per-ride state reset** (#3): a ride is a `RideSession` value created by `ride_load`; nothing ride-scoped
  lives outside it, so a new workout cannot see the previous ride.

**Implementation notes (#217, `crank-engine` core).** Conventions the rules above leave open, fixed by code and tests:

- A tick is workout time iff nothing paused the ride when it began; every detection (8 missed ticks, 12 zero ticks, link event) takes effect from the next tick. So the 8 missed ticks of the loss window, and the 12 zero ticks of stopped pedalling, stay workout time (no rollback). Hold: a value is usable for the tick it arrived in and the next 4 (trainer) or 12 (HR).
- **Loss-window values.** The trainer hold (1.0 s = 4 ticks) applies first, then the value is `null` for the rest of the window: ticks after the last sample show it for 4 ticks, the remaining ticks of the 8-tick window and everything after are `null`. This is how §5's "null record values" and the hold rule compose.
- **Link events.** Driver link events carry no timestamp (#232 asks for one). The engine stamps each on receipt and applies it before the tick containing the stamp; samples keep their driver timestamp. Every device link has an event count: a sample is tagged with the count at receipt and is stale, and never enters a hold, once a later link transition has been applied. Every applied transition also clears that device's hold. Resume therefore needs a cadence > 0 sample received after `Ready`. Convention for stalls: a transition the engine learns of only after a stall is applied at the stall's end, so a stalled run and a live run agree on everything that carries a timestamp and can differ only around link transitions inside the stall (closed by #232).
- Pauses compose: `user`, `trainer_lost` and `stopped_pedalling` are independent flags (reported in that priority: lost, user, stopped), so a rider's pause survives a reconnect. Ticks of a paused ride record `null` for every metric and `workout_s`.
- After a loss detected by silence (no `Link(Lost)`), the engine re-issues the mode as soon as it detects the loss, because the link is still `Ready`. Control is sent only while the link is `Ready`; resume needs a re-applied mode and cadence > 0 on a tick.
- **Soft start.** It runs only while trainer values are present after the re-issued mode: it starts from the 3 s power at the tick samples start flowing (the window moves with tick time, so power older than 3 s is never an origin), holds its last step while samples are missing and restarts from the then-current power when they return. With no power in the last 3 s there is no origin; the target is sent as is until samples flow (design question #231).
- **No trainer, no ride.** `Start` is rejected with `NoTrainer` / `TrainerNotReady`: DESIGN §5.4 ("Start is enabled when the trainer is connected") and #95 ("Start disabled until trainer: connected"; makes impossible "a ride starting without a connected trainer"). DESIGN §15 has no trainer-less mode; a no-trainer ride is not a mode of the engine.
- **Shutdown.** When every `EngineHandle` is dropped mid-ride the engine finishes the ride (record closed through `RideRecorder::finish`) and clears the ride-active signal.
- The engine finishes the ride when the last block completes (or is skipped); a partial last second is not recorded. Block duration is `FlatBlock.dur_s` rounded to ticks; open-ended end rules are #224, gap filling from accumulated power is #223.
- `crank-engine` depends on neither `crank-ble` nor `crank-store`: the ride-active signal is the `RideActivity` trait (core passes `|a| gate.set_active(a)`), the record is the `RideRecorder` trait (core adapts it to `Store`).
- `VirtualClock::new(speed)` runs `speed`× faster than the tokio timer under it; with `start_paused` tests it is exact and takes no wall time.

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
| `athlete_profile` | singleton: weight, age, sex, crank length, health-check result (column `par_q_result`; name predates #193, rename tracked in #210) | |
| `workout` | `id, name, dsl, tree_json, ftp_at_creation, source, created_at` | `dsl` canonical text; `tree_json` validated on read |
| `snippet`, `layout` | `id, name, json, updated_at` | validated on read (#4); built-in defaults live in code |
| `ride` | `id, started_at (UTC), ended_at, state (finished/aborted), workout_json, compiled_json, trainer_device_id, hr_device_id, telemetry_source, summary_json` | snapshot of the workout as ridden, incl. coach changes (#16) |
| `ride_chunk` | `(ride_id, metric, start_s)`, `n, data BLOB, estimated BLOB` | 1 Hz per-second means indexed by **session** second, 60 s per chunk, `f32` LE, NaN = no data; `estimated` = bitmask of seconds filled from accumulated power (ADR-0002 §5) |
| `ride_rr` | `(ride_id, start_ms)`, `data BLOB` | RR intervals (`u16` ms) for DFA-α1/rMSSD recomputation |
| `lap` | `(ride_id, idx)`, `node_id, path, start_s, end_s, workout_start_s, workout_end_s, stats_json` | session and workout coordinates; per-lap stats by the backend's definition (#12) |
| `recovery` | `(ride_id, node_id, iteration)`, `fit_json, tau_s, observed BLOB` | finished `hr_recovery` blocks |
| `coach_decision` | `call_id, ride_id, t, verdict, band, p, message, proposed_pct, status, rating, useful, features_json` | ratings included (#16) |
| `checkin` | `id, at, sleep, legs, stress, motivation, minutes, note, proposal_workout_id` | |
| `plan_item` | `date, workout_id, status, rationale` | planner week plan (V7) |
| `device` | `id, kind, name, platform_id, serial, last_seen` | pairing memory (ADR-0002 §5) |
| `setting` | `key, json` | settings live here, not in files |
| `import_activity` | `source, external_id, ride_id, imported_at` | intervals.icu dedup |

Crash safety: the open chunk of each metric is upserted every 5 s, so a crash loses at most 5 s of the
ride record. **Recovery runs only in the process that has just acquired `ride.lock`** (§2), before it
starts or loads a ride: holding the lock proves no other process owns a live ride (the advisory lock is
released by the kernel when its holder dies), so any `ride` without `ended_at` is closed as `aborted` with
its data intact and shown in review. Processes ~~that only read or write the store without the lock~~ (superseded by §9.4: which tables a lock-less process may write)
without the lock (`crank mcp`, CLI queries) never run recovery and treat an open ride as live.

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

## 8. Decision: testing strategy and definition of done

### 8.1 Definition of done (owner, 2026-10-13)

v1 is done when **crank runs on sage, is fully tested, and the owner's only manual step is granting
Bluetooth once.** Each required test kind maps to an automated command and a place where it runs:

| Required by the owner | Covered by | Runs where | Command |
|---|---|---|---|
| Unit tests | `cargo nextest` (all crates), Vitest (app) | Nix sandbox | `nix flake check` |
| Property tests | `proptest` (DSL round trip, compiler invariants, filters, FE-C encode/decode, chunk codec) | Nix sandbox | `nix flake check` |
| Simulated-trainer integration | engine scenarios on `SimTrainer` + `SimHeart` + `VirtualClock` | Nix sandbox | `nix flake check` |
| FE-C codec against fixture frames | `crank-fec` tests on the Tacx worked example and committed HIL captures; `ReplayTrainer` through the real FE-C driver | Nix sandbox | `nix flake check` |
| UI e2e | WebdriverIO against the real app on the simulated trainer (§8.3) | sage GUI session, outside the sandbox | `just e2e` |
| Screenshot harness | same harness, view × state matrix to `docs/design/screens/` | sage GUI session, outside the sandbox | `just screens` |
| App on sage, Bluetooth granted once | `.app` from Nix, signed with `nix-tcc` (§9.3) | sage, outside the sandbox | `just bundle` |

`nix flake check` is the CI gate (§9.1). `just e2e`, `just screens` and `just bundle` run on sage
before merging any PR that touches `app/`, `crank-desktop`, `crank-core`'s API or `crank-ble`; the PR body
reports the commands and their exit codes. None of them needs the owner: e2e and screenshots use the
simulated trainer, so they need no Bluetooth. The one owner step is the Bluetooth prompt on the first
launch of the signed app; hardware-in-the-loop sessions (§8.4) are verification, not a setup step.

### 8.2 Test layers

| Layer | Tool | What |
|---|---|---|
| Unit | `cargo nextest` (+ `cargo test --doc`) | every crate; domain vectors from `shared/` (§3) |
| Property | `proptest` | DSL round trip, compiler invariants, filters, FE-C encode/decode round trip, chunk codec |
| Fuzz | `cargo-fuzz` | `parse_dsl`, `crank_fec::decode` never panic; corpora committed, short runs on sage, not in the gate |
| Snapshot | `insta` | compiled workouts, store migrations' resulting schema |
| Bindings drift | `bindings-drift` flake check | regenerated `bindings.ts` equals the committed file |
| Engine integration | `SimTrainer` + `SimHeart` + `VirtualClock` | 45-min workout in milliseconds; scenarios: dropout mid-interval (incl. a block boundary inside the 2 s detection window), reconnect ordering, HR loss, skip, manual and auto pause, low-cadence ERG release, coach auto-apply, stall catch-up (timestamp-assigned samples), packet gaps filled as estimated, compliance across ramps and soft starts, stalled stream consumer (ack window, coalescing, `resync`), crash recovery only under `ride.lock` |
| FE-C fixtures and replay | `.fecap` captures through `crank-fec` + FE-C driver over a fake transport | real NEO behaviour without Bluetooth |
| Store | temp DB | migrate empty, migrate fixtures of every released schema, kill-during-ride recovery |
| UI unit | Vitest + Testing Library | editor tree ops, undo/redo, DSL↔tree sync incl. invalid drafts (#9), stores; `@tauri-apps/api/mocks` `mockIPC` for command calls |
| UI e2e + screenshots | WebdriverIO `@wdio/tauri-service` (§8.3) | flows and the screenshot matrix on the real app |
| Hardware-in-the-loop | owner-run, batched, scripted | ADR-0002 §9 |

### 8.3 UI e2e on macOS

`tauri-driver` supports Windows and Linux only, "as macOS has no WKWebView driver tool available"
([Tauri WebDriver docs](https://v2.tauri.app/develop/tests/webdriver/)). WebdriverIO's Tauri service
uses an embedded provider on macOS: the `tauri-plugin-wdio-webdriver` plugin (1.5.0) runs a WebDriver
server inside the app ([platform support](https://webdriver.io/docs/desktop-testing/tauri/platform-support)).

- The plugin opens an HTTP automation endpoint, so it is behind a cargo feature `e2e` of `crank-desktop`.
  The flake builds a separate package `crank-desktop-e2e` with the feature on; `crank-app` (the bundle,
  §9.3) never enables it, and a flake check asserts that from `cargo metadata`.
- `just e2e` builds `.#crank-desktop-e2e` with Nix, then runs WebdriverIO (from the app's pnpm
  devDependencies) against it with `--sim --seed 42 --clock virtual --data-dir <tmp>`. Under the virtual
  clock a debug command `debug_advance(sim_s)` (feature `e2e` only) moves the ride to each state, so
  screenshots are deterministic.
- A WebView window needs the logged-in GUI session. On sage, flock panes run under sshd and have none, so
  agents launch `just e2e` / `just screens` through the Alacritty IPC wrapper (the same path as BLE runs)
  and read the result files. The screenshot matrix is the one #24 defines; #24's Playwright harness
  against the `shared/` mock remains the design-review tool until this harness reaches parity.
- **#35 gates M1 UI work.** It validates the embedded provider on sage (navigation, element queries,
  screenshots, `debug_advance`). If it fails, the decided alternative is: Playwright drives the Vite
  build in Chromium, and the real Rust core runs as `crank devserver --sim`, reached through a WebSocket
  shim that implements `invoke` and `Channel` for the generated bindings (`window.__TAURI_INTERNALS__`),
  dispatching to the same `Core` methods. That path tests the real core and UI but not WKWebView itself,
  so #35 must record the trade-off and amend this section. Either way `just e2e` and `just screens` keep
  their names and outputs, so §8.1 holds.

#### 8.3.1 Validated setup (#35 part 1, 2026-10-14)

The embedded provider works on sage (macOS 26, aarch64, WKWebView) against the Nix-built
`crank-desktop-e2e`, launched in the GUI session through the Alacritty wrapper: `just e2e` passes. It
covers app launch, `$`/`toHaveText`/`toHaveTitle` element queries, a Rust command round trip through
`window.__TAURI_INTERNALS__.invoke` with a DOM change, and `saveScreenshot` (1440×900 PNG). Two
screenshots of an unchanged page are byte-identical, and so are those of separate app launches, so
no extra font or animation pinning was needed for the shell.

| Part | Pin |
|---|---|
| `tauri-plugin-wdio-webdriver` (cargo feature `e2e`) | `=1.5.0`, with Tauri 2.12.3 |
| `@wdio/tauri-service` | `1.5.0`, `driverProvider: 'embedded'` |
| `webdriverio`, `@wdio/cli`, `@wdio/local-runner`, `@wdio/mocha-framework` | `9.31.9` |
| `@wdio/globals`, `@wdio/spec-reporter` | `9.31.3`, `9.31.2` (the versions the service pins) |

Decisions and findings:

- Only `tauri-plugin-wdio-webdriver` is built in. The service's second plugin `tauri-plugin-wdio` (and its
  frontend import `@wdio/tauri-plugin`, `withGlobalTauri`) is **not** used: it would put test code into
  the production frontend bundle, and IPC from specs works through `window.__TAURI_INTERNALS__` anyway.
  Without it the service's multi-window auto-focus times out for 5 s before every query, so the config
  issues one explicit WebDriver `switchToWindow` in `before`, which turns the auto-focus off (the shell
  is single-window). The service-only helpers (`browser.tauri.execute`, `mock`) are unavailable; revisit
  if a spec needs them.
- The plugin's build script writes generated files next to its manifest, which is read-only in the Nix
  vendor tree; `nix/patches/tauri-plugin-wdio-webdriver-writable-build.patch` runs it from a writable
  copy (flake `cargoVendorDir`). The plugin declares no commands, so it needs no capability.
- The plugin is optional in `crank-desktop`. The `deps-edges` check fails if `crank-desktop`'s production
  graph reaches any `tauri-plugin-wdio*` crate or enables `e2e`, and fails if the `e2e` feature stops
  pulling the plugin in. `build-e2e` keeps the package building in CI.
- A flock pane has no GUI session; `scripts/e2e.sh` hands the run to Alacritty over IPC and reads
  the log and exit code back (`CRANK_E2E_DIRECT=1` runs in the current terminal). `nix develop`
  repoints `TMPDIR`, so the Alacritty socket is looked up in the per-user temp dir.
- Still open: `--sim --seed --clock virtual` with `debug_advance` (#228) and the `just screens` matrix (#229).

### 8.4 Hardware-in-the-loop protocol

Test scripts live in `hil/` and run in a terminal that has the Bluetooth grant (on sage: an Alacritty
window opened via `alacritty msg create-window -e <wrapper>`, see the crank traps). Sessions are batched
into one owner request with a checklist, a time estimate and the exact pedalling needed; every run writes a
capture file and a result note to `docs/hil/<date>-<topic>.md`. Captures become FE-C fixtures (§8.2). No
agent assumes a hardware result it did not see.

## 9. Decision: build, packaging and signing

### 9.1 Nix flake with crane; `nix flake check` is the CI gate

- `flake.nix` at the repo root; inputs `nixpkgs` (nixpkgs-unstable, pinned by `flake.lock`), `crane`, and
  `rust-overlay` for the toolchain from `rust-toolchain.toml` (`craneLib.overrideToolchain`). The devshell
  (`nix develop`) provides the same toolchain, `pnpm_12` (matches `packageManager: pnpm@12.9.0`; present
  in nixpkgs-unstable as of 2026-10-13), Node, `just`, `cargo-nextest`, `cargo-deny` and `cargo-fuzz`.
- **crane:** one `buildDepsOnly` produces the shared `cargoArtifacts`; every Rust check reuses it
  ([crane API](https://crane.dev/API.html)). Sources are explicit `lib.fileset` unions per derivation, so
  doc or UI edits don't rebuild Rust and nothing a derivation reads is silently filtered out:

  | Derivation | Source fileset |
  |---|---|
  | `buildDepsOnly` | `craneLib.fileset.commonCargoSources`; crane's `mkDummySrc` (the default inside `buildDepsOnly`) replaces every `.rs` file with a stub but needs to see which `src/lib.rs`, `src/main.rs`, `src/bin/*`, tests, benches and examples exist to keep Cargo's implicit targets, so Rust edits don't invalidate `cargoArtifacts` ([mkDummySrc.nix](https://github.com/ipetkov/crane/blob/master/lib/mkDummySrc.nix)) |
  | Rust checks (`clippy`, `nextest`, `doctest`, `doc`) | `craneLib.fileset.commonCargoSources` + `crates/*/migrations` + `crates/*/tests/fixtures` + `spec/knowledge-pack/rules.md` + `app/src-tauri/{tauri.conf.json,capabilities,icons,Info.plist}` (read by `tauri::generate_context!`) |
  | `bindings-drift` | the Rust check set + **`app/src/bindings.ts`** (the committed file it compares against) |
  | `crank-desktop`, `crank-desktop-e2e` | the Rust check set + the `app-dist` output (`frontendDist`) |
  | `app-dist`, `app-*` checks | `app/` without `src-tauri/`, `pnpm-lock.yaml`, `package.json` |
  | `crank-app` | `crank-desktop` and `crank-cli` outputs + `app/src-tauri/{Info.plist,icons}` |

  Commonly missed inputs (`commonCargoSources` keeps only Cargo, Rust and TOML files) are therefore named
  explicitly; a derivation that needs a new kind of file adds it to its row in the same PR.
- **Flake `checks`** (per system, `aarch64-darwin` and `x86_64-linux`):

  | Check | crane / derivation |
  |---|---|
  | `fmt` | `cargoFmt` |
  | `clippy` | `cargoClippy`, `--all-targets -- -D warnings` |
  | `nextest` | `cargoNextest` (unit, property, engine-sim, store, FE-C fixtures) |
  | `bindings-drift` | runs the bindings export binary and `diff`s its output against the committed `app/src/bindings.ts` |
  | `doctest` | `cargoDocTest` |
  | `doc` | `cargoDoc` with `RUSTDOCFLAGS=-D warnings` (`missing_docs` denied) |
  | `deny` | ~~`cargoDeny` (bans, licenses, sources)~~ `cargoDeny` (advisories, bans, licenses, sources; advisories amended by §9.5) |
  | `deps-edges` | script over `cargo metadata`: forbidden crate edges (§2) and `e2e` feature not in `crank-app` |
  | `app-typecheck`, `app-lint`, `app-test` | frontend derivations: `tsc --noEmit`, ESLint, Vitest |

- **Packages:** `crank-cli`, `crank-desktop`, `crank-desktop-e2e`, `app-dist` (Vite build), `crank-app`
  (the unsigned macOS bundle, `aarch64-darwin` only).
- **CI:** ~~the fleet runners (#25, anvil vm-runner, `x86_64-linux`) run `nix flake check` on every PR into
  `dev`, as the required job. sage runs `nix flake check` for `aarch64-darwin` plus the §8.1 GUI commands
  before merge.~~ **Superseded by the Amendment 2026-10-13 (below):** GitHub-hosted runners run the gate.
  `just check` is a local alias for `nix flake check`; sage still runs the §8.1 GUI commands.
- **Binary cache:** ~~anvil's atticd is a substituter~~ **superseded (Amendment 2026-10-13).** The flake
  declares **no `nixConfig`**, per the architect's #28 round 2 decision: hosts that have a cache get it
  from host configuration; everyone else uses cache.nixos.org and source. Never use
  `--accept-flake-config` or change nix.conf for this project.

### 9.2 Frontend and generated bindings in Nix

- **pnpm dependencies:** a fixed-output derivation via nixpkgs `fetchPnpmDeps` (pnpm 12, `hash` in
  `flake.nix`), consumed with `pnpmConfigHook` by `app-dist` and the frontend checks. Updating
  `pnpm-lock.yaml` requires updating that hash in the same commit; the check fails otherwise, by design.
  Rejected: `node2nix`/`dream2nix` (generated Nix per package, a second lockfile to keep in sync) and
  building the frontend outside Nix (the gate would not cover it).
- **Bindings:** `app/src/bindings.ts` is generated by a small export binary (`crank-desktop --bin
  export-bindings`, built with the Rust check set) and committed. The frontend derivations consume the
  committed file, so the frontend builds without building Rust; the `bindings-drift` check regenerates it
  and fails on any diff (its source explicitly includes the committed file, §9.1), so it cannot go stale.
  Developers run `just bindings` after changing API types.
- **Embedding:** Tauri embeds the frontend at compile time, so `crank-desktop` takes `app-dist` as input
  (`frontendDist` set to the derivation's output path).

### 9.3 macOS bundle and stable TCC from day 1

- `crank-app` (Nix, sandboxed) assembles an **unsigned** `crank.app`: `Contents/MacOS/crank-desktop`, the
  CLI as `Contents/MacOS/crank`, `Resources/` (icon), and `Info.plist` from `app/src-tauri/Info.plist`
  with `CFBundleIdentifier = io.github.gerchowl.crank`, `CFBundleExecutable = crank-desktop`,
  `LSMinimumSystemVersion = 13.0` and `NSBluetoothAlwaysUsageDescription` (ADR-0002 §8). It is a file
  layout step; Tauri's bundler and `codesign` don't run in the sandbox (the bundler wants to sign, and
  signing needs the login keychain).
- **`just bundle` (on sage, outside the sandbox):** `nix build .#crank-app`, copy to
  a writable temp path beside `~/Applications/crank.app`, sign it there inside-out with the self-signed
  **`nix-tcc`** identity, selected by its SHA-1 (the identity the check below verifies), and swap it atomically
  into the stable path (`scripts/install-app.sh`; never a window with no app). The identity already exists
  in sage's login keychain (g-fleet `bin/nix-tcc-cert`, the same identity g-fleet's `tcc-sign.nix` uses so
  TCC grants survive rebuilds):
  `codesign --force --sign <SHA-1> --identifier io.github.gerchowl.crank.cli Contents/MacOS/crank`, then
  `codesign --force --sign <SHA-1> --identifier io.github.gerchowl.crank crank.app`. It finishes with
  `codesign --verify --strict` and asserts that `codesign -d -r-` prints the designated requirement
  `identifier "io.github.gerchowl.crank" and certificate leaf = H"<nix-tcc SHA-1>"`.
- TCC records a grant against the app's designated requirement (bundle id + signing certificate) rather
  than its cdhash, so the owner grants Bluetooth on the first launch and the grant is expected to survive
  rebuilds. That is the owner's only manual step (§8.1). Apple notes that support for self-signed identities
  varies by subsystem ([TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)),
  and the fleet's `nix-tcc` precedent (g-fleet `tcc-sign.nix`) covers other TCC services, not Bluetooth, so
  this is an **acceptance gate of #37**: launch from Finder → one Bluetooth prompt → grant → scan finds the
  NEO; then change a source file, rebuild, re-sign with `just bundle`, relaunch → scan works with **no**
  prompt. If the gate fails, local builds switch to an Apple Development certificate from the owner's
  Apple Developer account (the account #47 uses); that adds one owner step (installing the certificate)
  and this section is amended.
- **No hardened runtime and no App Sandbox for local builds.** The Nix-built binaries load dylibs from
  `/nix/store`, and hardened-runtime library validation rejects libraries not signed by the same team (a
  self-signed certificate has no team ID). The local bundle is not relocatable to another Mac; that's fine
  for sage.
- **Distribution** to other Macs (Developer ID Application with the owner's Apple Developer account,
  hardened runtime, self-contained dylibs, notarization, stapling) is #47 (M3). Windows/Linux bundles: #41.
- The `crank` CLI on PATH is a symlink to `~/Applications/crank.app/Contents/MacOS/crank`. Run from a
  terminal, its Bluetooth grant belongs to the terminal (TCC responsible process); BLE runs by agents use
  the Alacritty path (ADR-0002 §8).

### 9.4 Amendment 2026-10-13: CI is GitHub-hosted (#122, #166)

*Amends §1, §2, §6 and §9.1. The struck-through text above is kept as history.*

- **CI.** Since #58 the gate is `nix flake check -L` on GitHub-hosted `ubuntu-latest` and `macos-latest`;
  the `CI Summary` job is required on `dev` and `main`. The fleet runner (anvil vm-runner) was offboarded
  from this repository because **self-hosted runners must never serve a public repository**: a pull request
  from a fork would execute untrusted code on a machine inside the owner's network (g-fleet#170).
  The attic binary cache is not used by this repository either; CI uses the magic Nix cache on hosted
  runners. No internal hostnames or cache keys are published here; the `public-hygiene` flake check
  (`scripts/check-public-hygiene.sh`) fails on internal hostnames and absolute home paths in tracked files.
- **Ride lock (amends §2 item 3).** The ride lock is acquired by `crank-store` (`Store::open_owner`), the
  narrowest place that can guarantee "recovery only in the lock holder". `crank-core` decides *when* to open
  the store as owner; it does not implement the lock.
- **Lock-less writers (amends §6).** A process without the lock may write the non-ride tables (`rating`,
  `checkin`, `setting`, `plan_item`, …) on its own connection, relying on SQLite WAL locking plus
  `busy_timeout`. It never writes `ride` or `ride_chunk` and never runs recovery. The current store is
  strictly read-only for lock-less processes (`StoreError::ReadOnly`) and stays so until the first lock-less
  writer (MCP/CLI) lands; #166 tracks that implementation.

### 9.5 Amendment 2026-10-13: supply-chain gate (#124)

*Amends §9.1 (`deny` row). Dependency policy is `deny.toml`; this section is the rationale.*

- **RustSec in the gate.** The `deny` check now runs `advisories` in addition to `bans licenses sources`.
  The Nix sandbox has no network, so the database is the pinned flake input `advisory-db`
  (`github:rustsec/advisory-db`, `flake = false`), copied into a git checkout at build time and read with
  `cargo deny --offline`. Advisory updates therefore arrive as ordinary `flake.lock` bumps:
  `nix flake update advisory-db`, then `nix flake check -L`; a new advisory fails the check in that PR
  and is fixed or ignored there. `deny.toml` stays the single policy file; Nix only injects the db path.
- **Policy.** Vulnerabilities, `unmaintained = "all"` and `unsound = "all"` (transitive included) and
  `yanked = "deny"` all fail. There are no blanket ignores: every `ignore` entry in `deny.toml` carries a
  reason and a tracking issue. Today that is three Linux-only transitive entries tracked in #180, which
  must be empty before it closes: `glib` 0.18 (RUSTSEC-2024-0429) is a Linux **runtime** dependency
  (tauri's gtk3 stack) whose advisory is *unsound*-only (`VariantStrIter`, never called by crank), not a
  build-time crate; `paste` (RUSTSEC-2024-0436, via specta) and `proc-macro-error`
  (RUSTSEC-2024-0370, via gtk3-macros/glib-macros) are *unmaintained* proc-macro crates that exist at
  build time only and are not linked into the app.
- **Fuzz workspace.** `crates/crank-fec/fuzz` is a separate workspace with its own `Cargo.lock`, so the root
  `deny` never sees it. The flake check `deny-fuzz` runs `advisories` against that lockfile, offline
  against the same pinned `advisory-db`, and `advisories.yml` runs `cargo deny --manifest-path` on it
  live. Only advisories: the fuzz workspace is dev-only and never shipped, so bans/licenses/sources
  (policy for shipped code) are not applied to it, but a vulnerable dev dependency still fails.
- **What the sandbox cannot see.** Yanked releases need the crates.io index, so the in-gate run silences
  `index-failure` (`-A`) and the pin means advisories published after the last bump are invisible to PRs.
  `.github/workflows/advisories.yml` (#181) runs weekly, on lockfile/`deny.toml` PRs and on demand against
  the live db and index, outside `CI Summary` so an upstream advisory cannot block unrelated work.
  An automated "bump the lock and open a PR" job was rejected: PRs opened with `GITHUB_TOKEN` do not
  trigger the required CI, and a GitHub App for that is not worth it for one input. A red weekly run is
  the prompt for a manual bump. To make that prompt visible, a failed *scheduled* run opens (or comments
  on) a single GitHub issue labelled `security` with the run link (`report-failure` job, `issues: write`
  scoped to that job only, `GITHUB_TOKEN`, GitHub-hosted runner).
- **Frontend.** `pnpm audit` queries the registry, so it cannot run in the sandbox. Decision: run
  `pnpm audit --prod` in the same workflow (production dependencies only: dev tooling never ships in the
  app). Rejected: a pinned offline npm advisory database (no maintained Nix-packaged source) and gating
  PRs on it (same upstream-blocks-unrelated-work problem as above).

## 10. Alternatives considered (summary)

| Decision | Alternative | Why rejected |
|---|---|---|
| Rust core in workspace crates | single crate | can't enforce purity / BLE-free engine |
| Engine + BLE in the app process, ride lock | separate daemon | extra signed process for a single-user app; revisit in #40 |
| tauri-specta generated bindings | ts-rs; TauRPC; hand-written TS | types-only drift; extra router; caused #2 |
| One multiplexed `Channel<StreamFrame>` | channel per widget; events | IPC volume, no ordering; events are JSON and unordered |
| Integer 250 ms ticks on a monotonic clock | wall-clock timers; float time | drift and non-determinism |
| rusqlite + rusqlite_migration, chunked 1 Hz | sqlx; KV store; row per sample; files | see §6 |
| WebdriverIO embedded provider | tauri-driver; Playwright on the web build only | no macOS driver; would not exercise WKWebView (kept as the #35 alternative, §8.3) |
| Nix flake + crane, `nix flake check` as gate | plain cargo + pnpm in CI; `nix develop -c just check` | not hermetic, no shared dependency artifacts, no attic reuse |
| pnpm deps as fixed-output derivation (`fetchPnpmDeps`) | node2nix / dream2nix; frontend outside Nix | second lockfile; gate would not cover the UI |
| `.app` assembled in Nix, signed by `just bundle` with `nix-tcc` | Tauri bundler; Apple Development / Developer ID for local builds | bundler signs inside the build; Apple identities are distribution concerns (#47) and not needed for a stable TCC grant |

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
- CI runs everything except BLE hardware and the GUI-session steps; hardware truth comes from captures
  replayed in CI, and `just e2e` / `just screens` / `just bundle` run on sage before UI-touching merges.
- Every `pnpm-lock.yaml` change also updates the pnpm deps hash in `flake.nix`.
- The local `crank.app` depends on `/nix/store` and runs only on sage (or another Nix Mac); a
  distributable bundle is #47's work.
- The owner's only manual step for v1 is granting Bluetooth once to the `nix-tcc`-signed app.

## 12. Open questions (filed)

| Question | Issue | Milestone |
|---|---|---|
| Port plan with golden vectors (implementation) | #31 | M1 |
| Engine implementation (clock, pause, dropout, frames) | #32 | M1 |
| Store schema v1 + crash recovery (implementation) | #33 | M1 |
| Core + desktop commands and bindings (implementation) | #34 | M1 |
| Does `@wdio/tauri-service`'s embedded provider work for E2E/screenshots on macOS? | #35 | M1 |
| Spec/contract text alignment with this ADR | #36 | M1 |
| `just bundle`: Nix-assembled `crank.app` signed with `nix-tcc`, DR check (implementation) | #37 | M1 |
| Keychain secrets (implementation) | #38 | M2 |
| Judge backend: local rules vs a System One model; latency and offline behaviour | #39 | M2 |
| MCP/CLI access to a live ride owned by the desktop app | #40 | M3 |
| Developer ID signing + notarization for a distributable `crank.app` | #47 | M3 |
| Windows/Linux bundles and BLE behaviour | #41 | M3 |

Scaffold (#28) acceptance gains: crate list of §2, pinned specta trio, bindings drift test, dependency-edge
check, `clippy::unwrap_used` deny outside tests, and the flake of §9.1–9.2 (crane checks, `fetchPnpmDeps`,
no `nixConfig`; the attic substituter was superseded, §9.4) with `nix flake check` green.

## 13. References

- Tauri v2, calling the frontend (channels vs events): https://v2.tauri.app/develop/calling-frontend/
- Tauri v2, WebDriver (platform support): https://v2.tauri.app/develop/tests/webdriver/
- Tauri v2, mocking (`mockIPC`): https://v2.tauri.app/develop/tests/mocking/
- tauri-specta (v2 RC; Channel support in `examples/app`): https://github.com/specta-rs/tauri-specta
- WebdriverIO Tauri service, platform support: https://webdriver.io/docs/desktop-testing/tauri/platform-support
- btleplug README (platforms, macOS usage description): https://github.com/deviceplug/btleplug
- crane API (`buildDepsOnly`, `cargoClippy`, `cargoNextest`, `cargoDocTest`, `cargoDoc`, `cargoDeny`, filesets): https://crane.dev/API.html
- nixpkgs-unstable (evaluated 2026-10-13): `pnpm_12` = 12.9.0, `fetchPnpmDeps`, `pnpmConfigHook` present
- g-fleet: `bin/nix-tcc-cert` and `modules/home/tcc-sign.nix` (stable `nix-tcc` signing for TCC)
- crates.io versions as of 2026-10-13: tauri 2.12.3, tauri-specta/specta 2.0.0-rc.25, specta-typescript
  0.0.12, ts-rs 12.0.1, rusqlite 0.40.2, rusqlite_migration 2.6.0, keyring 4.2.0, directories 6.0.0,
  rmcp 3.5.1, btleplug 0.13.5, tauri-plugin-wdio-webdriver 1.5.0
- In-repo: `spec/UX-SPEC.md`, `spec/ONBOARDING.md`, `spec/TRAINER-BLE.md`, `spec/SPIKE-RESULTS.md`,
  `shared/README.md`, `shared/DSL.md`, `shared/src/contract.ts`, `spec/knowledge-pack/rules.md`
