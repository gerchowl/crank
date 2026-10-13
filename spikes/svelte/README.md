# neo-trainer · Svelte 5 spike (runes only)

The full spike UI (V2 check-in, V3 ride with the Interval/Recovery/Nerd pages, V4 editor, V5 review,
V9 layout editor) in **Svelte 5 with runes only**. It uses plain Vite + TypeScript (not SvelteKit)
against `@neo/shared`.

## Run

```sh
pnpm install                       # at the repo root
pnpm -C spikes/svelte dev          # demo ride auto-plays at 10×
pnpm -C spikes/svelte check        # svelte-check (--fail-on-warnings) + eslint (--max-warnings 0)
pnpm -C spikes/svelte build
pnpm -C spikes/svelte shots        # starts Vite itself, writes shots/*.png (dark + light), ~2 min
```

URL parameters for demos:

| Parameter | Effect |
| --- | --- |
| `?speed=1\|10\|60` | ride speed |
| `?autoplay=0` | don't start the demo ride |
| `?page=interval\|recovery\|nerd` | pin a page (turns auto page switching off) |
| `?layout=laptop\|tablet\|tv` | pick a layout |
| `?mode=dark\|light\|auto` | theme mode |
| `?preset=ocean\|graphite\|ember` | theme preset |
| `?zones=classic` | classic zone colours |

Routes are `#/ride` (the default, full-screen), `#/checkin`, `#/editor` and `#/review`. Press `?`
anywhere for the shortcut overlay.

**Guardrails.** `svelte.config.js` sets `compilerOptions.runes = true`, which makes Svelte 4
syntax a hard compile error; `export let` fails `svelte-check`, and I checked that.
`eslint-plugin-svelte` runs its recommended rules plus `require-each-key`,
`prefer-svelte-reactivity`, `prefer-writable-derived`, `valid-compile` and `button-has-type`.
On top of that, `typescript-eslint` runs `strictTypeChecked`. TypeScript is strict with
`noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`. The prek hook is in
`.pre-commit-config.yaml` and runs `pnpm -C spikes/svelte check`.

## Libraries

- **uplot** handles the time-series and line charts (log-time panels, sparklines, MMP, HR
  recovery, review stack).
- **playwright** (dev only) drives `pnpm shots`.

That's all. There is no drag-and-drop or grid library. Grid drag and resize is a 60-line
attachment over pointer events (`src/ride/gridDrag.ts`). Outline reordering uses native HTML5
drag-and-drop. Popovers and dialogs use the native `[popover]` and `<dialog>` elements.

## What's done

All six scope items are done.

1. **Ride screen.** The widget grid renders laptop, tablet and tv layouts from data. It has the
   Interval, Recovery and Nerd pages, and pages can switch automatically by phase (toggleable).
   You can move between pages with the dots, with ←/→, or by swiping on touch.
   - All 10 widget kinds are built, and each subscribes at its own `rateHz` and `filter`.
   - The top bar shows elapsed and remaining time, ERG mode, the phase, trainer and HR status,
     ±5 %, skip, pause and speed 1×/10×/60×.
   - Keyboard: Space, + and −, S, Y and N, ←/→, E (edit), 1/2/3 (speed), ?.
   - Tapping a big number cycles its smoothing.
   - The strip marks blocks the coach changed (AI) and dims finished blocks. Click a future
     block to jump to it.
2. **HR recovery.** Shows observed dots, the fitted curve, a dotted prediction with its 90 %
   band, the target line, ready-in (≈ m:ss with lo–hi), τ vs the usual τ (with a "slower" status)
   and an "unreachable" warning. It polls at 1–2 Hz only during recovery.
3. **Editor.** Profile (canvas), outline and DSL text are three views of one tree.
   - Profile: drag a block's right edge to change duration, drag its top to change power.
     Shift-click extends the selection.
   - Outline: a recursive tree with collapse, inline fields for every step and repeat option,
     and drag-and-drop before, after or into a repeat.
   - Commands: wrap in a repeat (⌘G), unwrap (⇧⌘G), duplicate (⌘D), delete, ⌥↑/⌥↓ to move,
     ↑/↓ to select.
   - Text: parse errors are highlighted at the exact offset (click the error to jump there).
     Valid text updates the tree on every keystroke.
   - Undo and redo (⌘Z/⇧⌘Z) span all three views. A whole drag gesture is one undo step.
   - Summary: duration with its range, TSS, IF, time-in-zone bars and validation issues.
   - AI on the selection (vary, harder, easier, shorter with the same stimulus) shows a P24 diff:
     the new profile solid, the old one dashed, changed blocks outlined, the replaced outline rows
     marked, and summary deltas. Enter accepts, Esc rejects.
   - Snippets: the seeds, plus "save selection as snippet" (stored in localStorage).
