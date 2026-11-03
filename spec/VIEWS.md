# crank — views, panels, plot types & wireframes (v0)

Companion to `UX-SPEC.md`. Defines **every screen (V-xx), panel and plot type (P-xx)**, what
data each binds to, and wireframes. Data comes from the backend (`shared/` mock → Rust core);
plots never compute physiology themselves. "Contract" in the tables below means the ADR-0001 §4
bindings (frozen spike reference: `shared/src/contract.ts`; see UX-SPEC "Contract status"): live
metrics arrive on the window's `StreamFrame` channel (ADR-0001 §4.3), every other data source is an
**async query** (§4.2), and all time axes use **session seconds** with gaps for pauses and dropouts (§5).

---

## 0. Visual conventions (apply everywhere)

- **No dual axes.** Quantities on different scales (W, bpm, rpm) are drawn as **stacked panels sharing
  the x-axis** (one crosshair across all), never two y-scales on one plot.
- **Fixed series identity**, never cycled and never re-assigned by rank:
  power → slot 1, HR → slot 2, cadence → slot 3, DFA-α1 → slot 4, torque → slot 5.
  **Target** = neutral dashed line (text-muted ink), never a series colour.
- **Zones are ordered magnitudes** → **sequential single-hue ramp** (light→dark) with zone labels
  (Z1…Z7) on the axis/bands. A "classic rainbow zones" option may exist as a user preference, but it's off by default.
- **Fatigue / time-in-ride colouring** (kJ spent, ride quarter) → sequential ramp of a *different*
  hue than zones.
- **Deltas** (Δ to target, Δ vs typical) → diverging pair with a neutral grey midpoint.
- **Status colours are reserved** for coach/sensor state (good / warning / serious / critical) and
  always come with an icon and a label.
- Values and labels use text ink, never the series colour (the colour chip next to the text carries identity).
- Thin marks (2 px lines), recessive grid, legend for ≥ 2 series plus direct labels for ≤ 4.
- **Hover by default**: crosshair + tooltip on time plots, per-mark tooltip on bins and cells.
  On the ride screen hover is secondary (glanceability first).
- **Dark mode is designed, not inverted** (the pain cave default); light mode exists. The palette is validated
  with the dataviz validator for both surfaces (spikes: load the `dataviz` skill and run the validator).
- Every analytics plot has a **table view** toggle (accessibility + exact values).
- Time axes: `log` (time-ago, now at right) or `fisheye` (linear last N s, compressed before) or
  `linear` (whole ride / trends).

---

## 1. Plot-type catalogue

