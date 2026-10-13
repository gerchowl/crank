# neo-trainer: SolidJS UI spike

This is the full UX-SPEC/VIEWS scope in **SolidJS 1.9** (plain Vite, not SolidStart), built against the
`@neo/shared` mock backend. It uses signals, stores, `createResource` and `<For>`/`<Show>`/`<Switch>`/`<Index>`/`<Dynamic>`/`<Portal>`
throughout, and props are never destructured.

## Run

```sh
pnpm install                       # from the repo root (pnpm workspace)
pnpm -C spikes/solid dev           # http://localhost:5175 — demo ride auto-plays at 10×
pnpm -C spikes/solid check         # tsc (strict, noUncheckedIndexedAccess, exactOptionalPropertyTypes) + eslint-plugin-solid
pnpm -C spikes/solid build
pnpm -C spikes/solid shots         # builds, serves, drives Playwright → shots/*-{dark,light}.png
node spikes/solid/scripts/gen-palette.mjs   # regenerate + re-validate src/theme/palette.css
```

The app starts on the ride screen (`#/ride`). `#/ride?speed=60` sets the speed. The left rail
(Check-in · Ride · Editor · Review) appears outside the ride. The ride is a full-screen modal; ◀ leaves to Review.

Guardrails: `eslint.config.js` (typescript-eslint `strictTypeChecked` + `eslint-plugin-solid`
typescript config, with `solid/reactivity`, `no-destructure`, `prefer-for`, `prefer-show` and `components-return-once` all
set to **error**). `.pre-commit-config.yaml` is a prek local hook that runs `pnpm check` (prek workspace mode, so run it from this dir).

## Libraries

| lib | why |
| --- | --- |
| `solid-js` 1.9, `vite-plugin-solid` | the framework |
| `uplot` | time-series/line charts (log-time P04, sparklines P02, MMP P09, HR recovery P12, review P05, τ per block) |
| `eslint-plugin-solid`, `typescript-eslint` | guardrails |
| `playwright` (dev) | `pnpm shots` |

There are no UI kits, no DnD/grid libs and no Solid wrapper libs. The uPlot binding (`src/charts/UPlot.tsx`, about 60 lines),
the canvas host, the grid drag/resize and the outline drag-and-drop are hand-written. TypeScript is pinned to `~5.9`
because typescript-eslint's peer range is `<6.1` (TS 7 is out).

## What's done (against the six scope items)

1. **Ride screen**: three pages (Interval / Recovery / Nerd) from `Layout` data, laptop/tablet/tv layouts
   (auto-picked by width), page dots, ←/→, touch swipe, and an auto page switch by phase. Rests under 60 s don't flip to
   Recovery, because the 30/15 s blocks would otherwise make the page strobe. **All 10 widget kinds** are implemented. Each widget
   subscribes at its own `rateHz` and filter chain (`createSample`), and tapping a big number cycles its smoothing.
   The top bar has elapsed/−remaining, ERG, NEO/HR status (icon + label), −5 %/+5 % with the current offset, skip, pause, and
   1×/10×/60×. Keys: Space, +/−, S, ←/→, Y/N, 1/2/3, E, ?, Esc, with a `?` overlay. The demo auto-plays at 10×.
2. **HR recovery widget**: observed dots, fitted mono-exp, a dotted prediction with a 90 % band, a dashed target, "ready in ≈ m:ss
   (lo–hi)", τ vs usual τ with a "slower" status badge, and an unreachable warning. It polls at 1–2 Hz only during recovery.
3. **Workout editor**: profile (canvas), outline and DSL stay in sync, with nested repeats. Parsed text is re-identified **by
   tree position** (`carryIds`), so outline rows stay mounted and the selection survives while you type. Supported operations:
   - **Profile:** drag a block's right edge to change duration, or its top to change power (a drag coalesces into one undo step).
   - **Outline:** drag rows to reorder, including into and out of repeats. Wrap is ⌘G, unwrap ⇧⌘G, duplicate ⌘D, delete Del.
     Steps are edited inline as DSL fragments, repeats via count, W/rep and `!last`.
   - **Undo/redo:** ⌘Z / ⇧⌘Z across all three views.
   - **Summary:** duration range, TSS, IF, time-in-zone bars and clickable issues.
   - **AI rewrite:** vary/harder/easier/shorter run through `createResource`. The P24 diff shows the old profile ghosted and the
     changed blocks outlined, plus an outline highlight and the proposal's summary; you can accept or reject it (Esc).
   - **Snippets:** insert the seed snippets, or save the selection as a snippet (stored in localStorage).
   - **Ride this:** loads the workout into the ride.
