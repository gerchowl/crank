# spike/react (f4ed4ae) — review scores: spec 8 · correctness 5 · robustness 5 · clarity 8

Paths are relative to `spikes/react/src/`.

- [ ] **R1 High — Review shows previous ride.** `review/record.ts:14-27` module-level record is never reset; the reset check
  `r.toS < cur.toS` can't fire (backend returns `toS = max(fromS, len)`). Repro: ride the demo to 37 min → Review → check-in →
  accept Endurance 30′ → ride 23 min → Review shows the demo's 30/15 laps; later rides stitch together. (X3)
- [ ] **R2 High — AI accept resurrects undone edits.** `editor/AiPanel.tsx:76-81` captures `state.nodes` at click; accept
  commits `diff.after` (`editor/state.ts:86`). Repro: select "3m 60%" → Harder → delete Cool-down while pending → Accept →
  Cool-down returns. (X1)
- [ ] **R3 High — DSL editable during an open diff**; the outline shows `diff.after`, the textarea `state.text`; Accept drops typed text. (X1)
- [ ] **R4 Medium — Drag duplicates nodes.** `editor/tree.ts:201` `move` accepts a selection containing a repeat and its own
  child. Repro: Cmd-click Threshold + "4m 95%", drag above Warm-up → `4m 95%` twice with the same id.
- [ ] **R5 Medium — Finished ride never reaches Review**; "✓ Done" (`ride/TopBar.tsx:56-60`) doesn't navigate. (X6)
- [ ] **R6 Medium — Stale coach nudge after a new ride.** `nudgeStore` (`lib/live.ts:72`) never cleared. (X3)
- [ ] **R7 Medium — Layout edits dropped after "+ Page" → "Reset to default".** `pageIdx` stays 3; dispatches target a
  missing page (`layout/LayoutEditor.tsx:97`). (X8)
- [ ] **R8 Low — Histogram table "typical" column misindexed.** `ride/widgets/HistogramWidget.tsx:88` maps filtered index.
- [ ] **R9 Low — No error boundary**; malformed snippets in localStorage (`editor/Snippets.tsx:13`) unmount the app. (X4)
- [ ] R10 Chart span/scale in edit mode not live (`LogTimeChart.tsx:68` `useState(widget.spanS)`, `HeatmapWidget.tsx:19`). (X11)
- [ ] R11 Unsaved layout edits lost on Exit without warning.
- [ ] R12 Collapsed repeats re-expand on every DSL keystroke (ids reissued per parse).
- [ ] R13 After wrap, the selection stays on the children, so ⇧⌘G is a no-op.
- [ ] R14 AI "changed" highlight marks every rewritten node, even unchanged ones.
- [ ] R15 1024×768: outline clipped (scrollWidth 454 vs 294), "!last" only reachable by horizontal scroll. (X7)
- [ ] R16 UI-side physiology: lap compliance (`review/Review.tsx:104-110`), Δ% (`BigNumber.tsx:29`), target band (`ZoneBar.tsx:253`). (X2)
- Not probed: touch swipe, OS-driven Auto mode, unreachable HR-recovery path, profile drag-edge editing.