| ID | Plot | Data (contract) | Encoding | Interactions | Used in |
|---|---|---|---|---|---|
| **P01** | Big number tile | `subscribe(metric)` (stream sub) | value large, unit small, optional zone chip, Δ to target as diverging chip | tap → cycle smoothing | V3 |
| **P02** | Number + sparkline | `subscribe` (stream sub) + short buffer | number + 60–300 s line | — | V3 |
| **P03** | Target band bar | value, target, zone bands | horizontal bar, target band shaded, marker = value | — | V3 |
| **P04** | **Log-time stacked panels** | async `history()` per metric (session s, null gaps) | per panel: mean line + min/max envelope; target dashed in the power panel; shared log/fisheye x (1 s…2 h) | crosshair across panels, span picker | V3, V5 |
| **P05** | Linear time stacked panels | async `streams()` (session s, null gaps) | power, HR, cadence, (DFA-α1), W'bal panels; workout profile as background bands; coach events as markers | brush-zoom, select interval → stats | V5 |
| **P06** | Time-in-bin histogram | `histogram()` | bars per bin, zone-ramp fill, "typical" as outline step line | bin width picker | V3 Nerd, V5, V6 |
| **P07** | 2D heatmap | `heatmap(x,y)` | sequential cell fill = seconds; zone gridlines | cell hover; x/y picker (power×HR, power×cadence) | V3 Nerd, V5 |
| **P08** | **Quadrant analysis** | AEPF × CPV (needs crank length) | density cells, quadrant lines at FTP-force / median CPV, % per quadrant labels | toggle points/density | V5, V6 |
| **P09** | **Power-duration curve** | `mmp()` + PD model fit | log x (1 s–60+ min); today / 90 d / all-time lines; **OmPD fit** dashed; CP as horizontal ref line; W' annotated | hover → duration/power/% of best; model toggle (CP2 / CP3 / OmPD) | V3 Nerd, V5, V6 |
| **P10** | **Durability curves** | PD curves after X kJ (0/1000/2000/3000) | ordered sequential ramp (fresh light → fatigued dark), log x; or small multiples | kJ thresholds picker; normalise to fresh | V5, V6 |
| **P11** | W'bal / stamina | W'bal trace + now | gauge (now) + trace panel (stacked under power); floor line | — | V3, V5 |
| **P12** | **HR recovery fit** | `hrRecovery()` | observed dots, fitted curve, dotted prediction + band, target line, readyIn text, τ vs usual | — | V3 Recovery, V5 (per block) |
| **P13** | Workout profile | `compileWorkout()` blocks | blocks: width = time, height = power, zone ramp fill; ranges hatched (texture) for open-ended blocks; AI-changed outline; "you are here" | editor: drag edges; strip: click → jump | V3, V4, V1, V7 |
| **P14** | DFA-α1 trace | `dfa_a1` | line with reference bands at 0.75 (≈LT1) and 0.5 (≈LT2), labelled | — | V3 Nerd, V5 |
| **P15** | **Cadence economy** | HR/W (cardiac cost) by cadence bin at matched power, across rides | dot + CI per cadence bin, one panel per power band (small multiples) | power band picker | V6 |
| **P16** | Fatigue-coloured scatter | power×HR (or torque×cadence) points | point colour = kJ spent (sequential), density fallback when > 5k points | time-window brush | V5 |
| **P17** | Torque & cadence over time | torque, cadence | two stacked panels, shared x; ERG periods shaded | — | V5 |
| **P18** | Trend small multiples | daily/weekly series: CTL, ATL, TSB, CP, W', durability score, τ, HRV, resting HR | one panel per metric, shared date x; TSB in its own panel (diverging around 0) | range: 6 w / 3 m / 1 y; brush | V6 |
| **P19** | Calendar heatmap | load (TSS) per day | sequential cells, week rows | click day → V5 | V6, V7 |
| **P20** | Coach decision timeline | `coach.decisions()` | event markers on ride time (icon + band label), accepted/declined state | click → detail + rate | V5 |
| **P21** | Fueling tracker | carbs taken vs target g/h | cumulative step line vs target line; next reminder countdown | big "+30 g" button | V3 Fuel page |
| **P22** | Intensity distribution | zone time per week | 100 % stacked bars per week (zone ramp, 2 px gaps), polarized/pyramidal reference markers | — | V6, V7 |
| **P23** | Readiness panel | check-in + wellness (later: intervals.icu) | small labelled tiles (sleep, HRV vs baseline, RHR, legs) with status icons | — | V1, V2 |
| **P24** | Diff overlay | old vs new workout | old profile ghosted, new solid, changed blocks outlined | accept / reject | V4, V1 |

---

## 2. Views

| ID | View | Purpose | Main panels |
|---|---|---|---|
| **V1** | Today | landing: readiness, today's proposal, week context | P23, proposal card with P13, week P19 mini, "Start" |
| **V2** | Check-in | ≤ 10 s subjective input → proposal | 4×(1–5) rows, minutes chips, note, P23 |
| **V3** | Ride | the core | pages: **Interval**, **Recovery**, **Nerd**, **Fuel**, plus user-defined |
| **V4** | Editor | build / edit workouts | P13 profile, outline tree, DSL text, summary (P06-style zone bars), P24 diff |
| **V5** | Ride review | one ride in depth | summary tiles, P05, P20, P09, P10, P08, P07, P16, P17, P12 per block, laps table |
| **V6** | Progress / analytics | multi-week | P18, P09 (all-time with model), P10 trend, P15, P22, P19 |
| **V7** | Plan | week plan from planner | calendar with workouts (P13 minis), P22 planned vs done, "replan week" |
| **V8** | Coach | settings + log | autonomy level (auto-apply strong / ask always / off), backend choice, decision log with ratings |
| **V9** | Layout editor | edit V3 pages | grid, widget palette, per-widget settings |
| **V10** | Settings | profile & devices | FTP/CP/W' (auto vs manual), weight, max/rest HR, crank length, zones model, devices, units, theme, connectors (later) |