4. **Layout edit mode.** Drag and resize on the 12-column grid. On drop, overlapping widgets are
   pushed down.
   - The catalog adds widgets and new pages.
   - Per-widget settings: kind, metrics, display rate on a logarithmic 0.1–10 Hz slider (with
     the source Hz shown), filter-chain editor (sma, ema, median, one-euro, hold), colour by zone,
     span and time scale. There is also a per-page auto-phase list.
   - Save, Cancel and Reset to default. Edits are live, and the widget being edited re-subscribes
     immediately.
5. **Check-in → proposal.** Four rows of 1–5 tap targets, time chips plus a custom value, an
   optional note and readiness tiles. Submitting shows a proposal card with a mini profile and
   the rationale. Accept rides it, Regenerate asks again, and Edit opens it in the editor.
6. **Review.**
   - Summary tiles: time, kJ, NP, IF, TSS, decoupling, compliance and τ̄.
   - P05 stacked panels: power (with target and lap bands by zone), HR, cadence, DFA-α1 (with
     the 0.75 and 0.5 reference lines from P14) and W'bal. Coach decisions appear as ◆ markers.
     Brush-zoom applies to all panels, and the selection shows its own stats.
   - P20 decision list with 👍/👎, which calls `coach.rate`.
   - P09, P07, the P16 fatigue-coloured scatter (density fallback above 5k points) and P12 τ per
     block.
   - A laps table with compliance and τ.
   - The review works mid-ride and polls `streams()` incrementally at 1 Hz.

**Theming.** Dark (the default), light and auto (follows the OS live). Presets are Graphite,
Ocean and Ember, plus the classic-zones toggle and OLED black. Everything is switchable live in
the ◐ Theme popover, including every canvas and uPlot chart.

**Visual conventions.** There are no dual axes; quantities with different units go in stacked
panels with one shared crosshair. Series colours are fixed by metric. Target lines are neutral
dashed. Zone colours come from a sequential ramp. Status always has an icon and a label. Every
analytics chart has a table toggle. Charts hover by default.

**Performance.** The interval page measured **60 fps at 60×** (Chromium, M-series Mac). Telemetry
never re-renders the tree: each widget owns its subscription, and sparklines push straight into
uPlot. Polls respect the budget: history 1 Hz, histogram/heatmap 0.5 Hz, mmp 0.2 Hz,
hrRecovery 2 Hz only in recovery.

### Not done or partial

- Responsive: the tablet layout is chosen below 1200 px wide, but there is no bottom tab bar
  and no TV-specific sizing beyond the tv layout data.
- There is no in-page custom-theme validator (the spec lists it under custom themes, which are
  not part of the spike ship list), and no high-contrast mode.
- Profile drag changes duration only for time-based steps; dragging a ramp shifts both ends.
- Layout collisions push widgets down but never compact the layout upwards.
- Swipe only reacts to touch and pen. Mouse users use the dots or arrow keys.
- There are no unit tests. Behaviour was checked with Playwright scripts: keyboard, DnD,
  profile drag, wrap/duplicate/undo/redo, and the AI diff.

## Palette validation (dataviz validator)

- Series slots 1–3 (power, HR, cadence) pass `--pairs all` in both modes. Worst CVD ΔE is 9.2
  in light and 9.4 in dark; worst normal-vision ΔE is 24.0 in light and 20.9 in dark.
  - Light cadence `#1baf7a` is 2.74:1 against the surface, a WARN under the relief rule. It is
    covered: values are always printed as text and analytics charts have table views.
- Zone ramps (7 steps) and fatigue ramps (5 steps) for every preset pass `--ordinal` in light
  (vs `#fcfcfb`) and in dark (vs `#1a1a19`). The dark steps were picked separately, not by
  inverting the light ones.
