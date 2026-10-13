# neo-trainer · React 19 UI spike

The full spike UI (UX-SPEC / VIEWS §5) in **React 19.3** with the **React Compiler** on,
running against the mock backend `@neo/shared`.

```sh
pnpm install                      # at the repo root
pnpm -C spikes/react dev          # http://localhost:5173 — the demo ride auto-plays at 10×
pnpm -C spikes/react check        # tsc -b + eslint (react-hooks incl. compiler rules), 0 warnings allowed
pnpm -C spikes/react build
pnpm -C spikes/react shots        # starts its own Vite server, writes shots/*.png (dark + light), ~100 s
node spikes/react/scripts/gen-themes.mjs   # regenerate + re-validate theme tokens
```

URL knobs (handy for demos/screens): `?speed=1|10|60`, `?autostart=0`, `?mode=dark|light|auto`,
`?theme=graphite|ocean|ember`, `?zones=classic`; routes `#/ride`, `#/ride?page=nerd`, `#/ride?edit=1`,
`#/editor`, `#/checkin`, `#/review`.

## Stack and libraries

| lib | why |
| --- | --- |
| react / react-dom 19.3 | the framework under test |
| babel-plugin-react-compiler 1.0 via `@rolldown/plugin-babel` + `reactCompilerPreset` | the compiler (Vite 8's plugin-react no longer runs Babel itself) |
| uplot | time series / line charts (P04, P05, P09) as required |
| eslint-plugin-react-hooks 7 (`recommended-latest`) | rules-of-hooks, exhaustive-deps (as error) and all React Compiler rules (`refs`, `purity`, `immutability`, `set-state-in-effect`, …) |
| typescript-eslint `strictTypeChecked`, eslint-plugin-react-refresh | type-aware lint, no `any`, fast-refresh boundaries |
| playwright | `pnpm shots` |

No UI kit, no DnD or grid-layout library: grid drag/resize is ~60 lines of pointer events, the outline uses
native HTML5 drag and drop. TypeScript is pinned to 5.9 (TS 7 has no JS API yet, which typescript-eslint needs).
`prek` hook: `.pre-commit-config.yaml` runs `pnpm check`.

## Architecture (how telemetry stays out of React)

- `lib/live.ts`: every widget owns its subscription. Numbers use `useMetric()` (one external store
  per subscription, read with `useSyncExternalStore`), so **only that leaf re-renders, at its own rate**.
  Charts/sparklines use `useMetricCallback()` (`useEffect` + `useEffectEvent`) and draw imperatively.
  Ride status and nudges are one backend listener each, read through selectors.
- Measured, not assumed: over 10 s at 10× on the Interval page, `RideScreen`, `PageGrid`, `TopBar`, the
  log-time chart, sparkline and strip rendered **0 times**; the three `BigNumber`s rendered exactly at
  their subscribed rates (16 Hz × 10 s, doubled by StrictMode in dev).
- `charts/useUPlot.ts` / `useCanvas.ts`: React 19 **callback refs with cleanup** create/destroy the chart
  with the element (works for conditionally mounted canvases too); `setData` is imperative; analytics are
  polled per the budget (`usePoll`: history ≤ 1 Hz, histogram/heatmap ≤ 0.5 Hz, mmp ≤ 0.2 Hz,
  hrRecovery 1–2 Hz only in the recovery phase).
- Theme: CSS tokens (`theme/tokens.css`, generated). `theme/theme.ts` keeps prefs in a tiny store, reads
  tokens with `getComputedStyle` into a `ChartTheme` object on every change (incl. OS scheme for Auto);
  charts take it as a dependency, so switching is live and repaints every canvas/uPlot.
- Editor: one `useReducer` (tree is the truth, DSL text derived; while typing in the DSL the text leads and
  the tree follows on each successful parse), snapshots for undo/redo across all three views, context via
  React 19 `<Context value>` + `use()`. AI rewrite uses an **async transition** (`useTransition`) for the
  planner latency; check-in is a **form action** with `useActionState`.

## Scope status

| # | item | status |
| --- | --- | --- |
| 1 | Ride screen: 12-col widget grid from `Layout` data, Interval/Recovery/Nerd pages, auto page by phase (toggle, key A), ←/→, swipe, page dots; all 10 widget kinds; per-widget rate + filter subscriptions; top bar (elapsed/remaining, ERG, NEO/HR status, ±5 %, skip, pause, 1/10/60×); shortcuts + `?` overlay | done |
| 2 | HR recovery: observed dots, fitted curve, dotted prediction + 90 % band, target line, ready-in ≈ m:ss (lo–hi), τ vs usual τ with status tag, unreachable warning | done |
| 3 | Editor: SVG profile (drag right edge = duration, drag top = power, hatched open-ended blocks + range whisker), outline (collapse, DnD before/after/into repeats, inline label/count/+W per rep/!last, step fields edited as their DSL spec), DSL textarea with in-place error highlight + line/col, summary (duration range, TSS, IF, time-in-zone, issues → click selects node), undo/redo ⌘Z/⇧⌘Z, wrap ⌘G, unwrap ⇧⌘G, duplicate ⌘D, delete, AI vary/harder/easier/shorter → ghost-vs-new profile diff + outline highlight → accept/reject, snippets (seed + save selection, localStorage) | done |
| 4 | Layout edit mode: drag/resize on the grid, add (kind × metric catalogue)/remove widgets, settings (kind, metrics, rate 0.1–10 Hz log slider, filter chain editor, colour-by-zone, span, log/fisheye), add page, save, reset to default | done |
| 5 | Check-in → proposal: 4 × 1–5 tap rows, 30/45/60/90/custom, note, readiness tiles (P23-ish), proposal with profile mini + rationale → accept & ride / regenerate / edit in editor | done |
| 6 | Review: summary tiles, P05 stacked panels (power+target/HR/cadence/DFA-α1 with P14 bands/W'bal) with workout bands, coach markers, brush-select → interval stats + zoom; P20 decision list with 👍/👎 → `coach.rate`; P09, P07, P06 ×2, P16 fatigue scatter, P12 τ per block, laps table | done |

Themes: Dark/Light/Auto × Graphite/Ocean/Ember + Classic-zones toggle, switcher in the ◑ Theme popover (top bar and rail).
All zone/fatigue ramps and series slots are generated in OKLCH and **validated with the dataviz validator at
generation time** (`scripts/gen-themes.mjs` exits non-zero on a hard fail; report in `src/theme/palette-report.txt`).
Light zone ramps had to be darkened at the pale end to clear the 2:1 floor — the validator caught that.

Not done / partial:
- No in-page validator for *custom* themes (only presets are required); OLED/high-contrast variants not built.
- Table-view toggle exists on histogram, heatmap and MMP; the review's P05/P16/τ plots rely on the laps table instead.
- Hover: per-mark tooltips on histogram/heatmap; uPlot plots use a live legend readout + synced crosshair, not floating tooltips.
- P04 has no piecewise fisheye *controls* beyond a log ↔ fisheye toggle; P09 has no model fit (out of scope).
- Layout editor: no collision avoidance (widgets may overlap if you drop them so), no per-page `autoPhase` editor.
- Tablet: the tablet layout is picked below 1200 px wide and the top bar compacts; the strip-into-top-bar rule from VIEWS §4 is not implemented.
- No unit tests of the editor tree ops (exercised through Playwright flows only).

## Where React helped

1. **External stores + `useSyncExternalStore`**: per-widget subscriptions map 1:1 onto the backend's
   `subscribe`; a widget is literally `useMetric(metric, rateHz, filter)`. Isolation of re-renders falls out
   of the component tree, and it's measurable (above).
2. **React Compiler + its lint rules**: no hand-written `useMemo`/`useCallback` soup in widgets, and the
   rules caught real mistakes (see below). JSX-literal widget configs in the review stay referentially stable.
3. **React 19 primitives fit the async bits**: `useActionState` form action for check-in, async
   `useTransition` for the AI rewrite (`isPending` for free), callback refs with cleanup for uPlot/canvas
   lifecycles, `useEffectEvent` for keyboard handlers and subscriptions that need fresh state without
   resubscribing. One reducer + context made "edit in any view updates all + undo across views" easy.

## Where React hurt

1. **Effect/ref ceremony around imperative charts.** Anything that must not re-render needs refs, effect
   events, "latest value in a layout effect" or callback refs — and the compiler's `refs` rule forbids
   the obvious ref-in-options pattern, so uPlot zoom had to move into an effect (`setXRange`).
   `useEffectEvent` can't be called from a `useCallback`, which is exactly what a `redraw()` wants.
2. **StrictMode and React 19 semantics bite silently.** The reducer initializer runs twice (it consumed a
   one-shot hand-off store → fixed by consuming it in an effect); React 19 **resets uncontrolled forms after
   an action** (check-in chips snapped back to defaults → defaults now follow the last submission). Neither
   was caught by lint or types; both only by screenshots.
3. **`useEffect` + `useRef` doesn't notice late-mounted elements**: the HR recovery canvas (rendered only once
   data exists) was never observed; switching to callback refs fixed it. Also fast-refresh rules force
   splitting helpers out of component files (5 extra small modules).

## Linter / typechecker catches of framework-specific mistakes: 8

- `react-hooks/rules-of-hooks` + compiler `preserve-manual-memoization` (1): calling a `useEffectEvent` from
  a `useCallback` redraw in `useCanvas`.
- `react-hooks/refs` (2, one incident): passing a ref and a closure reading `ref.current` into an options
  builder that runs during render (review P05 zoom).
- `react-refresh/only-export-components` (5): exporting helpers (`zoneOf`, `bucketsToTable`, `toMmpData`,
  `paintRecovery`, `respondLatest`) from component files.

Example: `RideTimeline.tsx` — `useMemo(() => panelOpts(…, rangeRef, …))` → *"Cannot access refs during render"*;
fixed by keeping the range in state and applying it with `u.setScale` in an effect.

## Backend issues / contract notes

- Before `streams()`/`recoveries()` landed there was no full-ride record or per-block recovery history; now
  used as intended (incremental `streams(metrics, previous.toS)` at 1 Hz in `review/record.ts`).
- No actual block start/end times in the contract (only expected `FlatBlock.startS`, which drifts with
  HR-guided recoveries and skips). Laps are derived from the recorded `block_index` stream.
- `subscribe` rate is wall-clock: at 60× a 1 Hz display gives one point per 60 ride-seconds, so sparklines use a
  second, ≥ 4 Hz subscription for the line and the widget rate for the number.
- `coach.rate(callId, rating, useful)`: the UI has only 👍/👎, mapped to `right`/`wrong` with `useful = right`.
- Uses presentation helpers exported by `@neo/shared` (`timeAgoToU`/`uToTimeAgo`/`timeAgoTicks`, `zoneIndex`,
  `reId`) and `demoWorkout` as the editor's fallback; the shared README sanctions these.
- uPlot ships `Distr` as an ambient `const enum`, unusable under `isolatedModules`; one justified
  `eslint-disable` line in `MmpWidget.tsx`.

## Size

`find src -name '*.*' | xargs wc -l` → **5670** total (incl. 185 lines of generated `tokens.css` and
~1500 lines of CSS). Scripts (`gen-themes`, `shots`) not included.

## Screenshots (`shots/`, each in `-dark` and `-light`)

`ride-interval` (threshold interval, coach nudge), `ride-recovery` (during the first hr< block),
`ride-nerd`, `editor-nested-selected`, `editor-ai-diff`, `layout-edit`, `checkin`, `review` (full page).
