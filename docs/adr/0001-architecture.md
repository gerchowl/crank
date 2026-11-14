# ADR-0001: Architecture, crate layout, Rust↔UI contract, persistence, testing

- **Status:** Proposed; amended 2026-11-09 (#588: §4.2, §4.3, §10); 2026-11-11 (#585: §7.2 `slew` fields)
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
| `crank-engine` | lib | ride engine actor: tick clock, ride state machine, metric bus, display series (§4.3.3; subscriptions + filter chains until #588 step 3), ride record, laps, status, ERG policy, judge hook | `crank-domain`, `crank-devices` |
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
| **Commands** (request/response, async) | every query and mutation: `history`, `histogram`, `heatmap`, `mmp`, `hr_recovery`, `streams`, `recoveries`, `laps`, `compile_workout`, `parse_dsl`, `print_dsl`, ride control (`ride_load/start/pause/resume/stop/adjust/skip_block`), `coach_decisions`, `ui_ack`, layouts, snippets, workouts, athlete model, coach respond/rate, planner propose/rewrite, devices, settings |
| **One `Channel<UiFrame>` per window** (§4.3, amendment #588) | all live data, with or without a ride: the full UI state (ride, devices, nudges) and the series points not yet acknowledged. Acked with a window of 1. Until #588 step 3 the code has two channels instead: `Channel<StreamFrame>` (subscribed samples, ride status, link changes) and `Channel<LinkFrame>` (the link keeper's state, ADR-0002 §5.1.4) |
| **Events** | only low-rate, app-wide invalidation hints for multiple windows: `store_changed { table }` |

Commands mutate or query durable data. They answer `()`, an id or an error, never live state that the page would
have to merge with the channel (§4.3.8). Queries become async (they were synchronous in the mock). The polling budget in `shared/README.md`
stays (history ≤ 1 Hz per chart, distributions ≤ 0.5 Hz, MMP ≤ 0.2 Hz); the UI animates between polls.
Spec text update: #36.

### 4.3 Live data: one snapshot channel per window (amendment 2026-11-09, #588)

**Status.** Owner decision of 2026-11-09 (#588, which holds the reasons). It replaces the original §4.3 (the multiplexed
`Channel<StreamFrame>` with per-widget subscriptions: #219, #34, #89) and ADR-0002 §5.1.4's `Channel<LinkFrame>` (#470,
#561); `dev` keeps them until steps 2 and 3 (§4.3.10). "Today" cites `dev` at 2de24c5.

#### 4.3.1 The design (normative)

The rest of §4.3 defines what these sentences name. Where they disagree, this list wins.

1. Each window opens one ordered snapshot channel to the core.
2. The core keeps each loaded ride's series in an append-only store; the engine appends to it and publishes its status
   under the store's lock.
3. At most every 100 ms, and only when the last frame has been acked, the core sends a `UiFrame`: the full current state
   plus up to 256 new points per series from that channel's cursor. It sends one at least every second.
4. Sending moves the cursor; the ack only opens the window.
5. The page replaces its state, appends the points (or replaces a series when `from` is 0), and acks.
6. A stalled page holds one frame and catches up without loss when it acks.
7. A sent frame can still be lost (§4.3.4). If no frame arrives for 2 s, the page opens a new channel, and the core
   closes the old one.
8. A new channel or a ride load starts the cursors at 0, so the traces backfill while the live numbers and safety state
   show at once.
9. With one frame of ≤ 32 KB in flight, a safety change reaches the page within about 200 ms.
10. Commands answer only `()`, an error or the ride id, and never feed what is drawn. A page load closes the window's
    old channels by id; the core sets `engine_unresponsive` itself.

#### 4.3.2 Types and the client rule

```rust
// commands: #[tauri::command] #[specta::specta]; types: #[derive(Serialize, specta::Type)], camelCase on the wire
async fn ui_open(core: State<'_, Core>, window: Window, channel: Channel<UiFrame>) -> Result<UiChannelId, ApiError>;
async fn ui_ack(core: State<'_, Core>, window: Window, channel: UiChannelId, seq: u32) -> Result<(), ApiError>;

pub struct UiFrame {
    pub channel: UiChannelId,    // the channel the frame belongs to; the page acks with it
    pub seq: u32,                // per channel, from 0, +1 per frame; the ack names it
    pub ride: Option<RideView>,  // the loaded ride, or the last ended one; null before any, after an unload, and
                                 // from the start of a load until it installs its ride (§4.3.3)
    pub loading: bool,           // a `ride_load` is under way; its per-device progress is `devices`
    pub devices: DevicesView,    // both roles, always
    // pub nudges: Vec<NudgeView>, added by #248: the live nudges, latest per call id, at most 8
    pub series: SeriesDelta,     // the points from this channel's cursor
}
pub struct RideView {
    pub id: String,                  // the id `ride_load` answered
    pub workout_id: Option<String>,  // null for a free ride
    pub status: StreamStatus,        // the engine's status (§4.3.6)
    pub live: LiveValues,            // this tick's values, for the big numbers (§4.3.3)
    pub ended: Option<RideEnding>,   // null until the ride is over
    pub engine_unresponsive: bool,   // §4.3.5
}
pub enum RideEnding { Finished, Forced }  // snake_case; forced: the core ended it, the engine stuck or gone (#331)
pub struct LiveValues { pub power_w: Option<u16>, pub cadence_rpm: Option<u16>, pub hr_bpm: Option<u16> }
                                     // null: paused, lost, a gap, or not riding
pub struct DevicesView { pub trainer: DeviceView, pub strap: DeviceView }
pub struct DeviceView {
    pub paired: Option<PairedDevice>,  // null when nothing is paired for the role; strings cut (§4.3.3)
    pub phase: Option<LinkPhase>,      // the keeper's phase (ADR-0002 §5.1.1); null exactly when not paired
    pub link: Option<LinkStatus>,      // the ride's link state; set exactly when `phase` is `riding`
    pub sample: Option<LinkSample>,    // the keeper's latest sample while `warm`; null otherwise
}
pub struct SeriesDelta { pub power: Points, pub cadence: Points, pub hr: Points }
pub struct Points {
    pub from: u32,                               // index of the first point; 0 starts the series again
    pub t: Vec<f64>, pub w: Vec<f64>, pub v: Vec<u16>,  // session s (§5), workout s, and W, rpm or bpm
}
```

**The one client rule.** For each frame of the channel it holds, in arrival order, the page (1) replaces `ride`,
`loading` and `devices` (and `nudges`, once #248 adds it); (2) per series, replaces its points when `from` is 0 and
appends them otherwise; (3) calls `ui_ack(frame.channel, frame.seq)`. It compares no `seq`, runs no re-query, merges no
command's answer, keeps nothing from an earlier frame but the series, and drops the frames of a channel it replaced. It
routes on the frame's `ride`, never on `ride_load`'s id. **Watchdog:** no frame for 2 s on an open channel, and it calls
`ui_open` again (§4.3.4). The core guarantees `from` equals the length the page holds (tested, §4.3.9); a mismatch is a
core bug, reported as `internal`, not repaired.

#### 4.3.3 Series, size and rate

**Series.** One append-only series per metric for the loaded ride, in memory, on session time: `power` (W) every tick (4
Hz), `cadence` (rpm) and `hr` (bpm) every second tick (2 Hz), today's rates (`app/src/ride/Ride.tsx:357-359`, DESIGN
§6.4). It is display data; the 1 Hz record (§6) stays the truth. A tick appends a point only while riding and when the
value is present and finite: gaps are absent (§5), as the page skips them today (`Ride.tsx:398-401`). The big numbers
read `ride.live`, the latest tick (`null` while paused, lost, in a gap or not riding, as subscriptions deliver today,
`crates/crank-engine/src/engine.rs:1216-1237`). A 6 h ride is about 3 MB per process (18 B a point).

**Store and lock.** The core owns the store (behind a `std` mutex). The engine appends and `send_replace`s the tick's
status under its lock (`engine.rs:1597-1607`), and the frame builder reads both under it, so a frame's status and points
are one tick's. The engine's wait is bounded, not zero: one builder's section (a status clone, ≤ 256 points per series).
It never sees a channel or awaits a page and publishes its status on its `watch` as now (`EngineHandle::subscribe`,
`engine.rs:351`).

**Ride lifetime.** From the start of `ride_load`, `loading` is true, `ride` is `null` and the series are dropped, so the
next frame carries `from: 0` and no points (today the core keeps the previous status until the new session is installed,
`crates/crank-core/src/core.rs:1834`). A failed load leaves `ride` `null`; an unload makes it `null`; an ended ride
stays, with `ended` set, until the next load.

**Frame bound.** Each bound is measured by a test with every part at its cap:

| Part | Bound |
|---|---|
| State (`ride`, `loading`, `devices`) | ≤ 4 000 B. The worst status is 1 080 B (`crates/crank-engine/src/stream/tests.rs:1257-1271`). Each `PairedDevice` string is cut to 64 B on a character boundary, as `record_error` is (§4.3.6) |
| Nudges (#248, with its own size test) | ≤ 6 000 B: 8 messages of ≤ 512 B plus their fields |
| Series | ≤ 256 points per series, oldest first. A point is ≤ 26 B of JSON up to 999 999.75 s, so ≤ 20 KB |
| Whole frame | ≤ 32 KB. A live frame (no backlog, no nudges) is 1–2 KB |

Tauri sends a frame under 8 192 B by `eval`; a larger one is parked and fetched by the page, one more round trip, in
index order (tauri 2.12.3 `src/ipc/channel.rs:36`, `:284`, `:293-320`). **Why 256:** a safety change (trainer lost, ERG
released, a pause) waits at most for the frame in flight and the 100 ms spacing, and ≤ 32 KB keeps a round trip ≤ 100
ms, so it reaches the page within about 200 ms during a backfill. #598 measures the round trip on sage, failing above
100 ms.

**Rate.** A frame is **due** when the state differs from the last frame sent, when points are past the cursor, or 1 s
after the last frame (the **heartbeat**); the first goes out at once on `ui_open`. A due frame is **sent** once the last
is acked and ≥ 100 ms after it on the unscaled pacing axis (`Clock::pacing_now`), so ≤ 10 Hz at any simulated speed. The
newest state wins; points accumulate. A ride sends at its 4 Hz tick; outside one, on a change or the heartbeat.

#### 4.3.4 Flow control and recovery

- **Ack window of 1.** After a frame the channel sends nothing until `ui_ack(channel, seq)` names that frame. An ack of
  any other `seq` is ignored. An ack for a channel the calling window does not hold (another window's, or a closed one)
  is `NotFound`. The link channel works this way today (`crates/crank-core/src/keeper/channel.rs:96-113`, `:121-144`).
- **Cursor.** There is one cursor per channel and series: the index of the first point not yet sent. A frame carries the
  points from `cursor` up to `min(len, cursor + 256)`, and **sending it moves the cursor** past them. A load starting
  resets every cursor to 0.
- **Stall.** A stalled page (busy, hidden, paused in devtools) holds one frame and costs one cursor. Its ack brings the
  state now and the points after its last frame. No channel has a buffer; nothing is re-queried.
- **Lost frame.** A successful send does not guarantee delivery. A frame of 8 192 B or more is fetched in a promise that
  ends in `.catch(console.error)`, so a failed fetch loses it silently (`channel.rs:306-320`), and a send to a page
  being replaced returns `Ok` and delivers nothing (`:310-316`). Unacked, the channel would stall for good. The
  heartbeat means a frame at least every second, so after 2 s without one the page's watchdog calls `ui_open`; the core
  closes the older channel (§4.3.5), and the new one backfills from 0. No resend, no cursor on the ack. A watchdog
  firing after a suspension (a hidden window) costs only a backfill.
- **Backfill.** A new channel starts every cursor at 0, so its first frames carry the ride's whole series, oldest first,
  256 points per series per frame, with the current state in each. A 3 h ride (43 200 power points) takes 169 frames,
  about 17 s at 10 Hz, until #599 downsamples the backfill to the trace's vertices.

#### 4.3.5 Channel lifecycle, by channel id (#564)

- **Id.** `ui_open` answers a `UiChannelId`: a per-process `u32` counter from 1, never reused within a process. The
  channel belongs to the calling window.
- **End.** A channel ends (1) when a send fails; (2) when its window starts loading a page (`PageLoadEvent::Started`) or
  is destroyed: the handler closes the ids the window holds **at the event**, synchronously in the core's channel map (a
  `std` mutex), never one the next page opens; (3) when the same window opens another (a remount, the watchdog). Nothing
  is closed by label after the fact, so #564's race (a deferred close by label removing a newer stream,
  `core.rs:2615-2635`, `app/src-tauri/src/lifecycle.rs:276-290`) cannot occur.
- A channel opens with or without a ride. At most 16 are open per process, `Conflict` beyond that, as for streams today
  (`crates/crank-engine/src/stream/hub.rs:23`).
- **Engine unresponsive (#351).** Today acks go through the engine (`core.rs:2583-2595`), so a wedge shows within 1 s.
  Now the core sets `ride.engine_unresponsive` when a bounded engine request times out (`REQUEST_TIMEOUT`,
  `core.rs:85`), or when a ride in progress has not written its status watch for `REQUEST_TIMEOUT`. A paused ride writes
  it: a pause stays `Active` and ticks (`engine.rs:868-872`, `:1552-1556`), and every wake ends in `publish` (`:863`),
  whose `send_replace` (`:1607`) marks the watch changed. The flag stays until the ride ends; an unstarted ride does not
  tick, so there Start answers `engine_unresponsive`, as today.

#### 4.3.6 What the core computes into the snapshot

- **One source per device.** A device a ride holds has `phase: riding` and the ride's `link` (`connecting`, `ready`,
  `lost`), from the same engine status as `ride.status` (`RideStatus` carries both, `engine.rs:52-78`), so a loss and
  its pause share a frame. Every other device is the keeper's (ADR-0002 §5.1.4).
- **`loading`** replaces `RideStatusView.load` (`crates/crank-core/src/dto.rs:193-199`, `:428-434`); per-device progress
  is in `devices`.
- **`ride.ended`** replaces `RideEnded.forced` (`dto.rs:417-423`) and `app/src/ride/forcedEnds.ts`, which a reload loses
  (`forcedEnds.ts:1-5`); the core keeps it until the next load (`core.rs:736`, `:2229-2233`).
- **Laps** stay a query (`laps`, #243 part c). `status.laps_closed` tells the page when to read it again.

**Status notes** (#219, #243, #324, #329) stay in force, documented on each field of `StreamStatus`
(`crates/crank-engine/src/stream/types.rs:293-364`) and in `git show 2de24c5:docs/adr/0001-architecture.md`.
Thresholds and flags (#243 part b) and nudges (#248) enter later, each a field with its own size test.

#### 4.3.7 Per-widget rates or filters

If a widget ever needs its own rate or filter, the core computes it into the snapshot.

#### 4.3.8 Commands, and what else is affected

| Command | Today (`dev` 2de24c5) | New | Step |
|---|---|---|---|
| `links_intent(intent)` | `LinkFrame`, the keeper state its resolving step published (`app/src-tauri/src/commands.rs:93-98`, `core.rs:1251-1270`) | `Result<(), ApiError>`, answered at the same moment and with the same refusals; the effect shows in the next frame | 2 |
| `ride_load(workout_id)` | `RideLoaded { ride_id, workout_id, hr_linked }` (`commands.rs:156-161`, `dto.rs:405-412`) | `Result<String, ApiError>`, the ride id, for logs and tests. `hr_linked` is `devices.strap`, `workout_id` is `ride.workout_id` | 3 |
| `ride_end()` | `RideEnded { ride_id, forced }` (`commands.rs:210-219`, `dto.rs:417-423`) | `Result<(), ApiError>`; the end is `ride.ended` | 3 |
| `device_pair(id, role)` | `PairedDevice` (`commands.rs:58-64`) | `Result<(), ApiError>`; the pairing is `devices.<role>.paired` | 3 |
| `ride_status()` | `RideStatusView { load, status }` (`commands.rs:227-229`, `core.rs:2460-2476`) | deleted: `ride` and `loading` | 3 |
| `device_links()` | `Vec<DeviceLinkInfo>` (`commands.rs:82-84`, `core.rs:1479-1520`) | deleted: `devices` | 3 |
| `open_stream(channel)`, `stream_subscribe`, `stream_unsubscribe`, `stream_ack`, `stream_snapshot` | the stream, its subscriptions, acks and re-query (`commands.rs:237-305`) | deleted: `ui_open`, `ui_ack` | 3 |
| `links_open(channel)`, `links_ack` | the link channel (`commands.rs:106-128`) | deleted: `ui_open`, `ui_ack` | 3 |

Unchanged: `core_ready`, `display_width_mm`, `devices_scan` (its scan's result), the `()` commands (`device_forget`,
`ride_start`, `ride_pause`, `ride_resume`, `ride_skip`) and the durable queries (§4.2). The facade keeps a one-shot
`Core::snapshot()`, the frame's state without the series, for tests and for the CLI and MCP; it is not a command.

**Unaffected.** **hil / Part B (#44)** reads the `.fecap` captures (`crates/crank-ble/examples/hil_fec/partb.rs:1-7`),
`<dir>/logs.jsonl` under `CRANK_CAPTURE_DIR` (`AGENTS.md:54`, `:63`) and the `crank_core::links` log lines (ADR-0002
§5.1.6), never a channel. **The recorder**'s 1 Hz record (`crates/crank-engine/src/record.rs`,
`crates/crank-core/src/recorder.rs:1-2`) is written on the engine's task, apart from the streams (`hub.rs:4-7`). **FIT
export** (#257, #548) is not built and will read the record, not the series. **The CLI and MCP** are scaffolds
(`crates/crank-cli/src/main.rs:1-3`, `crates/crank-mcp/src/lib.rs:1`); with #40 they read `Core::snapshot()` or a
`UiSink`, which replaces `FrameSink` (`core.rs:201-212`) and `LinkSink` (`keeper/channel.rs:35-46`).

#### 4.3.9 Deletion list, and what replaces each test's intent

| Code, by crate | Deleted | Kept or changed |
|---|---|---|
| `crank-engine` | `src/stream/flow.rs`, `hub.rs` (the series append replaces `on_source`) and `work.rs`. In `src/stream/types.rs`: `StreamId`, `SubId`, `MetricId`, `SubscribeOpts` and its caps, `SubSample`, `LinkChange`, `StreamFrame`, `StreamSnapshot`, `StreamError`, `StreamStatus::same_discrete`. In `engine.rs`: the stream variants of `StreamReq` (`:214-226`) and their handling (`:1513-1535`), the hub calls (`:1236`, `:1597-1606`), and `EngineHandle::open_stream` … `stream_snapshot` (`:262-334`). `hold_tests.rs`'s use of the hub | `StreamStatus` and its parts, `LinkStatus`, and `StreamStatus::in_progress` (`core.rs:1769`). The `Laps` request (`EngineHandle::laps`, `:336-343`) |
| `crank-core` | `keeper/channel.rs`, whose run loop becomes the model for the new `ui` module. In `core.rs`: `open_stream`, `owned`, `subscribe`, `unsubscribe`, `stream_ack`, `stream_snapshot`, `close_streams`, `links_open`, `links_ack`, `ride_status`, `device_links` (`:1291-1320`, `:1479-1520`, `:2460-2635`) and `FrameSink`. In `dto.rs`: `LinkFrame`, `LinksId`, `DeviceLinkInfo`, `RideLoaded`, `RideEnded`, `RideStatusView` | `LoadProgress`, for the log and the keeper |
| `crank-desktop` | `ChannelSink`, `LinkChannelSink`, the commands of §4.3.8 and their registry entries (`app/src-tauri/src/lib.rs:31-58`) | `lifecycle.rs` `drop_streams` closes channel ids (§4.3.5) |
| App | `app/src/stream/state.ts`, `app/src/ride/linkOrder.ts`, `app/src/ride/forcedEnds.ts`. In `app/src/stream/client.ts`, `Requery`, the subscriptions and the ack pump; it becomes a client of about 60 lines: open, apply, ack, watchdog. In `hooks.tsx`: `useSubscribe`, `useLinks`. The protocol half of `app/src/ride/rideBackend.ts` (`:27-51`, `:517-572`). `RideSession.record`, `Ride.tsx` `useRecord` and the page-built history. The `device_links` polling (`app/src/views/Today.tsx:144`, `app/src/views/Devices.tsx:205`) | `StreamProvider` becomes app-scoped. `app/src/stream/lint.test.ts` (the `structuredClone` ban) applies to the new client |

**Docs**, each updated by the step that changes its subject: AGENTS.md (the streams, link-channel and `app/src/stream/`
paragraphs), `spec/UX-SPEC.md:18-21`, `:76-78`, `spec/VIEWS.md:7`, `:42-43`, DESIGN §6.4 and DESIGN.md:621,
`docs/safety/FMEA.md` FM-STEADY-21 to 24, and the doc comments in `dto.rs` and `bindings.ts`. ADR-0002 §5.1.4 is updated
by this amendment.

**Tests.** Each replacement fails on `dev` today, or on the step before it.

| Test today | Intent | Replaced by |
|---|---|---|
| `flow_control_holds_over_arbitrary_interleavings`, `the_generator_reaches_the_split_and_bare_paths`, `a_full_window_sends_nothing_and_coalesces_to_the_latest`, `an_ack_for_a_frame_never_sent_is_ignored`, `seq_wraps_at_u32_max_and_the_ack_window_follows`, `a_split_across_the_wrap…` (`stream/tests.rs:303-445`, `:1426-1472`) | flow control is bounded and loses nothing it promises | **one property test** of the `ui` channel over arbitrary interleavings of ticks, keeper changes, ride loads and ends, and acks (right, stale, foreign, none for a long time). It asserts: at most one frame unacked; each frame's state equals `Core::snapshot()` at its send; the received chunks, applied by the client rule, equal the core's series exactly (no hole, no duplicate, `from` always the page's length, also when a load starts while a frame is in flight or a channel closes mid-backfill); `ride.live` equals the engine's latest tick; frames ≥ 100 ms apart; every frame within the bound |
| (none: a new risk) | a lost frame does not stall the page | **a dropped frame is recovered**: drop one frame mid-ride; the page reopens and, within 3 s, holds the current state and a complete series |
| `every_frame_stays_under_the_byte_budget_at_every_cap`, `oversized_samples_split…`, `a_split_stops_at_the_window…`, `discrete_state_over_budget_becomes_the_bare_resync_frame` (`:482-618`), `the_largest_frame_is_under_the_byte_budget` (`keeper/channel.rs:337-358`) | a frame has a known size | **the frame-bound test** (§4.3.3) |
| `subscription_chain…`, `each_subscription_has_its_own_chain`, `subscriptions_are_bounded_and_validated`, `unsubscribe_drops_the_pending_sample`, `filter_chains_are_capped…`, `subscribe_opts_wire_contract_for_the_ui_mock`, `ids_skip_zero…`, `stream_and_subscription_ids_stay_unique…` (`:633-741`, `:1018-1065`, `:1473-1608`) | subscriptions and chains | deleted with subscriptions |
| `the_tick_cost_stays_bounded_at_every_cap` (`:1066-1144`) | a tick's work is bounded | **the series append costs one push per series per tick, however many channels are open** |
| `non_finite_values_are_gaps…`, `a_chain_that_overflows_to_infinity_emits_a_gap` (`:1180-1239`) | a NaN never reaches the page | **a non-finite or absent value appends no point** |
| `frames_never_exceed_ten_per_second…` (`stream/tests.rs:742-770`), `frames_are_spaced_and_sent_only_on_change` (`keeper/channel.rs:266-300`) | ≤ 10 Hz, only on change | the same on the `ui` channel, with the series and the heartbeat |
| `status_is_sent_on_change_and_otherwise_at_two_hertz`, `the_block_start_and_the_lap_count_are_sent_on_change`, `no_heartbeat_while_idle_or_finished`, `a_blocked_stream_does_not_beat_into_resync`, `a_status_change_restarts_the_heartbeat_period`, `a_blocked_stream_neither_beats_nor_flags_resync_after_the_ack`, `the_loss_fields_are_discrete…`, `a_hub_sends_a_status_when_only_a_loss_field_changed` (`stream/tests.rs:788-880`, `:955-1017`, `:1609-1668`) | no status change is missed; the heartbeat is cheap | **a change visible at an ack is in the next frame**, and **an idle core sends one frame a second, no more** |
| `a_new_stream_starts_with_the_current_status_and_links` (`stream/tests.rs:881-892`), `the_first_frame_is_the_state_at_the_open` (`keeper/channel.rs:204-216`) | a new page starts from the truth | **the first frame is `Core::snapshot()` at the open, sent at once**, and **a channel opened mid-ride backfills the series from index 0, contiguously** (fails today: a new page's trace starts empty) |
| `a_consumer_that_never_reads_or_acks_is_bounded`, `a_stalled_page_gets_only_the_latest_state_once_it_acks` (`keeper/channel.rs:221-242`), `a_stalled_consumer_never_blocks_the_ride_or_the_record` (`crates/crank-engine/tests/streams.rs:189`) | a stalled page costs nothing and loses nothing | **the page converges after a stall**: it stalls 60 s mid-ride, then acks; its next frames carry the current state and series that start exactly where its own ended. The ride and the record are unaffected (fails today: a hole in the trace, #557 item 4) |
| `window_cleanup_drops_streams_and_dead_sinks`, `closed_receivers_are_reaped…`, `a_blocked_dead_stream_is_reaped`, `opening_a_stream_frees_the_closed_ones…`, `unlabelled_streams_never_evict_each_other` (`stream/tests.rs:909-985`, `:1145-1255`), `reopen_close_and_a_failed_send_drop_the_channel`, `a_close_issued_before_a_reopen_never_drops_the_new_channel` (`keeper/contract_tests.rs:443-468`) | lifecycle | **lifecycle by id**: a failed send, a page-load event and a reopen each close exactly the channels of §4.3.5. The #564 reproduction: the close is issued, the new page opens, the close runs, and the new channel is alive and acks (fails today: `NotFound`) |
| `keeper/contract_tests.rs`: `each_device_has_one_source_an_unlinked_strap_stays_the_keepers` (`:351`) and the other phase tests (`:77-290`, `:379-568`) | the devices' state | **the overlay test**: a riding device shows the ride's link, a lost trainer shows `lost` in the same frame as `trainer_lost_since_s`, and an unlinked strap keeps its own phase. The phase tests move to `devices` |
| `keep_newest`, `applied_in_arrival_order_the_newest_state_wins…`, `a_discards_answer_carries_the_seq_of_its_own_state`, `links_intent_answers_the_state_its_resolving_step_published` (`:421-441`, `:569-708`), `app/src/ride/linksBackend.test.ts:350-410` | ordering two sources | deleted: one source. **The client acks every frame it receives** replaces #577's concern |
| `app/src/stream/stream.test.tsx` | the client applies frames and re-queries | **client tests** (mockIPC): applies every frame in order, acks each, replaces the state, appends the series, replaces on `from: 0`, drops a replaced channel's frames, reopens after 2 s of silence |
| `app/e2e/specs/stream.e2e.mjs` | no status is lost across a 1 s webview freeze | **the same freeze on the real app**: afterwards the page's state equals `Core::snapshot()`, and the series have no hole |
| `app/src-tauri/tests/commands.rs:97-190`, `app/src-tauri/tests/lifecycle.rs:61-94` | the stream through the commands, and window cleanup | `ui_open` and `ui_ack` through the commands; a page load closes the old page's channel and never the new one |
| FMEA FM-STEADY-21 to 24 | stalled consumer; filter cost; frame over budget; non-finite value | 21 → the stall test; 22 → retired with the chains; 23 → the frame-bound test; 24 → the non-finite test |

The status wire tests stay beside the type (`the_worst_status_with_the_erg_fields_has_a_measured_size` and the other
wire-status tests). The design fixtures (`app/e2e/design/fixture.tsx`) send frames; the baseline PNGs must not change.

#### 4.3.10 Migration, in PR-sized steps (sub-issues of #588: #597, #598, #599)

**Step 2 (#597): link commands return nothing.** `links_intent` answers `()`; the ordering rule (ADR-0002 §5.1.4) and
`linkOrder.ts` are deleted; `LinkFrame.seq` keeps its meaning, used only by `links_ack`; #577 (closed) is superseded.
After an intent the UI changes on the next link frame (within about 100 ms plus one IPC hop), not on the answer, and
#561's per-device split stays until step 3, since the overlay needs the status and the devices in one frame.

- **Acceptance:** the mocks model every refusal of `links_intent`; the bindings are regenerated; AGENTS.md's
  link-channel paragraph is updated.
- **Fails before it:** `links_intent` through the commands answers `null` for every intent, with unchanged refusals.

**Step 3 (#598): the snapshot channel replaces the ride stream and the link channel.** It covers §4.3.2 to §4.3.9, one
source per device included (the overlay; `StreamFrame.links` and `RideLoaded.hr_linked` deleted), in up to three PRs
that each leave `dev` working: (3a) the core's `ui` channel and its tests beside the old channels; (3b) the app switches
(client, hooks, mocks, design fixtures, e2e); (3c) the deletions and docs.

- **Acceptance:** §4.3.9's replacement tests pass, including a dropped frame recovered within 3 s with a complete
  series; `just e2e`'s freeze spec passes on sage; a 32 KB frame's round trip on sage is ≤ 100 ms; the design baselines
  are unchanged; the bindings and the docs of §4.3.9 are updated; the mocks model every refusal (`ui_ack`: `NotFound`;
  `ui_open`: `Conflict` beyond 16 channels, `Storage` when the keeper cannot start).
- **Fail before it:** the #564 reproduction; the page converging after a stall; the mid-ride backfill; a forced end that
  survives a reload; a lost trainer as `{phase: riding, link: lost}` in the same frame as `trainer_lost_since_s`; the
  Ready sheet's strap row following `devices.strap` (today `hrLinked` decides it, `app/src/ride/ReadySheet.tsx:65-80`).

**Step 4 (#599): what is left of #557.** The UI's own chunked append-only series store, so an append does not copy the
history (`app/src/ride/session.ts:47-55` today); incremental trace rendering without the `Math.max(...values)` spread
(`app/src/ride/Trace.tsx:75`; a 6 h ride can exceed JavaScriptCore's argument limit); a pixel-downsampled backfill; a
`since` cursor on `ui_open`.

- **Acceptance:** an append costs O(1), counted; a 6 h synthetic ride renders and backfills in a few frames; the drawn
  vertices equal those of the full series at one per second.
- **Fails before it:** the 6 h ride's trace throws `RangeError` at `Trace.tsx:75`.

**Order.** Each step leaves `dev` working, with no lost data and no regressed screen. #471 starts after step 2, ideally
after step 3. #248 adds `nudges`; #309 reads them.

#### 4.3.11 Risks

- **Re-render per frame.** Components that compare by reference re-render at the frame rate; `spec/UX-SPEC.md:78` asks
  never to re-render the whole tree per frame. Step 3 measures it on the ride screen; too high a cost changes this
  section, not the client.
- **Lost intermediate states.** Coalescing loses a state shorter than a frame or a stall (one reconnect attempt before
  the next). What the page must not miss is a counter or timestamp in the snapshot (`laps_closed`, `last_gap_s`,
  `reconnected_at_s`) or durable data, never an edge.
- **Lost frames.** Each costs up to 2 s of frozen state and a backfill. The core logs each reopen that closes an older
  channel, so the logs show how often it happens on real hardware.
- **Memory and growth.** About 3 MB per 6 h ride, nothing per channel (integer ticks would make a point 10 B). A new
  snapshot field that does not fit the frame-bound test is durable data and a query.

#### 4.3.12 Alternatives

| Alternative | Why rejected |
|---|---|
| Keep the multiplexed subscription stream (the original §4.3) | Ack window 3, split frames, a re-query merged through a "touched" set, and still holes after a stall (#557 item 4), for rates and filters no widget uses |
| State deltas instead of full state | The page must merge again, and a lost delta needs a resync path. The full state is about 1.5 KB |
| Live state by polling queries | Unordered against the commands, and either late or wasteful |
| A channel per widget; events for telemetry | IPC volume and no ordering; events are JSON and unordered |
| Fixed-rate full snapshots without acks | A stalled page queues unbounded work: Tauri's `Channel::send` has no backpressure (`channel.rs:293-320`) |
| A resend or a cursor that moves on the ack, for lost frames | More state in the core for a rare fault the watchdog already recovers |

**Reversed.** The original §4.3 rejected fixed-rate full snapshots because "per-widget rates are a product requirement".
The owner reversed this on 2026-11-09 (#588): no widget uses them, and §4.3.7 covers one that would.

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
- **Sample holding** (per field, #437). Each trainer field holds its last real value until its own bound,
  then it is `null`: power and cadence (page 25) for up to 1.0 s (4 ticks; page 25 may arrive at only ~2 Hz,
  ADR-0002 §5), speed (page 16) for up to 6.0 s (24 ticks) but never longer than page 25 itself is held. Speed
  may be held longer because it drives no control decision (ERG release, soft start and the pauses read power
  and cadence) and the NEO sends page 16 much less regularly. HR is held for up to 3.0 s. A field the
  sample's page does not carry leaves the held value; a field the page carries but reports invalid is
  `null` at once. Details and the measured intervals: implementation notes below.
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
- **Per-field hold (#437).** A trainer sample is one FE-C page: page 25 carries power and cadence, page 16 speed (ADR-0002 §5); a field the page does not carry is "not in this sample", never zero, while a field the page carries and reports invalid (page 25's cadence `0xFF`, power `0xFFF`) is unknown from that tick on, with no hold, so an invalid cadence counts as 0 for the §7.4 release at once (`crank_devices::Reading`). The samples of a tick are merged field by field in arrival order, and each field keeps its last real value (cadence 0 included) until its own bound: power and cadence 1.0 s (4 ticks, the hold above), speed 6.0 s (24 ticks) but never longer than page 25 itself is held, so no value outlives a stopped page-25 stream. The 6 s comes from the #44 Part A captures: the NEO sends page 16 instead of about one page 25 in four, and the page-16 interval is about 1 s at the median but 5.0–5.5 s at the 99th percentile (`latency.fecap`, `spacing.fecap`; at most 5.97 s outside one 25 s outlier), so a 1.0 s speed hold would blank speed most seconds. The silence rule of §5 and ADR-0002 §6 counts ticks without a **page-25** sample (one with an event count): page 16 alone keeps no trainer alive. An 8-tick silence, like an applied link transition, clears every field's hold. HR is held as before.
- **Loss-window values.** The trainer hold (1.0 s = 4 ticks) applies first, then the value is `null` for the rest of the window: ticks after the last sample show it for 4 ticks, the remaining ticks of the 8-tick window and everything after are `null`. This is how §5's "null record values" and the hold rule compose.
- **Link events.** Driver link events carry no timestamp (#232 asks for one). The engine stamps each on receipt and applies it before the tick containing the stamp; samples keep their driver timestamp. Every device link has an event count: a sample is tagged with the count at receipt and is stale, and never enters a hold, once a later link transition has been applied. Every applied transition also clears that device's hold. Resume therefore needs a cadence > 0 sample received after `Ready`. Convention for stalls: a transition the engine learns of only after a stall is applied at the stall's end, so a stalled run and a live run agree on everything that carries a timestamp and can differ only around link transitions inside the stall (closed by #232).
- Pauses compose: `user`, `trainer_lost` and `stopped_pedalling` are independent flags (reported in that priority: lost, user, stopped), so a rider's pause survives a reconnect. Ticks of a paused ride record `null` for every metric and `workout_s`.
- After a loss detected by silence (no `Link(Lost)`), the engine re-issues the mode as soon as it detects the loss, because the link is still `Ready`. Control is sent only while the link is `Ready`; resume needs a re-applied mode and cadence > 0 on a tick.
- **Soft start.** Rule and origin: ADR-0002 §7.5 amendment (#231). It runs only while pedalling trainer values are present after the re-issued mode, from the 3 s power at the tick they start flowing (the window moves with tick time); until then the re-issued mode is 0 W, never the target. It holds its last step while samples are missing and restarts from the then-current power when they return.
- **No trainer, no ride.** `Start` is rejected with `NoTrainer` / `TrainerNotReady`: DESIGN §5.4 ("Start is enabled when the trainer is connected") and #95 ("Start disabled until trainer: connected"; makes impossible "a ride starting without a connected trainer"). DESIGN §15 has no trainer-less mode; a no-trainer ride is not a mode of the engine.
- **Shutdown.** When every `EngineHandle` is dropped mid-ride the engine finishes the ride (record closed through `RideRecorder::finish`) and clears the ride-active signal.
- The engine finishes the ride when the last block completes (or is skipped); a partial last second is not recorded. A skip ends the current block's lap at the skip tick; in a free ride (no blocks) it closes the lap and opens the next, and the ride goes on (#243 Q1). Block duration is `FlatBlock.dur_s` rounded to ticks; open-ended end rules are #224, gap filling from accumulated power is #223.
- `crank-engine` depends on neither `crank-ble` nor `crank-store`: the ride-active signal is the `RideActivity` trait (core passes `|a| gate.set_active(a)`), the record is the `RideRecorder` trait (core adapts it to `Store`).
- `VirtualClock::new(speed)` runs `speed`× faster than the tokio timer under it; with `start_paused` tests it is exact and takes no wall time.

Engine sketch:

```rust
pub enum EngineCmd {
    Load(Workout), Start, Pause, Resume, Stop, Adjust { pct: f64 }, SkipBlock,
    SetSpeed(SimSpeed), RespondNudge { call_id: CallId, accept: bool },
    // Subscribe / Unsubscribe are gone with the subscriptions (§4.3, #588): the engine appends display series.
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
| `athlete_profile` | singleton: weight, age, sex, crank length, health-check result (column `health_check_result`; renamed from `par_q_result` by migration 0003, #210) | |
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

### 7.1 Amendment 2026-11-02: logs via vigil (#414)

*Amends the **Logging** bullet of §7 (#414 and its owner comment call it "§9"; logging is decided here).
The raw-frame capture and the #411 HIL contract are unchanged.*

**Context.** #411 shipped `app/src-tauri/src/logging.rs`: a plain-text `crank.log` in Tauri's
`app_log_dir`, size-rotated at 5 MiB with one previous generation, and fixed filters. That was enough
for the #44 Part B session, but it is not what §7 specifies (JSON lines, 7 days kept, `CRANK_LOG`,
spans). The owner decided on 2026-11-02 to adopt [vig-os/vigil](https://github.com/vig-os/vigil)
instead of building that on `tracing-appender`. #431 (bug reports, ADR-0003) reads vigil's output.

**Decision.** The desktop entry point initialises `vigil::Config::new("crank")` once, holds the
`Guard` in Tauri state, and shuts it down on exit within the quit budget below. This replaces
`RotatingFile`. Each §7 requirement maps to a vigil setting as follows:

| §7 requirement | vigil 0.2.0 setting | Deviation |
|---|---|---|
| JSON lines | `logs.jsonl`: one OTLP/JSON `LogsData` per line (OTel file-exporter spec) | OTLP envelope, not flat `tracing-subscriber` JSON. Readers must unwrap `resourceLogs[].scopeLogs[].logRecords[]` (ADR-0003 §2.2) |
| `tracing-appender` daily files, 7 days | `Config::retention(Some(7 days))`, plus `signal_rotation(Logs, max_bytes(8 MiB).max_segments(Some(6)))`, `signal_rotation(Traces, max_bytes(8 MiB).max_segments(Some(2)))` and `max_total_bytes(Some(96 MiB))` | Segments rotate by size, not by day. The 7-day age limit is kept, and the count and byte caps (off by default upstream, vigil#168) bound the directory to under 96 MiB |
| `<log_dir>` | normal launch: `Config::dir(app_log_dir)` (`~/Library/Logs/io.github.gerchowl.crank/`) | none. The explicit builder value wins over `VIGIL_DIR` and `XDG_STATE_HOME` |
| `CRANK_LOG` (`EnvFilter`), default `info` | normal launch: `filter_env("CRANK_LOG").default_filter("info")`. Capture mode: see below | none. Without `filter_env`, vigil would read `RUST_LOG` |
| Spans `ride{id}`, `device{kind,id}`, `coach{call_id}` | Feature `traces`: spans go to `traces.jsonl`, and records inside an entered span carry its `traceId`/`spanId` | **Span fields are not on log records in 0.2.0** (spike below). Correlate by span id. Tracked in #432 |
| Default `info`, raw frames at TRACE to `.fecap` | unchanged: `.fecap` is written by `crank-ble`, not by vigil | none |
| — | `version(CARGO_PKG_VERSION)`, `revision(<build commit>)`, the same provenance ADR-0003 §2.6 uses | addition |
| — | `console(Console::Stderr)` in debug builds only | addition. Its text is not parsed |

**Dependency.** `vigil-telemetry = { git = "https://github.com/vig-os/vigil", tag = "0.2.0",
default-features = false, features = ["traces"] }`, imported as `vigil` (Apache-2.0). The exact tag
is pinned because a pre-1.0 minor may break the API, and moving the tag is a deliberate PR.
`metrics` is off because crank has no metrics consumer, and `audit` is not needed. `deny.toml [sources]`
gains `allow-git = ["https://github.com/vig-os/vigil"]`. The implementation PR runs the full `deny`
check (licences, advisories, bans) over vigil's OpenTelemetry tree and fixes or justifies each
finding there. vigil's MSRV is 1.95, which crank's pinned 1.96 toolchain satisfies.

**#411 HIL contract (kept, tested).** When `CRANK_CAPTURE_DIR=<dir>` is set:

- **Location.** `Config::dir(<dir>)`. The `.fecap` files stay in `<dir>`, separate from
  `logs.jsonl`/`traces.jsonl`. vigil keeps foreign files when it prunes.
- **No retention, no caps:** `retention(None)`, `max_total_bytes(None)` and, for both signals,
  `signal_rotation(_, max_bytes(8 MiB).max_segments(None).retention(None))`. They are set explicitly,
  so the `VIGIL_*` environment cannot re-enable them. The reason: with the normal-launch
  limits, a long TRACE session would prune its own early segments, and a reused capture directory
  would lose an earlier session's logs. A capture directory is evidence the owner curates (#44
  Part B), so vigil never deletes anything in it; only size rotation (8 MiB segments) remains. The
  cost is that disk use there is unbounded, which the owner accepts for an explicitly opted-in session.
- **Filter (architect decision 2026-11-02, the owner may override).** The capture's `crank_ble=trace`
  always applies, and `CRANK_LOG` adds to it; the app does not exit.
  - vigil 0.2.0 reads its filter only from one environment variable or `default_filter`, so in capture
    mode crank resolves the filter itself and passes the result as `default_filter`:
    `info` (or `CRANK_LOG`'s directives, if it is set and valid), then any directive whose target is
    `crank_ble` or `crank_ble::…` removed, then `crank_ble=trace` appended, then
    `crank_desktop::logging=warn`, so the WARN records below survive a `CRANK_LOG` that turns the desktop off.
  - Removing those directives is what makes "always" true: `EnvFilter` lets the most specific
    directive win, so `CRANK_LOG=crank_ble::driver=warn` would otherwise lower the capture. Each removed
    directive is reported in a WARN record. An invalid `CRANK_LOG` is reported the same way, and the
    capture default is used.
  - `filter_env` names `CRANK_LOG_CAPTURE_RESERVED`, a variable crank documents as reserved, so vigil
    cannot override the resolved filter with `CRANK_LOG`. If that variable is set, the app reports it
    and exits without riding, because vigil would apply it. #432 tracks asking vigil for an explicit
    filter, which removes the reserved name.
- **Exits.** The existing "capture dir cannot be opened" exit and the reserved-variable exit call
  `Guard::shutdown()` first, because `std::process::exit` skips vigil's flush.

The tests that exist today for the filter plan (`crank_ble` at TRACE only with a capture directory)
move to the vigil configuration unchanged. New tests pin the merge (`CRANK_LOG=warn` and
`CRANK_LOG=crank_ble::driver=warn` both still enable `crank_ble::driver` at TRACE), the
`None` retention and caps in capture mode, and the reserved-variable refusal.

**Quit budget (decision; the owner may override).** AGENTS.md's quit path must finish within 3 s,
because macOS may kill a terminating app a few seconds in. The 3 s are split:

- `Core::shutdown` waits at most **2.5 s** for the devices to be released. That is lowered from 3 s.
- vigil then gets the rest: `Guard::shutdown_with_timeout(3 s − elapsed)`, measured from the
  exit request, and nothing once the 3 s are used up (no floor: a floor would break the cap; #414
  review). Because shutdown is explicit, the guard's Drop does nothing, so its 5 s wait never runs.
- On a quit that never asked (Dock Quit, logout), the clock starts at `RunEvent::Exit`
  (`applicationWillTerminate:`), the first event Tauri delivers. It understates the quit by AppKit's
  own work after `terminate:` plus the wait for the main thread: tao answers no
  `applicationShouldTerminate:`, crank is not document-based, and crank's main-thread handlers never
  block, so no crank code runs in that gap.
- The ride-end flush (below) runs on a blocking thread and is not awaited, holding its own handle,
  never the lock the quit takes. A quit that finds that flush in flight waits for it within the
  budget, then shuts down with the rest. If the flush outlasts the budget, vigil is not shut down:
  the process exits under the flush and loses what it had not written, and the guard's Drop never
  runs (#414 review).
- Metrics are off, so the 5 s metrics floor (vigil#112) does not apply.
- When the bound is hit, the records still queued are lost. That is at most the last ~1 s batch
  interval: typically the shutdown's own lines. Spans not yet exported are lost from `traces.jsonl`.
  vigil reports the loss on stderr, which a Finder launch does not have.
- A quit that gives up on a flush, a crash or a SIGKILL can leave `logs.jsonl` ending in a **partial last
  line**. Readers (`vigil::rotate::read_since` returns it as a line; the bug report's `flatten`, ADR-0003 §2.2) take every
  complete line and skip the one that does not parse, as squelch already does. crank pins the rule with `read_since`
  (`tests/logging.rs`) and with `flatten` (`report::flatten` tests, and the partial last line of the privacy fixture).
- Because crank also flushes when each ride ends (below), the ride's records are on disk before quit.

**Migration (architect decision 2026-11-02, the owner may override).** The app never deletes the old plain-text `crank.log` or
`crank.log.1`; they stay in `app_log_dir` next to `logs.jsonl`. AGENTS.md's logging paragraph is
rewritten in the implementation PR and says the old files can be deleted by hand. Sequencing follows
#414: implement after the #44 Part B session.

**Spike (2026-11-02, outside the repo, crank's dev shell, vigil tag `0.2.0` @ `64f9b8f`).** A binary
initialised vigil, emitted two events inside an entered `info_span!("ride", ride_id = 7)` and shut
down. `cargo run` exited 0 for both builds:

- **Logs-only build.** One `logs.jsonl` line held both records. `attributes` carried only the event
  fields, `traceId`/`spanId` were empty, and there was no `ride_id`.
- **Default-features build.** The same records carried a non-empty `spanId`/`traceId`, and
  `traces.jsonl` held the span. Hence `features = ["traces"]`.
- **Resource attributes** included `host.name`, which is the machine name. ADR-0003 §2.2 drops them.

**Alternatives.** `tracing-subscriber` JSON with `tracing-appender`, as §7 originally said: rejected
by the owner in favour of the vig-os shared crate, which brings caps, multi-process-safe rotation and
an OTLP format Loki reads. Keeping #411's `RotatingFile` and adding JSON: hand-rolled rotation and no
spans. vigil logs-only: loses span correlation entirely (spike).

**Consequences and known limits.**

- Pre-1.0: a vigil minor may break crank's build. Bumps are reviewed PRs.
- Open spans at shutdown are not exported (vigil#8 8d), so `RunEvent::Exit` ends the ride and device
  sessions before `shutdown()`. This affects `traces.jsonl` only. Tracked in #432.
- **Spans as built (#465).** `ride{id}` is opened by `Core`'s load and held by the session until the reap,
  which every end, replacing load, discard and quit goes through; `device{kind,id}` is its child, opened as
  each device connects and held the same way, so the devices close first. The engine and the drivers hold no
  clone: a wedged engine detached from its session would keep a span open for ever, and vigil exports none still
  open. Records the core logs inside the span (the ride's release) carry its `traceId`/`spanId`. A quit that
  times out or fails closes the spans of the session it leaves loaded (`Core::shutdown`). `id` is crank's ride
  id; a device is named by its role placeholder (`<trainer-id>`, `<strap-id>`, ADR-0003 §2.3), never its name,
  serial, platform id or the store id that embeds it. `coach{call_id}` has no call site yet (#474).
- vigil supports Unix targets only (`flock` rotation). Windows (#41) needs vigil support or another
  backend.
- Records overflowing the 65 536-record queue are dropped with a stderr warning. A Finder launch has no
  stderr, so crank calls `Guard::flush()` when a ride ends. If the flush returns a `FlushError`,
  crank writes its `lost(Signal::Logs)` count as a WARN record.

### 7.2 Amendment 2026-11-09: rider tuning (#589, #590)

*Adds a **configuration** decision to §7, which until now covered only the environment variables above. The
mechanism is the one the #590 audit proposed; #589 is its first consumer.*

**Context.** Values a rider feels were constants in code (#590 lists them): the ERG release thresholds of
ADR-0002 §7.4 first. A rider must be able to change them without a build, and a ride must say which values it
ran with.

**Decision.**

- **One typed struct.** `crank_engine::tuning::Tuning { version, erg: ErgTuning, slew: SlewTuning }` (`slew` from
  #585, below). Later groups (auto-pause #594, coach #596) are new fields of `Tuning`, each a struct of its own. Durations are **seconds** and
  cadences **rpm**, never ticks: the tick is an engine detail (§5). A duration must lie on the 0.25 s tick grid,
  so the engine never rounds a rider's value; `validate()` refuses one that does not.
- **Documented in code.** Every field has rustdoc with its unit, what it does and the decision behind its default.
  `Tuning::default()` is the decided default (for `erg`: ADR-0002 §7.4); a test pins it to the ADR's numbers.
  `ErgTuning::BOUNDS` is the table of inclusive ranges per field, and `Tuning::validate()` checks the ranges and
  the cross-field rules (ADR-0002 §7.4: re-engage cadence above release cadence).
- **The engine is given it.** The core passes the ride's tuning in `RideSetup.tuning`; the engine builds its policy
  from it and never reads the store, the environment or a file. The tuning is frozen for the ride: a change takes
  effect at the next load.
- **The core resolves it** at `ride_load`, before anything is touched, as layers over the defaults: the JSON of
  `Tuning::default()`, then the stored setting `"tuning"` (table `setting`, §6) if present, merged field by field
  (an object merges into an object, any other value replaces). The stored layer may be partial; it carries the store's
  integer envelope `v` (§6), which is the tuning's `version` (serialised as `v`) and must be `Tuning::VERSION`. The result is deserialised with unknown fields refused and validated. A
  stored value that fails is not ignored: the load is refused with `Storage` and a message that names the field,
  so a bad setting is never ridden silently with other values. Nothing writes the setting yet (Settings UI #595).
- **Seams.** The merge takes a list of layers (`Tuning::resolve`), so the `CRANK_TUNING` environment layer (#592)
  is one more entry after the stored one, and recording the effective values with a source tag per field (#591)
  reads which layer set each leaf. hil_fec reads the recorded values instead of its own copies (#593).
- **No config files.** Settings live in SQLite (§6, table `setting`); the environment stays for process-level
  configuration (above).

**The fields (amended 2026-11-11, #585).** JSON paths, units and defaults; the bounds, the cross-field rules and
the reasons are in the ADR section named:

| Field | Unit | Default | Bounds | Decided in |
|---|---|---|---|---|
| `erg.releaseEnabled` | bool | `true` | — | ADR-0002 §7.4 (#589) |
| `erg.releaseRpm` | rpm | 30 | 20–50 | ADR-0002 §7.4 |
| `erg.releaseAfterS` | s, 0.25 s grid | 5 | 1–8 | ADR-0002 §7.4 |
| `erg.reengageRpm` | rpm, above `releaseRpm` | 45 | 30–90 | ADR-0002 §7.4 |
| `erg.reengageAfterS` | s, 0.25 s grid | 1.5 | 1.5–5 | ADR-0002 §7.4 |
| `slew.riseWPerS` | W/s, integer | 20 | 5–50 | ADR-0002 §7.10.4 (#585) |
| `slew.fallFactor` | × `riseWPerS`, on a 0.25 grid; the product ≥ 20 W/s | 2.0 | 1–4 | ADR-0002 §7.10.4 |
| `slew.easeS` | s, 0.25 s grid: the ease time, rest to full rise rate; Ruckig's acceleration limit `2·rise/τ` and jerk limit `4·rise/τ²` | 2 | 1–4 | ADR-0002 §7.10.4 |

`slew` is implemented by #620. A new group changes no existing field's meaning or unit, so `Tuning::VERSION` stays
1; a stored layer written before it resolves unchanged, because the merge starts from the defaults.

**Alternatives.** A config file (TOML) next to the database: a second source of truth that a ride does not record,
rejected. Ticks in the struct: ties a rider's setting to the engine's sampling, rejected. Falling back to the
defaults on a bad stored value: hides a misconfiguration behind other values, rejected.

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
`window.__TAURI_INTERNALS__.invoke` with a DOM change, and `saveScreenshot` (1440×900 PNG).

Determinism, exactly what is checked: within one launch the spec compares two screenshots of an unchanged
page byte for byte, and `scripts/e2e.sh` runs the specs in two separate app launches and compares the
screenshots of launch 1 and launch 2 byte for byte (`cmp`). Both held on sage with no font or animation
pinning for the shell. No golden hash is recorded, because a hash would change with the macOS version and
fonts; the comparison across launches on one machine is the guard. It says nothing about other machines.

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
- The published plugin crate omits `permissions/autogenerated/reference.md`, so its build script tries to write
  generated files into the read-only Nix vendor tree; the e2e vendor dir adds that one file (flake
  `cargoVendorDirE2e`, used by the e2e package only; other builds vendor the crate untouched; upstream report
  webdriverio/desktop-mobile#730, tracking in `nix/patches/README.md`, #241). The plugin declares no commands, so it needs no capability.
- The plugin is optional in `crank-desktop`. The `deps-edges` check fails if `crank-desktop`'s production
  graph reaches any `tauri-plugin-wdio*` crate or enables `e2e`, and fails if the `e2e` feature stops
  pulling the plugin in. `build-e2e` keeps the package building in CI.
- A flock pane has no GUI session; `scripts/e2e.sh` hands the run to Alacritty over IPC and reads
  the log and exit code back from a per-run directory, the exit file written atomically (`CRANK_E2E_DIRECT=1`
  runs in the current terminal). On Linux the script skips with a message. `nix develop`
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
  the `CI Summary` job is required on `dev` and `main` (since §9.6 macOS runs on release candidates and after each merge into `dev`, not on PRs). The fleet runner (anvil vm-runner) was offboarded
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

### 9.6 Amendment 2026-11-05: Linux PR gate, release candidates, attested pre-releases (#480, #485)

*Amends §9.1 (CI), §9.3 (Distribution) and §9.4 (CI). Owner direction on #480 and #485. The workflows are
`.github/workflows/{ci,prepare-release,release,abandon-release}.yml`; the owner's steps are in RELEASING.md.*

- **Required PR CI is Linux only.** `CI Summary` (still the one required check on `dev` and `main`) requires the
  `linux` job (`nix flake check -L`, x86_64-linux). The `macos` job (`nix flake check -L`, aarch64-darwin) runs
  only where a macOS gate is wanted: a branch `release/*` (its pull request or the CI run Prepare Release
  dispatches on it), **post-merge on every push to `dev`**, and a manual dispatch with `macos: true`. When it
  runs, `CI Summary` requires it too; when it is not wanted it must be skipped. A post-merge run never gates a
  PR: it marks the `dev` commit red or green. Hosted macOS arm64 runners were not acquired often enough to
  gate every PR (#480). A nightly schedule was rejected: GitHub runs `schedule` only from the default branch,
  `main`, and nothing reaches `main` before #426, so it would never fire.
- **Cost.** One macOS `nix flake check -L` took about 41 min of runner time (run 42104526304). At the recent
  rate of roughly 12 merges into `dev` a day that is up to about 8 h of hosted macOS time a day. The repository
  is public, so hosted runner minutes are not billed; the cost is runner time and queueing, and it competes
  with release-candidate builds for the account's concurrent macOS jobs. Pushes to `dev` do not cancel a
  running macOS job; GitHub keeps at most one queued run per ref, so on a busy day intermediate merges are
  skipped and the next run covers them.
- **What is checked less often, and the risk.** Code and checks that only a Darwin evaluation exercises are
  checked after each merge into `dev` and on every release candidate, not per PR: the CoreBluetooth backend of `btleplug` as
  `crank-ble` builds it, the `target_os = "macos"` code of `crank-desktop` (`lifecycle.rs`, the quit handling
  for Cmd+Q and Dock Quit; `display.rs`, the CoreGraphics display width of the ride tiers), the `crank-app` derivation (§9.3) and its
  `bundle-no-nix-store` check (#413), the `install-app` check of `just bundle`, and anything that builds
  differently with the Darwin stdenv (linker, libiconv). A PR can therefore break the macOS build and land
  green; the post-merge run on `dev` turns red within the hour, and the next rc cannot be published until it
  is fixed. Triage (RELEASING.md, AGENTS.md): a red post-merge macOS run on `dev` gets a `bug` issue before the
  next merge into `dev`; a runner failure (not acquired, #480) is re-run once and noted on the run's PR. The
  §8.1 GUI-session commands on sage (`just e2e`, `just bundle`) are unchanged and still run before
  UI-touching merges. Accepted because macOS-specific code is small and changes rarely, and every build that
  reaches a test Mac has passed the full macOS gate (below).
- **Versions.** Tags, release names, `Cargo.toml` and every other version source carry the semver version:
  `X.Y.Z` for a final release, `X.Y.Z-rc.N` (N ≥ 1) for a release candidate of `X.Y.Z`. macOS needs numeric
  bundle versions, so `app/src-tauri/Info.plist` carries a mapping: `CFBundleShortVersionString` = `X.Y.Z`,
  `CFBundleVersion` = the single integer `X·1000000 + Y·10000 + Z·100 + B`, with `B = N` for `rc.N` and
  `B = 99` for the final release (0.2.0-rc.1 → 20001, 0.2.0-rc.2 → 20002, 0.2.0 → 20099). The integer
  orders exactly like semver precedence, so LaunchServices and the update check (#522) see a later build as
  newer. The mapping therefore bounds Y and Z to 0–99 and N to 1–98; `scripts/release-version.py` refuses
  anything else, as well as any version that is not later than the current one. Its `check` (the
  `release-versions` flake check) fails when any source disagrees with the mapping.
- **Release candidates do not touch `main`.** Prepare Release on `dev` with `X.Y.Z-rc.N` pushes
  `release/X.Y.Z-rc.N` (only the version bump; the changelog stays in `## Unreleased`), dispatches CI on it
  (Linux and macOS) and opens a draft PR into **`dev`**. The owner merges it with a merge commit. The merge
  starts Release, which runs the full macOS gate (`nix flake check -L` on aarch64-darwin) on that exact commit,
  builds `.#crank-app`, zips it, writes its SHA-256, attests it, and **publishes** a GitHub pre-release
  `X.Y.Z-rc.N` (tag created at the merge commit). Fixes an rc surfaces land on `dev` by ordinary PRs; the next
  rc is cut from `dev` again. `dev` therefore carries the version of the last rc, and no main→dev sync is needed.
  "Agents never merge release PRs" is policy only, not enforced: agents act as the owner's account (gerchowl), and
  any merge of `release/X.Y.Z-rc.N` into `dev` publishes a pre-release. `main` stays safe without that policy:
  Prepare Release refuses a final version and Release refuses to build one until #426.
- **Final releases are gated on Developer ID signing and notarization** (#47, #426). Until #426 lands,
  Prepare Release refuses a final version (a dry run is allowed) and Release refuses to build one. The final
  path keeps #428's shape: `release/X.Y.Z` into `main` with the changelog frozen, then Release on the merge.
  #426 replaces the refusal with signing, notarization and stapling, and brings back the main→dev sync PR
  (#428's `sync-dev` job, removed here because nothing reaches `main` before #426), failing the run when the
  PR cannot be opened instead of only warning (#439).
- **Assets and provenance.** Every published build carries `crank-<version>-aarch64-darwin.zip` (`ditto -c -k
  --keepParent` of `crank.app`), `crank-<version>-aarch64-darwin.zip.sha256` (one line of `shasum -a 256`,
  verifiable with `shasum -a 256 -c`) and `crank-<version>-aarch64-darwin.zip.sigstore.json`, the Sigstore
  bundle of a SLSA build-provenance attestation of the zip (`actions/attest-build-provenance`), which is also
  stored in the repository's attestations (`gh attestation verify <zip> -R gerchowl/crank`). The names are a
  contract: g-fleet's installer (g-fleet#1203) downloads them, verifies digest and provenance (signer workflow
  `release.yml` of this repository), and re-signs the app with `nix-tcc` (§9.3) so the Bluetooth grant is
  stable. A dispatched Release on any branch is a dry run: the same gate, zip, checksum and attestation,
  uploaded as a workflow artifact, with no tag and no release.
- **Signing and Gatekeeper.** The CI bundle carries only the ad-hoc signatures the `crank-app` derivation
  writes (§9.3, #413); CI adds none, and no Developer ID is used before #426. A zip downloaded by a browser is
  quarantined, and Gatekeeper refuses to open the app ("cannot be verified") until the quarantine attribute is
  removed (`xattr -dr com.apple.quarantine`) or the owner allows it in System Settings > Privacy & Security.
  `gh release download` and `curl` set no quarantine attribute. An ad-hoc signed app gets a new Bluetooth TCC
  prompt for every build; the `nix-tcc` re-sign of g-fleet#1203 or `just bundle` avoids that.
- **Tokens.** All workflows use `GITHUB_TOKEN` only, no PAT, no secrets. A push or PR made with it starts no
  workflow, so Prepare Release dispatches CI explicitly, and Release runs on the merge the owner makes. Opening
  PRs needs the repository setting "Allow GitHub Actions to create and approve pull requests"; without it the
  step fails with the compare URL to open by hand (#439).

## 10. Alternatives considered (summary)

| Decision | Alternative | Why rejected |
|---|---|---|
| Rust core in workspace crates | single crate | can't enforce purity / BLE-free engine |
| Engine + BLE in the app process, ride lock | separate daemon | extra signed process for a single-user app; revisit in #40 |
| tauri-specta generated bindings | ts-rs; TauRPC; hand-written TS | types-only drift; extra router; caused #2 |
| One `Channel<UiFrame>` of full UI snapshots per window, ack window 1 (§4.3, #588) | the multiplexed subscription stream (the original §4.3); state deltas; channel per widget; events | machinery for rates and filters no widget uses, and the page merging two sources; deltas need merging and resync; IPC volume, no ordering; events are JSON and unordered |
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