- Classic rainbow zones are opt-in and labelled "not CVD-safe".

## Where Svelte 5 helped

1. **Fine-grained reactivity with equality cut-offs.** `ride.svelte.ts` replaces `status` on
   every 2 Hz heartbeat, but consumers read deriveds like `ride.blockIndex`, `ride.phase` and
   `ride.tSec`. A derived only notifies when its value changes, so the heartbeat re-renders the
   clock text and nothing else.
   - `liveMetric()` is just `$state` plus `$effect`. Unsubscribe-on-destroy needs no extra code,
     and so does re-subscribing when a widget's `rateHz` or filter changes in edit mode.
   - There are no memo, selector or dependency-array decisions anywhere.
2. **Deep `$state` proxies as the editing model.**
   - The editor tree is one `$state` object. Profile drag, outline `bind:value` and DSL parsing
     all write into it.
   - Undo and redo is a single effect that serialises the tree and coalesces bursts. There are
     no command objects (`editor/state.svelte.ts`).
   - Layout edit mode binds widget config directly, so changing `rateHz` re-runs exactly that
     widget's subscription effect.
3. **Template primitives fit imperative charts.**
   - `{@attach}` gives uPlot and canvas a lifecycle. The attachment body rebuilds when theme or
     config changes; a nested `$effect` only calls `setData` or redraws.
   - The outline is a recursive `{#snippet}` with no row component and no prop drilling.
   - Components are values, so widget-kind dispatch is a typed `Record<WidgetKind, Component>`.
     A missing kind is a compile error.

## Where it hurt

1. **Proxies leak at runtime, and no tool catches it.** `structuredClone` throws on a `$state`
   proxy. That broke the AI proposal and duplicate at runtime while `pnpm check` stayed green.
   You need discipline: use `$state.raw` for values replaced wholesale, and `$state.snapshot()`
   before anything leaves the reactive world (backend calls, clones, storage).
2. **typescript-eslint plus the Svelte parser create friction.**
   - `{@render}` is typed as a void call, so `no-confusing-void-expression` had to be turned off
     for `.svelte` files.
   - `prefer-svelte-reactivity` flags scratch `Map`s that are local to a derived (4 disables
     with reasons).
   - Function bindings (`bind:value={get, set}`) on `<select>` are typed `any`, so I fell back to
     `value` plus `onchange`.
   - uPlot's `const enum` can't be used under `isolatedModules`.
3. **TypeScript narrowing in templates is shallow.** A null-check outside an `{#each}` doesn't
   carry inside it, so `{@const}` is needed to re-bind.
   - Seeding `$state` from a prop silently captures only the first value. `svelte-check` warns
     about it, and the fix is a writable `$derived`, which is non-obvious.
   - `editor.svelte.ts` next to `Editor.svelte` collides on case-insensitive filesystems.

**Linter and type-checker catches of framework-specific mistakes: 6.**

- `state_referenced_locally` (the local `scale`/`spanS` state seeded from the widget prop).
- The component registry rejecting `CoachCard` because it lacked the `widget` prop.
- An a11y role on an interactive `<canvas>`.
- An unused `svelte-ignore`.
- A `<select>` function binding typed `any`.
- Narrowing lost inside a nested `{#each}`.

The `structuredClone`-on-proxy bug was **not** caught statically.

## Backend issues (worked around; `shared/` untouched)

- **No full-ride stream.** `streams()` and `recoveries()` were added on main mid-spike and are
  used now (`review/record.svelte.ts`). Before that, the UI would have had to record streams
  itself.
- **No plan-position metric.** `elapsed` drifts from the compiled timeline after skips and
  HR-guided blocks, so the strip's "you are here" is computed as block start plus
  (durS − `block_remaining`).
- **No seek.** Clicking a future block on the strip calls `skipBlock()` N times.
- **`block_index` in `streams()` is a per-second mean.** At block boundaries it can be
  fractional (e.g. 13.75), so laps round it.
- **`Zone.color` carries rainbow hexes.** These conflict with the theming spec. The UI ignores
  them and maps the zone index to the token ramp.
- **`hrRecovery()` before any recovery block** returns `targetBpm: 0`. The UI hides the target.

## LOC

`find src -name '*.*' | xargs wc -l` → **6915 total**, which includes 224 lines of design tokens
and 138 of base CSS.