4. **Layout edit mode (V9)**: E or "✎ Layout" opens it. You can drag to move, drag the corner to resize, and remove widgets; add any kind
   from the palette; edit per-widget settings (kind, metric(s)/target/x-y, display rate 0.1–10 Hz, filter chain with presets
   plus an editable stage list, colour-by-zone, span, log/fisheye); edit pages (rename, add/delete, auto-show phases); switch
   device layouts; and save, discard or reset to default.
5. **Check-in (V2) → proposal**: 4×(1–5) tap targets, 30/45/60/90/custom minutes, and a note. `createResource(planner.propose)`
   shows a loading skeleton, then a proposal card with its P13 profile, stats and rationale, and Accept & ride / Regenerate / Edit.
6. **Post-ride review (V5)**:
   - **Summary tiles:** duration, kJ, NP, IF, TSS, decoupling, compliance and τ̄.
   - **P05 stacked panels:** power / HR / cadence / DFA-α1 (0.75/0.5 bands) / W'bal with one shared crosshair, workout bands
     and coach ◆ markers. Drag to select an interval for avg P/HR/cadence, then zoom.
   - **P20 decisions:** each decision has 👍/👎, which call `coach.rate`.
   - **Charts:** P09 MMP, P07 heatmap, P16 power×HR coloured by kJ (fatigue ramp, density fallback over 5k points) and
     τ per recovery block.
   - **Laps table:** per block target, avg W/HR/cadence, compliance and τ.

**Theming:**
- **Tokens:** all colour comes from CSS custom properties. Canvas and uPlot read them through `getComputedStyle` into a `ThemeTokens` signal. Every canvas
  draw function and uPlot options builder reads that signal, so switching mode, preset or classic zones redraws every canvas live with no reload.
- **Variants:** Dark (default), Light and Auto (OS); Graphite, Ocean and Ember presets; a classic rainbow zones toggle, flagged "not
  colour-blind safe".
- **Generated palette:** `scripts/gen-palette.mjs` builds the 7-step zone ramps and 5-step fatigue ramps per preset × mode in OKLCH,
  validates them with the dataviz validator (ordinal checks) and the five series slots (adjacent plus all-pairs for power/HR/cadence)
  in **both modes**, and writes `src/theme/palette.css`. Every set passes.
- **Ember:** status-critical shifts to magenta.

**Performance:**
- **No tree re-renders:** components run once. A telemetry sample updates exactly the text node or uPlot that reads it, so there is
  no app or widget re-render.
- **Ride status:** `RideStatus` is a store updated with `reconcile`, so the 2 Hz heartbeat only touches readers of `status.t`.
- **Chart lifecycle:** charts are created and destroyed with the widget (`onCleanup`) and updated with `setData`.
- **Poll rates:** `history` runs at ≤ 1 Hz, `histogram`/`heatmap` at ≤ 0.5 Hz, `mmp` at ≤ 0.2 Hz and `hrRecovery` at 1–2 Hz only in recovery.
- **Measured:** 60.3 fps (worst frame 16.8 ms) during a 60× ride at 1024×768 in headless Chromium.

Screenshots (`shots/`, dark and light): ride-interval, ride-recovery, ride-nerd, layout-edit, editor-nested,
editor-ai-diff, checkin, review.

## What's not done / partial

- The Fuel page and P08/P10/P21 are out of spike scope per VIEWS §5.
- "Strip click → jump" isn't implemented: the contract has no seek.
- Custom themes with the in-page validator aren't built. The validator runs at build time; OLED and high-contrast have tokens but no toggle.
- Not every plot has a table toggle. The log-time chart, histogram, heatmap and MMP do, and review laps are a table.
  The strip, HR-recovery, scatter, τ and P05 charts don't.
