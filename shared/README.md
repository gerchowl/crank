# @crank/shared — mock backend

Framework-agnostic TypeScript implementation of `Backend` (`src/contract.ts`) for the UI
spikes. It stands in for the future Rust core: all metrics, filters, aggregation, fitting,
DSL and compilation live here. The UI only subscribes, arranges and edits. No runtime deps.

```ts
import { createMockBackend, type Backend } from "@crank/shared";

const backend: Backend = createMockBackend();      // demo 45′ workout loaded, idle
backend.ride.setSpeed(10);
backend.ride.start();
const off = backend.subscribe("power_3s", { rateHz: 4, filter: [{ kind: "ema", tauS: 1 }] }, (s) => {
  el.textContent = s.v === null ? "—" : Math.round(s.v).toString();
});
// on unmount: off()
```

Consume it as `"@crank/shared": "workspace:*"`. The package exports TypeScript source
(`src/index.ts`), so Vite compiles it directly; use `moduleResolution: "bundler"`. It typechecks
under `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes` and `erasableSyntaxOnly`.

Options: `createMockBackend({ seed = 42, speed = 1, autoStart = false, workout = demo, storage, clock })`.
`MockBackend` adds dev helpers (`advance(simS)`, `advanceReal(ms)` with `clock: "virtual"`,
`pastRecoveries()`, `dispose()`); spikes should stick to `Backend`.

## Timing and event model

- **Ride time** (`Sample.t`, `RideStatus.t`, seconds) is simulated. The simulator advances in
  fixed 0.25 s ticks driven by a 20 Hz wall-clock timer × speed (1×/10×/60×). Same seed ⇒ same
  ride, whatever the frame rate or timer jitter.
- **Source rates** (`MetricInfo.sourceHz`): trainer metrics 4 Hz, HR-derived 1 Hz, `rr` per beat.
  At 10×/60× there are 10/60 times more source samples per wall-clock second.
- **`subscribe(metric, { rateHz, filter }, cb)`**: `rateHz` (clamped 0.1–10) is *wall-clock*
  display rate. The filter chain runs on **every source sample** (windows/time constants are in
  ride seconds) and only that subscription's output is filtered. The callback gets the latest
  filtered sample at most `rateHz` times per second, and only if a new source sample arrived (so
  nothing while paused; at 1× a 10 Hz power subscription gets 4 Hz). The first sample (current value
  or `{t, v: null}`) is delivered asynchronously (microtask). Unsubscribe clears the timer.
- `v: null` = no data (HR strap dropout, trainer reconnecting). `hold` fills nulls with the last
  value; other filters pass nulls through. Loading a workout resets filters and emits `{t: 0, v: null}`.
- **`onStatus`** fires immediately, on every discrete change (state, block, phase, connection,
  speed, manual offset) and as a 2 Hz heartbeat while riding.
- **`onNudge`** fires when a nudge is created *and* each time its status changes (same `callId`).
  Strong nudges arrive already `auto_applied`; lean/uncertain ones are `pending` and expire after
  30/20 s of ride time. `coach.decisions()` returns them all.
- `planner.propose` / `planner.rewrite` resolve after ~600–1000 ms.

## Polling budget (please respect)

| call | cost | poll at |
| --- | --- | --- |
| `history()` | O(buckets), a few thousand ops | ≤ 1 Hz per chart (never per animation frame) |
| `histogram()`, `heatmap()` | O(ride seconds) | ≤ 0.5 Hz |
| `mmp()` | cached per ride second | ≤ 0.2 Hz |
| `hrRecovery()` | VP fit over ≤ 240 points | 1–2 Hz while `status.phase === "recovery"` |
| `ride.current()` | deep copy of workout + compiled | on status/nudge change, not per frame |

Animate between polls in the UI if you want smooth motion.

## Metric notes

- `power_3s/10s/30s` rolling means; `w_per_kg`, `pct_ftp`, `power_zone`, `delta_target` use
  `power_3s`. `delta_target = power_3s − target_power`.
- `power_zone`, `hr_zone` are 1-based zone numbers; `block_index` is **1-based** ("block x/n"),
  `RideStatus.blockIndex` is a 0-based index into `compiled.blocks`.
- `target_cadence` is `null` when the block has no cadence target.
- `block_remaining` for `hr<` blocks is the fitted ready-in estimate; `remaining` uses expected
  durations of the rest of the plan.
- `hr_vs_typical`: HR minus the athlete's personal model HR for the same power history (bpm).
  `hr_drift`: mean of that residual over the last 5 min minus minutes 5–15 (null before 15 min).
  `decoupling`: Pa:HR, first vs second half of the ride so far (%, null before 10 min).
- `compliance`: % of ERG seconds within ±5 % of target (3 s power, 5 s settle per block).
- `rmssd` over the last 60 s of beats; `dfa_a1`: DFA α1 over the last 120 beats, box sizes
  4–16, after removing slow trends with a 25-beat moving average. RR intervals are synthetic
  (intensity-dependent mix of correlated/white/anti-correlated noise), so α1 is roughly 1.1 easy,
  0.75 around VT1, 0.6 at threshold, < 0.4 above. Plausible, not physiology.
- `wbal_pct`: Skiba differential model, CP = FTP = 280 W, W′ = 20 kJ.

## Charts

- `history()` returns buckets **oldest → newest**; `tAgo` is the time-ago of the bucket's newest
  edge, `dur` its width; buckets tile exactly and empty ones are **omitted** (draw a gap when
  `prev.tAgo !== cur.tAgo + cur.dur`). You get *about* `buckets` buckets (sub-second ones merge).
  Use `timeAgoToU(tAgo, spanS, scale)` (0 = now, 1 = span) for the x axis so it matches the
  binning; `timeAgoTicks(spanS)` gives sensible tick values.
- `histogram()` may widen `binWidth` to keep ≤ 400 bins; `typical` exists for power, % FTP, HR
  and cadence. `heatmap()` is row-major (`rows × cols`, y = row).
- `mmp()`: `today` is `0` for durations longer than the ride so far.

## HR recovery

`hrRecovery()` is live while the current block ends with `hr_recovery`; afterwards it returns the
last finished block's final state with `active: false`. The fit starts at the post-interval HR
peak; τ is profiled on a 10–200 s grid with (hrInf, amp) solved linearly per τ, priors on usual
τ / expected hrInf / amp, posterior-weighted 90 % band. `unreachable` = fitted `hrInf ≥ target`.
`readyInS` is relative to now and already clamped to the block's min/max.

## Caveats

- `parseDsl` assigns fresh ids on every call; use `reId()` for inserted snippets/duplicates.
- The simulated heart is two-component, the fit mono-exponential: early ready-in estimates
  wobble by ±20–30 s before converging (as they would on real data).
- Coach changes are applied to the ride's compiled blocks (`aiChanged: true`); the workout tree
  returned by `ride.current()` is the original.
- Layouts persist under `localStorage["crank.layouts.v1"]` (in-memory fallback).
- Run tests: `pnpm -C shared test`, types: `pnpm -C shared typecheck`.

## Ride record for review views
- `streams(metrics, fromS?)`: full-ride 1 Hz per-second means (`null` = no data). Poll at ≤ 1 Hz and pass `fromS = previous.toS` to fetch only new seconds.
- `recoveries()`: finished `hr_recovery` blocks (final fit, τ) of the current ride, oldest first.
