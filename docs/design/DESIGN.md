# crank — first-principles UX design

Status: **proposed** (#24), round 1. It drives the M1 UI issues once the astra and opus reviews accept it.
Prototype: `design/prototype/` (`pnpm -C design/prototype dev`). Screenshots: `docs/design/screens/`
(`pnpm -C design/prototype screens`). Where this document conflicts with `spec/UX-SPEC.md` or
`spec/VIEWS.md`, this document wins; §9 lists every such place.

---

## 1. Who rides, and what they are trying to do

One rider, indoors, on a Tacx NEO in ERG (the trainer sets the power) or SIM/free (the rider sets it),
with an HR strap and a laptop or tablet on a stand about 1 m away. The rider wants to get fitter
without wasting sessions, and wants to trust that the plan fits how they feel today.

| Moment | The rider's job | What they can give | What they need back |
|---|---|---|---|
| **First run** | "Get me riding without a test I'm not ready for." | 3 min of answers, device pairing | A starting model with honest uncertainty, a first ride today |
| **Before** (at the bike, kit on) | "What do I ride today, and is it right for me?" | 4 taps on how they feel, time available | One proposal with a reason they can check; ride now |
| **Starting** | "Is everything connected?" | nothing | A go/no-go on trainer + HR, then the ride |
| **During, working** | "Am I doing it right? How long is left? What's next?" | almost nothing: a glance, one key | Countdown, target vs actual, the next change |
| **During, recovering** | "When can I go again?" | a glance | Time until ready, the next effort |
| **During, coach speaks** | "Should I take this advice?" | one key: yes / no | The verb, the amount, one reason, and that it can be undone |
| **During, something breaks** | "Is my ride ruined?" | nothing | What broke, what the app is doing about it, what still counts |
| **After** | "Did it work? Was the coach right?" | an RPE tap, ratings of coach calls | What the session achieved vs its intent, the evidence |
| **Long term** | "Am I improving? Can I trust these numbers?" | rides | CP / W′ and friends **with uncertainty and source**, a ride history |
| **Building** | "Make me a workout / tweak this one." | text, drags, AI requests | Three synced views, safe AI edits |

### Constraints that drive everything else
1. **Attention collapses under effort.** Above threshold the rider reads one thing per glance, maybe two.
   Anything that needs comparison of two numbers ("target 350, actual 341") costs a glance too many.
2. **Distance and sweat.** ~1 m, eyes moving, sweat in them. Legibility is a visual-angle budget (§6.2),
   not a font-size preference.
3. **Hands on the bars.** No typing, no pointing mid-ride. Input = single keys (keyboard or a bar
   remote that sends keys), each with a large on-screen twin for mouse/touch.
4. **ERG changes what matters.** In ERG the trainer holds the power; the rider's actionable variables are
   *cadence* (stay above the spiral of death), *breathing/HR* and *whether to accept a change*. Actual power is
   a compliance check, not the thing to steer. In SIM/free (sprints, open efforts) power is the steering
   variable again.
5. **The coach is probabilistic.** It speaks with a band (strong / lean / uncertain). Trust comes from
   seeing *what* it changed, *why*, and from being able to undo it — not from hiding it.
6. **Failure is normal.** BLE drops (the NEO allows one central), HR straps lose contact, a laptop sleeps.
   A design that only works on the happy path fails the rider at the worst moment.
7. **Estimates are uncertain at the start** (spec/ONBOARDING.md): every athlete number has a ± and a source.

### Principles (each one is referenced below as P1…P8)
- **P1 One question per moment.** Each screen and ride phase answers the rider's question for that moment
  first and biggest; everything else is visibly secondary.
- **P2 Encode, don't make them subtract.** Target vs actual is a position on a band, Δ is a signed chip;
  "how long" is a countdown, never two times to subtract.
- **P3 The screen follows the ride, not the other way round.** The ride view changes its emphasis by
  phase (warm-up ramp, steady work, short on/off, sprint, HR recovery); the rider does not page or swipe.
- **P4 Nothing moves but values.** Fixed geometry, tabular numerals, reserved space for every optional
  element (coach, warnings). A state change recolours or fills a reserved slot; it never re-flows the layout.
- **P5 The coach proposes in the open.** Verb + amount + one reason, a visible deadline, and the change
  visible on the road ahead; strong calls auto-apply only with an undo window; silence keeps the plan.
- **P6 Say what broke and what still counts.** Lost data is shown as lost (never a stale number posing as
  live), with the policy the app applies.
- **P7 Honest numbers.** Estimates show ± and source; a "calibrating" state is a first-class look, not a
  footnote.
- **P8 Every view is derived data.** The UI subscribes and displays; physiology, laps, thresholds and bands
  come from the backend (#2). Per-ride state lives in one scope recreated on load (#3).

---

## 2. Conventions of existing trainer apps, challenged

| Convention (where) | Verdict | Reason |
|---|---|---|
| Virtual world / avatar (Zwift, Rouvy) | **Drop** | Spends the screen and the rider's attention on scenery; crank's value is the plan and the coach. Riders who want video run it elsewhere. |
| Equal tiles for power / HR / cadence / speed (most apps) | **Change** | Equal weight hides what matters now (P1). One hero per phase, the rest secondary. Speed is meaningless indoors in ERG and is dropped from the ride screen. |
| Target and actual as two numbers (TrainerRoad, SYSTM) | **Change** | Mental subtraction at threshold (constraint 1). Actual becomes a marker on a target band; Δ is a signed chip in W (P2). |
| Actual power as the biggest number in ERG | **Change** | In ERG it is the trainer's job; the rider steers cadence. The hero in ERG is *time left in the block* with the target beside it; actual power is the band marker. In free/sprint the actual power becomes the hero (P3, constraint 4). |
| Interval countdown | **Keep, promote** | The single most asked question while working. It is the largest element in every work phase. |
| Workout profile strip at the bottom (TrainerRoad) | **Keep, change** | The road ahead is essential, but a full-ride strip at ride scale makes the next 2 minutes 3 px wide. crank shows a **two-scale road**: the next ~10 min magnified, plus a thin whole-ride overview. |
| Paged ride screens / swiping (Garmin, Wahoo, our spikes) | **Drop mid-ride** | Paging is a manual answer to "this phase needs other data". The screen adapts by phase instead (P3). Analytics pages (histograms, heatmaps) move to Review where they can be read. |
| Free-form widget grid editor (spec V9, our spikes) | **Drop for v1** | It produced the most state bugs (#8, #11) for the least rider value. Replaced by phase templates with a few configurable **slots** (§5.3) edited as a validated draft in Settings. |
| Rainbow zone colours (TrainingPeaks, Zwift) | **Keep as opt-in only** | Already decided in THEMES: sequential ramp by default, rainbow behind a toggle, never the only carrier (zone labels are text). |
| Mandatory FTP/ramp test before training (Zwift, TrainerRoad) | **Change** | Testing tired riders on day 1 is bad advice and bad onboarding. Start with priors ± uncertainty, ride today, refine from rides or an optional assessment week (spec/ONBOARDING.md, P7). |
| A calendar as the home screen (TrainerRoad, TrainingPeaks) | **Change** | The home question is "what do I ride now?". Home = **Today**: check-in and one proposal. The week is context under it. |
| "Regenerate" for a new random plan | **Change** | Random re-rolls teach nothing. Directed alternatives: **Shorter · Easier · Harder · Other type**, each with a reason (planner variants, #15). |
| ERG intensity ± (TrainerRoad, Zwift) | **Keep** | A real need (bad day, good day). Fixed-width, shows the cumulative offset, keys `−`/`+`. |
| Coach pop-ups / toasts (SYSTM tips, Zwift messages) | **Change** | Toasts cover data and vanish. The coach has a reserved **lane** on the ride screen (P4, P5) and a decision log in Review. |
| "Workout complete" fanfare (most apps) | **Change** | Replaced by an automatic hand-over to Review that leads with what the session achieved and asks for the 2 inputs that make the coach better (RPE, ratings) (#6). |
| Social, kudos, leaderboards | **Drop** | Not a job any of the moments above asks for. |
| TSS/IF as the headline of a proposal | **Change** | Shown, but the headline is duration + what the session is for + why today. Load numbers are secondary. |
| Pause button | **Keep** | Space; fixed position and width; pause dims the hero, it doesn't remove it (P4). |
| Lap / skip | **Keep** | `S` skips the block; open-ended `lap` blocks end on `S`. |

---

## 3. Information architecture

Four places plus one mode. The coach is not a place: it acts in Today (proposal), Ride (lane), Review
(decision log) and is configured in Settings. This replaces VIEWS' six-item rail (§9).

```
 ┌────────── rail ──────────┐
 │ Today    (home)          │──Start──▶ Ready check ──Space──▶ RIDE (full screen, no rail)
 │ Workouts (library+editor)│                                        │ state: finished (auto, #6)
 │ History  (rides + model) │◀─────────────── Review ◀───────────────┘
 │ Settings (devices, model,│
 │           coach, screen,  │
 │           appearance)     │
 └───────────────────────────┘
 First run: Onboarding (6 steps, resumable) → Today
```

- **Ride is a mode, not a page.** No rail, no navigation; exits only via finish (→ Review) or End ride
  (confirm, → Review of the partial ride).
- **Review** is reachable from History for any past ride (needs the ride library, #29); it auto-opens on `state: finished`.
- Rail at ≥ 1024 px wide (both target sizes). Below that the app is out of scope for v1.

---

## 4. Flows

1. **Daily ride (the 95 % path):** Today → (check-in already pre-filled from yesterday, change what differs:
   ≤ 4 taps) → proposal updates → **Start** → Ready check (both sensors green → Space) → Ride → auto Review →
   RPE + rate calls → History.
2. **First run:** Onboarding 1–6 → "Your starting model" (CP 255 ± 35 W, prior) → Today with a calibrating
   proposal (conservative targets, P7).
3. **Build/edit:** Workouts → pick or new → edit in profile / outline / text → AI on selection → review diff
   → accept (verified, #1) → Ride this.

---

## 5. Views

All views share: the left rail (except Ride), a 8 px spacing grid, tokens from `spec/THEMES.md`, tabular
numerals for every number, and keyboard access (`?` lists keys).

### 5.1 Today (home) — *"What do I ride today, and is it right?"*
- **How are you?** four rows (sleep, legs, stress, motivation), five tap targets each (≥ 48 px), plus
  time-available chips (30 / 45 / 60 / 90 / custom). Pre-filled with the last answer and marked as such, so
  the usual day is 0–2 taps. Changing any answer re-asks the planner; the proposal shows a pending state in
  place (no layout shift).
- **Proposal card** (the hero): name, duration, what it is for, a profile (P13) with zones, the planner's
  *why* in one or two sentences, and the confidence of the athlete model it used. Actions: **Start** (primary,
  Enter), **Shorter / Easier / Harder / Other** (directed variants, #15), **Edit** (opens the editor on a copy).
- **Model card**: "CP 265 ± 20 W · from 4 rides · calibrating" until confidence is medium (spec/ONBOARDING §4).
- **Last 7 days**: seven day cells (done / rest / today, TSS) — context, not navigation.

### 5.2 Ride — *"Am I doing it right, how long, what next?"*

Fixed anatomy at every size (geometry does not change between phases or states; only contents do, P4):

```
┌ status bar: ⏸ │ 23:41 ride · 21:19 left │ block 7/22 │ ERG ±0% [−][+] │ NEO ● │ HR ● │ ? ┐  fixed widths
├──────────────────────── hero (≈ 62 % width) ───────────────┬──── side (≈ 38 %) ─────────────┤
│ PHASE LABEL · block name · rep 3/5                         │ HR  164  bpm          zone 4    │
│                                                            │                                │
│                 0:23            ← T1 countdown              │ CADENCE 91  rpm  target 90–95  │
│                                                            │                                │
│ TARGET 350 W   ▕━━━━━━━━━━▓▓▓▓|▓▓▓━━━━━▏  +6 W   ← band     ├──── coach lane (reserved) ─────┤
│ next: OFF 0:15 @ 140 W                                     │ coach is watching · or a call  │
├────────────────────────────────────────────────────────────┴────────────────────────────────┤
│ road ahead: next ~10 min magnified, "now" line at 1/5, coach changes outlined               │
│ whole ride overview (thin) with you-are-here                                                │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Phase templates** (what fills the hero; derived from the current block, P3):

| Phase / block | Hero (T1) | Hero line 2 (T2) | Why |
|---|---|---|---|
| **Warm-up / ERG ramp** | block countdown | target *now* + "↗ to 210 W" | The target is moving; the rider needs to know it will keep rising and to where. |
| **Steady work (ERG)** | block countdown | target + band with actual marker + Δ chip | In ERG the trainer holds power; the band is a compliance check, cadence (side) is the lever. |
| **Short on/off (≤ 60 s reps)** | countdown | target + band; ON / OFF word + rep pips ●●●○○ in the header | Reps are counted, not read; pips are glanceable. |
| **Sprint (free / non-ERG)** | **actual power** | target (if any) + countdown | The rider steers power; the time is short and secondary. |
| **HR-guided recovery** (`hr<`) | **ready in ≈ 1:35** (lo–hi) | HR target + recovery curve with prediction band | The question is "when can I go?", not "how long is this block". |
| **Cool-down** | block countdown | target + band; "next: ride complete" | Signals the end. |

The **side** shows the rider's two configured slots (default HR and cadence, with zone / cadence target) in
**every** phase: the eye learns where HR is, and nothing in the hero duplicates a side value (P4).

**States** (overlay a template without moving it):

| State | What changes | Keys |
|---|---|---|
| Coach call, lean/uncertain (`pending`) | Lane fills: verb + amount ("EASE OFF −3 %"), one-line reason, band word, a draining deadline bar; the affected blocks are outlined on the road. Timeout = **keep plan**. | `Y` accept · `N` keep |
| Coach call, strong (`auto_applied`) | Lane: "Eased −5 % · HR 9 bpm above usual" + undo deadline (15 s); road shows changed blocks outlined | `U` undo |
| Trainer lost / reconnecting | Status chip NEO turns *warning* with icon + "reconnecting 3 s"; band marker and actual power become "—" with "no power data"; hero keeps countdown; a one-line policy: "plan continues; ERG resumes on reconnect" (§6.5) | — |
| Trainer reconnected | Chip shows "NEO back · 4 s gap" for 10 s, then normal | — |
| HR lost | HR value → "—" + "HR strap lost"; coach lane notes "HR-based advice paused" | — |
| Paused | Hero dims, a "PAUSED · Space to resume" label sits in the reserved band row; the status-bar pause button keeps its width (⏸ ↔ ▶) | `Space` |
| Finished | Auto-navigate to Review (#6) | — |

Keys during the ride: `Space` pause/resume · `+`/`−` intensity ±5 % · `S` skip block · `Y`/`N` answer coach ·
`U` undo auto-applied call · `?` keys · `Esc` end ride (confirm).

### 5.3 Ride-screen configuration (replaces the layout editor, §9)
The rider can choose what the **two side slots** show (the same in every phase, §5.2) (from the metric catalogue:
HR, cadence, W′bal, DFA-α1, HR vs typical, power 3 s / 10 s, kJ, …) and a **density** (Focus: hero + road only /
Standard / Detail: adds a third side slot). It is a form in Settings, edited as a **draft** with Save / Cancel /
Reset to default, validated per slot on load and on save (#4, #8); every control reads the draft, never a
prop-seeded copy (#11). Geometry is fixed, so a configuration can never produce an unusable layout at 1024 (#7).

### 5.4 Ready check (pre-ride)
A sheet over Today: workout name + duration, **NEO** (FE-C, connected / scanning / not found → retry, help),
**HR** (connected / not found → "ride without HR" with the consequence: HR-based coaching off), mode (ERG),
"starts with 8:00 warm-up". **Space / Start** enabled when the trainer is connected; HR is optional but called out.

### 5.5 Review — *"Did it work, and was the coach right?"*
Order follows the rider's questions:
1. **Outcome headline** from the backend: duration, work completed vs plan, compliance, what changed (coach
   calls applied) — one sentence plus tiles (time, kJ, NP, TSS, IF, compliance, decoupling).
2. **Two inputs** (each ≤ 1 tap): **RPE 1–10** and **rate each coach call** (right / wrong / unsure) — they
   train the athlete model and the coach (#29, `coach.rate`).
3. **Evidence**: stacked linear-time panels sharing x (power + target, HR, cadence; P05), workout bands behind,
   coach events as markers; HR recoveries (τ per block vs usual, P12); work intervals table (B1, #12).
4. **Deep analytics** (moved here from the ride's Nerd page): time at power, PD curve; the remaining V5 plots (power × HR heatmap, quadrants, durability, …) follow VIEWS when their data exists (#18).

### 5.6 History — *"Am I improving, can I trust the numbers?"*
- **Athlete model** card: CP, W′, HRmax, HR-recovery τ, each value ± sd, source, n, last update, and a
  "calibrating" badge (P7, spec/ONBOARDING §3).
- **PD curve**: today / 90 days / all time (backend `mmp()`).
- **Rides**: list (date, name, duration, TSS, RPE, mini profile) → Review of that ride.

### 5.7 Workouts (library + editor)
Library list on the left; the editor keeps the spike's strongest idea — **three synchronised views**:
profile (P13), outline (tree), text (DSL, `shared/DSL.md`) — plus the compiled summary (duration range, TSS,
IF, zone time, issues). **AI on selection** (vary / harder / easier / shorter same stimulus) shows a diff
(old ghosted, new solid, changed outlined, P24) and **locks all editing** with a banner until Accept / Reject;
Accept verifies that the target node ids and content hash are unchanged, else re-requests (#1). Undo/redo
spans all three views; the tree ops, undo/redo and DSL ↔ tree sync are unit-tested in production (#9).

### 5.8 Onboarding (first run)
Six resumable steps from spec/ONBOARDING §1: About you · Background · Current numbers (unknown is fine) ·
Health check (PAR-Q style; any "yes" → intensity cap until confirmed) · Devices (pair + test) · Baseline path
(import / assessment week / passive learning). It ends on **"Your starting model"**: the estimates with ±,
what will narrow them, and what the coach will do meanwhile (conservative targets). Every step: one question
group, big targets, **Skip** with its consequence stated.

### 5.9 Settings
Sections: **Devices** (trainer + HR, protocol, status, battery, pair / forget; one-central note), **Athlete
model** (estimates ± source, manual override with provenance "self-report"), **Coach** (autonomy: auto-apply
strong / always ask / off), **Ride screen** (§5.3), **Appearance** (mode dark / light / auto, preset, classic zones).

---

## 6. Ride screen specification

### 6.1 Grid
Status bar 56 px (48 px at ≤ 800 px height). Hero and side split 62 / 38. Road ahead 22 % of height (min 128 px).
All regions are fixed by CSS grid with `minmax(0, …)` and `overflow: clip`; contents are sized by the visual
budget below, never by their text.

### 6.2 Visual budget (legibility at ~1 m)
At 1 m, 1° of visual angle ≈ 17.5 mm. A 13–14″ laptop at 1440×900 pt shows ≈ 4.4 pt/mm; an iPad-class
1024×768 ≈ 5 pt/mm. Digit cap height ≈ 0.72 em.

| Tier | Use | Cap-height target | Size rule | At 900 / 768 px tall |
|---|---|---|---|---|
| **T1 glance** | the hero (countdown, ready-in, sprint power) | ≥ 2° | `clamp(120px, 24vh, 240px)` | 216 / 184 px |
| **T2 glance** | target, HR, cadence | ≥ 1° | `clamp(64px, 10vh, 112px)` | 90 / 77 px |
| **T3 read** | labels, next, coach reason | ≥ 0.35° | `clamp(18px, 2.6vh, 26px)` | 23 / 20 px |
| T4 chrome | key hints, axis ticks, band word, deadline captions — never needed mid-effort | | 14–16 px | |
| — | nothing the rider must read mid-effort is smaller than T3 | | 18 px floor | |

Numbers: tabular, lining figures; `font-variant-numeric: tabular-nums`; durations always `m:ss`.

### 6.3 Colour
Tokens and presets from `spec/THEMES.md` (Ocean by default; dark is designed, not inverted). On the ride
screen colour carries **state**, not decoration:
- Target band: neutral; actual marker: ink; out-of-band ends: diverging pair (`--div-neg` below, `--div-pos`
  above). Δ chip uses the same pair with a sign.
- Zones appear as text labels and the sequential ramp on the road ahead, never as the hero's colour.
- Status colours only for sensors and the coach, always with icon + label.
Palette validation is tracked in #30.

### 6.4 Motion
Values update at the backend subscription rate (hero 4 Hz, side 2 Hz, road 1 Hz). No animated transitions of
layout. The coach deadline and undo windows drain linearly.

### 6.5 Failure policy (what the UI shows, the core decides)
| Event | Policy shown | Data |
|---|---|---|
| Trainer blip < 10 s | "plan continues; ERG resumes on reconnect" | power shown as "—", gap kept as no-data in the record |
| Trainer lost ≥ 10 s | "plan held at 4:12 of block; reconnecting…" (block clock holds) | |
| HR lost | "HR-based advice paused" | HR "—" |
| Both lost | full-width banner in the reserved band row, ride auto-pauses | |
Thresholds and the held/continued state come from the core (`RideStatus`, #29), not the UI.

---

## 7. Coach interaction model
- **In-ride (System 1)**: verdicts `ease_off · hold · push · replan`, each a single verb in the lane. Bands
  decide autonomy: **strong** auto-applies with a 15 s undo (`U`) when autonomy = auto-strong; **lean /
  uncertain** ask with a visible deadline (30 / 20 s) and default to **keep plan** on silence. With autonomy
  "always ask", strong calls ask too; "off" hides the lane's calls but still logs them.
- **The change is visible**: affected blocks are outlined on the road ahead; the target updates in place.
- **Before (planner)**: one proposal + why + directed variants; never an unexplained re-roll.
- **After**: every call is listed in Review with time, verdict, band, outcome, and a one-tap rating.
- **Confidence**: the coach states what it is unsure about ("calibrating: HR-vs-typical unlocks after 20 min at this power").

---

## 8. Window sizes
| Size | Target | Rules |
|---|---|---|
| **1440×900** | laptop/desktop landscape (primary) | as specified |
| **1024×768** | tablet landscape | same anatomy (no reflow by breakpoint); type from the vh-based tiers; status bar drops text labels for icon + value, keeps every control; editor stacks outline above text |
| resize | any between | layout follows the window continuously (CSS, no device setting) (#7) |
| TV / phone remote | out of v1 scope | VIEWS §4 rules stay for later |

---

## 9. Where this supersedes `spec/UX-SPEC.md` / `spec/VIEWS.md`
| Spec | Was | Now |
|---|---|---|
| UX-SPEC §2 widget grid, pages, auto-page | user-paged widget grid with an edit mode | phase templates + fixed anatomy (§5.2), no paging mid-ride |
| UX-SPEC §2 edit mode; VIEWS V9 | free-form drag/resize grid editor | slot configuration in Settings as a validated draft (§5.3) |
| UX-SPEC §2 default layout pages Interval / Recovery / Nerd; VIEWS V3 pages incl. Fuel | four pages | Interval → work templates; Recovery → HR-recovery template; Nerd → Review §5.5(4); Fuel → deferred (P21, B7 #18) |
| UX-SPEC §2 top bar speed (1×/10×/60×) | rider-facing | dev/prototype only |
| VIEWS §2 rail Today · Plan · Editor · Progress · Coach · Settings | six places | Today · Workouts · History · Settings; Plan folded into Today's week; Coach split into lane / Review / Settings (§3) |
| VIEWS V1/V2 Today and Check-in separate | two views | one view (check-in inline, pre-filled) |
| VIEWS P01 big number "tap → cycle smoothing" | per-tile interaction | smoothing per slot in Settings; no mid-ride pointing |
| VIEWS §4 tablet: strip moves into top bar | different layout at 1024 | same anatomy, scaled (§8) |
| UX-SPEC §1 "Regenerate" | random re-roll | directed variants (§5.1, #15) |
| UX-SPEC §4 Review "minimal" | numbers + chart + decisions | outcome first, two inputs, evidence, analytics (§5.5) |

Everything else in UX-SPEC / VIEWS (plot catalogue P01–P24, visual conventions, editor capabilities, the
non-functional rules) still applies.

---

## 10. The spike defects #1–#11, designed out
| # | Defect | Design element that prevents it |
|---|---|---|
| #1 | AI rewrite accept loses/misplaces edits | Editor **locks** during pending and diff (banner, all inputs disabled); Accept verifies node ids + content hash, else re-requests (§5.7). Prototype: `editor/model.ts` (`lock`, hash check in `accept`). |
| #2 | Physiology in the UI | P8: every derived number comes from the backend; missing ones are contract additions (#12, #13, #29) stood in by **one** fixture module (`mock/ext.ts`), never computed in a view. |
| #3 | Per-ride state not reset | One `RideSession` object (status, nudges, compiled, undo windows, sensor gaps) created on `ride.load` and disposed when the next ride loads (§5.2). Prototype: `session.ts`. |
| #4 | Unvalidated localStorage | Everything persisted (ride-screen config, theme, onboarding progress) is parsed by a schema on load; invalid → default + notice; a top-level error boundary shows a recoverable error, not a white screen. Prototype: `persist.ts`, `ErrorBoundary` in `App.tsx`. |
| #5 | structuredClone on reactive proxies | React state holds plain immutable values; the only snapshot path is `snapshot()` (production lint rule). Prototype: no clone of state anywhere. |
| #6 | Finished ride doesn't reach Review | `state: finished` → Review automatically, owned by the ride session (§3, §5.2). |
| #7 | 1024×768 unusable, layout ignores resize | Fixed anatomy scaled by vh tiers (§6.2, §8); one layout that follows the window; screenshots at 1024×768 are part of the harness. |
| #8 | Layout-editor state bugs | Free grid editor dropped; slot config is a transactional draft with per-slot validation (§5.3). |
| #9 | No unit tests for editor ops | Editor model (tree ops, undo/redo, DSL ↔ tree) is a pure module with tests in production (§5.7); the prototype keeps that module boundary (`editor/model.ts`). |
| #10 | Ride-screen layout shift | P4: fixed grid, fixed-width controls (pause, chips, intensity), tabular numerals, reserved coach lane and band row; states recolour/fill, never re-flow (§5.2, §6.1). |
| #11 | Widget settings seeded from props once | Settings forms read the draft directly (no `useState(prop)` seeding); the ride screen reads the saved config live (§5.3). |

---

## 11. Contract additions this design needs
Per-lap stats (#12), explicit thresholds incl. target band (#13), live cursor (#14), planner variants (#15),
athlete model with uncertainty (#18), and #29: coach revert, sensor-loss policy in status, ride library,
post-ride RPE, devices, ride-screen config, coach autonomy, a non-ERG sprint in the mock.

## 12. Prototype and screenshots
- `design/prototype/`: Vite + React 19 + React Compiler + TypeScript strict, against `@crank/shared`. A design
  artifact; components may be reused by the production UI, the fixtures in `src/mock/ext.ts` may not.
- Interactive: `pnpm -C design/prototype dev` (real-time mock; `?speed=10` or `60` for a faster ride; `?first=1`
  starts in onboarding).
- Known prototype limits (all tracked): Easier / Harder / Other variants are disabled (#15); past rides in History
  cannot be opened (#29 ride library); the sprint scene runs in ERG because the mock cannot simulate a free-mode
  sprint (#29); per-interval stats, the athlete model, devices and the week strip are fixtures in `src/mock/ext.ts`
  (#12, #18, #29); onboarding and Settings forms are not persisted except the ride-screen config and theme.
- Scenes: `?scene=<id>&theme=dark|light` builds a seeded mock (seed 42) on a virtual clock, advances it to the
  scene's moment and freezes it. `pnpm -C design/prototype screens` renders every scene at 1440×900 and
  1024×768 in dark and light to `docs/design/screens/<scene>.<w>x<h>.<theme>.png` and fails on any page error
  or horizontal overflow.

## 13. Review guide
Reviewers (astra, opus) should judge, with the screenshots open:
1. **Glanceability** — in every `ride-*` shot, can you answer "how long, am I on target, what next" in one
   look from arm's length (view at ~25 % zoom)? Is T1/T2/T3 sizing met at 1024×768?
2. **Phase fit** — does each ride template put the right thing in the hero (§5.2 table)? Is anything shown
   that the rider cannot act on?
3. **Stability** — compare `ride-threshold`, `ride-paused`, `ride-coach-ask`, `ride-dropout`: does anything
   other than values and reserved slots change position?
4. **Coach trust** — is the call understandable in one glance (verb, amount, reason, deadline, undo)? Is the
   default on silence safe?
5. **Failure honesty** — do dropout/reconnect shots say what broke, what the app does, and avoid stale values?
6. **First principles** — challenge §2: any convention kept without a reason, or dropped that riders need?
7. **Honest numbers** — are estimates shown with ± and source wherever they are used (Today, History,
   onboarding result)?
8. **Defects** — does §10 really make #1–#11 impossible, or only less likely?
9. **Coverage** — every primary view and state in §5 has a screenshot; light and dark both read as designed.
10. **Supersessions** — are the §9 changes to UX-SPEC/VIEWS acceptable to the owner?

## 14. Open questions (not blocking the prototype)
- What does the NEO do on BLE loss in ERG (hold last target or go free)? Decides the §6.5 wording → #29 / #23 (owner hardware test).
- Bar remote hardware (which keys it can send) → decides whether `Y/N/U` need single-button alternatives (#29).
