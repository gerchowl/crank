# neo-trainer — UX spec for UI spikes (v0)

Indoor-cycling training app for a Tacx NEO (BLE FTMS) + BLE HR strap, with an AI coach
(System-1 judge in-ride, LLM planner pre/post-ride). Final app: Rust core + Tauri v2.
**These spikes are UI only**: a plain Vite + TS web app against the shared mock backend
in `shared/` (stands in for the Rust core / Tauri channel). All metrics, filters,
aggregation, fitting, DSL parsing and workout compilation live in `shared/` (later Rust);
the UI only subscribes, arranges and edits.

## Defaults for open questions (v0)
- Primary form factor: laptop landscape (≥1280×800); must also work on tablet landscape 1024×768.
  Readable from ~1 m: big numbers ≥ 64 px on the ride screen.
- Interaction mid-ride: big on-screen buttons + keyboard shortcuts (no typing required).
- HR-limited / open-ended blocks: in v1 (shown as duration ranges).
- Workout text DSL: own syntax (intervals.icu-inspired), defined in `shared/` (`parseDsl`/`printDsl`).
- Theme: dark by default (pain cave), light supported.

## Screens

### 1. Pre-ride check-in (≤ 10 s to complete)
Sleep, legs, stress, motivation (1–5 each, tap targets), time available (30/45/60/90/custom),
optional free-text "how do you feel". Submit → mock planner returns a proposed workout
(`backend.planner.propose(checkin)`) with a short rationale → accept / regenerate / edit.

### 2. Ride screen (the core)
- **Widget grid**, layouts are **data** (`Layout` type in contract). Multiple **pages** per layout;
  swipe / ←→ keys / page dots. **Context-aware**: optional auto page switch by phase
  (`warmup | work | recovery | cooldown | free`).
- **Edit mode**: drag / resize widgets on a grid, add/remove widgets from the metric catalog,
  per-widget settings (metric, kind, display rate 0.1–10 Hz, filter chain, smoothing, colour-by-zone,
  chart span). Save layout; reset to default.
- **Widget kinds** (all must exist):
  - `big_number` (optional zone colour, Δ to target)
  - `number_sparkline`
  - `zone_bar` (value vs target band)
  - `log_time_chart` — x = time-ago on log scale (now at right; 1 s … 2 h), mean line +
    min/max envelope from the multi-resolution store; optional piecewise "fisheye" scale;
    overlay target power; multiple series (power/HR/cadence)
  - `histogram` — time-in-bin, zone-coloured, optional "typical" overlay
  - `heatmap2d` — power×HR and power×cadence
  - `mmp_curve` — mean-maximal power, log x (1 s–60 min), today vs 90-day vs all-time
  - `workout_strip` — full profile, "you are here" marker, AI-changed blocks highlighted
  - `hr_recovery` — live HR recovery with fitted exponential, dotted prediction, uncertainty
    band, target line, "ready in ≈ m:ss (lo–hi)", τ vs usual τ, unreachable warning
  - `coach_card` — s1 nudge text + band (strong/lean) + accept/decline (keys: Y / N)
- **Display rate** is per widget and drives a **subscription** at that rate
  (`backend.subscribe(metric, {rateHz, filter})`); never re-render the whole tree per frame.
- Top bar: elapsed / remaining, trainer mode (ERG/sim/free), connection status of trainer + HR,
  pause, manual ± 5 % (keys + / −), skip block (key S).
- Default layout (page "Interval"): power 3s big + target/Δ, interval countdown + block x/n,
  HR + zone sparkline, cadence + target, W'bal bar, log-time chart (power/HR/target, 2 h span),
  workout strip, coach card. Page "Recovery": hr_recovery large, next-block preview, HR, power.
  Page "Nerd": histograms, power×HR heatmap, MMP curve.

### 3. Workout editor
- Model = tree (`WorkoutNode` in contract): `step` and `repeat` (nested to any depth,
  `progression` per rep, `skipLastRest`). Step end conditions: time, kJ, lap, `hr_recovery`
  (target, min, max, recovery power, onUnreachable).
- **Three synchronised views**, an edit in any updates all:
  1. **Profile** (canvas/SVG): blocks proportional to time, height = power; drag block's right edge
     to change duration, drag top to change power; ranges for open-ended blocks; zone colours.
  2. **Outline**: indented tree, collapse, drag-to-reorder (incl. into/out of repeats),
     wrap selection in repeat (⌘/Ctrl+G), unwrap, duplicate, delete, edit fields inline.
  3. **Text DSL**: editable textarea; parse errors shown inline with position; valid text updates tree.
- **Summary** panel from `compileWorkout`: total duration (range), TSS, IF, time-in-zone bars,
  validation issues (max power, W'bal floor, cooldown present).
- **Undo / redo** (⌘Z / ⇧⌘Z) across all three views.
- **AI on selection**: "vary" / "harder" / "shorter same stimulus" → `backend.planner.rewrite(subtree, intent)`
  → show **diff** (old vs new overlaid on profile + outline highlight) → accept / reject.
- Snippet library: save selection as snippet; insert snippet (seed: 30/15s, 4×4, over-unders, Tabata).

### 4. Post-ride review (minimal)
Summary numbers, log-time chart of the whole ride, list of coach decisions (time, verdict, band,
accepted?) each rateable 👍/👎 (`backend.coach.rate(callId, ...)`), τ per recovery block chart.

## Non-functional
- 60 fps while riding on a mid laptop; no full-tree re-render per telemetry frame.
- Keyboard shortcuts listed in a `?` overlay.
- Strict TS, framework linter clean, no `any` in app code.
- Must run with `pnpm dev`; mock ride auto-plays a 45-min workout at selectable speed (1×/10×/60×).
