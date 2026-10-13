# spike/svelte — review scores: spec 8 · correctness 5 · robustness 5 · clarity 8

Paths are relative to `spikes/svelte/src/`.

- [ ] **S1 High — AI accept replaces the wrong nodes.** The proposal stores parent path + index; Delete, ⌘G, ⌥↑/↓ and the toolbar stay
  enabled while pending; accept doesn't verify the target (`editor/state.svelte.ts:286-294`, `Editor.svelte:54-76`).
  Repro: select "3m 60%" → Harder → Delete → Enter → the "VO2 sets" repeat is deleted. Repro 2: Warm-up → Easier → ⌥↓ → accept
  → two warm-ups, "3m 60%" lost. (X1)
- [ ] **S2 High — Undo history corrupted after undo + new edit.** `#record` early return (line 104) leaves `#restoring` true;
  the next edit isn't pushed and redo isn't cleared (`state.svelte.ts:103-109`). A→B→C, undo to B, wrap → D: redo still enabled,
  undo jumps to A (B lost), redo resurrects C.
- [ ] **S3 Medium-high — DSL/tree desync after a parse error.** `text = draft ?? printed`; the draft is only cleared when valid
  (`state.svelte.ts:67,264`). Type `3x(` → delete Warm-up in the outline → the text still shows it → fixing the text resurrects it.
- [ ] **S4 Medium — Top bar off-screen at 1024×768** (`TopBar.svelte:75` `flex-wrap: nowrap`, 1213 px wide; Layout/Theme/?/Exit
  at x ≥ 1027, Exit being the only route to Review). The device layout doesn't follow resize. (X7)
- [ ] **S5 Medium — Reset ignores Cancel** (writes storage immediately, `layout.svelte.ts:90`); device switch + Cancel leaves the
  selector on "tablet" with tablet's row count (`:53,76`). (X8)
- [ ] **S6 Medium — Lap compliance differs from the backend** (`Review.svelte:86-90`, 1 Hz means without settle: 89.5 % vs 95 %). (X2)
- [ ] **S7 Low-medium — First "+ filter" click does nothing.** `(w.filter ??= []).push(...)` pushes into the raw array, not the proxy
  (`LayoutPanel.svelte:143`); empty/0 filter params reach the backend (`windowS:null`, `n:0`).
- [ ] **S8 Low-medium — Empty repeats allowed**; they print as `5x()`, which the parser rejects; validation repeats the issue 3×.
- [ ] S9 Arrow keys on a focused rate slider change the page.
- [ ] S10 After undo the selection keeps its path, so the next ⌘G wraps a different node.
- [ ] S11 Malformed `neo-trainer.snippets.v1` blanks the editor (unchecked cast, `state.svelte.ts:28`). (X4)
- [ ] S12 Empty custom-minutes field sends `minutes: null` to the planner.
- [ ] S13 "Ride this" starts an empty workout without warning.
- [ ] S14 Runtime `ownership_invalid_mutation` warnings (`StepFields.svelte:38`) despite a clean `pnpm check`.
- [ ] S15 UI-side rules: τ "slower" 1.1× (`HrRecovery.svelte:20`), W′bal floor 25 (`ZoneBar.svelte:35`), Δ% chip, lap averages. (X2)
- [ ] S16 Spec gaps: no P14 DFA trace with 0.75/0.5 bands on the Nerd page; no table toggle on the log-time chart; no auto Review on finish. (X6)
- [ ] S17 No unit tests for tree logic. (X9)
- Not probed: touch swipe, real OS theme switch (emulated OK), a 45-min ride at 1×, tv layout.