Navigation: left rail on laptop (Today · Plan · Editor · Progress · Coach · Settings); **Ride is
full-screen modal** (no rail), exits to Review. Tablet: bottom tab bar. TV: ride only.

---

## 3. Wireframes

### V1 Today
```
┌ rail ┬───────────────────────────────────────────────────────────────┐
│Today │ READINESS  sleep ●●●○○  HRV −8% ⚠  RHR 52  legs ●●○○○         │
│Plan  ├───────────────────────────────────────────────────────────────┤
│Edit  │ TODAY'S PROPOSAL  "Z2 + 3×8' sweetspot"  62' · TSS 71          │
│Prog  │ ▁▂▃▄ ▆▆▆▆ ▂ ▆▆▆▆ ▂ ▆▆▆▆ ▂▂▂▂  (P13)                             │
│Coach │ why: HRV below baseline → dropped VO2 block, kept SS stimulus   │
│Set   │ [Start ride]  [Regenerate]  [Edit]  [Check-in again]           │
│      ├───────────────────────────────────────────────────────────────┤
│      │ THIS WEEK  ▢▣▢▣▣▢▢ (P19 mini)   load 312/420   TSB −12          │
└──────┴───────────────────────────────────────────────────────────────┘
```

### V3 Ride — page "Interval" (revised: stacked panels, no dual axis)
```
┌─ 23:41 / 62:00 ── ERG ── NEO ● HR ● ── [−5%][+5%] [skip] [⏸] ───────────┐
│  POWER 3s            TARGET     │  INTERVAL 2:47   block 3/8           │
│     312 W            300 W  +4% │  next: 4' Z2 @ 180 W                 │
│  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░ (P03)          │                                      │
├────────────┬──────────────┬─────┴──────────┬───────────────────────────┤
│ HR 164 Z4  │ CADENCE 91   │ W'bal 62% (P11) │ DFA-α1 0.62 (P02)         │
│ ▁▂▃▅▆▇     │ target 90–95 │ ████████░░      │ ▇▆▅▄▃                     │
├────────────┴──────────────┴────────────────┴───────────────────────────┤
│ P04  W   ─╮╭─╮  mean + min/max envelope, target dashed                  │
│      bpm ─────╯                                                        │
│      rpm ─────                                                         │
│          2h        10m         1m        10s   now  (log time-ago)      │
├────────────────────────────────────────────────────────────────────────┤
│ P13 strip ▁▁▃▃█▁█▁█▁█▁▃▁▁  ◆ you are here                               │
├────────────────────────────────────────────────────────────────────────┤
│ ● COACH (lean) HR 9 bpm above usual for 300 W → −5% next rep?  [Y] [N]  │
└────────────────────────────────────────────────────────────────────────┘
```

### V3 Ride — page "Recovery"
```
┌─ top bar ───────────────────────────────────────────────────────────────┐
│ P12  HR ╲                                                               │
│          ╲__      target 120 ─ ─ ─ ─ ─ ─ ─                              │
│             ╲___ ········▒▒▒▒░░░  prediction + band                     │
│          0    30s    60s    90s    120s                                 │
│  READY IN ≈ 1:35 (1:20–1:55)     τ 48 s (usual 41 s) ⚠ slower           │
├──────────────────┬──────────────────┬───────────────────────────────────┤
│ HR 138           │ POWER 120 W      │ NEXT: 4×30/15 @ 120% (P13 mini)    │
└──────────────────┴──────────────────┴───────────────────────────────────┘
```

