# spike/solid — review scores: spec 7 · correctness 4 · robustness 5 · clarity 7

Paths are relative to `spikes/solid/src/`.

- [ ] **L1 Critical — Profile and summary never update after edits.** `editor/state.ts:82` compiles from an untracked copy.
  Repro: replace the DSL with `10m 50%\n5m 100%` → the outline updates, but the Summary still shows `43:15 | 51 | 0.84` and the profile
  draws the demo. Profile drag gives no visual feedback and hit-tests stale blocks.
- [ ] **L2 Critical — "Ride this" crashes.** `editor/Editor.tsx:87` calls `structuredClone` on a store proxy → `DataCloneError`;
  stays on `#/editor`. (X5)
- [ ] **L3 High — AI accept discards edits made during the diff** (`state.ts:89-91`, `:219`); DSL and shortcuts stay live during the diff;
  an undo during the diff is reverted on accept. (X1)
- [ ] **L4 High — Auto theme + dark OS = light surfaces with dark series colours.** `theme/tokens.css:37` beats `:57`.
- [ ] **L5 Medium — Ride shortcuts die after clicking the "auto" checkbox** (`RideScreen.tsx:34` ignores keys while any input,
  including a checkbox, has focus); Space toggles the checkbox.
- [ ] **L6 Medium — Heatmap Y metric "—" throws** `unknown metric` (`LayoutEditor.tsx:160` → `Heatmap.tsx:23`); the widget stops
  updating; the dirty flag isn't set (`layout.ts:336-337`). (X8)
- [ ] **L7 Medium — Changing widget kind keeps the old metrics** → "POWER 138 W | target 106 bpm | ▲ +32 W" (`LayoutEditor.tsx:133`). (X8)
- [ ] **L8 Medium — "You are here" uses elapsed time vs the planned schedule** (`WorkoutStrip.tsx:222`); wrong after skips and
  HR-guided recoveries. (X3)
- [ ] **L9 Medium — Laps table recomputes compliance in the UI** over planned block times, ±5 % of 1 s power vs the backend's 3 s
  (`Review.tsx:265-277`). (X2)
- [ ] L10 Heatmap pair switch shows mislabelled data for up to 5 s (polls ignore settings until the next tick, `primitives.ts:31-41`).
- [ ] L11 Repeat-count field can disagree with the model (999 shown, DSL says 50x; `Outline.tsx:142-145`).
- [ ] L12 Spec gaps: cadence big number 53 px (35 px at 1024) < 64 px; no DFA-α1 0.75/0.5 bands on the Nerd page; device layout only
  picked at startup. (X7)
- [ ] L13 Suspected: heatmap table sorts clock text lexically (`Heatmap.tsx:63`); legend chip colour fallback (`LogTimeChart.tsx:375`).
- [ ] L14 UI-side physiology: recovery curve evaluation (`HrRecovery.tsx:31`), τ "slower" 1.15× (`:54`), nudge expiry copy
  (`CoachCard.tsx:143`), Δ as value − target (`BigNumber.tsx:220`). (X2)
- [ ] L15 Header jumps on pause (user report; Pause/Resume width). (X10)
- Not probed: touch swipe; `pnpm shots` (another agent's Chromium was running).