- The layout grid has no collision avoidance: widgets can be dragged on top of each other.
- Swipe is implemented with pointer events but was only tested with a mouse and keyboard, not real touch.
- Tablet (1024) uses the backend's tablet layout. The responsive rules for "strip moves into the top bar" aren't implemented.

## Where Solid helped (concretely)

1. **Fine-grained updates with no ceremony.** `createSample(spec)` is about 15 lines: an effect that subscribes, `onCleanup` that
   unsubscribes, and a signal. A widget reading `sample()?.v` in JSX updates one text node. The "no full-tree re-render" rule
   holds by construction; there's no memo, `useCallback` or dependency array anywhere.
2. **Imperative libs fit naturally.** Components run once, so uPlot and canvases are created once in an effect, rebuilt only when the
   options builder's tracked reads change (theme, span, scale), and fed via `setData`. A canvas `draw()` executed inside an
   effect auto-subscribes to whatever it reads (data poll, theme, selection, drag state). That made live theme switching and
   the editor's drag preview essentially free.
3. **Stores plus `reconcile`.** `RideStatus` heartbeats, layout drag/resize (path updates on one widget proxy) and the editor
   tree (`reconcile(next, {key: "id"})` after every immutable edit) keep DOM nodes and focused inputs alive across edits.
   `createResource` gave loading, error and refetch for propose/rewrite with no extra state.

## Where Solid hurt (concretely)

1. **`solid/reactivity` and imperative code.** The rule can't see that a `draw(ctx)` passed to `<Canvas>` runs inside an
   effect, so `forEach` callbacks and helper closures inside it were flagged ("captures the value of a reactive variable"). I
   moved every painter to a pure module-level function, which is arguably better, but it was forced. ESLint core's
   `no-unassigned-vars` also flags Solid's `let el; <div ref={el}>` idiom.
2. **Accessor discipline everywhere.** Never destructuring means `() => props.widget.metrics[0]` style getters and `n()` in every
   `<Show>` callback. I once wrote `classList={{ selected }}` (the accessor, not its value). TypeScript caught it,
   but it's an easy slip. Stores are proxies, so you have to `unwrap()` before `structuredClone`, and with
   `exactOptionalPropertyTypes` removing optional keys needs `produce`/`Reflect.deleteProperty`.
3. **Thin ecosystem.** There's no maintained Solid grid-layout or tree-DnD library (solid-dnd is 0.7 and stale) and no first-party uPlot
   binding, so grid drag/resize, outline DnD (native HTML5) and the uPlot wrapper were all hand-rolled. uPlot's
   `Distr` is an ambient const enum, so log scales need an eslint-disable.

## Linter/typechecker catches

There were 7 framework-specific findings:
- **`solid/reactivity` (5).** Three were valid "read outside a tracked scope" hints: a uPlot sync key from `props.widget.id` at setup,
  signals read inside a uPlot tooltip formatter, and a one-time `device()` read during store init. One was borderline (a store read
  inside a backend callback) and one was a false positive (canvas `draw` closures).
- **`solid/prefer-for` (1).** A React-habit `NAV.map(...)` in JSX.
- **tsc (1).** `classList={{ selected }}` passed the accessor instead of `selected()`.

Separately, ESLint core's `no-unassigned-vars` misread the `ref` idiom once (tooling friction, not a framework catch).

## Backend issues

- **Resolved upstream:** the full-ride linear streams and per-block recoveries needed for V5 were missing. The supervisor added
  `streams()`/`recoveries()` in main (e1c1d14); the UI recorder workaround was removed.
- `planner.propose` is deterministic per check-in, so **Regenerate** returns an identical proposal. The UI just re-calls it.
- `coach.rate(callId, rating, useful)`: the UI maps 👍 to `right`/useful and 👎 to `wrong`/not useful.

## LOC

`find src -name '*.*' | xargs wc -l` → **6788 total**: 4956 TS/TSX, 1444 hand-written CSS and 308 lines of generated
`palette.css`. Scripts (`shots.mjs`, `gen-palette.mjs`, the vendored validator) are not counted.