### V3 Ride — page "Nerd"
```
┌───────────────────────────┬───────────────────────────┬────────────────┐
│ P06 power time-in-bin     │ P07 power × HR heatmap    │ P09 PD curve   │
│ ▁▂▅█▇▃▂▁  (typical ─)     │ ░▒▓█ cloud drifts up →    │ log x, OmPD ┄  │
├───────────────────────────┼───────────────────────────┤ today vs best  │
│ P06 HR time-in-bin        │ P07 power × cadence       │                │
├───────────────────────────┴───────────────────────────┴────────────────┤
│ P14 DFA-α1 trace  ── 0.75 ── 0.5 bands                                  │
└────────────────────────────────────────────────────────────────────────┘
```

### V3 Ride — page "Fuel"
```
│ CARBS 64 g / target 90 g/h   next in 4:10   [ +30 g gel ] [ +20 g drink ] │
│ P21 cumulative ▁▁▂▂▃▃▄  vs target ┄┄┄                                     │
│ fluids 0.6 l  [ +250 ml ]                                                 │
```

### V5 Ride review
```
┌ summary: 62:14 · 812 kJ · NP 247 · IF .88 · TSS 79 · decoupling 4.1% · τ̄ 44 s ┐
├──────────────────────────────────────────────────────────────────────────────┤
│ P05 stacked: power / HR / cadence / DFA-α1 / W'bal, workout bands, coach ◆    │
├──────────────────────────────────────────────────────────────────────────────┤
│ P20 coach decisions: 18:20 ◆ ease_off strong (auto) 👍👎 · 31:05 ◆ push lean ✓ │
├────────────────────┬────────────────────┬────────────────────────────────────┤
│ P09 PD + OmPD      │ P10 durability     │ P08 quadrants                      │
├────────────────────┼────────────────────┼────────────────────────────────────┤
│ P16 power×HR by kJ │ P17 torque/cadence │ P12 τ per recovery block (dots)    │
├────────────────────┴────────────────────┴────────────────────────────────────┤
│ laps table (block, target, avg P, avg HR, cadence, compliance, τ)  [table]   │
└──────────────────────────────────────────────────────────────────────────────┘
```

### V6 Progress
```
┌ range: [6w] 3m 1y ─────────────────────────────────────────────────────────┐
│ P18 small multiples (shared date x):                                        │
│   CTL/ATL │ TSB (diverging) │ CP & W' │ durability score │ τ │ HRV/RHR       │
├──────────────────────────────┬──────────────────────────────────────────────┤
│ P09 all-time PD + OmPD       │ P15 cadence economy (by power band)          │
├──────────────────────────────┼──────────────────────────────────────────────┤
│ P22 intensity distribution   │ P19 calendar heatmap                         │
└──────────────────────────────┴──────────────────────────────────────────────┘
```

V4 Editor: see UX-SPEC §3 (profile / outline / DSL / summary, plus the P24 diff overlay).

---

## 4. Responsive rules

- **Laptop (≥1280)**: as wireframed.
- **Tablet (1024)**: Ride keeps the top 2 rows + P04 + coach card; the strip moves into the top bar as a mini;
  analytics grids collapse to 2 columns.
- **TV (1920, 3 m distance)**: ride only, big numbers ≥ 120 px, no hover affordances, max 6 widgets/page.
- **Phone**: remote + check-in only (start/pause/±/Y/N/fuel buttons), no charts.

## 5. Scope for the framework spikes

Spikes build **V2, V3 (Interval, Recovery, Nerd), V4, V5, V9**, with plot types P01–P07, P09 (MMP lines,
no model fit yet), P11–P14, P16, P20, P24. **Not in the spike** because backend v0 has no data for them yet:
P08 quadrants, P10 durability, the OmPD/CP fit in P09, P21 and the Fuel page.
**V1, V6, V7, V8, V10** and P15/P17/P18/P19/P22/P23 are out of spike scope (later). The extra
metrics they need (AEPF/CPV/torque, OmPD fit, durability, stamina, carbs, trends) get added to
the contract after the mock-backend v0 lands.
