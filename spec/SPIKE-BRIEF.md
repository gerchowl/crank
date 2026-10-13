# UI spike brief (identical for React / Svelte 5 / Solid)

You build the full UI described in `spec/UX-SPEC.md` in **one** framework, going all in on that
framework's idioms, against the mock backend `@crank/shared` (`shared/`, read its README, DSL.md
and `src/contract.ts`). Three agents do this in parallel in different frameworks; the results are
compared head-to-head, so build the best version **this framework** can produce. Don't hedge
toward a lowest common denominator.

## Fixed stack (same for all)
- `spikes/<framework>/`, a pnpm workspace member depending on `@crank/shared` (`workspace:*`)
- Vite + TypeScript strict (`noUncheckedIndexedAccess` on), no `any` in app code
- Charts: **uPlot** for time-series and line charts; plain canvas/SVG for heatmap, profile and strip
- Styling: plain CSS (CSS modules or the framework's scoped styles). **No component/UI kit**
  (no MUI, shadcn, Skeleton, etc.) so styling effort is comparable. Small focused libs are allowed
  where idiomatic for the framework (e.g. a drag-and-drop or grid-layout lib); list them in the README.
- Do not modify `shared/`. If you hit a backend bug or contract gap, work around it in a thin
  adapter file and record it in your README under "Backend issues".

## Guardrails (must be set up first, before feature work)
- `pnpm check` = typecheck + framework linter, must be 0 errors and 0 warnings at the end:
  - React: `eslint-plugin-react-hooks` (rules-of-hooks, exhaustive-deps) + React Compiler lint rules
  - Svelte 5: `svelte-check` + `eslint-plugin-svelte`, with `compilerOptions.runes = true`
    (Svelte 4 syntax must be a hard error)
  - Solid: `eslint-plugin-solid` (recommended, with `solid/reactivity` as error)
- Run `pnpm check` after every meaningful change and fix issues immediately; don't batch them.
- Add a `prek` hook config (`spikes/<framework>/.pre-commit-config.yaml`, local hooks running `pnpm check`).

## Scope
Views/plots per `spec/VIEWS.md` §5 (V2, V3 pages Interval/Recovery/Nerd/Fuel, V4, V5, V9; plot types
listed there), following VIEWS.md §0 visual conventions (no dual axes, sequential zone ramp, fixed
series identity, validated palette for dark and light; load the `dataviz` skill before chart work).
Theming per `spec/THEMES.md` (tokens; Dark/Light/Auto; Graphite/Ocean/Ember + Classic-zones toggle;
live switching incl. canvases). Screenshots in both dark and light.
Priority order if time runs short:
1. Ride screen with widget grid, pages, every widget kind, per-widget rate/filter subscriptions,
   top bar controls, keyboard shortcuts, auto-play of the demo workout at 1×/10×/60×
2. HR recovery widget (fit, prediction band, ready-in, unreachable)
3. Workout editor: profile + outline + DSL in sync, nested repeats, wrap/unwrap, drag reorder,
   undo/redo, summary/validation, AI rewrite with diff, snippets
4. Layout edit mode (drag/resize, widget settings, save/reset)
5. Pre-ride check-in → proposal flow
6. Post-ride review with decision rating

## Performance rules
- Telemetry must not re-render the app tree: only the subscribing widget updates.
- Charts update imperatively (uPlot `setData`) and are created and destroyed with the widget lifecycle.
- `history()`/`histogram()`/`heatmap()`/`mmp()` are polled at ≤ 1 Hz per widget.

## Done means
- `pnpm -C spikes/<framework> check` clean, and `pnpm -C spikes/<framework> build` succeeds
- `pnpm -C spikes/<framework> dev` runs, and the demo ride plays out of the box at 10×
- Screenshots via Playwright (a script `pnpm shots` writing to `spikes/<framework>/shots/`) of:
  ride Interval page mid-interval, Recovery page during an hr_recovery block, Nerd page,
  editor with a nested repeat selected, AI-rewrite diff, layout edit mode, check-in, review
- `spikes/<framework>/README.md`: how to run, libs used and why, what's done and what isn't,
  where the framework helped or hurt (be concrete and honest), backend issues, LOC count
  (`find src -name '*.*' | xargs wc -l` total)
- Commit on your own branch `spike/<framework>` in your worktree; do not merge.

## Final report (your last message)
Under 300 words: completion against the six scope items, check/build status (paste output
summary lines), LOC, the 3 places the framework helped most, the 3 places it hurt most,
and how many times the linter/typechecker caught a framework-specific mistake you made
(with one example).
