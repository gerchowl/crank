# crank — first-principles UX design

Status: **proposed** (#24), round 5 — the final design round (after three rounds of astra and opus reviews; items
still open after this round are filed as issues, not further rounds; the PR body maps every
finding to its resolution). It drives the M1 UI issues once the reviews accept it.
**Addendum §15** (#87, round 3): training profile, planner wish, Why?/Background.
**Amendment §16** (#175, owner-accepted 2026-10-13): layout revisit of Ride, Today and the shell.
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
| "Regenerate" for a new random plan | **Change** | Random re-rolls teach nothing. Directed alternatives: ~~**Shorter** (the time choice) ·~~ superseded by §16.2: the time choice in the check-in · **Easier / Harder** (the wish one row down / up, §15.2) · **Other wish** (the matrix or the remote's wish mode; ~~a button on the card~~ superseded by §16.2: no card button, S / Pause), each answered with the same outcome table and reasons (§15.2; planner variants, #15). |
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
 First run: Onboarding (8 steps, no rail, resumable) → Today
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
2. **First run:** Onboarding (8 steps) → "Your starting model" (CP 255 ± 35 W, self-report) → Today with a
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
  answer re-asks the planner; the proposal shows a pending state in place (no layout shift). Once confirmed and the
  red flags are answered, the check-in collapses to **one summary line** with *Edit* (§16.2).
- **Proposal card** (the hero): name, duration, what it is for, a profile (P13) with zones, and **why today
  as checkable facts** (bullets: what you answered, the last similar session and when, load over the last 7
  days vs usual, time available) plus the confidence of the athlete model used. ~~Actions: **Start** (primary,
  Enter), **Shorter** (time), **Easier / Harder** (= the wish − / +, one outcome table, §15.2), **Edit** (opens the
  editor on a copy); "Other" is the wish matrix / wish mode (§15.2). After the check-in the proposal becomes the
  **saddle proposal** (§15.2).~~ superseded by §16.2: **one** proposal card — actions Start ⏎ · − Easier · + Harder ·
  Ride my wish / Ride the full X; the facts and *Edit a copy* sit behind "Why today: every fact" on the desk card;
  Shorter is the check-in's time; "Other" is the matrix / wish mode.
  Structured reasons need a planner contract change (#29).
- **Model card**: CP / W′ / HRmax with the **± at the same size as the value** (secondary ink), and under each
  its provenance and age ("from 9 rides · 2 d ago"); older than 6 weeks → stale styling; what is still
  calibrating (spec/ONBOARDING §4).
- **Health cap** (injury answer, or the over-45 advisory not yet acknowledged, #195): a status card states the
  cap (threshold, 100 % of CP, in watts), why, and where to lift it; the **planner's proposal respects it** (e.g. "Tempo 2×10′ (capped)"), the profile draws the cap line,
  and the reasons say so. A medical stop replaces the whole proposal card (§5.8).
- **Symptom question**: every check-in ends with the red-flag question (no default); Start needs it answered. The
  check-in also asks the *changes* and *unwell today* questions (§5.8, #194, #196); a yes to unwell replaces the
  proposal with a rest proposal.
- **Pre-filled vs confirmed**: yesterday's answers are drawn as dashed outlines until the rider confirms
  ("Same as yesterday") or changes one; confirmed answers are filled.
- **Last 7 days**: seven day cells (done / rest / today; the number is labelled "TSS") — context, not navigation.

### 5.2 Ride — *"Am I doing it right, how long, what next?"*

Fixed anatomy at every size (geometry does not change between phases or states; only contents do, P4).
~~The round-1 anatomy below~~ superseded by §16.1 (content-sized status-bar groups with one device pill; hero rows
`row 1 + caption | T1 | band`; "next" pinned on the road; the coach lane a fixed grid row) — kept for reference:

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

~~Rows 1–5 exist in every template with the same heights~~ superseded by §16.1: the three hero rows exist in every
template with the same heights; a template that has nothing for a row leaves it empty. The side and the coach lane
are identical in every phase.

**Phase templates** (what fills the hero; derived from the current block, P3):

| Phase / block | ~~Row 2~~ superseded by §16.1: Row-1 caption | Row 3 (T1) | Row 4 (T2) + sub-line | Why |
|---|---|---|---|---|
| **Warm-up / ERG ramp** | left in block | block countdown | target now + band; sub: "↗ to 210 W" | The target is moving; the rider needs to know it keeps rising and to where. |
| **Steady work (ERG)** | left in block | block countdown | target + band with actual marker + Δ chip | In ERG the band is a compliance check; cadence (side) is the lever. |
| **Short on/off (≤ 60 s reps)** | left in rep | countdown | target + band; row 1 shows ON/OFF + rep pips | Reps are counted, not read; pips are glanceable. |
| **Sprint (free / non-ERG)** | power | **actual power** | **fill bar** of actual vs target (target tick at 100 %) + countdown | The rider steers power; the fill shows "enough?" without a second number to compare. |
| **HR-guided recovery** (`hr<`) | HR below 118 in | **range or estimate** (P7, below) | "15 bpm to go" + recovery curve with a live dot, axes and the band; sub: "block ends between 2:00 and 4:00" | The question is "when can I go?", with honest uncertainty and the hard limits. |
| **Cool-down** | left in block | block countdown | target + band; ~~row 5: "ride complete — Review opens automatically"~~ superseded by §16.1: the road's "next" label at the ride's end: "ride complete → Review" | Signals the end. |

**Recovery estimate (P7).** The core gives the posterior mean and a 90 % band of "time until HR is below the
target", clamped to the block's min/max. A clamped value is the block's **rule**, not a prediction, so the
screen says which event the number is about (core flags, #13; prototype `recoveryRead()`):

| Situation | ~~Row 2~~ superseded by §16.1: Row-1 caption | T1 | Row 4 sub-line |
|---|---|---|---|
| fit not ready yet | HR below 118 in | "fitting…" | block ends in m:ss–m:ss |
| HR predicted below target before the minimum | ~~**block ends at its minimum in**~~ superseded by §16.1: **block's minimum in** | time to the minimum | HR due below 118 before then |
| prediction inside the block's limits, band ≤ 30 s | HR below 118 in | ≈ mean, band "1:01–1:12" under it (the backend's interval, never widened or narrowed for display) | block ends in m:ss–m:ss |
| band > 30 s | HR below 118 in | the band ("1:40–3:40") at range size (§6.2) | block ends in m:ss–m:ss |
| band reaches the maximum | ~~HR below 118 in · or at the block maximum~~ superseded by §16.1: HR < 118 · or block max | the band | block ends in m:ss–m:ss |
| HR lost (ADR-0001 §5) | ~~HR lost · block ends at its maximum in~~ superseded by §16.1: HR lost · block max in | time to the maximum | the chart stops its forecast; the last HR is drawn hollow (stale) |

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
("Easy · after Threshold 1/2"; a label equal to the phase word is not repeated: "RECOVER rep 1/3"), rep pips,
the **cadence chip whenever cadence is out of range — in every density**, and "capped ≤ 280 W" while a cap is
active.

**Cadence salience (the ERG lever).** Out of range is coloured and weighted like the power band (cool ▲ spin
faster / warm ▼ ease cadence), the cadence slot gets a coloured edge, and row 1 carries a T3 chip. Hysteresis
keeps it from nagging: the state leaves the band only at lo − 3 / hi + 5 rpm and returns at lo / hi; a single-value
target ("90rpm") counts as 85–95 (#13).

**States** (overlay a template without moving it):

| State | What changes | Input (§7.2) |
|---|---|---|
| Coach asks (`pending`) | Tone (§7.3). Lane: verb and the **result in watts** on one line ("Ease off to 245 W?"), the band and % in the grammar line ("lean −3 % · Coach = accept · Keep = keep plan"), one short reason (≤ 24 characters), band word as a small tag, two **equal-weight** buttons, **numeric deadline** ("keeps plan in 24 s"); on the road the affected work blocks get a **thick dashed outline at their proposed height**, on top of any solid (already applied) outline, and a T3 label on the road states it ("proposed −3 % · 1 block") — readable from the saddle, not 13 px chrome | Coach = accept · Keep = keep · silence = keep |
| Coach applied (`auto_applied`, opt-in) | Tone. Lane: "Eased off −5 %" + reason + "**Back to plan · 11 s**"; applied blocks **solid** outline; the target's sub-line shows "coach −5 %" on the changed blocks | Keep (within the window) = back to plan · Coach does nothing |
| Trainer lost | Warning tone. Chip ⚠ NEO "lost"; row 4: "**⚠ Trainer lost 0:12**" / "paused 2:47 into block · ride saved" / "reconnecting (try 3)"; an HR-recovery forecast is frozen (~~"paused · then ends at its maximum in"~~ superseded by §16.1: "paused · block max in"); actual power, cadence and W′ balance "—"; countdown frozen and dimmed (ADR-0002 §6) | — |
| Trainer lost ≥ 30 s | Row 4 adds what to try ("power-cycle NEO · close Zwift") and **End…**, which opens the End dialog (Keep riding focused); reconnecting continues behind it | End (bindable) → End dialog |
| Trainer back after ≥ 2 min | The re-entry policy (§6.6) applies: row 4 "↻ Back after 3:10" / "re-warm-up 2:48 · then this interval from its start" | — |
| Trainer reconnected | Row 4 for the 5 s soft start: "Reconnected · easing in to 253 W" / "workout resumed" (or "pedal to resume" at 0 rpm); then the chip shows "NEO back · 0:14 gap" for 10 s | — |
| Trainer lost at a block boundary | The block shown is the one the core paused in (transitions inside the 2 s detection window stand, ADR-0001 §5): "paused at 0:00" of the new interval; on reconnect the soft start goes to *that* block's target | — |
| HR lost | Chip ⚠ HR "lost"; HR slot "—" + "HR strap lost"; lane: "HR lost: coach can't push" | — |
| Both lost | Row 4: "⚠ Trainer + HR lost · paused at m:ss"; stays paused until the trainer is back and cadence > 0 | — |
| ERG released (cadence < 40 rpm 2 s) | Row 4: "⚠ ERG released · spin up to 60 rpm" / "timer runs · target returns at 60 rpm" (the block timer keeps running, ADR-0002 §7.4); cadence slot "▲ spin faster" | — |
| Paused (user) | The plan dims (countdown, next, road); **the body stays live** (HR, cadence); row 4: "Paused 0:42" / "Resume continues the plan · trainer free". While paused the trainer is at 0 % resistance; ERG re-engages with the soft start. **A pause ≥ 2 min goes through the re-entry policy** (§6.6): "Paused 6:10" / "Resume: 3-min re-warm-up first" / "then this interval from its start" | Pause |
| "How did that feel?" (calibrating, after a hard block) | Lane: "How was that?" with **nothing preselected**. **One mapping, no inversion:** a click or tap selects exactly the word it shows; the Coach button (key or remote) steps easy → right → hard; Keep skips. The choice **saves 3 s after the last input**, with a live countdown ("saves “hard” in 2 s") and the confirmation tone; no input within 12 s records nothing. Feeds the athlete model (#18) | click = that word · Coach = next · Keep = skip |
| Symptom reported (§5.8) | The ride screen is replaced by "Workout stopped · trainer released": "Stop riding and rest", the emergency line, what to do, the ride is saved; the only action is **Save and close the ride**. The medical stop is now active everywhere | Report symptom (binding) or End dialog |
| Free ride (medical stop, §5.8) | Phase word FREE RIDE; row 4 "No targets · coach off" / "your choice, logged · flat-road resistance" / "stop at any chest pain or dizziness"; the lane says "Coach off: free ride" | Pause, End |
| Finished | Auto-navigate to Review (#6) | — |

### 5.3 Ride-screen configuration (replaces the layout editor, §9)
The rider chooses what the **two side slots** show (the same in every phase; from the metric catalogue: HR,
cadence, W′bal, DFA-α1, HR vs typical, power 10 s, kJ) and a **density**: *Focus* (hero + the first slot + coach
+ road; the second slot's space stays reserved so the coach lane is in the same place in every density), *Standard* (default; the idle coach lane shows W′ balance quietly at T3 and the
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
active, "starts with 8:00 warm-up". **Start** is enabled when the trainer is connected. It is read from the
saddle, so it uses the ride tiers (T3+ title, T3 rows, 72 px buttons) and every button shows its bound key or
remote button; the usual choice is primary and focused, "Start without HR" never is. A free ride says so.

### 5.5 Review — *"Did it work, and was the coach right?"*
Order follows the rider's questions:
1. **Intent verdict** (headline): one line per part of the workout, from the core's per-interval stats (#12):
   "Threshold 2 × 4:00 — 94 % on target, against the eased target (plan −5 % by coach)", "VO2 sets — 15 of 15
   reps", "HR recoveries — 3 of 3 within your usual τ". Then the coach (applied / not taken) and data gaps
   for **each** sensor ("0:04 without power, excluded from on target · 0:17 without HR, excluded from the coach and
   recoveries"). **"On target" has one definition everywhere** and it is printed under the verdict: share of
   ERG seconds within ±5 % of the target the rider was given, settling excluded (ADR-0002 §7.8).
2. **Two inputs** (each ≤ 1 tap): **RPE 1–10** with word anchors, and **rate each coach call**. Each call
   expands to its full reason and a **chart of the 3 minutes around it**, **zoomed to the data ± 15 %** so a −5 %
   target or a 9 bpm difference is visible: plan vs given target vs power, and HR with **your usual HR for that
   power** as a dashed line, the gap between them shaded and labelled ("+9 bpm vs usual").
3. **What the coach did**: a **coach timeline** under the ride chart (P20): every call at its time with verdict,
   amount, band and outcome ("applied (auto)", "accepted", "declined", "no answer → plan kept", "undone → plan");
   changed calls have a warm edge. The ride chart draws **the plan** (dashed) and **the target you were given**
   (solid, after coach changes) as separate lines, so an easing is visible as the gap between them; the
   work-interval table shows "266 → 253 W · coach".
4. **Evidence**: stacked linear-time panels sharing one x axis with time ticks (P05): **power as a line**
   over the plan and given-target lines, HR on an **80–200 bpm** scale (a
   0-based HR axis flattens the differences the coach reacts to), cadence; numbered coach markers; **gaps drawn
   as dashed-outline boxes** (hatching means "variable-length block" on profiles, so it is not reused) and never
   bridged; a legend; *Show as table* — per-minute rows **plus a row for every gap and
   coach call**, so nothing brief is sampled away. HR recoveries: τ **± its fit uncertainty**, your usual τ
   **± its spread**, the data the fit used, and the "slower" rule (≥ 1.15 × usual) stated. **Uncertainty changes the
   conclusion:** a recovery is "within" or "slower" only when the fit's ± stays on one side of the rule, otherwise
   "too close to call" — and the verdict counts it that way ("1 within · 2 too close to call"). Work-interval table
   with the same "on target" measure as the verdict (#12).
5. **All numbers** (collapsed by default): duration, kJ, NP, TSS, IF, on target, and metrics only when the
   core marks them valid for this session (e.g. decoupling only for steady rides, #13).
6. **Analytics**: time at power (axes: W bins, minutes, mid ticks), power–duration curve (5 s – 60 min, log x with
   ticks, W scale with a mid tick), each with *Show as table*. Point-by-point keyboard inspection of every chart: #71.

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
Eight steps (spec/ONBOARDING §1; §15.1 added *Training for*): About you · Background · Current numbers · Health
check · Training for · Devices · Baseline · Your starting model. A mode without the rail (§3); progress
persisted after every step, so a relaunch shows **"Welcome back — continue at Health check (4 of 8)"**.
- Every step states what **Skip** does ("Skipping keeps wide priors: CP ± 60 W, the coach stays conservative").
- **Current numbers**: value, *what it came from* (ramp test / 20-min test / app estimate / guess) and
  *when* (a date); the uncertainty shown grows with age and source.
- **Health check** (spec/ONBOARDING §1.5; item list **informed by** the PAR-Q+ and the ACSM 2015
  pre-participation screening, knowledge-pack entry #77; it is crank's own screen, **not** the PAR-Q+ and never
  called that, #193). Every question starts **not answered**; Continue stays disabled until
  all are answered, and with any question unanswered crank proposes no plan. Answers and consequences:

  | Question | Kind | If "yes" |
  |---|---|---|
  | Chest pain, pressure or tightness — at rest or when exercising? | **red flag** | medical stop |
  | Fainting, or dizziness that made you lose balance, in the last 12 months? | **red flag** | medical stop |
  | A racing or irregular heartbeat together with dizziness or breathlessness? | **red flag** | medical stop |
  | Unusual breathlessness at light effort? | **red flag** | medical stop |
  | A heart condition a doctor told you to limit exercise for? | **red flag** | medical stop |
  | Medication that changes your heart rate? | HR | HR zones, HR-guided recoveries and HR-vs-usual coaching off; targets by power and feel; no power cap |
  | An injury that limits hard efforts? | cap | cap at threshold (100 % of CP) and no sprints |
  | Pregnant? (asked of everyone: sex is optional) | medical stop (#196) | medical stop, lifted like any other by a dated clearance; message: consult a health-care practitioner, with the ePARmed-X+ link, not the generic red-flag "see a doctor" copy; not one of the five red flags, so not part of the daily re-check |
  | Do you regularly do vigorous exercise? (only when age > 45, #195) | advisory | on **no**: an advisory, not a stop: "consult a qualified exercise professional before vigorous or maximal efforts"; the rider acknowledges it, the acknowledgement is logged, and until then the **health cap** applies |

  **The screen's footer** says it is informed by the PAR-Q+ and the ACSM 2015 guidance and is not either, and links
  to the official **PAR-Q+ / ePARmed-X+** (https://eparmedx.com) for a full screening. The full PAR-Q+ is not
  shipped: its licence forbids changes and it would duplicate this screen (#193).

  **Over-45 advisory (#195).** Age is the About-you answer. Until the advisory is acknowledged the planner
  proposes nothing above the health cap (see **Health cap** in §5.1 and §15.2: one cap, one set of numbers and one
  start guard, whatever raised it) and says why ("Capped until you've read the exercise advice"). Recording the
  acknowledgement lifts the cap; it can be re-opened in Settings → Health. Evidence grade D (form advice,
  knowledge-pack entry #77). A cap raised only by this advisory follows every rule of the injury cap, including
  *medical stop plus cap = no ride at all*.

  **Medical stop — a global invariant, not a Today feature.** Any red flag (at onboarding, at a re-check, or a
  symptom reported in a ride) puts the stop on record. **While it is active, no planned or coached ride starts
  from any path**, and the coach proposes nothing anywhere. Start paths, all routed through one guard (prototype
  `requestStart()` in `safety.ts`; the Ready check refuses a planned ride under a stop as a second layer):
  Today's proposal · Workouts "Ride this" · the Resume sheet (Resume and "Ride the rest") · History "Ride again"
  · any coach or planner proposal · MCP and CLI starts. In production the **core enforces it** at its single
  ride-start command, so no client can bypass it (**#81**). Blocked starts show why (`workouts-start-blocked`,
  `resume-start-blocked`, `history-start-blocked`). The screen says *see a doctor before
  training*, with "if you have chest pain now, call emergency services". **The coach proposes nothing and stays
  silent** — there is no capped plan and no one-click override. Riding stays possible only as a **deliberate,
  logged free ride** (no plan, no targets, coach off, flat-road resistance): "Ride without a plan…" opens a
  confirmation that must be ticked ("I understand crank isn't advising this ride, and I'll stop at any symptom").
  **A medical stop plus a health cap means no ride at all**: the free ride has no targets, so the trainer can't
  hold the cap; "Ride without a plan…" is not offered and the guard refuses it on every path (the medical-stop
  gate wins, `today-stop-and-cap`).
  The stop is lifted only by **recording a medical clearance** in Settings → Health — the date of the check (not
  in the future), who cleared the rider, and an explicit statement — and every change is kept in a log.
  **Re-check:** once a day, before the first start, the rider answers **all five red flags** again (the same
  list as above, each yes/no, no default; "Answer all 5" until complete). It is part of the Today check-in, and
  any other start path that comes first (Workouts, Resume, History) shows the same check before the ride
  (`start-recheck`). Any "yes" puts the stop back.
  **Daily check-in questions beyond the five red flags (#194, #196).** Asked in the same Today check-in, no
  default, Start needs them answered:

  | Question | Kind | If "yes" |
  |---|---|---|
  | Since your last ride: any new or changed medication that affects heart rate, or an injury that limits exercise? (#194) | changes | routes to the impact questions I1 (medication) and I2 (injury) with their effects above; the answer is logged; **any yes also re-asks all five red flags in full** (their answers are cleared and must be given again) |
  | Unwell today (cold, fever, infection)? (#196) | day state | **no planned or coached ride today** and a **rest proposal**; a free ride stays available only as a logged choice, as under a medical stop (and not when a health cap is active, same rule as stop plus cap); the answer is not carried over and resets the next day |

  Clearance validity (12 months) is unchanged by the changes question. An unwell day is not a medical stop: it needs
  no clearance and leaves no standing state; the core guard (#81) refuses planned and coached starts on every path
  for that day, like the stop.
  **During a ride:** a symptom is reported with one deliberate action — the **Report symptom** binding (default
  `!`, bindable to a remote button) or "Stopping because of chest pain, dizziness or feeling unwell" in the End
  dialog. The workout stops at once and the trainer is released (production: ERG to 0 % resistance, workout
  closed, #81), the medical stop is set, and the screen gives the guidance: emergency line, "stop pedalling, sit
  down", the ride is saved (`ride-symptom-stop`, `ride-end-dialog`). Nothing on that screen resumes the ride. Screens `onboarding-health-stop`, `today-medical-stop`,
  `today-free-ride-confirm`, `ready-free-ride`, `ride-free`, `settings-health`.
  **Settings → Health** also shows the over-45 advisory (acknowledged or not, with *Read it again*) and the
  "Pregnant?" answer, each editable with the change logged; an unwell answer is today's only (§5.1).
- **Devices**: pairing shows **live readings** (W, rpm, bpm) so "pedal a few strokes" visibly works.
- **Baseline**: import / assessment week / passive learning, each with what it costs and how soon it is confident.
- **Your starting model**: estimates with value ± sd, a relative-uncertainty bar, source, what will narrow
  them, and what the coach does meanwhile.

### 5.9 Settings
Sections: **Devices** (trainer + HR, protocol, status, battery, pair / forget; one-central note),
**Training profile** (§15.1),
**Athlete model** (estimates ± source, manual override with provenance "self-report"), **Coach** (autonomy,
§7.1), **Controls & sound** (the five actions and their bindings with *Learn* for a remote, §7.2; tones and
volume, §7.3), **Health** (§5.8: clearance record and log, over-45 advisory, pregnancy answer), **Ride screen** (§5.3), **Appearance** (mode dark / light / auto, preset, classic zones), **Background** (§15.3).

---

## 6. Ride screen specification

### 6.1 Grid
Status bar 64 px (56 px at ≤ 800 px height). Hero and side split 62 / 38. Road ahead 22 % of height
(min 128 px). ~~Hero rows 1–5 have fixed heights from the type tiers.~~ superseded by §16.1: hero rows `row 1 (+ caption)
| T1 (1fr) | band`, row 1 a fixed T3 line + chip room, the band row from the tiers. All regions are fixed by CSS grid with
`minmax(0, …)`; contents are sized by the visual budget below. **Single-line rows never wrap** (~~rows 1, 2, 5~~ superseded by §16.1: row 1
with its caption, the road's "next" label (§16.1), slot labels, the
coach verb, reason and deadline): every string there has a maximum length (coach reason ≤ 24 characters; a
recovery caption ≤ 27, the idle lane note ≤ 28, §16.1).
**Banners** in row 4 are one bold line plus at most two short lines, each ≤ 36 characters so they fit on
one line at 1024. The harness fails on any clipped `[data-fit]` text, including the whole coach lane.
**Screens read from the saddle** — Ready check, Resume sheet, "ride screen restarted" — use the ride tiers too
(T3+ titles, T3 text, 72 px buttons) and show the bound key / remote button on every action.

### 6.2 Visual budget (legibility as visual angle)
Character height *h* (cap height) at distance *d* subtends θ ≈ *h*/*d*. Ergonomic standards set 16′ of arc
as the minimum and 20–22′ as preferred for reading (ANSI/HFES 100-2007, ISO 9241-303). Glance targets are
multiples of that. Reference devices: a 13–15″ laptop at default scaling (≈ 5.0 pt/mm) at **1.0 m**, a
9.7–10.2″ tablet (≈ 5.2 pt/mm) at **0.8 m**; cap height ≈ 0.72 em. Font px = θ · *d* · pt/mm / 0.72.

| Tier | Use | Target | Laptop 1440×900 @ 1.0 m | Tablet 1024×768 @ 0.8 m | Prototype rule |
|---|---|---|---|---|---|
| **T1 glance** | countdown, recovery range, sprint power | ≥ 90′ (1.5°) | 182 px | 151 px | ~~`clamp(150px, 21vh, 240px)` → 189 / 161~~ superseded by §16.1: `clamp(150px, min(24.5vh, 17.2vw), 260px)` → 220 / 176 |
| **T2 glance** | target, HR, cadence | ≥ 50′ | 101 px | 84 px | `clamp(84px, 11.4vh, 128px)` → 103 / 88 |
| **T3+ read** | next block, coach verb, banners | ≥ 20′ | 40 px | 34 px | `clamp(34px, 4.5vh, 44px)` → 41 / 35 |
| **T3 read** | labels, captions, coach reason, deadlines, status-bar values | ≥ 16′ | 32 px | 27 px | `clamp(27px, 3.7vh, 36px)` → 33 / 28 |
| T1r range | the recovery band when shown as the hero ("1:40–3:40") | ≥ 55′ | 117 px | 100 px | `0.62 × T1` → ~~117 / 100~~ superseded by §16.1: 137 / 109 |
| T4 chrome | key hints, axis ticks, band word, road labels — never needed mid-effort | — | 15–18 px | 15–18 px | fixed px |

Achieved at the prototype's sizes (laptop / tablet): ~~T1 94′ / 96′, T1r 58′ / 59′~~ superseded by §16.1: T1 109′ / 105′, T1r 68′ / 65′,
T2 51′ / 52′, T3+ 20′ / 21′, T3 16.5′ / 16.9′.
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
low-contrast composite fill never carries meaning on its own. **Rendered check:** `pnpm screens` computes, in the
browser, every chart mark's effective stroke colour (including opacity inherited from parents) composited over
its real background and fails below 3:1 — the token check alone missed an inherited opacity in round 3.

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
  to the Coach button (the screen listens to the rider's bindings itself, since the ride screen is unmounted)
  and taken **automatically after 5 s**; the End binding asks first, with Keep riding focused and Keep to back out; it re-subscribes from a snapshot (ADR-0001 stream
  snapshot). A second crash within 60 s adds **End ride and review** as a second, unfocused choice — never the
  default.
- **Process crash, forced quit, OS sleep**: the record is saved every 5 s (ADR-0001 §6), so at most 5 s are
  lost. On the next launch, before anything else, a **Resume sheet** shows the workout, where it stopped
  ("31:12 ridden · Threshold rep 1 of 2, 1:40 into it"), what was saved, and a **re-entry plan keyed to how
  long ago it stopped and where**:

  **The same policy applies to every interruption** — a crash or restart, a user pause, a trainer loss — keyed
  to the time away (prototype `resumePlan()`; ride states in §5.2):

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
Ride input is ~~**seven abstract actions**~~ superseded by §17.6: **nine abstract actions**, **End set** and **Undo** added: **Coach**, **Keep**, **Pause**, **Skip**, **End set**, **Undo**, **Intensity − / +**, **End**
(always asks first; Keep riding is focused), and **Report symptom** (stops the workout at once — it is a safety
action, so it is never shared with another binding; §5.8). They work on every ride-time screen, including the restart and
long-loss states.

**Decision model — no input can reverse a rider's coach decision by accident:**
- **Coach only ever says yes** (accept a question; step the feel prompt). It never undoes anything.
- **Labels are actions**: an on-screen button does exactly what its label says, for mouse, touch, keyboard focus
  and remote alike; nothing on screen cycles or inverts.
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

**Bindings**: keyboard `Enter` / `Y` (Coach), `N` / `Backspace` (Keep), `Space` (Pause), `S` (Skip), `−` / `+`,
`Esc` (End), `!` (Report symptom). The lane shows the grammar for its state ("Coach = accept · Keep = keep plan").
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
| Wish answered (Today, §15.2) | **granted**: three rising notes · **changed** (adapted, deferred, capped): two short notes and a low one — then the saddle proposal states what changed |
Sounds never carry information that is not also on screen. The prototype plays them in interactive mode
(Web Audio); production uses the same table (M1 UI).

---

## 8. Window sizes
The sizes are defined in [`sizes.json`](sizes.json); the table below must list exactly those, in that
order (the `window-config` check fails otherwise), and `tauri.conf.json` must match the first (default)
and the smallest (minimum).

| Size | Target | Rules |
|---|---|---|
| **1440×900** | laptop/desktop landscape (primary) | as specified |
| **1024×768** | tablet landscape | same anatomy and rows (no reflow by breakpoint); type from the tiers (§6.2); the status bar keeps every control with its label (Pause keeps its word); Today keeps every word label; ~~the editor stacks the summary under outline + text~~ superseded by §16.3: the library is a drawer below 1280 px, so Outline · Text · Summary stay in one row; Today stacks (§16.2) |
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
| ONBOARDING §1.4 Goals | focus incl. weight, time per session, preferred ride types | training profile (§15.1): six foci (weight dropped), weekly budget, formats in the wish vocabulary |

Everything else in UX-SPEC / VIEWS (plot catalogue P01–P24, visual conventions, editor capabilities, the
non-functional rules) still applies.

---

## 10. The spike defects #1–#11, designed out
| # | Defect | Design element that prevents it |
|---|---|---|
| #1 | AI rewrite accept loses/misplaces edits | Editor **locks** during pending and diff (banner, all inputs disabled); Accept verifies node id + content hash, else re-requests (§5.7). Prototype: `editor/model.ts` (`lock`, hash check in `accept`). |
| #2 | Physiology in the UI | P8: every derived number comes from the backend; missing ones are contract additions (#12, #13, #14, #15, #18, #29) stood in by **one** fixture module (`mock/ext.ts`), never computed in a view. |
| #3 | Per-ride state not reset | One `RideSession` object (status, nudges, compiled, undo windows, sensor gaps, pause time) created on `ride.load` and disposed when the next ride loads (§5.2). Prototype: `session.ts`. |
| #4 | Unvalidated persisted data | Everything persisted (ride-screen config, theme, sound, bindings, onboarding progress) lives in SQLite behind the core (ADR-0001 §6) and is read and written through the core's commands (layouts, settings; #34); the UI parses what the core returns with a schema; invalid → default + notice (#90); Settings forms read a draft, Save validates, and Cancel / Reset never touch the saved state (#90); error boundaries at the top and around the ride (§6.6). In the prototype `persist.ts` (localStorage, read in `App.tsx`) stands in for the core's commands; it is not the production path. |
| #5 | structuredClone on reactive proxies | React state holds plain immutable values; the only snapshot path is `snapshot()` (production lint rule). Prototype: no clone of state anywhere. |
| #6 | Finished ride doesn't reach Review | `state: finished` → Review automatically, owned by the ride session (§3, §5.2). |
| #7 | 1024×768 unusable, layout ignores resize | Fixed anatomy and rows scaled by the tiers (§6.2, §8); strings in reserved rows have maximum lengths (§6.1); the harness renders every scene at 1024×768 and fails on overflow. |
| #8 | Layout-editor state bugs | Free grid editor dropped; slot config is a transactional draft with per-slot validation (§5.3). |
| #9 | No unit tests for editor ops | Editor model (tree ops, undo/redo, DSL ↔ tree) is a pure module with tests in production (§5.7); the prototype keeps that module boundary (`editor/model.ts`). |
| #10 | Ride-screen layout shift | P4: fixed grid and rows, ~~fixed-width controls~~ superseded by §16.1: controls that reserve their widest content, tabular numerals, reserved coach lane (a fixed grid row with reserved button slots and deadline bar) and band row; states recolour/fill, never re-flow (§5.2, §6.1). The harness compares every ride scene's region boxes and fails on any move (§16.4). |
| #11 | Widget settings seeded from props once | Settings forms read the draft directly; the ride screen reads the saved config live (§5.3). |

---

## 11. Contract additions this design needs
Per-lap stats (#12); thresholds: target band, settle flag, cadence band, τ-slower, metric validity (#13);
live cursor (#14); planner variants (#15); athlete model, "how did that feel" taps (#18); and #29: coach
revert, short coach reason, coach offset, autonomy + bounds, pause reason / reconnect attempt / gap / soft
start / ERG released in `RideStatus`, ride library, post-ride RPE, devices with live readings, structured
planner reasons and rewrite rationale, fit quality, health-cap state, onboarding progress, ride-screen config,
a non-ERG sprint and long sensor losses in the mock; training profile, wish, wish outcome and structured
reasons with rule IDs (§15.4, #59); live workout edits (§17, #240: `RideStatus.{edit_seq, block_count, undo}`, plan queries, edit commands). Engine/ADR: resume (#50), sleep (#51), physical type
sizing (#52), remote test (#53).

## 12. Prototype and screenshots
- `design/prototype/`: Vite + React 19 + React Compiler + TypeScript strict, against `@crank/shared`. A design
  artifact; components may be reused by the production UI, the fixtures in `src/mock/ext.ts` may not.
- Safety: `pnpm -C design/prototype test:safety` drives the prototype with Playwright and fails unless every
  start path is blocked under a stop (only the ticked free ride starts), the re-check asks all five flags with no
  default, and the in-ride symptom action stops the workout and sets the stop. `?stop=1` starts the prototype with
  a stop on record.
- Start screen (cold start, #62, #138): shown only while the core boots (`?boot=<ms>`, `?boot=fail[:<ms>]`; the
  mock core is otherwise ready at once, so without them no start screen is shown). Its launch motion is **F · Spin-up**
  ([BRAND.md](BRAND.md) §F · Spin-up, chosen over A · Slice's letter settle in #131): the app mark spins up into a
  ring, which opens into the wordmark's C; R A N K come out and one slice cuts the word. It plays once from the
  first frame and rests on the wordmark whatever the boot does — never gated on readiness, never held (the
  `?hold` variant is a record in design/brand only), never restarted by "Try again". Ready at any time: the
  destination is live and focused at once, the start screen is inert and cross-fades out over 200 ms while the
  motion runs on at 1:1 (no cut at ≤ 150 ms, no rush after it). Reduced motion: no script, the settled
  wordmark, removed at once. The settled frame is A · Slice's, pixel for pixel. Status line at 1.2 s from boot;
  a failed boot replaces it with the alert (P6). `pnpm -C design/prototype test:start`
  drives all of it on a fake clock, including the ring's slit (no hairline before the faces are 1 px apart, #142).
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
  limit tracked in #78, not the design.
- Known prototype limits (all tracked): Easier / Harder act on the wish (§15.2), production variants are #15; past rides in History
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
7. **Safety** — `onboarding-health-stop`, `today-medical-stop`, `today-free-ride-confirm`, `ride-free`,
   `settings-health`: does a red flag ever lead to a proposed ride or a one-click way back?
8. **Honest numbers** — ranges, sources and ages wherever estimates appear (recovery, Today, History,
   onboarding)? Do stale values look stale? Do health answers start unanswered and does a cap change the plan?
9. **Evidence** — can Review's charts substantiate its verdict (axes, markers, gaps, tables)?
10. **Defects** — does §10 make #1–#11 impossible, or only less likely?
11. **Supersessions** — are the §9 changes to UX-SPEC/VIEWS acceptable to the owner?

## 14. Open questions (not blocking the design)
- What the NEO does on BLE loss in ERG (standalone mode?) — the soft start of §6.5 covers both answers;
  measured in HIL check 11 (ADR-0002, #23).
- Which bar remotes to recommend and their default binding (#53).
- Resume after a process restart needs the ADR-0001 amendment (#50).
- A referenced knowledge-pack entry for the health screen (#77); the mock's frozen timer on trainer loss (#78).

---

## 15. Addendum (#87): training profile, planner wish, Why? and Background

Status: **proposed**, round 3 (#87; round-1 reviews by astra and opus, the round-2 re-review by opus; resolution in §15.9). It is the design
gate for the owner requests #85 (training profile), #86 (planner wish) and #84 (help / about / background). It
extends §1–§14 and changes them only where §15.6 says so. Screens: `docs/design/screens/` (§15.7). The planner
and the coach behind these screens are #59; §15.4 records the contract fields they need.

### 15.0 Rider jobs and moments

| Moment | The rider's job | What they can give | What they need back |
|---|---|---|---|
| **First run** (onboarding) | "Know what I'm training for, without a plan builder." | one choice; optionally an event, a weekly budget, formats | what that choice changes, and how sure crank is about each part |
| **Life changes** (desk, any time) | "I signed up for a gran fondo / I'm back from a break." | edit the profile in Settings | before → after of what it changes; the next proposal uses it |
| **Before, at the bike** (kit on, remote in hand) | "I feel like hills today" / "just give me something" | one gesture, or nothing | the wish, the session, one reason and Start — readable from the bike, above the fold |
| **Before, at the desk** | "What would a long steady ride look like today?" | one click in the matrix, a time choice | the same, with every reason and its *Why?* |
| **After / any time** | "Why did the coach do that? Should I believe it?" | one tap on *Why?* | the claim, the evidence and its grade, the cut-off and its grade, sources — offline |
| **Curious** (rare) | "What does crank base its coaching on?" | browsing | the whole evidence base, by topic, with grades |

A wish is for **today's ride**. There is no "plan tomorrow": a wish the planner can't grant today is *deferred*
with a fixed expiry and is shown again on the day it was moved to (§15.2 lifecycle).

Constraints from §1 hold (glance, distance, hands on the bars, a probabilistic coach). Two more:

8. **A wish is a request, not a command.** The planner combines it with the check-in, the profile, the recent
   load and the athlete model; it may change it, and then says what it changed and why. **Safety is applied
   before the wish and never yields to it** (§5.8, #81).
9. **Evidence strength is part of the claim**, and so is the strength of the *cut-off* crank applies. A C or D
   rule, or a D cut-off, must look like one wherever it is shown.

### 15.1 Training profile — *"What am I training for?"* (#85)

**What it is.** The planner's **prior** for the mix of sessions and their progression: it shapes the week, it
does not fix a calendar. Each day the check-in, the wish and the load decide the ride (R-RDY-01 · A).

| Field | Values | Why this shape |
|---|---|---|
| **Focus** (one) | Endurance base · Climbing · Racing (crits, road) · Long events (gran fondo, sportive) · Time trial · General fitness & health | One prior is interpretable; several foci dilute each other. Each choice has a one-line description in rider words. |
| **Event** (optional) | name + date ("39 weeks away") | Orders the blocks; never a taper from fitness/fatigue curves (R-LOAD-01 · B). |
| **Your week, from now** | days (2–6) · **hours you can ride per week** (h / week) | A **budget**, not a target; each day's time comes from Today. It is *future availability*, unlike Background's "hours in the last 3 months", which pre-fills it in onboarding (one question, not two). |
| **Formats you enjoy** | the wish matrix's formats: Steady · Intervals · Hills · Race sim · Open ride | One vocabulary for profile and wish; enjoyed formats break ties between equivalent sessions. |
| **Coming back after 10+ days off** | on/off (+ since) | A temporary modifier, not a goal: a return block with widened uncertainty that ends by itself. **10 days is the athlete model's gap threshold** (spec/ONBOARDING §3) — one number everywhere; the model also detects it itself. |

**"What this changes"** (screens `settings-profile`, `onboarding-profile`) lists the prior one effect per line,
each with its evidence (*Why?* + grade where a pack rule exists, **D · convention** where it doesn't) and **the
conditions it needs** ("when: on days with 2 h+ to ride, budget 7 h/week"). **A C-grade line is worded as a
suggestion, never as a commitment**: "Optional: now and then a quality effort after 2 h or more, fuelled —
suggested, never scheduled on its own" (R-DUR-01 · C). When the budget or the return status removes the condition,
the line says so in place ("Not now: … needs 5 h/week or more (yours: 3 h)" / "not during a return block"). While
editing, the panel shows **before → after** against the saved version: new lines are tagged *new*, lines that
disappear are struck through as *no longer*. The goal-specific part is labelled convention (#143 for evidence).

**Versions.** Every Save creates a new profile version (*Save as version 4*); proposals record the version they
were planned with, and Review shows it ("planned with profile v3 · Long events"), so a later edit never rewrites
why an old ride was planned (§15.4).

**Onboarding** — step 5 of 8, **Training for**, *after* the Health check (a red flag makes goals moot) and before
Devices. It shows only the focus choice, the effects panel and Back / Skip / Continue in the first viewport at
1024×768; event, week, formats and "coming back" are behind **"Add an event, your week and formats (optional)"**.
Continue saves; **Skip** keeps general fitness & health ("a balanced week"). The draft is saved on every change
(like the step reached, §5.8), separately from the committed answers: relaunching restores the draft on this
step; Skip discards it.

**Reconciling spec/ONBOARDING.md §1.4 ("Goals")**

| ONBOARDING §1.4 | Now |
|---|---|
| event and date (optional) | **Event**, unchanged |
| focus: endurance / climbing / crits / general fitness / weight | **Focus**, six choices; *crits* → "Racing (crits, road)"; added *long events* and *time trial* (#85); **weight dropped** as a focus (below) |
| days per week and time per session available | **Your week** (days + hours per week, future); time *per session* is Today's daily time choice |
| preferred ride types | **Formats you enjoy**, in the wish vocabulary |

**Conventions challenged**

| Convention (where) | Verdict | Reason |
|---|---|---|
| Plan builder → fixed calendar to the event (TrainerRoad, TrainingPeaks) | **Change** | A calendar computed months ahead ignores how the rider is; the evidence supports a structure modulated daily (R-RDY-01 · A). |
| Rider "type" / phenotype labels (Xert, TrainerRoad) | **Drop** | Profiling uses CP, W′, Pmax, durability (R-VLA-01 · B); a label adds no information. |
| Weight loss as a training goal (ONBOARDING §1.4) | **Drop as a focus** | It doesn't change the session mix differently from general fitness; crank gives no diet, weight-loss or medical advice (the in-ride fuelling target is a training guideline, R-CHO-01 · A). |
| Taper scheduled from the form curve (CTL/TSB) | **Change** | R-LOAD-01 · B; a lighter final week is labelled convention until #143. |
| "Return from injury/break" as a plan type | **Change** | A state with an end, not a goal; injury goes through the health check (§5.8). |
| Hours-per-week budget | **Keep** | The cheapest honest input to the weekly structure. |
| Several concurrent goals | **Drop for v1** | One focus + one optional event keeps the prior explainable. |

### 15.2 The planner wish — *"I'd like to ride this today"* (#86)

**Three separate choices.** *How long* is Today's **time** (30 min · 45 min · 60 min · 1:30 h · 2 h · 3 h — 2 h and
3 h reach the profile's long rides); *how hard* and *how structured* are the matrix. Duration is never a format.

- **Intensity of the main effort** (rows, harder upwards): **Easy** (below VT1) · **Tempo** (VT1 to below CP) ·
  **Threshold** (at CP) · **VO2max** (above CP) · **Sprint** (all-out ≤ 30 s). The row names the main effort, not
  every minute (a threshold ride still warms up). Anchored on the rider's thresholds (R-GEN-01 · B), never zone
  numbers; "sweet spot" is a convention (D) inside Tempo.
- **Format** (columns): **Steady** (one continuous effort, ERG) · **Intervals** (repeats, ERG) · **Hills** (climbs
  in SIM) · **Race sim** (surges and finales, ERG) · **Open ride** (no targets, the rider sets the power; the coach
  watches the row as a ceiling — not the medical-stop free ride of §5.8, which is never offered here).

**19 of 25 cells are choices**; the six that describe no session (e.g. *VO2max · Steady*, *Easy · Race sim*) are an
empty dot, not a disabled button. Each cell is one session family the planner sizes to the time (e.g. *Easy ·
Intervals* → easy spin with cadence drills; *VO2max · Intervals* → 30/15 sets, R-INT-01 · B).

**At the desk** (before or after the check-in): the full matrix, **one click per wish** (again, or *Let the coach
pick*, clears it); every reason with its *Why?*; ~~Shorter / Easier / Harder / Edit on the proposal card~~ superseded by §16.2: Easier /
Harder on the card, every other reason and *Edit a copy* behind "Why today: every fact" (§16.2). The cells
are plain buttons in reading order (Tab), each with a 48 px target and a spoken label that includes its state
("Tempo · Intervals: plan", "… : adapted", "… — above your cap").

**At the bike: the saddle proposal.** Once the check-in is confirmed and today's red flags are answered, ~~the
proposal moves to the top of Today as one card~~ (superseded by §16.2: the one proposal card switches to its saddle
form in place, beside the matrix at 1440, under the check-in's summary line at 1024) with the ride tiers (§6.2), which the harness **checks inside the
viewport** at 1440×900 and 1024×768 (scenes `today-wish-*`, viewport captures). ~~The round-2 layout:~~ superseded by
§16.2 (no *Other wish* button: S / Pause open wish mode, hinted on the card) — kept for reference:

```
┌ saddle proposal (full width, above the fold) ─────────────────────────────────────┐
│ ▌Wish VO2max → Tempo · Intervals, 2 steps easier        T3+  (the outcome line)    │
│  Tempo 2×10′ · 40:00                                    T3+  (the session)          │
│  Legs dead, sleep short: two signals against hard work today.  [Why? B · good | D cut-off] │ T3
│  ▁▁▂▂██▂▂██▂▁  (profile, cap line when capped)                                     │
│  [⏎ Start]  [Ride my wish]  [S Other wish]  [− Easier]  [+ Harder]   72 px class   │
└────────────────────────────────────────────────────────────────────────────────────┘
```

**One gesture, every cell, from the bars** (the §7.2 actions, same meanings):

| Where | Action | Effect |
|---|---|---|
| ~~Saddle proposal~~ superseded by §16.2: the proposal card's saddle form | **Coach** | Start (ignored for 1.5 s after a wish change, §7.2 debounce) |
| | **Intensity − / +** = **Easier / Harder** | the wish one row down / up from **what is proposed**, within its format (bounded, skipping non-choices) |
| | **Keep** | back to the coach's pick |
| | **Skip** or **Pause** | open **wish mode** |
| Wish mode (sheet, ride tiers) | **− / +** | cursor one row (effort) |
| | **Skip** | next format (wraps: it is a picker, nothing applies until Coach), nearest intensity |
| | **Pause** | next choice in reading order (wraps) — a **two-button remote (Coach + Pause) reaches all 19 cells** |
| | **Coach** | use this wish → re-plan → tone (§7.3) → back to ~~the saddle proposal~~ superseded by §16.2: the card's saddle form |
| | **Keep** / **End** | cancel; the wish is unchanged |

Wish mode shows the cursor's name at T3+ ("Threshold · Hills"), the matrix with the cursor outlined (outline + the
name, never colour alone), and the grammar line with the bound keys. Focus sits on the sheet, not on a button,
so Space steps instead of pressing *Use* (§7.2: focused controls win). **Easier / Harder are the wish's − / +**:
one mechanism, one outcome table; the former *Other* variant is wish mode. Silence keeps the coach's pick.

**Two marks, one grid.** The wish is outlined ("wish"), the proposal filled ("plan"; "coach" without a wish). A
granted wish reads **"✓ wish"**; an **adapted** wish reads **"≈ wish"** (same cell, smaller dose) — never ✓.

**How the planner answers** (the outcome line + one reason on the saddle card, all reasons on the desk card):

| Outcome | When | Outcome line | Rider's way out |
|---|---|---|---|
| **No wish** | — | "Coach's pick: VO2max · Intervals" | Easier / Harder / ~~Other wish~~ superseded by §16.2: the matrix or wish mode (S / Pause) |
| **As wished** | nothing argues against it | "✓ Your wish: Threshold · Intervals" (+ a session note with its rule) | Start |
| **Adapted** | same cell, smaller dose, for a readiness reason | "≈ Wish, shortened: 60 min, not 2 h" · "Sleep short: long continuous efforts are cut first" (R-RDY-03 · A, cut-off D); *Why today* says "Shortened to 60 of your 120 min" | **Ride the full 2 h** (one tap, logged) |
| **Deferred** | readiness or load argues against hard work | "Wish VO2max → Tempo · Intervals, 2 steps easier" · "Legs dead, sleep short: two signals against hard work today" (R-RDY-02 · B, cut-off D) · "Your wish moves to Thu 15 Oct" | **Ride my wish** (logged) |
| **Blocked by safety** | medical stop; health cap | stop: **no matrix, no proposal** (§5.8); cap: the cell is not a choice ("cap"), Harder stops at the cap; an API request for it → "⚠ Cap: …" | none — lifted only in Settings → Health |
| **Unwell today** (§5.8, #196) | the rider answered "Unwell today?" yes | no matrix, no planned or coached ride, a rest proposal | a free ride as a logged choice (not while a health cap is active: stop-plus-cap rule); nothing to lift: the answer resets the next day |

Rules: facts first, then the rule (ID + evidence grade + cut-off grade, *Why?*), never a score. **A deviation needs
an A/B rule for its direction; its cut-off (how many signals, how many minutes) may be convention (D) and is shown
as such** — this is how "C/D alone may not deviate" squares with D cut-offs: the evidence says *which way*, the
convention says *where*, and because the *where* is convention the rider can override it. "Ride my wish" is gated
like Start (check-in and red flags done), is logged with the reasons shown, keeps the in-ride bounds of §7.1, and
exists **only for readiness/load deviations, never for safety**. The rider's time choice is an input, never
adapted for convenience; only a readiness rule shortens it, and then it is overridable.

**The health cap is on actual power, not just targets** (architect decision; §5.8 cap = threshold, 100 % of CP,
no sprints). Option chosen: **(a) non-ERG formats are unavailable under a cap** — only ERG holds the rider's power,
so *Hills* (SIM) and *Open ride* (free) are "cap" cells, and so is *Race sim* (its surges are above CP). Rejected:
(b) an engine-enforced ERG takeover at the cap in SIM/free, because it changes the trainer's mode under effort
without the rider's action (a surprise at the worst moment, §1 constraint 6), needs new ADR-0002 §7 behaviour for
a temporary state, and a capped rider loses little by riding ERG for a few weeks. The invariant — **actual power
above the cap is prevented, or else flagged and logged** — is enforced at three points:
1. **Plan:** the planner clips the wish space before planning; every delivered block is ERG and **every planned
   target is ≤ 97 % of the cap** (272 W under a 280 W cap; `test:safety` checks every cell and every "ride my wish").
   The margin exists so that a threshold session doesn't sit exactly on the cap, where normal ERG ripple would
   trip the flag below.
2. **Start:** the single start guard (§5.8, #81) refuses any workout with a target above the cap or a non-ERG
   block, from every path including Workouts and the free ride ("Above your injury cap" or "Above your cap until you've read the exercise advice", naming the actual cap source: injury answer or over-45 advisory; `workouts-start-capped`);
   it never edits a workout silently. The medical stop outranks it.
3. **Ride:** the core clamps every ERG target to the cap (defence in depth) and watches measured power. **An
   exceedance is power above cap + 3 % (288 W) sustained for 3 s**; one sample at or below that ends it. Why these
   numbers: with targets ≤ 97 % the flag needs ≥ 6 % (~17 W) over the highest target, well above the few-percent
   ripple of a trainer holding ERG; 3 s is the window of the 3-s power the rider sees, so a flag never fires on a
   number the screen hasn't shown, and a single overshoot sample can't trip it. Both are conventions (D) to be
   checked against measured NEO ripple in the hardware tests (#23). On an exceedance, in the same frame:
   - **row 1 becomes the status** at the ride tier (T3, warning fill + ⚠ + words): "⚠ 312 W · over your 280 W cap
     · ease off" — what, how much, what to do, readable at 1 m;
   - **the gauge shows the same measured power** (band marker pinned high, "▲ 74 W" to the target);
   - **the coach lane asks a strong `ease_off`** ("Ease off to 231 W? · Above your injury cap", or "… · Above your cap until you've read the exercise advice" when the over-45 advisory is the source), the only verdict
     allowed while it lasts; the **warning tone** plays;
   - **a `CapExceeded { at, watts, forS }` entry is logged** with the ride (one per episode; a dip below the
     threshold ends the episode) and listed in Review.
   `test:safety` feeds measured power over time and checks both sides: 10 s at 288 W (+2.9 %) and 2.5 s at 300 W
   raise nothing; 3.5 s at 300 W raises the status, the call and exactly one log entry; a second episode adds a
   second entry (`ride-cap-exceeded`). Engine requirement: #32; core safety tests: #81.

**Deferred wish lifecycle.** A wish is a record (§15.4) with a stable id, the day it was for, and an **expiry fixed
at creation (7 days), never renewed**. States: `open` → `fulfilled` (a ride delivering the cell was *completed*) ·
`overridden` (ridden anyway) · `replaced` (the rider set another wish) · `deferred` (moved to a date by the
planner; may be deferred again, but not past its expiry) · `expired`. On the day it was moved to, Today shows it
pre-set in the matrix with "↻ Carried over from Sun 11 Oct: VO2max · Intervals · expires Sun 18 Oct" and *Drop
it* (`today-wish-carried`); the planner treats it like a fresh wish. **Until the check-in is confirmed nothing reads
as granted** — the cell says "↻ wish" and the card "↻ Your wish, after your check-in: …", because the answer
depends on today's check-in. One open wish at a time. Each outcome stores
the knowledge-pack version its reasons came from, so Review can show the explanation that was shown then.

**Conventions challenged**

| Convention (where) | Verdict | Reason |
|---|---|---|
| Browsing a workout library to pick today's ride (Zwift, TrainerRoad) | **Change** on Today, **keep** in Workouts | On Today the rider states the flavour; picking a specific workout stays in Workouts (§5.7), through the same guard. |
| Zone numbers Z1–Z7 as the intensity choice | **Change** | Words anchored on CP / VT1 (R-GEN-01 · B). |
| A continuous "level" slider (TrainerRoad levels) | **Change** | A level can't be checked; five named rows can. |
| Silent adaptation when tired (TrainerRoad Adaptive Training) | **Change** | Shown on the grid, explained with facts, rule and cut-off grades; the rider can ride the wish. |
| Duration as a workout category ("long ride") | **Change** | Time is its own choice; the matrix says how hard and how structured. |
| A free-form duration picker | **Drop** | Six time choices cover 30 min–3 h; anything else is Edit (§5.7), not a picker at the bike. |
| Disabled buttons for impossible combinations | **Drop** | Not drawn as choices at all. |
| A wish mid-ride | **Drop for v1** | Changes the session's intent under effort; in-ride levers stay ±, Skip, End (§7.2). |
| "Plan tomorrow" | **Drop** | The wish is about how the rider feels *today*; deferral covers "not today". |
| "Coach decides" as the default | **Keep** | Silence is safe; the 95 % path of §4 is unchanged. |

### 15.3 Why? and Background — *"Why this, and how sure is it?"* (#84)

**Contextual *Why?*** A button next to any claim that rests on the pack: "Why? **B · good**" and, when crank
applied a cut-off of its own, "**D · convention** cut-off". The chip always shows letter **and** word; C/D have a
dashed outline. The visual chip is compact; its hit area is 48 px (§6.7). It opens a **sheet that never hides the
claim**: at ≥ 1440 px wide it **pushes the page aside** (the page reflows to the left of it; this is not the ride
screen, P4 doesn't apply); below that it overlays and **repeats the claim verbatim with the rider's facts** at its
top. The sheet, in order (`today-why`):
1. **crank told you** — "Legs dead, sleep short: two signals against hard work today." · From: legs: dead · sleep: short
2. **The evidence** — grade chip + what it means for the rider, then the rule's rationale (verbatim from the pack)
3. **The cut-off crank applied** — its grade (usually D) and what it is ("two signals"), and why that makes an
   override legitimate
4. **What the pack recommends** — the rule text, verbatim and labelled as the pack's, *not* as crank's behaviour
5. **What crank uses here** — only inputs crank has (its input registry: check-in answers, time, rides, injury
   answer, profile); anything the pack mentions that crank lacks is named as not used ("heart-rate variability
   (HRV): crank doesn't measure it"; DFA-α1; lactate)
6. **Safety comes first** — "Chest pain, fainting, a racing heartbeat with dizziness or unusual breathlessness
   are not a readiness question: they stop every plan until a doctor clears you." The medical stop (§5.8) is
   evaluated before any rule in the pack; a rule's own wording ("severe flag → easy/rest") never applies to the
   red flags.
7. **Sources** (authors, year, title, journal, DOI link) · "Read the background: <topic> →" · footer: pack date,
   works offline (source links need a connection), *training guidance for healthy riders, not medical advice*.

Entry points: Today (outcome and reasons), Settings → Training profile (effects), Review (each coach call, with
the judge rule that fired), the model card (why CP not FTP, why ±; topic 03, R-PD-01), and — once #77 adds the
screening entry — the health check. **Mid-ride: no *Why?*** (constraint 1); every call's *Why?* is in Review.
Rider-friendlier wording per rule: #144.

**Background** (Settings → Background; `settings-background`, `settings-background-topic`): the grade legend
(A–D, plus the pack's qualifiers "A-" and "(observational)"); the 13 topics with their rules counted per grade
(letter + word); a topic shows *How sure*, **what the research says** (each finding's grade chip, headline,
number of sources) and **what the pack recommends** (its rules with *Why?*); **How the coach decides** and **Your
athlete model**, generated from the sources the coach's prompt is built from (#59; not in the prototype, built
with #84); **About** (version, pack date, not a medical device). It lives in Settings: reference for a rare job.

**Single source of truth.** Nothing from the pack is copied into the UI. At build time the app generates a
`knowledge` bundle (rules, topics with graded findings, references, the grade policy) and a **pack version**
(content hash + compiled date) that every planner reason records.

**Build-time check** (fails the build; `pnpm test:knowledge` is its executable form):
1. every rule parses into all fields, grade A–D, belongs to a topic (section or own `# (NN)` note; `R-GEN-*` may cut across);
2. every rule citation and every topic-file `[@key]` resolves in `references.bib`;
3. every reference has a DOI, PMID or OpenAlex ID;
4. every topic has a title, a how-sure line and at least one graded finding;
5. every rule ID the UI, the planner or the judge can emit resolves; every reason's **inputs** are in crank's input
   registry, which names nothing crank can't measure (HRV, α1, lactate); every deviation rests on an **A/B** rule
   and every cut-off carries a grade.

**Conventions challenged**

| Convention (where) | Verdict | Reason |
|---|---|---|
| Help centre / FAQ on a website | **Change** | Offline, in the app, opened from the claim. |
| Hover tooltips | **Change** | No hover on touch, remote or at 1 m; an explicit *Why?* with a sheet. |
| A modal that covers what it explains | **Change** | Push aside ≥ 1440; repeat the claim below. |
| Tips during the ride (SYSTM) | **Drop** | Attention under effort; *Why?* lives in Review. |
| "Science-backed" without grades | **Drop** | Every claim carries grade and sources; C/D and D cut-offs say so. |
| Articles written apart from the logic | **Change** | Generated from the pack the coach uses; build-time check. |

### 15.4 Contract fields for the planner and the coach (#59, M2)

Recorded on #59 (round-2 update there). A sketch for the ADR; names follow the contract's style.

```ts
type Focus = "base" | "climbing" | "racing" | "long_events" | "tt" | "fitness";
type Intensity = "easy" | "tempo" | "threshold" | "vo2max" | "sprint";       // main effort, anchored on CP / VT1
type Format = "steady" | "intervals" | "hills" | "race" | "open";           // structure only; never duration
interface TrainingProfile {
  version: number;                                   // every Save = new version; proposals record it
  focus: Focus; event: { name: string; date: string } | null;
  daysPerWeek: number; hoursPerWeek: number;          // future budget, not target
  likes: Format[]; returning: { since: string } | null;   // returning = after ≥ 10 days off (model's gap threshold)
  savedAt: string; source: "onboarding" | "settings";
}
interface Cell { intensity: Intensity; format: Format }
interface WishRecord {
  id: string; cell: Cell;
  setAt: string; via: "desk" | "saddle" | "carried";
  forDate: string;                                   // the day it is for (today, or the date it was deferred to)
  expiresAt: string;                                 // setAt + 7 d, fixed at creation, never renewed
  status: "open" | "deferred" | "fulfilled" | "overridden" | "replaced" | "expired";
  history: { at: string; status: WishRecord["status"]; proposalId: string | null }[];
}
type InputId = "legs" | "sleep" | "stress" | "motivation" | "time" | "load7d" | "cap" | "profile" | "history" | "power" | "hr" | "cadence";
interface Reason {
  kind: "readiness" | "load" | "safety" | "session" | "profile";
  text: string;                                      // rider words; the saddle shows the first, ≤ 90 chars
  facts: { label: string; value: string }[];         // "legs: dead" — what crank saw
  uses: InputId[];                                   // registry only (§15.3 check 5)
  ruleIds: string[];                                 // resolve in the knowledge bundle
  evidenceGrade: "A" | "B" | "C" | "D" | null;       // weakest of ruleIds; deviations need A/B
  cutoff: { grade: "A" | "B" | "C" | "D"; what: string } | null;   // the threshold crank applied
}
interface WishOutcome {
  wishId: string | null; wish: Cell | null;
  delivered: Cell; minutes: number;                  // minutes < asked only for an "adapted" readiness reason
  relation: "no_wish" | "as_wished" | "adapted" | "deferred" | "blocked_by_safety";
  reasons: Reason[];
  deferredTo: string | null;                         // ≤ the wish's expiresAt
  overridable: boolean;                              // readiness/load only; false for any safety reason
  packVersion: string; profileVersion: number;       // so Review can reproduce what was shown
}
// Proposal gains: cell, wishOutcome, reasons: Reason[] (replaces prose, #29), profileVersion.
// Commands (all through the core's single guard, #81): wish.set(cell | null), wish.drop(id),
//   proposal.rideAsWished(proposalId) → logged { at, wishId, reasonsShown }; refused unless overridable.
// Health cap (§15.2, option a):
//   CapState { active; capW; ergOnly: true; planPct: 97; tolPct: 3; windowS: 3; log: CapExceeded[] }
//   - the planner clips the wish space and keeps every target ≤ capW × planPct / 100
//   - ride.start refuses non-ERG blocks or targets > capW (`cap_requires_erg` / `cap_exceeded_target`);
//     with a medical stop as well, every start including the free ride is refused (§5.8)
//   - the engine clamps ERG targets to capW and emits CapExceeded { at, watts, forS } once per episode when
//     measured power > capW × (1 + tolPct / 100) for ≥ windowS; the judge may then only ease_off (#32, #81).
```

Invariants for the ADR and the golden evals (#59):
- **Safety first.** Under a medical stop the planner is not called. Under a cap the wish space is clipped before
  planning; no output has a target above the cap or a non-ERG block; `overridable` is false for any safety reason.
- **Every deviation has a reason** whose `ruleIds` resolve, whose evidence is A/B, and whose cut-off has a grade.
- **The wish never raises the load beyond what readiness allows**; a wish above it is deferred, not granted
  silently; riding it anyway is the rider's logged decision.
- **Profile, wish and reasons leave the machine only under #59's privacy rules.**
- Offline (no provider): the rule-based fallback implements the same outcome table with the same reasons.

### 15.5 Glanceability and tone check
The saddle proposal is the bike-distance read: outcome line and session at T3+, reason at T3, ~~64–72 px~~ superseded by §16.2: 72 px
(§16.2) buttons with their bound keys, all inside the viewport at both sizes (checked by the harness, which fails otherwise). The
desk matrix, the profile and the sheet are desk reading. Copy is calm and factual — no streaks, no "crush it", no
badges. Colour never carries meaning alone: wish = outline + "wish", plan = fill + "plan", adapted = "≈ wish",
cap = dashed edge + "cap", cursor = outline + its name at T3+, C/D = dashed chip + word.

### 15.6 Changes to §1–§14
| Where | Change |
|---|---|
| §2 "Regenerate" row, §5.1 Today | Shorter = time; Easier / Harder = the wish's − / +; "Other" = wish mode; one outcome table. Time choices add 2 h and 3 h. ~~After the check-in the proposal is the **saddle proposal** at the top.~~ superseded by §16.2: After the check-in the one proposal card takes its saddle form in place. "Why today" lists facts without interpretation. |
| §5.8 Onboarding | **eight** steps; *Training for* is step 5, after the Health check |
| §5.8 health cap | the cap is on actual power: ERG only under a cap; the start guard refuses workouts above it; over-cap power in a ride is flagged and logged (§15.2) |
| §5.9 Settings | sections gain **Training profile** (after Devices) and **Background** (last) |
| §7.2 | on Today: − / + = Easier / Harder, Keep = coach's pick, Skip / Pause = wish mode (§15.2) |
| §7.3 | a **Wish answered** row: granted vs changed tones |
| §9 | ONBOARDING §1.4 "Goals" is superseded by §15.1 |
| §11 | contract additions: §15.4 (#59, #85, #86; engine cap #32) |

### 15.7 Prototype, screens, tests
- Scenes (both themes, 1440×900 and 1024×768):
  - **desk:** `today` (no wish), `today-wish-carried`, `onboarding-profile`, `settings-profile`,
    `settings-background`, `settings-background-topic`
  - **saddle**, captured as the viewport (not the full page): `today-wish-granted`, `today-wish-deviates`,
    `today-wish-adapted`, `today-wish-capped`, `today-wish-remote`, `today-why`
  - **cap:** `workouts-start-capped`, `ride-cap-exceeded` (measured power fed through the cap monitor),
    `today-stop-and-cap`
  - **profile draft:** `settings-profile-editing` (before → after: *new*, *no longer*, "Not now")
  - `today-wish-long` from round 1 is removed (duration is no longer a format).
- Intentionally changed: `today`, `today-health-cap`, `today-medical-stop`, `today-free-ride-confirm` (wish card
  and/or the six time choices in the check-in); `start-handoff-100ms` (Today underneath); every `onboarding-*`
  (eight steps, new order); every `settings-*` (two sections, the wish sound row); `ride-capped` at 1024 (the cap
  chip became a component that can also flag over-cap power: 79 px of anti-aliasing, same text).
  Every other screen is byte-identical; `start-motion-*` frames are restored from the base commit (#127).
- `src/knowledge.ts` reads `spec/knowledge-pack/` directly (Vite `?raw`); `src/mock/wish.ts` stands in for the
  planner (labelled). Hills run as ERG targets in the prototype (SIM is not in the mock, #29).
- `pnpm screens` additionally fails when a saddle element leaves the viewport or falls below its tier.
- `pnpm test:knowledge` — the §15.3 build-time check; every wish cell compiles at every time choice; reasons use
  only registered inputs and A/B rules; cut-offs are graded.
- `pnpm test:safety` adds:
  - no matrix and no proposal under a medical stop
  - under the cap, every VO2max, sprint, race-sim, hills and open cell is disabled
  - under the cap, no cell, override or format delivers a target above the cap or a free block (checked on the delivered workout)
  - Harder stops at the cap
  - a workout above the cap is refused at start
  - every planned target under the cap is ≤ 97 % of it
  - over-cap power fed over time: within tolerance (+2.9 % for 10 s) and shorter than the window (2.5 s) raise
    nothing; 3.5 s at 300 W raises the row-1 status, the coach's ease-off and one log entry; the gauge shows the
    same power; a second episode is a second entry
  - a medical stop plus a cap: no free-ride offer, and the guard refuses the free ride
  - Pause alone reaches all 19 cells
  - Skip changes the format, − reaches Easy, and Coach uses the wish
  - an adapted wish is "≈ wish" and has a one-tap fix
  - a deviation cites its rule and cut-off, and the override delivers the wish
  - Easier = wish −

### 15.8 Review guide (round 2)
1. **Saddle** (`today-wish-*` viewport captures): wish, session, one reason, actions — readable at 1 m, above the
   fold at both sizes? Can a two-button remote reach any cell?
2. **Matrix** (`today`, `today-wish-adapted`, `today-wish-remote`): are effort, structure and duration separate,
   and is an adaptation never drawn as granted?
3. **Cap** (`today-wish-capped`, `workouts-start-capped`, `ride-cap-exceeded`): can actual power pass the cap?
4. **Deferral** (`today-wish-deviates`, `today-wish-carried`): is the lifecycle complete and the promise bounded?
5. **Why?** (`today-why` at both sizes): is the claim always visible, the pack's recommendation separate from what
   crank does, the cut-off graded, safety precedence explicit?
6. **Profile** (`settings-profile`, `onboarding-profile`): is no C line a promise? Before → after clear?

### 15.9 Round-1 findings → resolution
| Finding | Resolution |
|---|---|
| opus B1 · cap not enforced on SIM / open | **Fixed** — option (a), §15.2: non-ERG and race cells locked, start guard, in-ride flag + log, engine clamp (#32), tests (#81) |
| opus M2 · astra 2 · outcome not readable from the saddle | **Fixed** — saddle proposal at T3/T3+, viewport-checked; wish tones in §7.3 |
| opus M3 · astra 6 · sheet hides the claim | **Fixed** — push-aside ≥ 1440, claim + facts repeated at the top |
| opus M4 · "what crank does" overclaims (HRV) | **Fixed** — "What the pack recommends" vs "What crank uses here" (registry), "Not used: HRV …", check 5 |
| opus M5 · grade overstates the decision | **Fixed** — evidence grade and cut-off grade shown separately; A/B direction + D cut-off rule stated |
| opus M6 · Easier/Harder vs wish; "Other" | **Fixed** — Easier/Harder = wish ∓, one outcome table; Other = wish mode |
| opus M7 · astra 5 · deferral / plan tomorrow | **Fixed** — "plan tomorrow" dropped; WishRecord lifecycle, fixed expiry, carry-over on Today, pack version stored |
| opus M8 · astra 3 · adapted drawn as granted; format mixes duration | **Fixed** — time is separate (2 h, 3 h added), Long → Steady, "≈ wish" + one-tap fix |
| astra 1 · opus m9 · remote can't reach format / Easy | **Fixed** — wish mode: Skip = format, Pause = every cell; *Easy · Intervals* added |
| astra 4 · C line presented as commitment | **Fixed** — "Optional … never scheduled on its own", conditions, before → after |
| opus m10 · "Ride my wish anyway" gating | **Fixed** — gated like Start, logged, readiness/load only |
| opus m11 · astra 7 · onboarding duplication / gap / order / length | **Fixed** — budget pre-filled from Background (future vs past), one 10-day threshold, step after Health check, details collapsed, draft restore specified |
| opus m12 · profile Save/Cancel, before→after, unit, D link | **Fixed** — actions under the form, before → after, "h / week"; D line names #143 in this document (rider text never shows issue numbers, §5) |
| opus m13 · grade chips; "(observational)"; jargon | **Fixed** — letter + word everywhere, qualifiers in the legend; jargon remains #144 |
| opus m14 · astra 8 · grid role without arrows; contrast; cap copy | **Fixed** — plain buttons with state labels, 48 px targets; cap edge/text/cursor pairs in `contrast.mjs`; one cap sentence everywhere |
| opus m15 · profile versioning; time non-overridable | **Fixed** — `TrainingProfile.version`, `profileVersion` on proposals; time is never adapted except by a readiness rule (overridable) |
| astra 3 · custom duration | **Decided against** — six time choices to 3 h; anything else via Edit (no picker at the bike) |

**Round-2 re-review (opus, at `e3cc5e7`) → round 3**

| Finding | Resolution |
|---|---|
| r2-1 · over-cap warning is a 14 px chip, not consistent, single sample, "logged" without a log | **Fixed** — row-1 status at T3 with warning fill; gauge and coach `ease_off` agree; `CapMonitor` applies > cap + 3 % for 3 s over a measured-power stream; one `CapExceeded` log entry per episode; trace-driven tests on both sides of the rule |
| r2-2 · threshold targets the cap exactly → false flags from ERG ripple | **Fixed** — planned targets ≤ 97 % of the cap; flag at cap + 3 % for 3 s (justified in §15.2, checked on hardware in #23); in `CapState` (§15.4); both tested |
| r2-3 · carried wish shows ✓ before the check-in | **Fixed** — "↻ wish" / "↻ Your wish, after your check-in" until confirmed |
| r2-4 · "Why today" ignores the adapted minutes | **Fixed** — "Shortened to 60 of your 120 min: …" |
| r2-5 · "no nutrition advice" vs a carbohydrate target | **Fixed** — "no diet, weight-loss or medical advice"; the cue reads "Fuelling guideline (training, not diet)" with R-CHO-01 · A |
| r2-6 · no scene for the profile draft | **Fixed** — `settings-profile-editing` |
| r2-7 · medical stop + cap: free ride | **Fixed** — stop + cap = no ride (§5.8), no offer, guard refuses; tested |
| r2-8 · "▼2"; 1440 Why? layout | **Fixed** — "2 steps easier"; the outcome line wraps inside the pushed-aside layout (checked in `today-why` at 1440) |

---

## 16. Amendment (#175): layout revisit — Ride, Today, shell

Status: **accepted** by the owner on **2026-10-13** (#175; the wireframe canvas is private to the owner, the issue
states every decision). Only space, grouping and duplication change: anatomy, the type tiers and their angles
(§6.2), the safety rules (§5.8, §15.2), colour roles (§6.3) and the window sizes (§8) are unchanged. Text this
section replaces is struck through where it stood, followed by "superseded by" and the subsection. The UI issues build on it: shell #88,
Today #94, ride anatomy #96, #97, editor #111, layout shift #10.

### 16.1 Ride

```
┌ [❚❚ Pause] 13:25 left  ERG you [−] ±0% [+]  ─────── free ───────  (NEO ● │ HR ●) [?] ┐ groups sized to content
├──────────────────────── hero (62 %) ───────────────────────┬──── side (38 %) ────────────────┤
│ row 1  WORK Threshold rep 1/2 ●●○      left in block  ← T3 │ HEART RATE           zone 4    │ auto
│                                                            │ 164 bpm                         │ auto
│ T1                0:23                     ← 220 / 176 px  │ CADENCE      ✓ in range 90–95  │
│                                                            │ 91 rpm  ▕──▓▓|▓──▏             │
│ band   TARGET 350 W  ▕━━━━▓▓▓|▓▓━━━▏ ▲ 6 W                 ├━━ coach lane (fixed row) ━━━━━━━┤ 3 px rule
│        sub-line: ↗ to 210 W · coach −5 %                   │ head · reason · [buttons] ·     │ 1fr
│                                                            │ deadline + bar · grammar        │
├────────────────────────────────────────────────────────────┴────────────────────────────────┤
│       now ▏      ▼ next  Easy 0:15 @ 140 W               ← T3+, pinned at the block's start  │
│ ▁▁▂▂██▂▂██▂▂██▂▁ (near road −2 … +10 min; dashed = proposed, solid = applied)                │
│ whole ride (thin) · 30:58 ridden · block 36 of 39 · solid = coach applied · −2 … +10 min     │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

| Decision | What it is | Why |
|---|---|---|
| **Status bar groups sized to content** | Left: Pause · time left · mode chip with − / ±% / + (**48 × 48 px** targets at both sizes, §6.7 — the issue's "≥ 44 px" floor raised to the app-wide rule; the 40 px buttons at 1024 are gone). Right: **one device pill** "NEO ● │ HR ●"; a device expands to its status only on a fault or a reconnect note ("⚠ NEO lost", "↻ NEO back · 0:14 gap"), then `?`. | The old fixed widths summed to 1118 px (932 at 1024) and reserved room for statuses that are almost always "connected". **No layout shift (P4):** every value that changes reserves its widest form — Pause and Resume share one box, the clock reserves `h:mm:ss`, ±% a fixed width, the mode chip its widest word — and the pill is right-aligned, so it grows leftwards into free space and nothing else moves. No group may shrink; the widest pill — the trainer's reconnect note with the widest gap and HR lost ("↻ NEO back · 59:59 gap │ ⚠ HR lost", scene `ride-reconnected-hr-lost`) — fits at 1024 with ≈ 13 px to spare (8 px group gaps at ≤ 1100). |
| **Hero rows `auto \| 1fr \| band`** | The old row-2 caption moves onto row 1, right-aligned; the old row 5 leaves the hero. Row 1 is a fixed T3 line plus 10 px, so a chip (cadence, cap) never grows it. | Two rows of chrome (≈ 100 px) go to the number the rider reads most. The geometry check (§16.4) found that row 1 grew by 10 px when the cadence chip appeared — a #10 shift in the old anatomy too; the fixed height closes it. |
| **T1 re-derived** | `clamp(150px, min(24.5vh, 17.2vw), 260px)` → **220 px at 1440×900 (109′), 176 px at 1024×768 (105′)**, was 189 / 161 (94′ / 96′); T1r = 0.62 × T1 → 137 / 109 (68′ / 65′). | The freed rows give row 3 382 px at 1440 and 303 px at 1024, so height no longer bounds T1. At 1024 the hero's width (62 % of the window) bounds the widest T1 strings ("≈ 1:05" with its range, "312 W"), hence the vw term; the two terms meet the accepted 220 / 176. Both stay ≥ 90′ (§6.2); the vh/vw rule still holds only for the reference devices (production computes px from physical size, #52). |
| **Captions fit row 1** | A caption shares its line with the block's identity, so it is ≤ 27 characters: "block's minimum in", "HR < 118 · or block max", "HR lost · block max in", "paused · block max in" (were 28–41). "min" and "max" never stand alone, so they can't be read as minutes. | The long forms clipped at 1024 ("RECOVER rep 1/3" + caption, 603–781 px in 594 px). The caption still names the event (HR crossing vs the block's rule, P7); the times stay on the row-4 sub-line ("block ends in 2:00–4:00", "HR due below 118 before then"). The T3 tier is unchanged. |
| **"Next" on the road** | A T3+ label in a label lane above the near road, pinned to its block's start: it begins at the start when it fits, else ends there, so it never straddles the ▼ notch that always marks the start. When neither fits it sheds the word "next", then the watts; only if it still can't, it is right-aligned over the start with a 2 px leader under it from its left edge to the notch. On the last block it marks the ride's end: "ride complete → Review". | The forward look is in one place: the label sits on the block it names, at the time it begins. The label lane costs the near road ≈ 30 px (95 / 77 px tall at 1440 / 1024), the price of not overlaying the profile. The "now" tick stays; the time scale moves to the overview line. |
| **Coach lane = fixed grid row** | The side column is `auto auto 1fr`; the lane is the 1fr row with a 3 px top rule (border colour by state). Inside, five fixed rows: head (T3+) · reason (T3) · actions (button height) · deadline text + bar · grammar. Idle reserves the button and deadline rows; Detail density's T2 metric spans head–actions and its note takes the deadline row. A band word ("CALIBRATING", "AUTO") ends the head line after a 14 px gutter, never under the headline. | An ask, an applied change or the feel prompt fills rows that already exist, so no state moves any geometry (#10). The deadline bar is drawn in every timed state (ask, undo window, feel save). The idle note is ≤ 28 characters ("last call −3 %: not taken"; the call's time is in Review), which the new `ride-cooldown` scene showed was needed. |

Unchanged: the states of §5.2, the cap status in row 1 (§15.2 — during an exceedance row 1 is the status alone, the
T1 below it is still the block countdown), Focus keeps the second slot's exact box (now a hidden real slot, so even a
1 px drift is gone), the road's proposed-change label (T3, on the near road).

### 16.2 Today

```
1440 (check-in done)                                         1024 (check-in done), stacked
┌ sleep short · legs heavy · … · 60 min · no red flags  [Edit] ┐  ┌ summary line              [Edit] ┐
├──────────── proposal card 7fr ───────────┬─ wish matrix 4fr ─┤  ├ proposal card ────────────────────┤
│ ▌outcome line (6 px edge)          T3+   │ Wish: …           │  │ ▌outcome · session · profile ·    │
│ Tempo 2×10′ · 40:00                T3+   │ ▢▢■▢▢ (matrix)    │  │ reason + Why? · S / N hints (T3)  │
│ ▁▂██▂██▂▁ profile                        │                   │  │ [⏎ Start][− Easier][+ Harder][…]  │ 72 px
│ one reason + Why?                  T3    │                   │  ├ wish matrix ──────────────────────┤
│ S other wish · N coach's pick      T3    │                   │  ├ Last 7 days ──────────────────────┤
│ [⏎ Start][− Easier][+ Harder][Ride my wish]  72 px, pinned   │  └ Model ────────────────────────────┘
├──────────── Last 7 days ─────────────────┼────── Model ──────┤
```

| Decision | What it is | Why |
|---|---|---|
| **Check-in collapses** | Once confirmed (Same as yesterday, or any change) **and** today's red flags answered, the check-in is one summary line: "sleep short · legs heavy · stress some · motivation keen · 60 min · no red flags" + *Edit*. *Edit* re-opens it in place; while open, a change never collapses it mid-edit — **✓ Use these answers** closes it (disabled until the red flags are answered). | After confirming, the check-in is history; one line keeps it checkable without pushing the card down. **Start needs the check-in done and closed**: confirmed (Same as yesterday, or a change, §5.1), today's red flags answered (§5.8), and not open for editing — so Start, and Enter, only ever act on the saddle card the rider can see, above the fold. This is stricter than §5.1 alone (a change used to enable Start while the answers were still open), never looser. "Use these answers" is named differently from the red-flag check's *Done* so the two never read as one action. Under the disabled Start the desk card says what is missing: "Confirm how you feel and answer the health check to start" before the first confirmation, "Answer the health check to start" when only the red flags are left, and "Use these answers to finish editing and enable Start" while a done check-in is re-opened. |
| **One proposal surface** | The full-width saddle proposal and the desk proposal card are one card with two forms. **Desk** (check-in open): outcome line, session · duration, profile, one reason + *Why?*, actions Start ⏎ · − Easier · + Harder · Ride my wish / Ride the full X; "Why today: every fact" discloses the other reasons (each with *Why?*), the facts and the model's confidence, and *Edit a copy in Workouts*. **Saddle** (check-in done): the same card at T3+ / T3 with **72 px actions pinned to its bottom edge**, always above the fold; the same "Why today: every fact" disclosure (desk-size type, collapsed) above the key hints; S (Skip) / Pause open wish mode, Keep returns to the coach's pick, hinted on the card **at T3** ("S other wish · N coach's pick"), above the fold — it is the only saddle cue for wish mode, so it must read at 1 m. The desk card does **not** stretch to the height of the matrix beside it: its actions follow its content, so Start is above the fold at 1440×900 in every desk state (check-in open, injury cap, carried-over note) and no empty band sits under "Why today" (#179). A gap under the desk card is therefore expected whenever the wish column is taller (e.g. a carried-over note under the matrix): "Last 7 days | Model" is one grid row across both columns, and the gap sits below the fold (#189, decided as designed). | Two cards said the same thing twice. Information is kept, not cut: the §5.1 facts, the §15.2 reasons and *Edit a copy* are one click away in both forms, and the first reason's *Why?* is on the card. **Shorter** is dropped from the card because the time is a check-in answer (§15.2: how long is Today's time) — it was a duplicate control. *Other wish* is dropped as a button: the matrix is beside the card at 1440, and wish mode is the bars' path (§15.2). One cursor: − / + on the card, Intensity − / + and the matrix all move the same wish (one outcome table, §15.2). |
| **1440 layout** | Card 7fr | matrix 4fr, above the fold — the **complete** matrix, in every desk and saddle state. To keep it there with the check-in open: the injury-cap notice shares the date's line (one line at 1440: "⚠ Capped at threshold (100 % of CP, 280 W) · no sprints · ERG only — from your injury answer; lift it in Settings → Health"), and a carried-over wish's note sits under the matrix (the ↻ cell marks the wish itself). Last 7 days and Model in a strip under each column. Under a medical stop there is no matrix (§15.2), so the stop card spans both columns. | The proposal and the wish are the decision; history and the model are context. |
| **1024 layout** | Stacked: summary line, the card (actions pinned at its bottom), then the matrix, Last 7 days, Model. The layout switches on the page's own width (CSS container query, < 1000 px), so 1440 with the *Why?* sheet pushed aside stacks too and the card keeps its full width. | A 4fr matrix in 912 px is too narrow for its five formats; the card must stay above the fold. |

Above the fold: the saddle card (outcome, session, reason, actions) at both sizes, including `today-why`; the desk
card's outcome line and session at both sizes, and its actions — Start, the primary action — at 1440×900 (at 1024 the
open check-in stacks above the card, so its actions may sit below the fold there; `test:safety` checks Start is
disabled after *Edit*). The open check-in lays its answers and the red-flag check side by side.
Key glyphs, on every key hint (Today, the ride's `?` list and hints, wish mode, Settings → Controls): Enter ⏎, Backspace
⌫, Escape "Esc", the space bar "Space"; any other key as typed.

### 16.3 Shell

| Decision | What it is | Why |
|---|---|---|
| **Labelled icon rail** | 72 px at every size (the accepted 72–76 px range's lower end): icon over a 12.5 px word, 62 px items, ≥ 48 px targets, never an icon alone. Replaces the 184 px text rail (148 px at ≤ 1100). The page's max width rises from 1240 to 1368 px (= 1440 − rail). | Content width: 1304 px at 1440 (was 1176, +128 — the old max width capped the rail's gain), **912 px at 1024** (was 836). The words stay, so the rail is as findable as before. |
| **Workouts library drawer** | Below 1280 px the library is a drawer: **☰ Library** in the editor header opens it over the panes and a dimming scrim; its items are ≥ 48 px. While open it is **modal**: focus moves to its first item, Tab and Shift+Tab wrap inside it, and the editor behind it is inert. A pick, ✕, Esc or a click on the scrim closes it, and focus returns to ☰ Library. Growing the window to ≥ 1280 px closes it too — whichever place is showing when the window crosses 1280 px, so it cannot come back as a modal when the window shrinks or when Workouts is opened again. ≥ 1280 px keeps the 200 px column (not modal). | Outline · Text · Summary stay in one row at 1024 (#111), instead of the summary moving under them; the library is used once per edit session, the panes all the time. |

### 16.4 Prototype, screens, checks

- New scenes (both themes, both sizes): `today-checked-in` (summary line + saddle card, no wish), `today-checkin-edit`
  (the done check-in re-opened: desk card), `ride-cooldown` (the cool-down template; "next" at the ride's end),
  `workouts-library-drawer` (the drawer at 1024; at 1440 the column, as `workouts`), `ride-reconnected-hr-lost` (the
  widest device pill).
- Regenerated: every `ride-*` scene with the ride screen (status bar, hero, lane, road), every Today scene, every
  railed view (`ready*`, `review*`, `history*`, `workouts*`, `settings-*`, the start gates), `start-handoff-100ms`
  (Today underneath). Unchanged: `onboarding-*`, `resume`, `resume-stale` (no rail), `ride-screen-restarted`,
  `ride-symptom-stop` (the ride screen is replaced), `start-*` except the hand-off (`start-motion-*` restored from
  the base commit, #127).
- `pnpm screens` additionally fails when:
  - an above-the-fold element (`[data-above-fold]`: the saddle card, the desk card's outcome and session, the ride's
    "next" label) leaves the viewport in **any** direction, or a tiered one falls below its tier;
  - any **ride region moves, appears or disappears**: the set of `[data-geom]` boxes (Pause, clock, intensity, `?`,
    rows 1/T1/band, both slots, the lane and its five rows, the road's three rows) and each box must equal
    `ride-threshold`'s at the same size and theme, in every phase template and state (#10).
  - at 1440, the **complete** Today wish matrix is missing, stacked under the card, or leaves the viewport (every desk
    and saddle state — a full-page PNG alone does not prove the fold). Exempt: a medical stop (no matrix, §15.2) and
    the *Why?* sheet open (the page stacks, §16.2);
  - a status-bar group shrinks: no group may, so the widest device pill (`ride-reconnected-hr-lost`) must fit at
    1024 as it is (the geometry check catches the `?` moving).
- `pnpm test:safety` adds the check-in flow: desk card with Start disabled while open; Same as yesterday alone does
  not collapse it; confirmed + red flags → one summary line and the saddle card; *Edit* re-opens it with the desk
  card and Start disabled; a change keeps it open; *Use these answers* collapses it with the new answer; the footer
  under Start names what is missing in each case. It also drives the Workouts library drawer at 1024 **keyboard
  only**: Enter on ☰ Library opens it with focus inside; Tab / Shift+Tab stay inside, the editor is inert; Esc and
  Enter on an item close it with focus back on ☰ Library; the scrim closes it too.
- `pnpm test:layout` (#179) checks Start is inside 1440×900 with no empty band above the desk card's actions in `today`,
  `today-health-cap`, `today-checkin-edit` and `today-wish-carried`; and that opening the library drawer at 1024,
  growing the window to 1440 and shrinking it again leaves the drawer closed (crossing 1280 px closes it), also when the crossing happens while another place is showing.
- `pnpm screens` (here and `design/brand`) is byte-reproducible (#189): Chromium runs with one raster thread and no
  partial raster, the hand-off frames are pinned to their nominal time (the fake clock's `performance.now()` jitters
  by 1 ms between runs), and the brand's CSS animations are created paused (they run on the document timeline, so a slow
  load could finish one before it was set to its frame). Runs on an unchanged tree, also under host load, produce
  identical files (10 brand, 5 prototype).
- Not changed here, filed: the lane's buttons (58 / 49 px) are below §7.2's 64 px, and the status bar's fault words
  are 16 px, below T3 — both pre-date §16 (#177).

### 16.5 Review guide

1. **Ride** (`ride-threshold`, `ride-hr-recovery`, `ride-coach-ask`, `ride-trainer-lost`, `ride-reconnected`,
   `ride-cooldown`, at 1024): is "how long, on target, what next" one look? Is "next" readable on the road and on
   its block? Does any state move anything (the harness says no — does the eye agree)?
2. **Today** (`today`, `today-checked-in`, `today-checkin-edit`, `today-wish-*`, `today-medical-stop`,
   `today-health-cap`): one proposal, readable from the bike, nothing duplicated, nothing lost (facts, reasons,
   *Edit a copy*)?
3. **Shell** (any railed view at 1024; `workouts`, `workouts-library-drawer`): is the rail as findable, and do the
   editor panes work in one row?

---

## 17. Amendment (#240): live workout edits

Status: **proposed**; the owner confirmed the proposal and its slot in #240 on **2026-10-14** (proposal "as written", order
after #218 and #243, before #34). Architect sign-off: pending. This section is the design the engine, core and UI steps
of #240 build on; it contains no code. It builds on: ADR-0001 §5 (engine commands, ride record, `RideStatus`,
compiled workout), §4.3 (StreamFrame lanes, the 8 000-byte frame budget), §6 (store tables); ADR-0002 §7 (ERG policy,
soft start §7.5, compliance and settle §7.8); #243 (laps are per `FlatBlock` as ridden; a skip ends a lap; flags on
`RideStatus`, bulk data by query); #218 (start guard, in-ride clamp); DESIGN §5.8/§15.2 (health cap), §7.1/§7.2
(coach bounds, decision model), §16.1 (ride anatomy).

**What the rider gets.** Four edits to the plan *ahead of* or *at* the cursor, each logged: **End set** (finish the
current loop now), **Skip** / **Restore** (drop an upcoming block, rep or loop; bring it back before it starts),
**Extend** (+N s on the current block) and **Repeat once** (one more rep of a loop). End set, Extend and Repeat once
can be undone with **Undo** (§17.2); Skip is reversed by Restore. Today only "skip the current block" exists (#217).

### 17.1 The plan overlay

The compiled workout (`CompiledWorkout.blocks`, crank-domain) is **immutable** for the whole ride and is what the `ride`
row stores as `compiled_json`. Edits live in an **overlay** held by the engine's `RideSession`.

**Identities** (all derived from `FlatBlock`; nothing is added to the compiler's output):

| Id | Definition | Notes |
|---|---|---|
| `BlockId` | `(node_id, path, iteration)` of a `FlatBlock` | the #243 lap key; unique within a compiled workout |
| `GroupId` | `(path[..d+1], iteration[..d])`: the path of a repeat node and the iteration counters of the repeats *around* it | `d` is the repeat's depth (0 = outermost). A block with `iteration.len() = n` belongs to `n` groups, innermost = `d = n−1`. Relies on `path.len() = iteration.len() + 1`, which the compiler guarantees (`visit` pushes one of each per repeat level); the engine step adds a corpus test for it |
| `IterationId` | `(GroupId, k)`: every block of the group whose `iteration[d] = k`, nested descendants included | one **rep**: its work block and the rest that follows it. The UI's unit for "Skip rep 4" so no orphan rest is left |

**Overlay value** (a pure function of the edit log, §17.4: `overlay = fold(applied edits, in order, undone ones removed)`):

| Part | Type | Written by |
|---|---|---|
| `suppressed` | map `BlockId → { rider, set_ended }` | `suppress`, `end_set` (kind `set_ended`), cleared by `restore` (`rider` only) |
| `extend_s` | map `BlockId → s` (cumulative) | `extend` |
| `added` | map `GroupId → n` added iterations | `repeat_once` |

**Effective sequence** and **status** are two separate pure functions in `crank-domain` (`workout::overlay`, next to the
compiler; no I/O, no clock, so they are shared by engine, core, Review and tests):

```text
effective(compiled: &CompiledWorkout, overlay: &Overlay) -> Vec<EffectiveBlock>
block_status(effective: &[EffectiveBlock], cursor: Cursor, ride_log: &[RideEdit]) -> Vec<BlockStatus>
```

`effective` returns **only the sequence**: each `EffectiveBlock` has its identity (`BlockId`), timing (`start_s`,
`dur_s`, `extended_s`) and target (`power_from/to`, `phase`), plus provenance: `origin` (`planned | added`) and
`suppressed` (`None | rider | set_ended`). It knows nothing about the cursor: no done / current / skipped.
`block_status` derives `done | current | upcoming | suppressed | ended | skipped` from the effective sequence, the
cursor (the current `BlockId`, or finished) and the ride log, whose `skip` events are the only source of `skipped`.
The two never share state, so each is tested alone.

How `effective` is built:

1. Start from `compiled.blocks` in order.
2. For each group with `added = n`, append `n` copies of the group's **last iteration** (all its blocks, nested
   descendants included) directly after that iteration, with `iteration[d] = count, count+1, …` (new `BlockId`s; the
   copy keeps the original targets: **no further progression**, so an added rep is never harder than the last one
   ridden). If `skipLastRest` dropped that iteration's final child, the copy is taken from the previous iteration's
   final child and **restored** after the existing last iteration, and the added copy drops it again: the result equals
   compiling the same workout with `count + n` and the progression held. A group with `count = 1` and `skipLastRest`
   has nothing to copy the rest from: `repeat_once` is refused `no_template`.
3. Remove every block in `suppressed`.
4. Add `extend_s` to the block's `dur_s` (never to `dur_range_s`: open-ended blocks cannot be extended).
5. Recompute `start_s` as the running sum of the remaining expected durations (so the effective sequence tiles
   `[0, total_s_eff)` like the compiled one); `power_from/to`, `phase`, `node_id`, `path` are untouched.

**Planned versus ridden, one statement.** Every *planned* duration comes from `effective`: `ride_plan.total_s` /
`total_range_s` and the overview's planned length are sums of its `dur_s` (extensions included, suppressed blocks
excluded). Every *ridden* quantity comes from the ride log and the cursor through `block_status` and the recorder:
ridden time, and a lap's `edited_dur_s`, which is the compiled `dur_s` plus the net `extend` rows of the log for that
block as of the lap's end (§17.4). `effective` is never asked what has been ridden.

Suppressed blocks stay in the returned sequence (the road draws them greyed, §17.6) with their planned duration, and
contribute 0 s to the timeline; the *ridden* sequence the engine walks is the subsequence with `suppressed = None`.
`RideStatus.block_index` indexes the ridden sequence (§17.5).

**Invariants** (the property test, §17.7): the ridden sequence minus `added` is a subsequence of the compiled one
(order kept, no duplicates); an `added` block directly follows a block of its own group's last iteration; no block's
target, phase or identity is changed; the sequence tiles. **Engine invariant** (tested on engine commands, §17.7):
no command changes a block *before* the cursor, and the current block changes only by its own `extend` (its duration,
while it is current), with **one exception**, the undo of an `end_set` (§17.2), which moves the cursor back to the
interrupted block and returns the group's `set_ended` blocks to the sequence.

### 17.2 Commands

All five are `EngineCmd` variants (plus `UndoEdit`), exposed by the core (#34) as `ride_end_set`, `ride_suppress`,
`ride_restore`, `ride_extend`, `ride_repeat_once`, `ride_undo_edit`, and queried with `ride_plan` / `ride_edits`
(§17.5). Every command carries `actor: rider | coach` and `input: key | remote | touch | coach` (for the log only).

**Common preconditions**: a ride is loaded, started and not `finished` (`not_started`, `wrong_state`); the ride is
**not free** (`free_ride`: a free ride has no plan, so all edits are refused; its lap action is #243's). Edits are
accepted in `riding` **and** in every `paused(…)` state (§17.3). A command is applied between ticks and takes effect
from the next tick (the same rule as every detection in ADR-0001 §5). **A refused command changes nothing** (overlay,
status and log are byte-equal before and after; tested). Codes every command can return: `not_started`, `wrong_state`,
`free_ride`. **This table is the single list of the engine's refusal codes**: the command-matrix test (§17.7 test 6) is
generated from it, one case per row and per code. One code is **not** the engine's: `needs_confirmation` is returned
by the **core** (§17.2 Interactions), before a command reaches the engine, so the engine has no such row.

| Command | Effect | Preconditions (beyond the common ones) | Refusal codes |
|---|---|---|---|
| `end_set(group?)` | The current block ends now, every later block of the group (default: the **innermost group of the current block**) becomes `set_ended`, the cursor jumps to the first block after the group | the group has the current block; if `group` is given it is one of the current block's groups | `no_group` (current block is in no repeat), `unknown_group`, `not_current_group`, `stale_proposal` (a coach-origin edit whose target changed since the ask) |
| `suppress(target)` | `target` ∈ `BlockId` · `IterationId` · `GroupId`: the blocks become `rider`-suppressed; they stay on the road, greyed | every block of the target is **upcoming** (not current, not done); a group is suppressible only if *no* block of it has started | `unknown_block`, `already_started` (current or done; for a group in progress the message says "use End set"), `would_end_ride` (nothing ridable would remain after the cursor; the rider uses End), `removes_rest` (a block-level suppress of a `recovery` block between two un-suppressed `work` blocks; suppress the rep instead), `removes_warmup` (§17.3), `stale_proposal` (as for `end_set`) |
| `restore(target)` | undo of a `suppress`: `rider`-suppressed blocks of the target are back in the ridden sequence | blocks are `rider`-suppressed and still upcoming | `not_suppressed`, `not_restorable` (`set_ended`, or the block has been passed), `already_started` |
| `extend(+N s)` | adds N s to the **current** block's expected duration; the block timer's end moves, the target does not | current block is a fixed-time block (`dur_range_s = None`, time end rule); N and the cumulative sum within the table below; the extension must not predict a depleted W′bal (§17.3) | `not_extendable` (`lap`, `kJ`, `hr<` blocks), `extend_step` (N not in the table), `extend_limit` (cumulative), `would_deplete`, `coach_cannot_extend`, `cap_exceeded_target`, `cap_requires_erg` (§17.3 rule 1) |
| `repeat_once(group)` | `added[group] += 1` | the group has the current block or an upcoming block (it is not finished); `added < 2` (`added < 1` without a CP / W′ model, §17.3 rule 3: the second added rep is refused `repeat_limit`); a copy template exists (§17.1) | `unknown_group`, `group_finished`, `repeat_limit`, `no_template`, `would_deplete`, `coach_cannot_extend`, `cap_exceeded_target`, `cap_requires_erg` |
| `undo_edit` | removes the **most recent undoable rider-initiated edit** from the fold (§17.2 "Undo scope") | an undoable edit exists | `nothing_to_undo` |

**Extend limits** (a convention (D); owner may override, §17.9). The limit depends on the block's `phase`, because a
longer rest is the safe edit and a longer interval is not:

| Block `phase` | Allowed N | Cumulative maximum |
|---|---|---|
| `work` | 15, 30, 60 s | `min(+120 s, planned dur_s)` |
| `recovery`, `warmup`, `cooldown` | 30, 60, 120 s | +300 s |

`free` blocks (a `Free` target inside a plan) follow the second row. Without a CP / W′ model on the athlete (§17.3) the
cumulative maximum on `work` is +60 s.

**Undo.** Undo is **its own action** (§17.6). It reverses **rider-initiated** edits only. **Keep never undoes a rider
edit; its §7.2 role for coach changes is unchanged** (it answers asks and returns an auto-applied intensity change
to the plan within 15 s). An explicitly accepted coach change, a rider-confirmed coach structural edit included, is not
undone by any key (§7.2).

| Edit | Reversed by | On the undo stack until |
|---|---|---|
| `end_set` | `undo_edit` | **15 s of session time** after the edit (pausing does not extend it), **or until the block it entered stops being current** (Skip, natural end), whichever is first: only while the entered block is still current can Undo move the cursor back. It re-enters the interrupted block **from its beginning**: a block discontinuity (§17.3 rule 7); the seconds ridden in between stay in the record (their own lap) |
| `extend` | `undo_edit` (subtracts N) | the block's elapsed time passes the un-extended end, **or the block stops being current** for any reason (`end_set`, Skip, natural end) |
| `repeat_once` | `undo_edit` | the first block of the added rep starts, or the added rep is `set_ended` |
| `suppress` | **`restore`** (explicit) | **not on the undo stack and has no undo window**; `restore` is available until the block starts |
| `restore` | `suppress` | not on the undo stack |
| `skip` | nothing | not an overlay edit |

- **Undo scope.** `undo_edit` applies to the **most recent rider-initiated edit that is on the stack, not expired and not
  undone**. An entry that has expired (its window or condition above has passed) **drops off** and never hides an
  earlier valid one: `extend`, then `repeat_once`, then Undo reverses the `repeat_once`, and a second Undo reverses
  the `extend` (the block is still current).
- **An `extend` expires when its block stops being current.** After `extend` then `end_set`, Undo targets the
  `end_set`; the `extend` is expired, is never rewritten and is not reachable by any later Undo. If the `end_set` is
  undone, the re-entered block keeps its extension (it is still in the overlay); only its undo is gone. This is what
  keeps the §17.1 engine invariant to its single exception.
- **Skip is not on the stack** and does not block undoing an earlier edit (it expires the current block's `extend`,
  because that block is no longer current).
- A coach-proposed, rider-confirmed edit is **not** on the stack (see above).

**Interactions.**

- **Manual Skip** (`SkipBlock`, #217) is unchanged and not an overlay edit: it moves the cursor one block, is not
  undoable (§17.2 Undo) and changes no future block. It **is logged** (kind `skip`, with the skipped `BlockId` as target, §17.4) so planned-versus-done is complete and
  `block_status` can be rebuilt from the stored log.
  After an `end_set`, Skip skips the block now current. `end_set` while the current block is the last of the
  workout finishes the ride exactly as Skip does (#217).
- **Pause** (`user`, `stopped_pedalling`): edits are accepted: the paused plan is where a rider reads ahead. Workout
  time and the block timer stay frozen; a cursor move is applied to state only, the new block's target is sent by the
  resume's soft start (§17.3).
- **Dropout and reconnect** (`trainer_lost`; HR loss): edits are accepted (state only). A command arriving inside the
  2 s detection window is applied at the next tick like any other (transitions inside the window stand, ADR-0001 §5).
  On reconnect the §6/§7.5 sequence runs: the mode is re-issued as 0 W until a pedalling sample, then the soft start
  goes to the block the *effective* sequence has at the cursor. The §6.6 re-entry policy for long gaps applies to
  that block.
- **Coach `replan`** (R-INT-03, rules.md: "end set or convert to threshold"): the judge's `replan` verdict may
  *propose* a structural edit (`end_set` or `suppress`: **decreases only**). §7.1 stays authoritative: **a structural
  edit is never auto-applied**, whatever the autonomy setting (auto-apply covers `ease_off` and the intensity part of
  `replan` only). The proposal is always an ask (deadline; silence keeps the plan; Coach = accept, Keep = keep the
  plan) and executes, on accept, through the same command with `actor = coach` and `input = coach`. **The core
  enforces this** like every §7.1 bound, and is the **only** layer that does: before any command reaches the engine
  it reads the proposal's actor (`coach`) and its confirmation state (the nudge's answer: `accepted` by the rider, as
  opposed to `auto_applied`), and refuses an unconfirmed coach structural edit with `needs_confirmation`. The engine
  keeps no second copy of the rule. `extend` and `repeat_once` are **never** coach-initiated (`coach_cannot_extend`: the coach
  adds no work, §7.1), and no edit is proposed while the health cap is the reason a call is pending (only `ease_off`
  may be asked, §15.2). A proposal is re-validated when accepted: if a rider edit made it stale (target gone, started
  or already suppressed) the accept is refused `stale_proposal` and the ask expires "superseded by your edit". Once
  accepted, the edit is applied and **no key undoes it** (§7.2: an explicitly accepted change is not undone by any
  key; it is not on the undo stack); the rider still has Skip and Intensity −. A rider edit never changes the
  intensity offset or the coach's ±% (§7.1); those stay orthogonal and apply to the effective blocks' targets as
  before.

### 17.3 Safety

An edit **cannot create a target**: `suppress` and `end_set` remove blocks, `extend` keeps the block's target,
`repeat_once` copies targets that already passed the start guard. The health cap and the soft start are nevertheless
enforced at *edit time* (so the rider hears "no" before anything happens) and again at the *control point* (defence
in depth); neither relies on the other.

1. **Cap, at the command** (consistent with #218's start guard, which refuses and never edits silently): after
   applying a command to a *copy* of the overlay, the engine validates every block of the resulting ridden sequence
   that is not yet done against the active cap (`cap_w`, ERG only) with the start guard's own function, **on the targets
   after the rider's ± offset and the coach's change** (the controlled target, ADR-0002 §7 item 1, not the
   planned one); any target above `cap_w` or a non-ERG block under a cap refuses with `cap_exceeded_target` /
   `cap_requires_erg`. A **decrease-only edit** (`suppress`, `end_set`, and the undo of `extend` and of `repeat_once`) is **never
   refused by the cap**: it adds no target and a ride already over the cap is the monitor's business (§15.2), not the edit's.
   The undo of `end_set` returns blocks to the sequence and is validated like `extend` and `repeat_once`. Validation
   runs on the whole remaining sequence for those three, so a future change that lets an edit
   introduce a block (e.g. a template that is not a copy) cannot slip past it. A medical stop (§5.8) has already
   stopped the ride (`wrong_state`).
2. **Cap, at the control point**: the controlled target is clamped to `cap_w` as before (§15.2 point 3); an edit never
   widens what the clamp allows. The `CapMonitor` and `CapExceeded` log are unchanged.
3. **W′bal** (#243 Q4 flags; CP, W′ and the τ from `RideSetup.athlete`): `extend` and `repeat_once` on `work`
   blocks are refused `would_deplete` when the predicted W′bal at the end of the extended block, or of the added
   rep, at its target is < 0 J (the R-WB-01 floor, the same rule as #243's `depleted` flag). With no CP / W′ model
   the check cannot run; the engine then applies the conservative limits of §17.2 (+60 s cumulative on `work`, one
   added rep: a second is refused `repeat_limit`) and the refusal text says why. It never silently allows the full limit.
4. **Warm-up and rest protection** (a convention (D), owner may override): `removes_rest` as in §17.2; and
   `removes_warmup`: the edit suppresses a `warmup` block **before any `work` block has started** and would leave
   fewer than 180 s of `warmup` before the first `work` block (the compiler's cooldown convention, `MissingCooldown`).
   A rider who has already worked is not restricted.
5. **Entering a block by an edit** (only `end_set`, its undo and Skip move the cursor): the ERG target changes **exactly
   as at a natural block boundary** (ADR-0002 §7 item 2): while ERG is engaged, the rider is pedalling and no ramp is
   running, it is one step on the next tick (newest mode wins in the `control` slot, §4.4). It is not a new
   engagement, so no soft start of its own, as with Skip today. Settle follows **ADR-0002 §7.8 (a) as written**: the
   first 5 s of the entered block are settle seconds **only when its start target differs from the previous block's
   end target**; when they are equal nothing is settled.
5a. **An edit never cuts a soft-start ramp.** If a §7.5 ramp is running (after a reconnect, after ERG re-engagement,
   or the ride-start ramp of #250) when an edit changes the target, the ramp **continues** under the same per-tick rule
   (the same step schedule and 1 Hz updates) **toward the new target**; it is neither replaced by a direct step to
   the new block's target nor restarted. The settle of the new block starts when the ramp ends, as §7.8 (c) does.
5b. **Entering ERG from `Free` or `Sim` by an edit** (a plan with a `Free` block, then `end_set` or `suppress` of the
   free block's successor makes an ERG block current) is the ADR-0002 §7.5 item 5 case, exactly as at a natural
   boundary: a (re-)engagement, so the soft start runs from the current 3 s power.
6. **Not engaged → soft start, never the target.** When ERG is not engaged at the edit tick (paused, ERG released
   §7.4, link lost or connecting, no pedalling sample yet) the edit only moves state. The target is sent by the
   re-engagement rules: 0 W until the first pedalling sample of the epoch, then the §7.5 soft start from the 3 s power
   to the *then-current* block's target. An edit therefore never sends torque to a rider who is not pedalling.
7. **Undo of `end_set`** re-enters the interrupted block from its beginning. It is a **block discontinuity**: rules 5,
   5a and 5b apply as for any block entry, and §7.8 (a) applies as written (5 s settle only when the re-entered
   block's start target differs from the end target of the block ridden since).
8. **Rate**: one control write per tick (§4.4) holds however many edits arrive in a tick; edits are applied in
   arrival order and each is validated against the state the previous one left.

### 17.4 Record and Review

**The log.** Refused commands are *not* logged (a refusal changes no state and is returned to the caller); every
**applied** edit, `skip` and `undo` is appended to the ride record at command time (its own row, not a 5 s chunk, so a crash loses none). The overlay is always
`fold(log)`: crash recovery and the #50 resume rebuild it from the table; Review and the planned-versus-done view call
the same pure function on the stored `compiled_json` and log.

Forward migration sketch (ADR-0001 §6; `crates/crank-store/migrations/NNNN_ride_edit.sql`, next free number,
forward-only; the exact SQL is the engine step's):

```sql
CREATE TABLE ride_edit (
  ride_id       TEXT NOT NULL REFERENCES ride(id),
  seq           INTEGER NOT NULL,   -- dense, 1-based per ride; RideStatus.edit_seq after this entry
  at_session_s  REAL NOT NULL,      -- seconds, like lap.start_s and coach_decision.t
  at_workout_s  REAL NOT NULL,      -- frozen value while paused
  kind          TEXT NOT NULL CHECK (kind IN
                  ('end_set','suppress','restore','extend','repeat_once','skip','undo')),
  actor         TEXT NOT NULL CHECK (actor IN ('rider','coach')),
  input         TEXT NOT NULL CHECK (input IN ('key','remote','touch','coach')),
  target_json   TEXT NOT NULL,      -- BlockId | IterationId | GroupId; for skip the skipped BlockId; {"seq": n} for undo
  param_json    TEXT NOT NULL,      -- {"add_s": 60} for extend, else {}
  call_id       TEXT NULL REFERENCES coach_decision(call_id),
  PRIMARY KEY (ride_id, seq)
);
ALTER TABLE coach_decision ADD COLUMN proposed_edit_json TEXT NULL;  -- the replan's typed edit
```

Conventions followed: ids are `TEXT` and foreign keys plain `REFERENCES` like every other child of `ride` in
`0001_initial.sql` (`ride_chunk`, `lap`, `recovery`, `coach_decision`); the schema uses **no** `ON DELETE CASCADE`
anywhere, so none is added (a ride is not deleted by this table's rules); times are `REAL` seconds; `STRICT` is not used by `0001_initial.sql`, so it is not used here.

- **Applied or undone**: an undo is its own row (`kind = 'undo'`, `target_json = {"seq": n}`); an edit is *undone*
  iff such a row exists. The table is append-only. Snapshot test of the resulting schema, as for every migration.
- `ride.compiled_json` stays the **plan**; the as-ridden sequence is derived. (`ride.workout_json`, "as ridden incl.
  coach changes", is unchanged: coach intensity changes are not structure.)
- `lap` gets `iteration` from #243's migration; this section adds no `lap` column. A lap's `stats_json` carries
  `planned_dur_s` (compiled) and `edited_dur_s` (compiled plus the net `extend` rows of the log as of the lap's end,
  §17.1 "Planned versus ridden") so a lengthened block reads as such.

**Laps** (#243 rules, one lap per `FlatBlock` as ridden): `end_set`, like a skip, **closes the current lap at the
edit tick**, and the new current block opens its lap. A suppressed or `set_ended` block never starts, so it has **no
lap** (no zero-length rows). An added rep's blocks are laps keyed `(node_id, path, iteration = count + j)`. `extend`
adds no lap and lengthens the current one. Undoing an `end_set` opens a **second** lap with the same key; `idx`
disambiguates, and Review shows the interrupted one as "interrupted".

**Review** shows planned versus done (§5.5 item 3's plan line, extended):

- The ride chart's dashed *plan* line is the compiled workout; the solid *given target* line follows the effective
  sequence. Removed blocks are drawn as **outline-only ghosts** labelled "removed by you" / "ended early by you",
  added reps with a "+" mark, extensions with a notch at the planned end. Never colour alone (§15.5); never hatching
  (reserved for variable-length blocks, §5.5).
- The work-interval table lists every planned block, with state `ridden | removed by you | removed by coach |
  ended early | added`. The *Show as table* alternative carries the same rows plus one row per edit (time, what,
  who, undone or not).
- The coach timeline (§5.5 item 3) lists coach edits as calls ("end set early · accepted · applied"; a structural
  edit is never auto-applied and never undone by a key, §17.2); rider edits are a second, labelled lane, not mixed with calls.

**Compliance and verdicts** (#218, #106, #12): removed blocks are **rider-removed, not failed**.

- "On target" (ADR-0002 §7.8) is a share of ERG *seconds*; a block that never started has none, so it is outside the
  denominator by construction. An `end_set`-shortened block contributes the seconds ridden.
- The intent verdict counts reps as `held · not held · removed · ended early`; only `not held` is a failure:
  "Threshold 2 × 4:00 — 94 % on target · 1 of 4 reps removed by you", "VO2 sets — 12 of 12 reps ridden (3 removed by
  you)". A part with all reps removed reads "removed by you", with no percentage.
- R-INT-03 "≥ 2 consecutive failed reps" counts only *ridden* reps, as it does today, so removing reps cannot trigger
  a `replan`, and a rider-ended set is not a failed set. This needs **no new judge feature**: removed reps simply
  produce no rep record. Any new judge or athlete-model input (for example telling the judge a set was shortened by
  the rider) is **not** in this design; it would be filed as its own issue.

### 17.5 Status and stream

Follows #243's precedent: **flags on `RideStatus`, bulk by query**, no new lane.

| `RideStatus` field | Type | Meaning |
|---|---|---|
| `edit_seq` | `u32` | number of log entries; changes on every edit, skip and undo. The UI re-queries `ride_plan` when it changes (also covers resync) |
| `block_count` | `u32` | blocks in the *ridden* sequence ("block 36 of 37") |
| `undo` | `Option<{ seq: u32, kind: EditKind, expires_session_s: u32 }>` | the rider edit `undo_edit` would reverse now (§17.2 Undo scope; never a coach-confirmed edit) and its window end (a far value for edits whose window is a condition, not time); `None` when nothing is undoable. The UI counts down from `session_s`; no per-second status changes |
| `block_index` | existing | now an index into the **ridden** sequence *as of `edit_seq`* |

- **The UI never reads `block_index` against a plan of another `edit_seq`.** Between a status with a new `edit_seq`
  and the arrival of `ride_plan`, it keeps its last render and does not move the cursor (a sub-second state; tested
  with a delayed query).
- **Queries** (not stream, not frame-budgeted): `ride_plan() → { edit_seq, blocks: [{ id, start_s, dur_s, extended_s,
  origin, status, restorable }], total_s, total_range_s }` and `ride_edits() → [RideEdit]` (the log).
- **Frame budget**: the added fields serialize (camelCase JSON) to at most **128 bytes** in the worst case
  (`"editSeq":4294967295,"blockCount":4294967295,"undo":{"seq":4294967295,"kind":"repeat_once","expiresSessionS":4294967295}`
  is 120 bytes; 121 with the joining comma).
  Status is "on change": an edit emits one status frame; `undo`'s expiry emits one more (`None`). The maximal-status
  property test of #243 (all flags set) is re-run with these fields set; discrete state still falls back to the
  fixed resync frame when over 8 000 bytes. No new coalescing class: the latest status already coalesces.
- **Nudges**: a coach edit proposal is an existing nudge and adds no frame type. The typed edit is **not embedded** in
  the nudge: it lives in `coach_decision.proposed_edit_json`, keyed by the `call_id` the nudge already carries
  (§17.4). One source of truth, the nudge size and its 512-byte message cap are unchanged, and the engine reads the
  edit from the decision on accept. There is therefore no `CoachNudge.proposed_edit` field.

### 17.6 UI

Everything fits the accepted §16.1 ride anatomy: **no region is added, moved or resized** (the `[data-geom]` check
of §16.4 must hold in every state below). The edit surface is the road (the selection) and the coach lane's five
fixed rows (what to do with it).

| Surface | Behaviour |
|---|---|
| **Road ahead and overview (#98)** | A tap or click on the road **selects** the block under it (the near road's full height is the hit area; where two blocks are < 48 px apart a second tap on the same spot steps to the next block). The selection is drawn as a thick outline plus its name and time in the "next" label lane. In a repeat the selection is the **rep** (`IterationId`: its work and rest together, labelled "rep 4 of 5"); a second step up selects the **loop**. States: **suppressed** = outline-only, no fill, a ✕ glyph and "skipped" in the label (never colour alone); **set ended** = the same, labelled "ended"; **added** = a "+" on the block; **extended** = a notch at the planned end. The overview draws the same states thin, and its text reads "30:58 ridden · block 36 of 37 · 2 skipped". The "proposed/applied" dashed/solid vocabulary of coach changes is unchanged and separate |
| **Lane, selection state** (no coach call pending; a pending ask always takes the lane, the selection is kept) | head: the selection's name and when ("Rep 4 of 5 · in 2:10"); reason row: effect preview ("removes 4:00 @ 350 W and its 2:00 rest"); actions row, ≥ 64 px each: `◀ ▶` (step the selection), then the **context actions**: current block / its loop → **End set** (`end_set`) and **Extend +1:00** (`extend`); an upcoming rep or block → **Skip** (`suppress`); an upcoming loop → **Skip loop**; a suppressed one → **Restore** (`restore`); a loop in progress → **Repeat once**. Labels are actions (§7.2). The grammar row states the bound keys ("E = end set · S = skip current") |
| **Lane, edit applied** | the pattern of "coach applied": head "Set ended · 3 reps skipped", reason "next: Easy 0:15 @ 140 W", for a rider edit one **Undo** button (its own action, never Keep), the deadline text "Undo · 12 s" with its bar, grammar "U = undo". For `end_set` the bar and button also disappear **early**, as soon as the block it entered stops being current (Skip, natural end), not only when the 15 s run out (§17.2 Undo scope). For edits whose undo is a condition rather than a time (`extend`, `repeat_once`) the bar is not drawn and the button reads **Undo** until the condition ends. After `suppress` the lane offers **Restore**, not Undo. After a coach-proposed edit the rider accepted the lane states what was done and offers **no Undo** (§7.2) |
| **Dispatcher (#104)** | two new abstract actions, **End set** and **Undo** (nine actions: **Coach, Keep, Pause, Skip, End set, Undo, Intensity − / +, End, Report symptom**). Default keys `E` and `U`; any HID device can be bound (Learn). **End set** always means `end_set` on the **innermost group of the current block** (never the selection, never an outer group); **the dispatcher owns the second press**: while an `end_set` undo is offered, End set is a **no-op** (the engine takes every `end_set` at face value; the dispatcher does not escalate to an outer group). It joins Coach and Keep in the 1.5 s lane-change debounce (§7.2) and is refused with a spoken-label toast when there is no group ("no set to end"). **Undo** is `undo_edit` (§17.2 scope), also debounced. **Keep never undoes a rider edit**; its §7.2 role for coach changes is unchanged. The remote grammar's minimum is unchanged (two buttons = Coach + Pause): End set is **not reachable on a two-button remote**, which has Skip and the on-screen buttons; the three-button default is decided with #53 |
| **Sound and tone** | an applied edit plays the soft confirmation; an undo the low note (§7.3); a refusal is a text line in the lane for 4 s ("Not allowed: it would take you over your 280 W cap"), no warning tone (nothing is wrong) |
| **Saddle safety** | `End set` and `Skip` have no confirm dialog: Undo (15 s for `end_set`, or until the block it entered stops being current, whichever is first) and Restore (a skipped rep, until it starts) are the safety net. Dialogs still suspend ride shortcuts (§6.7); the End *ride* binding is untouched and still asks |

Refusal copy (rider-readable, no codes; one line each): `cap_*` "Above your cap (280 W)"; `would_deplete` "You'd run out
of reserve (W′ below 0)"; `removes_rest` "Skip the whole rep: this rest is between two hard efforts"; `removes_warmup`
"Keep at least 3 min of warm-up before the first hard block"; `extend_limit` "Longest you can add here is 2:00";
`group_finished` "That set is over"; `free_ride` "No plan in a free ride".

New scenes (both themes, both sizes): `ride-edit-select` (a rep selected, End set / Skip), `ride-edit-applied` (End set
applied: Undo · 12 s), `ride-edit-suppressed` (greyed road and overview, "2 skipped"). `pnpm screens` adds: every edit
control ≥ 64 px; the §16.4 geometry set equals `ride-threshold`'s in all three; the overview text fits at 1024.

### 17.7 Acceptance criteria for the engine step (#240 step 2)

Each is a test that fails without the change (the PR body shows the pre-change failure once, per repo process).

**Domain, `workout::overlay` (pure):**

1. **Identity.** `path.len() = iteration.len() + 1` for every block of the compile corpus; `GroupId`/`IterationId`
   derivation round-trips; ids unique.
2. **Empty overlay is the identity**: `effective(c, ∅)` equals `c.blocks` exactly (`start_s` included), and
   `block_status` over it with the cursor at block *k* and an empty log is `done` before *k*, `current` at *k*,
   `upcoming` after. **Purity**: `effective` takes no cursor and no log (a signature test), and returns equal values
   for equal arguments.
3. **Property** (proptest over arbitrary workouts × arbitrary valid edit sequences; includes nested repeats,
   `skipLastRest`, open-ended steps): the effective sequence satisfies every invariant of §17.1: *a valid ordered
   subsequence of the compiled one, plus added reps only as copies at the end of their group's last iteration*; ids
   unique; targets, phases and identities unchanged; tiles `[0, total_s_eff)`; extensions within §17.2's limits.
   **`block_status`** property: `skipped` appears exactly for the `BlockId`s in the log's `skip` rows (the row's
   `target_json` is the skipped `BlockId`), and `block_status` computed from the log as **stored and reloaded**
   equals the one computed live; `ended` only for `set_ended` blocks; exactly one `current` while the ride is not
   finished.
4. **Algebra**: `restore ∘ suppress`, the undo of `extend` / `repeat_once` / `end_set`, and `suppress`/`restore`/`extend`
   in any order are consistent (the fold is order-independent for commuting edits; each pair's inverse restores the
   overlay exactly).
5. **`repeat_once` golden** (explicit expected values, no compile oracle). For a group `g` of count `c ≥ 2` with
   children `k0 … km` at depth `d`, with `added[g] = 1`, the effective sequence equals the compiled one with these
   blocks inserted directly after the last block of iteration `c−1`: (i) without `skipLastRest`: one block per child
   `ki`, same `node_id` and `path`, `iteration[d] = c`, and the `dur_s`, `power_from/to` and `phase` of the child's
   block in iteration `c−1`; nested descendants are copied with their inner iterations; (ii) with `skipLastRest`
   (final child `km` absent from iteration `c−1`): first, **`km` of iteration `c−2` is inserted as `km` of iteration
   `c−1`** (`iteration[d] = c−1`, its targets) directly after iteration `c−1`'s last block, and then iteration `c`
   follows with children `k0 … k(m−1)` only. `start_s` is recomputed. The corpus includes nested repeats and
   `skipLastRest`; `c = 1` with `skipLastRest` asserts `no_template`.

**Engine (`SimTrainer`, `VirtualClock`, existing rig):**

6. **Command matrix**: generated from the §17.2 table: one success case per command row and one case per refusal
   code listed in it (including `nothing_to_undo`, `stale_proposal`, `repeat_limit` in both its forms, the common codes and the
   cap and W′bal codes); a refused command leaves overlay, status and log byte-equal.
7. **`end_set`** mid-rep: lap closed at the edit tick; cursor at the first block after the group; one ERG write on the
   next tick; the group's later blocks never start (no laps); with the entered block's start target different from
   the ended block's end target, 5 s of settle are excluded from compliance (and none when they are equal, §7.8 (a)).
   `undo_edit` at +14.75 s re-enters the block from its start as a block step (rule 7), with the settle only when the
   targets differ; at +15.25 s the entry has dropped off the stack and `undo_edit` is refused `nothing_to_undo`.
7a. **Undo scope**: (i) **`extend` then `end_set` → `undo_edit` targets the `end_set`**; the `extend` is expired,
   is never rewritten, and after the `end_set`'s window passes `undo_edit` is refused `nothing_to_undo`; after undoing
   the `end_set` the block keeps its extension. (ii) `extend`, `repeat_once`, `undo_edit`, `undo_edit` reverses the
   `repeat_once` and then the `extend`. (iii) `extend` then Skip: the `extend` is expired. (iv) a `skip` between two
   edits does not block undoing the earlier one while it is valid; (iv-a) `end_set`, then Skip the entered block,
   then `undo_edit` → `nothing_to_undo`, and the edits made before the `end_set` are unaffected (an earlier still-valid
   entry is reachable only if its own condition holds); (iv-b) `end_set`, the entered block ends **naturally**
   (< 15 s), then `undo_edit` → `nothing_to_undo`. (v) `suppress` is never on the stack (`undo_edit`
   leaves it, `restore` reverses it). (vi) the engine invariant of §17.1 holds over random command sequences with the
   `end_set` undo as its only exception.
7b. **Ramps** (§17.3 rules 5a, 5b), each with the control log asserted write by write: `end_set` **during the
   ride-start ramp** (#250): the ramp continues, same step schedule, toward the new target, no direct step; `end_set`
   **during a reconnect ramp**: same; an edit that makes an ERG block current **from a `Free` block**: a soft start from
   the 3 s power, not a step; the undo of `end_set` (test 7).
8. **Never torque without pedalling**: with an edit applied while *paused*, *ERG released*, *trainer lost* and
   *reconnecting*, the control log contains no target write until the re-engagement rules produce one: 0 W until the
   first pedalling sample, then the soft start to the then-current block's target (extends the #231 scenarios).
9. **No bypass of the cap or the soft start** (the model test): a proptest drives random edit sequences (rider and coach)
   interleaved with random link faults, pauses, ERG releases and ± changes against random plans under a random cap.
   Assertions on every tick: every `Erg` write is ≤ `cap_w`; no `Erg` write carries the block target before a pedalling
   sample after a re-engagement; **while a §7.5 ramp is running, every write is the ramp's own next step** (a step
   that jumps to the block target mid-ramp fails it); no non-ERG mode is entered under a cap. Mutation check (each
   mutation applied once, the named test must fail, shown in the PR): (a) cap check removed from the validator →
   test 6's `cap_exceeded_target` row; (b) W′bal check removed → the `would_deplete` row; (c) warm-up rule removed →
   the `removes_warmup` row; (d) **an edit replaces an in-flight ramp with the new block's target** (the "cut the
   ramp" mutation) → this test and 7b.
10. **Coach**: a `replan` with a structural `proposed_edit` is **always an ask**, also with auto-apply on (**core**
    test, §17.7 core list: an `auto_applied` structural edit is refused `needs_confirmation` and no command reaches
    the engine); accept → applied with `actor = coach`, and **no
    key undoes it**: `undo_edit` is `nothing_to_undo` and Keep leaves it (§7.2); Keep on the ask keeps the plan; `extend` and `repeat_once` from `actor = coach` refused `coach_cannot_extend`; a
    stale proposal refuses with `stale_proposal`; a rider edit leaves the intensity offset untouched.
11. **Free ride**: every edit command refused `free_ride`; state unchanged.
12. **Record**: the log rows equal the applied commands (kind, actor, input, ticks, target, params; a `skip` row
    carries the skipped `BlockId`); `fold(log)` reproduces the overlay and `block_status(effective, cursor, log)`
    reproduces the live statuses after a simulated crash and resume (#50); the schema snapshot matches the migration.
13. **Laps and verdict inputs** (#243): the lap rows of §17.4 (none for suppressed, closed at `end_set`, the keyed
    added rep, the second lap after undo); the planned-versus-done rows; suppressed reps are `removed`, never
    `not held`, and do not count towards "≥ 2 consecutive failed reps".
14. **Stream**: `edit_seq`, `block_count` and `undo` change only on edits; one status frame per edit; the
    maximal-status frame (§17.5) is ≤ 8 000 bytes; a coalesced or resync frame loses no plan state (the query
    returns the same `edit_seq`).

**Core list.** Core (#34) and UI (#98, #104) steps add: command wrappers + bindings regenerated (drift check), `ride_plan` /
`ride_edits` query tests; the `needs_confirmation` check (a coach structural edit without a rider's accept never
reaches the engine, whatever the autonomy setting); Vitest + mockIPC for selection, stepping, the three lane states, refusal copy and the
`End set` and `Undo` dispatcher actions (debounce, focus-wins, Learn; Keep never undoes a rider edit; a second End set press while an `end_set` undo is offered is a no-op); the three scenes above; e2e on the sim
trainer: End set mid-rep then Undo (and the stack scope with an earlier Extend); Skip a rep and see it greyed; a refused edit under a cap.

### 17.8 Changes to earlier sections

| Where | Change |
|---|---|
| §5.5 Review | planned versus done, removed-by-you rows and the edit lane as in §17.4; "On target" and the intent verdict count removed blocks as removed, not failed |
| §7.1 | a `replan` may propose a structural edit (decreases only); it is **never auto-applied** and the core enforces it (§17.2); no coach `extend` / `repeat_once`. §7.1's auto-apply and bounds are otherwise unchanged |
| §7.2 | **nine** abstract actions (End set and Undo added, default keys `E`, `U`); both join the 1.5 s debounce; Keep's §7.2 role for coach changes is unchanged and Keep never undoes a rider edit |
| §11 | contract additions: `RideStatus.{edit_seq, block_count, undo}`, `ride_plan`, `ride_edits`, the six commands, `coach_decision.proposed_edit_json` (§17.4, §17.5) |
| §16.4 | three new ride scenes; geometry set unchanged |
| ADR-0001 §5/§6, §4.2 | `EngineCmd` and command list gain the edit commands; `ride_edit` table; `RideStatus` fields (amendment lands with the engine step) |
| ADR-0002 §7.8 (a) | **not changed.** Edits follow it as written: settle only when the entered block's start target differs from the previous block's end target (§17.3 rule 5) |

**Additions beyond #240's scope**, each declared (the issue's proposal named block-or-group suppression, `end_set`,
`extend`, `repeat_once`, the road/overview selection, one new dispatcher action, and the log):

| Addition | Why | Where |
|---|---|---|
| `IterationId` (suppress / restore a rep with its rest) | a skipped rep must not leave an orphan rest | §17.1, §17.2, §17.6 |
| coach `proposed_edit` (a `replan` proposes `end_set` / `suppress`) | R-INT-03 names "end set" as the `replan` action; the verdict had no typed form | §17.2 (ask-only) |
| `coach_decision.proposed_edit_json` column | holds that typed edit by `call_id` (no nudge field) | §17.4 |
| ◀ ▶ selection stepping in the lane | road blocks can be narrower than the 48 px target floor | §17.6 |
| **Undo** as a ninth dispatcher action (rider edits only) | the architect separated Undo from Keep (round 2); §7.2's decision model is unchanged (its action list is edited here and declared in this table) | §17.2, §17.6 |
| `would_deplete` (W′bal check on `extend` / `repeat_once`) | the issue names the health cap and soft start only; extending work is the one edit that can deplete reserve (R-WB-01, #243 Q4) | §17.3 rule 3 |
| `removes_rest`, `removes_warmup` (protection refusals) | a block-level skip must not make hard efforts adjacent or remove the warm-up before the first hard block | §17.2, §17.3 rule 4 |
| logging **Skip** into `ride_edit` (kind `skip`) | planned versus done and `block_status` need it; today Skip leaves no trace but a shorter lap | §17.4 |
| **`end_set` undoable for 15 s after it takes effect** | extends #240's "undoable until it takes effect": an `end_set` takes effect at once, so without this it could not be undone. **Owner may override**, §17.9 | §17.2 |

No new judge or athlete-model feature is added (§17.4).
### 17.9 Open for the owner (recommended defaults; the owner may override any of these)

Each default is labelled with its source; "convention" means a (D) decision of this design with no external evidence.

1. **Extend limits**: `work` +15 / 30 / 60 s, at most `min(+120 s, planned)`; rests, warm-up, cool-down +30 / 60 / 120 s,
   at most +300 s (§17.2). *Source: convention* (the W′bal check of §17.3 rule 3 is R-WB-01).
2. **`repeat_once` limit**: at most 2 added reps per loop. *Source: convention.*
3. **Undo window of `end_set`**: 15 s of session time; this **extends** #240's "undoable until it takes effect" (§17.8). *Source: §7.1, the coach auto-apply undo window.* **Owner may override** (shorter, longer, or no undo for `end_set`).
4. **Warm-up protection**: refuse removing warm-up below 180 s before the first hard block (§17.3 rule 4). *Source:
   convention; the 180 s figure is the compiler's `MissingCooldown` constant.*
5. **Limits without a CP / W′ model**: `work` extension +60 s cumulative and one added rep (§17.2, §17.3 rule 3).
   *Source: convention* (half the model-backed `work` limit; `repeat_once` limit 2 halved).
6. **`removes_rest`**: refuse a block-level skip of a `recovery` block between two un-skipped `work` blocks (§17.2).
   *Source: convention*; skipping the whole rep is always allowed.
7. **End set and Undo default keys**: `E` and `U`; on a remote only with ≥ 3 buttons or by Learn. *Source:
   convention; #53 decides the remote defaults.*
