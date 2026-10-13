# crank — first-principles UX design

Status: **proposed** (#24), round 3 (after two rounds of astra and opus reviews; the PR body maps every
finding to its resolution). It drives the M1 UI issues once the reviews accept it.
Prototype: `design/prototype/` (`pnpm -C design/prototype dev`). Screenshots: `docs/design/screens/`
(`pnpm -C design/prototype screens`). Where this document conflicts with `spec/UX-SPEC.md` or
`spec/VIEWS.md`, this document wins; §9 lists every such place. Engine behaviour (pauses, reconnects, ERG
control, persistence) is decided in ADR-0001 / ADR-0002 (PR #46); this document designs what the rider
sees and does, and states a **design requirement** wherever it needs something the ADRs do not yet give.

---

## 1. Who rides, and what they are trying to do

One rider, indoors, on a Tacx NEO in ERG (the trainer sets the power) or SIM/free (the rider sets it),
with an HR strap and a laptop (≈ 1 m away) or a tablet (≈ 0.8 m) on a stand. The rider wants to get fitter
without wasting sessions, and wants to trust that the plan fits how they feel today.

| Moment | The rider's job | What they can give | What they need back |
|---|---|---|---|
| **First run** | "Get me riding without a test I'm not ready for." | 3 min of answers, device pairing | A starting model with honest uncertainty, a first ride today |
| **Before** (at the bike, kit on) | "What do I ride today, and is it right for me?" | 4 taps (or one "same as yesterday"), time available | One proposal with reasons they can check; ride now |
| **Starting** | "Is everything connected?" | nothing | A go/no-go on trainer + HR, then the ride |
| **During, working** | "Am I doing it right? How long is left? What's next?" | almost nothing: a glance, one button | Countdown, target vs actual, cadence state, the next change |
| **During, recovering** | "When can I go again?" | a glance | A range for when HR gets there, and the block's limits |
| **During, coach speaks** | "Should I take this advice?" | one button, or nothing | A sound, the verb and amount, one short reason, the deadline and the default |
| **During, something breaks** | "Is my ride ruined?" | nothing, or keep pedalling | What broke, what the app is doing, what to do, what was saved |
| **After a crash or sleep** | "Can I carry on?" | one choice | Resume where I was, or save what I rode |
| **After** | "Did it work? Was the coach right?" | an RPE tap, a rating per coach call | Whether the session met its intent, then the evidence |
| **Long term** | "Am I improving? Can I trust these numbers?" | rides | CP / W′ and friends **with uncertainty, source and dates**, a ride history |
| **Building** | "Make me a workout / tweak this one." | text, drags, AI requests | Three synced views, AI edits explained before they are accepted |

### Constraints that drive everything else
1. **Attention collapses under effort.** Above threshold the rider reads one thing per glance, maybe two.
   Anything that needs comparison of two numbers ("target 350, actual 341") costs a glance too many, and a
   glance is the most expensive input there is — **sound** reaches the rider without one.
2. **Distance and sweat.** ~1 m, eyes moving, sweat in them. Legibility is a visual-angle budget (§6.2),
   computed from the physical display, not a font-size preference.
3. **Hands on the bars.** No typing, no pointing mid-ride. Input is five abstract actions on a bar remote or
   keys (§7.2), each with a large on-screen twin, and every in-ride question has a safe default for silence.
4. **ERG changes what matters.** In ERG the trainer holds the power; the rider's actionable variables are
   *cadence* (stay above the spiral, ADR-0002 §7), *breathing/HR* and *whether to accept a change*. Actual
   power is a compliance check. In SIM/free (sprints, open efforts) power is the steering variable again.
5. **The coach is probabilistic.** It speaks with a band (strong / lean / uncertain). Trust comes from
   seeing *what* it changed, *why*, from being able to undo it, and from it never raising the load by itself.
6. **Failure is normal.** BLE drops (the NEO allows one central), HR straps lose contact, the laptop sleeps,
   the app crashes. A design that only works on the happy path fails the rider at the worst moment.
7. **Estimates are uncertain at the start** (spec/ONBOARDING.md): every athlete number and every prediction
   has a range and a source.

### Principles (referenced below as P1…P9)
- **P1 One question per moment.** Each screen and ride phase answers the rider's question for that moment
  first and biggest; everything else is visibly secondary.
- **P2 Encode, don't make them subtract.** Target vs actual is a position (band or fill), the difference a
  signed chip with an arrow; "how long" is a countdown; "how far to go" is stated ("15 bpm to go").
- **P3 The screen follows the ride, not the other way round.** The ride view changes its emphasis by
  phase; the rider does not page or swipe.
- **P4 Nothing moves but values.** Fixed geometry, tabular numerals, the same rows reserved in every
  template, reserved space for every optional element (coach, warnings). A state change recolours or fills a
  reserved slot; it never re-flows the layout.
- **P5 The coach proposes in the open.** Verb + amount + one short reason, a sound, a visible numeric
  deadline, the default on silence, the change shown on the road ahead; only decreases may auto-apply, and
  only when the rider opted in, always with undo.
- **P6 Say what broke, what the app does, what to do, what was saved.** Lost data is shown as lost, never
  as a stale number or a bridged line.
- **P7 Honest numbers.** Estimates and predictions show their range and source; when the range is wide the
  range *is* the number; a "calibrating" state is a first-class look.
- **P8 Every view is derived data.** The UI subscribes and displays; physiology, laps, thresholds, bands and
  validity come from the backend (#2). Per-ride state lives in one scope recreated on load (#3).
- **P9 Every chart can be checked.** Axes with units, labelled marks, a legend, gaps visible, and a table
  alternative; colour is never the only carrier.

---

## 2. Conventions of existing trainer apps, challenged

| Convention (where) | Verdict | Reason |
|---|---|---|
| Virtual world / avatar (Zwift, Rouvy) | **Drop** | Spends the screen and the rider's attention on scenery; crank's value is the plan and the coach. Riders who want video run it elsewhere. |
| Equal tiles for power / HR / cadence / speed (most apps) | **Change** | Equal weight hides what matters now (P1). One hero per phase, the rest secondary. Speed is meaningless indoors in ERG and is dropped from the ride screen. |
| Target and actual as two numbers (TrainerRoad, SYSTM) | **Change** | Mental subtraction at threshold (constraint 1). Actual becomes a marker on a target band (ERG) or a fill against the target (sprint); Δ is a signed chip with ▲/▼ (P2). |
| Actual power as the biggest number in ERG | **Change** | In ERG it is the trainer's job; the rider steers cadence. The ERG hero is *time left in the block*; actual power is the band marker. In free/sprint the actual power becomes the hero (P3, constraint 4). |
| Interval countdown | **Keep, promote** | The single most asked question while working. It is the largest element in every timed phase. |
| Beeps at interval changes (Zwift, TrainerRoad) | **Keep, extend** | The cheapest channel that needs no glance. crank adds distinct tones for coach asks, applied changes and sensor loss (§7.3). |
| Workout profile strip at the bottom (TrainerRoad) | **Keep, change** | The road ahead is essential, but a full-ride strip makes the next 2 minutes 3 px wide. crank shows a **two-scale road**: the next ~10 min magnified, plus a thin whole-ride overview. |
| Paged ride screens / swiping (Garmin, Wahoo, our spikes) | **Drop mid-ride** | Paging is a manual answer to "this phase needs other data". The screen adapts by phase instead (P3). Analytics move to Review where they can be read. |
| Free-form widget grid editor (spec V9, our spikes) | **Drop for v1** | It produced the most state bugs (#8, #11) for the least rider value. Replaced by two configurable side **slots** (§5.3) edited as a validated draft in Settings. |
| Rainbow zone colours (TrainingPeaks, Zwift) | **Keep as opt-in only** | Already decided in THEMES: sequential ramp by default, rainbow behind a toggle, never the only carrier (zone labels are text). |
| Mandatory FTP/ramp test before training (Zwift, TrainerRoad) | **Change** | Testing tired riders on day 1 is bad advice and bad onboarding. Start with priors ± uncertainty, ride today, refine from rides or an optional assessment week (spec/ONBOARDING.md, P7). |
| A calendar as the home screen (TrainerRoad, TrainingPeaks) | **Change** | The home question is "what do I ride now?". Home = **Today**: check-in and one proposal. The last days are context under it. |
| "Regenerate" for a new random plan | **Change** | Random re-rolls teach nothing. Directed alternatives: **Shorter · Easier · Harder · Other type**, each with a reason (planner variants, #15). |
| A readiness score as the reason (WHOOP-style, TrainerRoad AI) | **Change** | "Readiness 3.8/5" can't be checked. The reason lists facts the rider can verify: what they answered, the last similar session, recent load, time available (§5.1). |
| ERG intensity ± (TrainerRoad, Zwift) | **Keep** | A real need (bad day, good day). Fixed-width, shows the rider's own cumulative offset; the coach's change is shown separately, at the target (§5.2). |
| Coach pop-ups / toasts (SYSTM tips, Zwift messages) | **Change** | Toasts cover data and vanish. The coach has a reserved **lane** on the ride screen (P4, P5) and a decision log in Review. |
| "Workout complete" fanfare (most apps) | **Change** | Replaced by an automatic hand-over to Review that leads with whether the session met its intent and asks for the 2 inputs that make the coach better (RPE, ratings) (#6). |
| NP / TSS / IF tiles as the ride summary (TrainingPeaks, most apps) | **Change** | They describe load, not whether the session did its job. Review leads with the intent verdict; load numbers move into "All numbers" (§5.5). |
| Social, kudos, leaderboards | **Drop** | Not a job any of the moments above asks for. |
| TSS/IF as the headline of a proposal | **Change** | Shown, but the headline is duration + what the session is for + why today. |
| Pause button | **Keep** | Space / remote Pause; fixed position and width; pause dims the plan, not the body (HR stays live) (P4). |
| Lap / skip | **Keep** | `S` / remote Skip ends the block; open-ended `lap` blocks end on Skip. |

---

## 3. Information architecture

Four places plus two modes. The coach is not a place: it acts in Today (proposal), Ride (lane), Review
(decision log) and is configured in Settings. This replaces VIEWS' six-item rail (§9).

```
 ┌────────── rail ──────────┐
 │ Today    (home)          │──Start──▶ Ready check ──Start──▶ RIDE (full screen, no rail)
 │ Workouts (library+editor)│                                        │ state: finished (auto, #6)
 │ History  (rides + model) │◀─────────────── Review ◀───────────────┘
 │ Settings (devices, model,│          relaunch with an interrupted ride ──▶ Resume sheet (§6.6)
 │   coach, controls+sound, │
 │   screen, appearance)    │
 └───────────────────────────┘
 First run: Onboarding (7 steps, no rail, resumable) → Today
```

- **Ride is a mode, not a page.** No rail, no navigation; exits only via finish (→ Review) or End ride
  (confirm, → Review of the partial ride).
- **Onboarding is a mode.** No rail until it is finished or explicitly skipped, so a new rider can't land in
  an empty app; leaving mid-way resumes at the same step (§5.8).
- **Review** is reachable from History for any past ride (needs the ride library, #29); it auto-opens on
  `state: finished`.
- Rail at ≥ 1024 px wide (both target sizes). Below that the app is out of scope for v1.

---

## 4. Flows

1. **Daily ride (the 95 % path):** Today → check-in (pre-filled from the last answer; one tap "Same as
   yesterday", or change what differs) → proposal with reasons → **Start** → Ready check (trainer green → Start)
   → Ride → auto Review → RPE + rate calls → History.
2. **First run:** Onboarding (7 steps) → "Your starting model" (CP 255 ± 35 W, self-report) → Today with a
   conservative proposal (P7).
3. **Build/edit:** Workouts → pick or new → edit in profile / outline / text → AI on selection → read the
   change summary and before/after → accept (verified, #1) → Ride this.
4. **Interrupted:** crash, forced quit or OS sleep mid-ride → relaunch → Resume sheet → Resume (same block,
   soft start) or Save & review (§6.6).

---

## 5. Views

All views share: the left rail (except Ride and Onboarding), an 8 px spacing grid, tokens from
`spec/THEMES.md`, tabular numerals for every number, durations as clocks (`m:ss`, `h:mm:ss`) everywhere, and
keyboard access (`?` lists keys). Issue numbers and fixture notes never appear in rider-facing text.

### 5.1 Today (home) — *"What do I ride today, and is it right?"*
- **How are you?** four rows (sleep, legs, stress, motivation) answered with **words, not numbers**, and
  every row reads **worse → better from left to right** (stress therefore runs "very high … none"). The words
  carry the meaning at every window size; the stored value stays the contract's 1–5. Time available: 30 / 45 /
  60 / 90 min chips.
- **Confirming today.** The answers are pre-filled from the last check-in and labelled so. **Start stays
  disabled until today is confirmed**: one tap on **Same as yesterday**, or changing any answer. Changing any
  answer re-asks the planner; the proposal shows a pending state in place (no layout shift).
- **Proposal card** (the hero): name, duration, what it is for, a profile (P13) with zones, and **why today
  as checkable facts** (bullets: what you answered, the last similar session and when, load over the last 7
  days vs usual, time available) plus the confidence of the athlete model used. Actions: **Start** (primary,
  Enter), **Shorter / Easier / Harder / Other** (directed variants, #15), **Edit** (opens the editor on a copy).
  Structured reasons need a planner contract change (#29).
- **Model card**: CP / W′ / HRmax with the **± at the same size as the value** (secondary ink), and under each
  its provenance and age ("from 9 rides · 2 d ago"); older than 6 weeks → stale styling; what is still
  calibrating (spec/ONBOARDING §4).
- **Health cap** (from onboarding's health check): when active, a status card above the proposal states the cap,
  why, and *Confirm check*; the **planner's proposal respects it** (e.g. "Endurance 45′ (capped)"), the
  profile draws the cap line, and the reasons say so instead of the uncapped reasoning.
- **Last 7 days**: seven day cells (done / rest / today; the number is labelled "TSS") — context, not navigation.

### 5.2 Ride — *"Am I doing it right, how long, what next?"*

Fixed anatomy at every size (geometry does not change between phases or states; only contents do, P4):

```
┌ status bar: [⏸ Pause] │ 13:25 left │ ERG  you [−] ±0% [+] │ ● NEO │ ● HR │ ?  ─────────────────┐  fixed widths
├──────────────────────── hero (62 % width) ─────────────────┬──── side (38 %) ────────────────┤
│ row 1  PHASE · block name · set 2/3 · rep 3/5 ●●○○○        │ HEART RATE           zone 4    │
│ row 2  caption: "left in block" / "HR below 118 in"        │ 164 bpm                         │
│ row 3            0:23                         ← T1         │ CADENCE      ✓ in range 90–95  │
│ row 4  TARGET 350 W  ▕━━━━▓▓▓|▓▓━━━▏ ▲ 6 W   ← band / fill  │ 91 rpm  ▕──▓▓|▓──▏             │
│        sub-line (reserved): ↗ to 210 W · coach −5 %        ├──── coach lane (reserved) ─────┤
│ row 5  next  Easy 0:15 @ 140 W                ← T3+        │ idle: W′ 74 % · last call      │
├────────────────────────────────────────────────────────────┴────────────────────────────────┤
│ road ahead: −2 min … now … +10 min; coach changes: dashed = proposed, solid = applied        │
│ whole ride overview (thin) with you-are-here · 0:30:58 ridden · block 36 of 39               │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

Rows 1–5 exist in every template with the same heights; a template that has nothing for a row leaves it
empty. The side and the coach lane are identical in every phase.

**Phase templates** (what fills the hero; derived from the current block, P3):

| Phase / block | Row 2 caption | Row 3 (T1) | Row 4 (T2) + sub-line | Why |
|---|---|---|---|---|
| **Warm-up / ERG ramp** | left in block | block countdown | target now + band; sub: "↗ to 210 W" | The target is moving; the rider needs to know it keeps rising and to where. |
| **Steady work (ERG)** | left in block | block countdown | target + band with actual marker + Δ chip | In ERG the band is a compliance check; cadence (side) is the lever. |
| **Short on/off (≤ 60 s reps)** | left in rep | countdown | target + band; row 1 shows ON/OFF + rep pips | Reps are counted, not read; pips are glanceable. |
| **Sprint (free / non-ERG)** | power | **actual power** | **fill bar** of actual vs target (target tick at 100 %) + countdown | The rider steers power; the fill shows "enough?" without a second number to compare. |
| **HR-guided recovery** (`hr<`) | HR below 118 in | **range or estimate** (P7, below) | "15 bpm to go" + recovery curve with a live dot, axes and the band; sub: "block ends between 2:00 and 4:00" | The question is "when can I go?", with honest uncertainty and the hard limits. |
| **Cool-down** | left in block | block countdown | target + band; row 5: "ride complete — Review opens automatically" | Signals the end. |

**Recovery estimate (P7).** The core gives the posterior mean and a 90 % band of "time until HR is below the
target", clamped to the block's min/max. A clamped value is the block's **rule**, not a prediction, so the
screen says which event the number is about (core flags, #13; prototype `recoveryRead()`):

| Situation | Row 2 caption | T1 | Row 4 sub-line |
|---|---|---|---|
| fit not ready yet | HR below 118 in | "fitting…" | block ends in m:ss–m:ss |
| HR predicted below target before the minimum | **block ends at its minimum in** | time to the minimum | HR due below 118 before then |
| prediction inside the block's limits, band ≤ 30 s | HR below 118 in | ≈ mean, band "1:01–1:12" under it (the backend's interval, never widened or narrowed for display) | block ends in m:ss–m:ss |
| band > 30 s | HR below 118 in | the band ("1:40–3:40") at range size (§6.2) | block ends in m:ss–m:ss |
| band reaches the maximum | HR below 118 in · or at the block maximum | the band | block ends in m:ss–m:ss |
| HR lost (ADR-0001 §5) | HR lost · block ends at its maximum in | time to the maximum | the chart stops its forecast; the last HR is drawn hollow (stale) |

All times count **from now** (the sub-line too), so two numbers on screen never use different origins.

**ERG band.** The band shows the core's target band (#13); edges are solid lines, not gradients. The marker
and Δ are **hidden while the trainer settles** (first 5 s of a block whose target changed, after a ± or coach
change, after ERG re-engagement — the same settle the core uses for compliance, ADR-0002 §7.8): the band shows
"settling" instead, so normal ramp-up never reads as failure. Below the band: cool colour + ▼; above: warm
colour + ▲; inside: neutral, no arrow.

**Cadence** (the ERG lever) is a slot like any other but always carries a state: *✓ in range*, *▲ spin
faster*, *▼ ease cadence*, or *no target*, from the core's cadence band (#13, #29), with a mini band like
power's. If the rider chose slots without cadence, or Focus density, a cadence state that is not *in range*
appears as a chip in row 1. **ERG released** (cadence < 40 rpm for 2 s, ADR-0002 §7.4) takes row 4: "ERG
released — spin up to 60 rpm to re-engage", with a warning tone; re-engagement shows "easing back in"
during the 5 s soft start.

**Stale values (P6, P7).** A value whose input is lost is never shown as live: it becomes "—" with what is
missing ("— no power data"), or, where the last value still helps, it is drawn hollow/muted with its age.
Dependent estimates follow their inputs: W′ balance needs power, HR forecasts and "HR vs usual" need HR,
cadence needs the trainer.

**Row 1** names the block: its own label, else the enclosing label with "after" for the rests inside it
("Easy · after Threshold 1/2"), plus rep pips, a cadence chip when the cadence slot is hidden, and
"capped ≤ 75 % FTP" while a health cap is active.

**States** (overlay a template without moving it):

| State | What changes | Input (§7.2) |
|---|---|---|
| Coach asks (`pending`) | Tone (§7.3). Lane: verb + amount on one line ("Ease off −3 %?"), one short reason (≤ 24 characters), band word as a small tag, two **equal-weight** buttons, **numeric deadline** ("keeps plan in 24 s"); on the road the affected work blocks get a **dashed outline at their proposed height**, on top of any solid (already applied) outline, and the legend states the scope ("proposed −3 % on 2 work blocks") | Coach = accept · Keep = keep · silence = keep |
| Coach applied (`auto_applied`, opt-in) | Tone. Lane: "Eased off −5 %" + reason + "**Back to plan · 11 s**"; applied blocks **solid** outline; the target's sub-line shows "coach −5 %" on the changed blocks | Keep (within the window) = back to plan · Coach does nothing |
| Trainer lost | Warning tone. Chip ⚠ NEO "lost"; row 4: "**⚠ Trainer lost · paused at 2:47**" / "reconnecting (try 3) · ride saved"; actual power, cadence and W′ balance "—"; countdown frozen and dimmed (ADR-0002 §6) | — |
| Trainer lost ≥ 30 s | Row 4 adds what to try ("power-cycle the NEO · close Zwift/Tacx") and an **End & save** button; reconnecting continues behind it | Esc → End ride dialog |
| Trainer reconnected | Row 4 for the 5 s soft start: "Reconnected · easing in to 253 W" / "workout resumed" (or "pedal to resume" at 0 rpm); then the chip shows "NEO back · 0:14 gap" for 10 s | — |
| Trainer lost at a block boundary | The block shown is the one the core paused in (transitions inside the 2 s detection window stand, ADR-0001 §5): "paused at 0:00" of the new interval; on reconnect the soft start goes to *that* block's target | — |
| HR lost | Chip ⚠ HR "lost"; HR slot "—" + "HR strap lost"; lane: "HR lost: coach can't push" | — |
| Both lost | Row 4: "⚠ Trainer + HR lost · paused at m:ss"; stays paused until the trainer is back and cadence > 0 | — |
| ERG released (cadence < 40 rpm 2 s) | Row 4: "⚠ ERG released · spin up to 60 rpm" / "timer runs · target returns over 5 s" (the block timer keeps running, ADR-0002 §7.4); cadence slot "▲ spin faster" | — |
| Paused (user) | The plan dims (countdown, next, road); **the body stays live** (HR, cadence); row 4: "Paused 0:42" / "Resume continues the plan · trainer free". While paused the trainer is at 0 % resistance, so restarting is easy; ERG re-engages with the soft start | Pause |
| "How did that feel?" (calibrating, after a hard block) | Lane: "How was that?" with **nothing preselected**; each Coach press moves the choice easy → right → hard; the choice **saves 3 s after the last press** with the confirmation tone ("saves “hard” in 3 s"); Keep skips; no press within 12 s records nothing. Feeds the athlete model (#18) | Coach = choose · Keep = skip |
| Finished | Auto-navigate to Review (#6) | — |

### 5.3 Ride-screen configuration (replaces the layout editor, §9)
The rider chooses what the **two side slots** show (the same in every phase; from the metric catalogue: HR,
cadence, W′bal, DFA-α1, HR vs typical, power 10 s, kJ) and a **density**: *Focus* (hero + coach + road; the
cadence state moves to row 1), *Standard* (default; the idle coach lane shows W′ balance quietly at T3 and the
last call), *Detail* (the idle lane shows a third metric at T2 — it lives in the reserved lane, which a coach
call takes over, so no density adds a row). Cadence *state* survives every configuration (§5.2). Edited as a
**draft** with Save / Cancel / Reset to default, validated per slot on load and on save (#4, #8); controls
read the draft, never a prop-seeded copy (#11). Geometry is fixed, so no configuration can produce an
unusable layout at 1024 (#7); `ride-focus` and `ride-detail` show both. The rider's **screen distance** lives
here too (§6.2).

### 5.4 Ready check (pre-ride)
A sheet over Today: workout name + duration, **NEO** (FE-C, connected / reconnecting / not found → Retry,
"close other apps using the NEO"), **HR** (connected / not found → keeps scanning, **Retry now** focused —
never the riskier choice — and *Start without HR* with the consequence: "it joins as soon as it's found; until
then HR-guided recoveries run to their maximum and the coach can't push"), mode (ERG), the health cap if
active, "starts with 8:00 warm-up". **Start** is enabled when the trainer is connected.

### 5.5 Review — *"Did it work, and was the coach right?"*
Order follows the rider's questions:
1. **Intent verdict** (headline): one line per part of the workout, from the core's per-interval stats (#12):
   "Threshold 2 × 4:00 — 94 % on target, against the eased target (plan −5 % by coach)", "VO2 sets — 15 of 15
   reps", "HR recoveries — 3 of 3 within your usual τ". Then the coach (applied / not taken) and data gaps
   for **each** sensor ("0:04 without power, excluded from on target · 0:17 without HR, excluded from the coach and
   recoveries"). **"On target" has one definition everywhere** and it is printed under the verdict: share of
   ERG seconds within ±5 % of the target the rider was given, settling excluded (ADR-0002 §7.8).
2. **Two inputs** (each ≤ 1 tap): **RPE 1–10** with word anchors, and **rate each coach call**. Each call
   expands to its full reason and a **chart of the 3 minutes around it**: power over the planned target, and HR
   with **your usual HR for that power** as a dashed line, so "HR +9 bpm vs usual" is visible as a gap.
3. **Evidence**: stacked linear-time panels sharing one x axis with time ticks (P05): **power as a line**
   over the planned target drawn as a band **with its own ≥ 3:1 edge**, HR on an **80–200 bpm** scale (a
   0-based HR axis flattens the differences the coach reacts to), cadence; numbered coach markers; **gaps
   hatched** and never bridged; a legend; *Show as table* — per-minute rows **plus a row for every gap and
   coach call**, so nothing brief is sampled away. HR recoveries: τ **± its fit uncertainty**, your usual τ
   **± its spread**, the data the fit used, and the "slower" rule (≥ 1.15 × usual) stated. Work-interval table
   with the same "on target" measure as the verdict (#12).
4. **All numbers** (collapsed by default): duration, kJ, NP, TSS, IF, on target, and metrics only when the
   core marks them valid for this session (e.g. decoupling only for steady rides, #13).
5. **Analytics**: time at power (axes: W bins, minutes), power–duration curve (5 s – 60 min, log x with
   ticks, W scale), each with *Show as table*. Point-by-point keyboard inspection of every chart: #71.

### 5.6 History — *"Am I improving, can I trust the numbers?"*
- **Athlete model** card: CP, W′, HRmax, HR-recovery τ, each value ± sd, source, n, last update **and its age**;
  priors say "not measured yet"; an estimate older than 6 weeks is drawn stale (P7, spec/ONBOARDING §3). The
  same τ is the "usual τ" Review compares against.
- **CP trend** with dates on x, watts on y, the ± 1 sd band labelled in a legend; *Show as table*.
- **PD curve**: latest ride / 90 days / all time (backend `mmp()`), with axes; *Show as table*.
- **Rides**: list (date, name, duration, TSS, RPE, mini profile) → Review of that ride.

### 5.7 Workouts (library + editor)
Library on the left; the editor keeps the spike's strongest idea — **three synchronised views**: profile
(P13), outline (tree), text (DSL, `shared/DSL.md`) — plus the compiled summary (duration range, TSS, IF,
zone time, issues). **AI on selection** (vary / harder / easier / shorter same stimulus) **locks all editing**
until Accept / Reject; Accept verifies the target node id and content hash, else re-requests (#1). The open
suggestion is explained before it is accepted:
- a **plain-language change summary** ("VO2 sets: 3 times → 4 times, 125 % FTP → 127 % FTP · +5:45 · load
  51 → 61 TSS") — words, not workout-file syntax; the exact text is in the diff below. Production takes it
  from the planner's rewrite rationale (#29);
- **two stacked profiles**, *Current* and *Proposed*, on the same time scale, the changed item outlined;
- the **text and outline show the diff**: the old item struck through, the new item highlighted;
- the **summary shows Current → Proposed** for duration range, TSS, IF and zone time, every value labelled.
Undo/redo spans all three views; tree ops, undo/redo and DSL ↔ tree sync are unit-tested in production (#9).

### 5.8 Onboarding (first run)
Seven steps (spec/ONBOARDING §1): About you · Background · Current numbers · Health check · Devices ·
Baseline · Your starting model. A mode without the rail (§3); progress persisted after every step, so a
relaunch shows **"Welcome back — continue at Health check (4 of 7)"**.
- Every step states what **Skip** does ("Skipping keeps wide priors: CP ± 60 W, the coach stays conservative").
- **Current numbers**: value, *what it came from* (ramp test / 20-min test / app estimate / guess) and
  *when* (a date); the uncertainty shown grows with age and source.
- **Health check**: four yes/no questions with **no default** — each starts "not answered" and Continue
  stays disabled until all are answered; unanswered (or Skip) is treated as the cautious answer. Each "yes"
  has its own consequence, shown at once: chest pain / fainting → "get a medical check before hard riding",
  cap at endurance (75 % FTP) and no coach push until confirmed; HR-changing medication → HR zones,
  HR-guided recoveries and HR-vs-usual coaching off (targets by power and feel), no power cap; injury → cap at
  threshold, no sprints. A cap changes what the planner proposes (the capped proposal is drawn with the cap
  line, `today-health-cap`) and appears on the Ready check (`ready-capped`) and in the ride (`ride-capped`).
- **Devices**: pairing shows **live readings** (W, rpm, bpm) so "pedal a few strokes" visibly works.
- **Baseline**: import / assessment week / passive learning, each with what it costs and how soon it is confident.
- **Your starting model**: estimates with value ± sd, a relative-uncertainty bar, source, what will narrow
  them, and what the coach does meanwhile.

### 5.9 Settings
Sections: **Devices** (trainer + HR, protocol, status, battery, pair / forget; one-central note),
**Athlete model** (estimates ± source, manual override with provenance "self-report"), **Coach** (autonomy,
§7.1), **Controls & sound** (the five actions and their bindings with *Learn* for a remote, §7.2; tones and
volume, §7.3), **Ride screen** (§5.3), **Appearance** (mode dark / light / auto, preset, classic zones).

---

## 6. Ride screen specification

### 6.1 Grid
Status bar 64 px (56 px at ≤ 800 px height). Hero and side split 62 / 38. Road ahead 22 % of height
(min 128 px). Hero rows 1–5 have fixed heights from the type tiers. All regions are fixed by CSS grid with
`minmax(0, …)`; contents are sized by the visual budget below. **Single-line rows never wrap** (rows 1, 2, 5, slot labels, the
coach verb, reason and deadline): every string there has a maximum length (coach reason ≤ 24 characters).
**Banners** in row 4 are one bold line plus at most two short lines, each ≤ 36 characters so they fit on
one line at 1024. The harness fails on any clipped `[data-fit]` text.

### 6.2 Visual budget (legibility as visual angle)
Character height *h* (cap height) at distance *d* subtends θ ≈ *h*/*d*. Ergonomic standards set 16′ of arc
as the minimum and 20–22′ as preferred for reading (ANSI/HFES 100-2007, ISO 9241-303). Glance targets are
multiples of that. Reference devices: a 13–15″ laptop at default scaling (≈ 5.0 pt/mm) at **1.0 m**, a
9.7–10.2″ tablet (≈ 5.2 pt/mm) at **0.8 m**; cap height ≈ 0.72 em. Font px = θ · *d* · pt/mm / 0.72.

| Tier | Use | Target | Laptop 1440×900 @ 1.0 m | Tablet 1024×768 @ 0.8 m | Prototype rule |
|---|---|---|---|---|---|
| **T1 glance** | countdown, recovery range, sprint power | ≥ 90′ (1.5°) | 182 px | 151 px | `clamp(150px, 21vh, 240px)` → 189 / 161 |
| **T2 glance** | target, HR, cadence | ≥ 50′ | 101 px | 84 px | `clamp(84px, 11.4vh, 128px)` → 103 / 88 |
| **T3+ read** | next block, coach verb, banners | ≥ 20′ | 40 px | 34 px | `clamp(34px, 4.5vh, 44px)` → 41 / 35 |
| **T3 read** | labels, captions, coach reason, deadlines, status-bar values | ≥ 16′ | 32 px | 27 px | `clamp(27px, 3.7vh, 36px)` → 33 / 28 |
| T1r range | the recovery band when shown as the hero ("1:40–3:40") | ≥ 55′ | 117 px | 100 px | `0.62 × T1` → 117 / 100 |
| T4 chrome | key hints, axis ticks, band word, road labels — never needed mid-effort | — | 15–18 px | 15–18 px | fixed px |

Achieved at the prototype's sizes (laptop / tablet): T1 94′ / 96′, T1r 58′ / 59′, T2 51′ / 52′, T3+ 20′ / 21′,
T3 16.5′ / 16.9′.
The vh rules only hold for the two reference devices. **Production computes px from the monitor's
physical size, its scale factor and the rider's distance** (Settings → Ride screen, default 1.0 m) and warns
when the window is too small to meet T2 (#52). Numbers: tabular, lining figures.

### 6.3 Colour and contrast
Tokens and presets from `spec/THEMES.md` (Ocean by default; dark is designed, not inverted). On the ride
screen colour carries **state**, not decoration, and never alone (P9): the band has ▲/▼, sensors an icon and
a word, series a dash pattern or a direct label. Minimum contrast: text ≥ 4.5:1 (≥ 3:1 at T2 and above), chart
marks and UI boundaries ≥ 3:1 against their surface, in both modes (WCAG 2.2 1.4.3 / 1.4.11). The prototype's
key pairs are checked by `scripts/contrast.mjs` (part of `pnpm screens`); the full palette validation is #30.
**Fills are decoration, edges carry the encoding:** every area and band (planned target, uncertainty bands)
also has an edge stroke in a token that passes 3:1, and gaps are hatched in `--ink-muted` (≥ 4.5:1), so a
low-contrast composite fill never carries meaning on its own.

### 6.4 Motion
Values update at the backend subscription rate (hero 4 Hz, side 2 Hz, road 1 Hz). No animated layout
transitions. Deadlines and undo windows count down in whole seconds, as numbers ("keeps plan in 24 s",
"Undo · 11 s").

### 6.5 Sensor loss and reconnection (engine policy from ADR-0002 §6, ADR-0001 §5)
| Event | Engine (decided in the ADRs) | What the rider sees and hears |
|---|---|---|
| Trainer silent < 2 s | still `riding`; record seconds `null` | power "—"; nothing else changes |
| Trainer lost (event or 2 s silence) | `paused(trainer_lost)`: block timer and workout time freeze, session time runs, record `null`; reconnect loop with backoff | warning tone; row 4 banner (what broke · "workout paused at m:ss of this block" · "reconnecting (attempt n)" · "keep pedalling or stop — your ride is saved"); countdown frozen; chip ⚠ |
| Reconnected | driver ready → engine re-applies ERG with a **5 s soft start** from current power to the block target → workout resumes when cadence > 0 | row 4 "Reconnected · easing back in to 253 W"; then "NEO back · m:ss gap" for 10 s |
| Lost across a block boundary | transitions inside the 2 s window stand; the soft start targets the block the ride paused in | as above; the block name in row 1 tells which interval resumes |
| HR lost | ride keeps going; HR `null`; `hr<` blocks fall back to their maximum; the judge gets no HR features (cannot push) | chip ⚠ HR; HR slot "—"; lane note; in `hr<` blocks the caption switches to "HR lost · block ends at its maximum" |
| Both lost | `paused(trainer_lost)` as above | one banner for both |
| ERG released (cadence < 40 rpm 2 s) | `Resistance 0 %`; re-engage at ≥ 60 rpm for 2 s with soft start; released seconds excluded from compliance | warning tone; row 4 "ERG released — spin up to 60 rpm" |
The UI never infers these states: `RideStatus` carries the pause reason, reconnect attempt, gap length,
soft-start and released flags (ADR-0001 §5, #29).

### 6.6 Crashes, restarts and sleep
- **UI crash** (the webview throws or reloads): the engine runs in the core process, so the ride keeps
  recording and the trainer keeps its target. The ride's error boundary shows: "The ride screen restarted —
  your ride kept going (31:07 recorded) and the trainer is holding 253 W" with **Back to ride** focused, bound
  to the Coach button and taken **automatically after 5 s**; it re-subscribes from a snapshot (ADR-0001 stream
  snapshot). A second crash within 60 s adds **End ride and review** as a second, unfocused choice — never the
  default.
- **Process crash, forced quit, OS sleep**: the record is saved every 5 s (ADR-0001 §6), so at most 5 s are
  lost. On the next launch, before anything else, a **Resume sheet** shows the workout, where it stopped
  ("31:12 ridden · Threshold rep 1 of 2, 1:40 into it"), what was saved, and a **re-entry plan keyed to how
  long ago it stopped and where**:

  | Stopped | Offer (focused first) | Re-entry |
  |---|---|---|
  | ≤ 2 min ago | **Resume ride** · Save & review | same point; ERG eases in over 5 s |
  | 2–15 min ago | **Resume with re-warm-up** · Save & review | a 3-min ramp to the block's target first; an interrupted work interval restarts from its beginning |
  | > 15 min ago | **Save & review** · Ride the rest as a new ride | no resume: a cold threshold rep is not the same session; "the rest" starts with a warm-up |

  Resume stays disabled until the trainer is connected (shown on the sheet). The gap is recorded as no data.
  Today's ADR closes open rides as aborted, which allows only *Save & review*; resuming needs the ADR
  amendment in **#50** (policy recorded there). Screens `resume` (4 min ago), `resume-stale` (22 h ago).
- **Sleep prevention**: while a ride is loaded the app holds a power assertion so the system and display
  don't sleep (**#51**). If it sleeps anyway, BLE drops → trainer lost (§6.5) → on wake the reconnect loop runs.

### 6.7 Accessibility
Every control is a native button or input with a visible focus ring; a focused control keeps its native
Enter/Space meaning over any ride shortcut (§7.2); dialogs (End ride, keys, Resume) take
focus, trap Tab, close on Escape (and consume it) and **suspend ride shortcuts while open**; the End ride
dialog focuses **Keep riding** first, so a Pause / Space press can never end a ride; live regions announce coach calls
and sensor changes; every chart has a table alternative (P9) and keyboard point inspection is specified in #71; targets ≥ 48 px;
contrast per §6.3.

---

## 7. Coach interaction model

### 7.1 Autonomy and bounds
- Verdicts `ease_off · hold · push · replan`, each a single verb in the lane. Lean / uncertain calls ask
  with a deadline (30 / 20 s) and **silence keeps the plan**.
- **Default autonomy is "always ask"**: every call asks, strong ones included. The rider can opt in to
  **"auto-apply strong decreases"**: only `ease_off` and the decreasing part of `replan`, with a 15 s undo.
  **Increases (`push`) always ask** and are never applied by silence.
- Bounds, enforced by the core: one call changes the remaining work by at most −5 % or +3 %; all coach
  changes together stay within −10 % … +5 % of the plan; no `push` without HR (ADR-0001 §5) or while the
  health cap is active. The prototype's mock auto-applies strong calls regardless of the setting (#29);
  the `ride-coach-applied` screens show the opt-in path.
- **The change is visible**: proposed changes are dashed on the road, applied ones solid; the target's
  sub-line says "coach −5 %"; the rider's own ± stays separate in the status bar.
- **Before (planner)**: one proposal + checkable reasons + directed variants.
- **After**: every call is listed in Review with time, verdict, band, outcome, reason, the 3 minutes around
  it, and a one-tap rating.
- **Confidence**: the coach says what it is unsure about ("calibrating: HR-vs-typical unlocks after 20 min
  at 300 W").

### 7.2 Hands-free input and the decision model (decided)
Ride input is **five abstract actions**: **Coach**, **Keep**, **Pause**, **Skip**, **Intensity − / +**.

**Decision model — no input can reverse a rider's coach decision by accident:**
- **Coach only ever says yes** (accept a question; choose in the feel prompt). It never undoes anything.
- **Keep is the only way back to the plan**: decline a question, or return an auto-applied change to the plan
  within its 15 s window ("Back to plan · 11 s"). An explicitly accepted change is not undone by any key; the
  rider can still ease off with Intensity −.
- **Debounce**: after the lane changes (a call appears, is answered, applies, expires) Coach and Keep are
  ignored for **1.5 s**, so a press meant for the previous state can't act on the new one; key repeat is ignored.
- **Focused controls win**: when a button or input has keyboard focus, Enter and Space do what that control
  says (Enter on "Keep plan" keeps the plan); global shortcuts apply only when nothing interactive is focused.
- **Silence**: a question **keeps the plan** when its deadline runs out (logged "expired · no answer"); an
  auto-applied change **stays applied** (the rider opted in) after its window; the feel prompt **records
  nothing**.

**Bindings**: keyboard `Enter` / `Y` (Coach), `N` / `Backspace` (Keep), `Space` (Pause), `S` (Skip), `−` / `+`.
Any HID device can be bound in Settings → Controls with *Learn*, so a remote that sends media keys or arrows
works without code changes. **Minimum remote grammar: two buttons = Coach + Pause** (Keep then only via
silence for questions, on screen for "back to plan"); a one-button remote binds Coach only and everything
else relies on silence being safe and the on-screen buttons. Which remotes send which keys, and whether
long-press is distinguishable, decides the defaults (owner hardware test, **#53**).

**Fallback without a remote**: on-screen buttons at ≥ 64 px in the lane, equal weight (Accept is not the bright
default, because silence means keep), and the safe defaults above.

### 7.3 Sound
Sound is the channel that needs no glance (constraint 1). Distinct, short tones, each with a mute in
Settings → Controls & sound and one master volume:
| Event | Sound |
|---|---|
| Block change | one chime; a rising pair when the next block is harder, a falling pair when it is easier |
| Last 3 s of a timed block | three short ticks |
| Coach asks | two-note question (rising) |
| Coach change applied / undone | one soft confirmation / one low note |
| Trainer or HR lost, ERG released | warning (two low notes), repeated every 15 s while it lasts |
| Reconnected | one confirmation |
Sounds never carry information that is not also on screen. The prototype plays them in interactive mode
(Web Audio); production uses the same table (M1 UI).

---

## 8. Window sizes
| Size | Target | Rules |
|---|---|---|
| **1440×900** | laptop/desktop landscape (primary) | as specified |
| **1024×768** | tablet landscape | same anatomy and rows (no reflow by breakpoint); type from the tiers (§6.2); the status bar keeps every control with its label (Pause keeps its word); Today keeps every word label; the editor stacks the summary under outline + text |
| resize | any between | layout follows the window continuously (CSS, no device setting) (#7) |
| TV / phone remote | out of v1 scope | VIEWS §4 rules stay for later |

---

## 9. Where this supersedes `spec/UX-SPEC.md` / `spec/VIEWS.md`
| Spec | Was | Now |
|---|---|---|
| UX-SPEC §2 widget grid, pages, auto-page | user-paged widget grid with an edit mode | phase templates + fixed anatomy (§5.2), no paging mid-ride |
| UX-SPEC §2 edit mode; VIEWS V9 | free-form drag/resize grid editor | slot configuration in Settings as a validated draft (§5.3) |
| UX-SPEC §2 default layout pages Interval / Recovery / Nerd; VIEWS V3 pages incl. Fuel | four pages | Interval → work templates; Recovery → HR-recovery template; Nerd → Review §5.5(5); Fuel → deferred (P21, B7 #18) |
| UX-SPEC §2 top bar speed (1×/10×/60×) | rider-facing | dev/prototype only |
| UX-SPEC §2 top bar "block x/n" | status bar | road overview (it doesn't answer "how long is left") |
| UX-SPEC §2 coach card keys Y / N | two keys | five abstract actions with Coach / Keep (§7.2); Y stays an alias |
| UX-SPEC §1 / ONBOARDING §5 strong nudges auto-apply | default | opt-in, decreases only, bounded (§7.1) |
| VIEWS §2 rail Today · Plan · Editor · Progress · Coach · Settings | six places | Today · Workouts · History · Settings; Plan folded into Today; Coach split into lane / Review / Settings (§3) |
| VIEWS V1/V2 Today and Check-in separate | two views, 1–5 scales | one view; worded scales, worse → better (§5.1) |
| VIEWS P01 big number "tap → cycle smoothing" | per-tile interaction | smoothing per slot in Settings; no mid-ride pointing |
| VIEWS P24 diff overlay (old ghosted over new) | overlaid profiles | stacked Current / Proposed profiles + text diff + change summary (§5.7) |
| VIEWS §4 tablet: strip moves into top bar | different layout at 1024 | same anatomy, scaled (§8) |
| UX-SPEC §1 "Regenerate" | random re-roll | directed variants (§5.1, #15) |
| UX-SPEC §4 Review "minimal" | numbers + chart + decisions | intent verdict, two inputs with context, evidence, all numbers, analytics (§5.5) |

Everything else in UX-SPEC / VIEWS (plot catalogue P01–P24, visual conventions, editor capabilities, the
non-functional rules) still applies.

---

## 10. The spike defects #1–#11, designed out
| # | Defect | Design element that prevents it |
|---|---|---|
| #1 | AI rewrite accept loses/misplaces edits | Editor **locks** during pending and diff (banner, all inputs disabled); Accept verifies node id + content hash, else re-requests (§5.7). Prototype: `editor/model.ts` (`lock`, hash check in `accept`). |
| #2 | Physiology in the UI | P8: every derived number comes from the backend; missing ones are contract additions (#12, #13, #14, #15, #18, #29) stood in by **one** fixture module (`mock/ext.ts`), never computed in a view. |
| #3 | Per-ride state not reset | One `RideSession` object (status, nudges, compiled, undo windows, sensor gaps, pause time) created on `ride.load` and disposed when the next ride loads (§5.2). Prototype: `session.ts`. |
| #4 | Unvalidated localStorage | Everything persisted (ride-screen config, theme, sound, bindings, onboarding progress) is parsed by a schema on load; invalid → default + notice; error boundaries at the top and around the ride (§6.6). Prototype: `persist.ts`, `App.tsx`. |
| #5 | structuredClone on reactive proxies | React state holds plain immutable values; the only snapshot path is `snapshot()` (production lint rule). Prototype: no clone of state anywhere. |
| #6 | Finished ride doesn't reach Review | `state: finished` → Review automatically, owned by the ride session (§3, §5.2). |
| #7 | 1024×768 unusable, layout ignores resize | Fixed anatomy and rows scaled by the tiers (§6.2, §8); strings in reserved rows have maximum lengths (§6.1); the harness renders every scene at 1024×768 and fails on overflow. |
| #8 | Layout-editor state bugs | Free grid editor dropped; slot config is a transactional draft with per-slot validation (§5.3). |
| #9 | No unit tests for editor ops | Editor model (tree ops, undo/redo, DSL ↔ tree) is a pure module with tests in production (§5.7); the prototype keeps that module boundary (`editor/model.ts`). |
| #10 | Ride-screen layout shift | P4: fixed grid and rows, fixed-width controls, tabular numerals, reserved coach lane and band row; states recolour/fill, never re-flow (§5.2, §6.1). |
| #11 | Widget settings seeded from props once | Settings forms read the draft directly; the ride screen reads the saved config live (§5.3). |

---

## 11. Contract additions this design needs
Per-lap stats (#12); thresholds: target band, settle flag, cadence band, τ-slower, metric validity (#13);
live cursor (#14); planner variants (#15); athlete model, "how did that feel" taps (#18); and #29: coach
revert, short coach reason, coach offset, autonomy + bounds, pause reason / reconnect attempt / gap / soft
start / ERG released in `RideStatus`, ride library, post-ride RPE, devices with live readings, structured
planner reasons and rewrite rationale, fit quality, health-cap state, onboarding progress, ride-screen config,
a non-ERG sprint and long sensor losses in the mock. Engine/ADR: resume (#50), sleep (#51), physical type
sizing (#52), remote test (#53).

## 12. Prototype and screenshots
- `design/prototype/`: Vite + React 19 + React Compiler + TypeScript strict, against `@crank/shared`. A design
  artifact; components may be reused by the production UI, the fixtures in `src/mock/ext.ts` may not.
- Interactive: `pnpm -C design/prototype dev` (real-time mock with sound; `?speed=10` or `60` for a faster
  ride; `?first=1` starts in onboarding).
- Scenes: `?scene=<id>&theme=dark|light` builds a seeded mock (seed 42) on a virtual clock, advances it to the
  scene's moment and freezes it. `pnpm -C design/prototype screens` checks the contrast pairs, then renders
  every scene at 1440×900 and 1024×768 in dark and light to `docs/design/screens/<scene>.<w>x<h>.<theme>.png`,
  and fails on any page error, horizontal overflow, ride-screen overflow or clipped reserved-row text.
- States the mock cannot produce (a trainer lost for longer than its 3–6 s blip with the ADR's pause, a loss
  at a block boundary, both sensors lost, ERG released with low cadence, a crashed screen, an interrupted
  ride, a feel prompt, a missing trainer, a health cap) are **scene fixtures**: `mock/ext.ts` overlays the status
  and metric values the core would send, labelled as such. The mock keeps the block timer running during a
  trainer loss (the ADR freezes it), so consecutive loss/reconnect scenes show a few seconds of drift — a mock
  limit tracked in #29, not the design.
- Known prototype limits (all tracked): Easier / Harder / Other are disabled (#15); past rides in History
  cannot be opened (#29); the sprint scene runs in ERG (#29); per-interval stats, intent verdicts, proposal
  reasons, the athlete model, devices and the week strip are fixtures (#12, #18, #29); onboarding answers
  are not persisted except the step reached.

## 13. Review guide
Reviewers should judge, with the screenshots open:
1. **Glanceability** — in every `ride-*` shot, can you answer "how long, am I on target, what next" in one
   look at ~25 % zoom? Do the tier sizes (§6.2) hold at 1024×768?
2. **Phase fit** — does each template put the right thing in the hero (§5.2)? Anything shown that the rider
   cannot act on?
3. **Stability** — compare `ride-threshold`, `ride-warmup-ramp`, `ride-hr-recovery`, `ride-paused`,
   `ride-coach-ask`, `ride-trainer-lost`, `ride-focus`, `ride-detail`: does anything other than values and
   reserved rows change position?
4. **Coach trust** — one glance: verb, amount, reason, deadline, default? Can any input sequence reverse a
   decision (§7.2)? Is silence always safe?
5. **Failure honesty** — `ride-trainer-lost`, `ride-trainer-lost-long`, `ride-lost-at-boundary`,
   `ride-reconnected`, `ride-both-lost`, `ride-hr-lost-recovery`, `ride-erg-released`, `ride-screen-restarted`,
   `resume`, `resume-stale`: what broke, what the app does, what to do, what was saved, how it re-enters?
6. **First principles** — challenge §2: any convention kept without a reason, or dropped that riders need?
7. **Honest numbers** — ranges, sources and ages wherever estimates appear (recovery, Today, History,
   onboarding)? Do stale values look stale? Do health answers start unanswered and does a cap change the plan?
8. **Evidence** — can Review's charts substantiate its verdict (axes, markers, gaps, tables)?
9. **Defects** — does §10 make #1–#11 impossible, or only less likely?
10. **Supersessions** — are the §9 changes to UX-SPEC/VIEWS acceptable to the owner?

## 14. Open questions (not blocking the design)
- What the NEO does on BLE loss in ERG (standalone mode?) — the soft start of §6.5 covers both answers;
  measured in HIL check 11 (ADR-0002, #23).
- Which bar remotes to recommend and their default binding (#53).
- Resume after a process restart needs the ADR-0001 amendment (#50).
