# crank — theming (colour schemes, dark/light)

All colour comes from **CSS custom properties (design tokens)**. Components and charts reference
roles, never hex. A theme = values for the token layers below × mode (dark / light).

## Token layers

| Layer | Tokens | Rule |
|---|---|---|
| Surfaces & ink | `--surface-0/1/2`, `--ink-primary/secondary/muted`, `--border`, `--focus` | neutral; text always uses ink, never series colours |
| Accent (chrome) | `--accent`, `--accent-ink`, `--accent-soft` | buttons, selection, active tab; never used for data |
| Series (identity) | `--series-power`, `--series-hr`, `--series-cadence`, `--series-dfa`, `--series-torque`; `--target` (neutral dashed) | fixed slots; validated as a categorical set (adjacent and all-pairs for the first 3) |
| Zone ramp | `--zone-1 … --zone-7` | **sequential single hue**, light→dark (dark mode: dark→light), labelled Z1…Z7 |
| Fatigue ramp | `--fatigue-1 … --fatigue-5` | sequential, hue ≠ zone hue |
| Diverging | `--div-neg`, `--div-mid` (grey), `--div-pos` | Δ to target, Δ vs typical, TSB |
| Status | `--status-good/warning/serious/critical` | reserved for coach/sensor state; always with icon + label |

## Modes
- **Dark** (default, "pain cave"), **Light**, **Auto** (follows OS `prefers-color-scheme`; an explicit choice wins).
- Dark values are **selected per step from the same ramps** and validated against the dark surface, not inverted.
- Optional **OLED black** variant (surface-0 = #000) for TV / night riding; optional **high contrast**.

## Presets (accent hue + zone/fatigue hues)

| Preset | Chrome | Zones | Fatigue | Notes |
|---|---|---|---|---|
| **Graphite** | grey | grey ramp | blue ramp | quiet, all attention on numbers; zone labels mandatory |
| **Ocean** (default) | blue | blue ramp | amber ramp | calm, CVD-friendly |
| **Ember** | red/orange | red ramp | teal ramp | `--status-critical` shifts to magenta so it doesn't vanish into the zone reds; icon+label carry status anyway |
| **Forest** | green | green ramp | violet ramp | |
| **Classic zones** | (any preset) | rainbow Z1 grey · Z2 blue · Z3 green · Z4 yellow · Z5 orange · Z6 red · Z7 purple | — | opt-in toggle for cyclists used to TrainingPeaks/Zwift colours; flagged as not CVD-safe |

Series slots stay the same across presets (identity must not change with theme); only their
light/dark steps differ.

## Custom themes
- User picks accent hue + zone hue (+ mode). The app runs the **dataviz palette validator in-page**
  (`validate_palette.js` as a module) on the resulting set; failing steps are **snapped to the nearest
  passing step** or the combo is refused with a reason.
- Themes are data (JSON, like layouts): savable, per-device (e.g. TV = OLED Graphite).

## Implementation notes (for the spikes)
- `:root[data-theme="ocean"][data-mode="dark"] { --… }`, with the dark block duplicated under
  `@media (prefers-color-scheme: dark)` guarded by `:where(:not([data-mode="light"]))` for Auto.
- **Canvas and uPlot don't see CSS variables by themselves**: read tokens with `getComputedStyle` into a
  JS theme object, and **redraw every canvas/uPlot on theme change**. Switching must be live, with no reload.
- Spikes must ship: Dark + Light + Auto, presets **Graphite, Ocean, Ember**, the Classic-zones toggle,
  and a theme switcher (Settings popover is fine). Screenshots in both modes.
