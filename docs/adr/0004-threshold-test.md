# ADR-0004: Threshold (FTP) test

- **Status:** Proposed
- **Date:** 2026-11-03
- **Issue:** #486
- **Related:** ADR-0002 §7 (ERG behaviour, §7.4 low-cadence release, §7.5 soft start, §7.9 end mode),
  ADR-0001 §5 (loss and stopped pedalling), DESIGN §5.8 and §15.2 (medical stop, health cap), #247 (athlete
  inputs), #81 (single start guard), `spec/ONBOARDING.md` §2–§3, knowledge pack `03-power-duration-models.md`

## 1. Context

Every built-in workout is written in % FTP, and FTP is fixed at 250 W (`library.rs` `FTP_W`); the athlete
inputs are #247. A rider has no way to measure their own threshold in crank. A new rider has no FTP at all,
so the test that sets it cannot depend on it.

### 1.1 The candidate tests

"Evidence" means a peer-reviewed comparison against a lab marker; "convention" means a platform or coaching
rule with no validation behind its exact numbers.

| Test | Protocol | FTP from | Accuracy and bias | Rider burden | Pacing | Trainer mode |
|---|---|---|---|---|---|---|
| **Ramp** (Zwift, TrainerRoad) | 1-min steps to failure; Zwift: start 100 W, +20 W/min [Z1] | 0.75 × best 1-min power [TR1, TR2] | 0.75 × best 1-min is **convention**: no study validates it for a 20 W/min ramp. **Related evidence**, not a validation: in 70 trained triathletes, FTP (0.95 × a 20-min TT) = 16.7 W + 0.75 × Pmax of a lab GXT whose protocol is not reported, sy.x 18 W for that model; mean FTP ≈ 80 % of Pmax [M22]. Bias follows the rider's anaerobic share, so a sprinter reads high and a diesel low; that this is ±3–6 % is coaches' estimate, not measured [C1] | ~15–25 min, one maximal minute | none; follow the target | ERG (TR's default and recommendation [TR1]); an anecdote reports a lower peak in ERG than in resistance [C2] |
| **20-min** (Allen & Coggan) | warm-up, priming, 20-min all-out TT | 0.95 × 20-min mean | **Evidence**: FTP20 ≈ LT on average (r = .95) but **below LT in recreational riders**, the bias tracking fitness [V18]; FTP20, FTP60 and IAT agree on the mean (bias −1 to −5 W) with wide LoA, and TTE at FTP20 is 50.9 ± 15.7 min, not 60 [B18]; MLSS ≈ 88.5 % of FTP20 (93 % of FTP), so FTP20 sits above MLSS [I19]. Not a steady-state marker [W23] | ~60 min; the 20 min hurt | **pacing skill required**; a novice blows up or holds back | resistance/SIM, so the rider paces freely; TR switches out of ERG for the effort [TR2] |
| **2×8-min** (Carmichael) | two 8-min all-out efforts, 10 min apart | 0.90 × the better or the mean (sources disagree) | 0.90 is **convention**. **Evidence** (19 well-trained riders): 8-min TT power is near Wmax, and the 8-min FTP differs moderately to very largely from lab lactate thresholds [C3]. Reads high for riders with a large W′ | ~45 min, two hard efforts | pacing, twice | resistance/SIM [TR2] |
| **CP** (3 + 12 min, or 3–4 efforts of 2–15 min) | maximal self-paced TTs, ≥ 30 min apart or on separate days | fit P = W′/t + CP | **Evidence**: CP is above FTP20 by 7 ± 13 W, LoA −19 to 33 W [K21]; CP sits near MLSS/RCP on the mean [knowledge pack 02]. The review recommends ≥ 3 efforts of 2–15 min and **advises against single-effort FTP tests** [L22] | 2 sessions or a long one | pacing each effort | resistance/SIM |
| **MAP step test** (lab GXT, e.g. 25 W per 2–3 min) | longer steps to failure | a % of MAP (72–77 % is a coaching rule) | as the ramp; the % depends on the step length, so it is not interchangeable with the ramp's 0.75 (convention) | ~20–30 min | none | ERG |
| **Submaximal HR** (e.g. LSCT) | fixed HR stages, ~17 min | regression on power at fixed HR | **Evidence**: reliable stage power (TE 1.3–4.4 %), predicts PPO and 40-km TT (r ≈ .9) in well-trained men [L11]; not validated for novices; needs HR | low | none | ERG steered by HR |

What all of them share: none equals MLSS or lab threshold for an individual. Wide LoA is the norm
[B18, I19, K21]. The knowledge pack already says so (`03-power-duration-models.md`: "FTP is not a
physiological threshold"), and its rule is **never** to use a single-effort FTP as the only anchor without
a caveat. Those facts shape §2.5: the result is an estimate with an uncertainty, never a measurement.

## 2. Decision

### 2.1 crank ships the ramp test first

**The ramp test is the first and only test of phase 2.** Reasons, in order:

1. **It needs no FTP.** The steps are absolute watts (§2.2), so it is the only candidate a new rider can take
   with a fair start. The 20-min and 2×8 need a pacing target, and in practice that target is an FTP.
2. **It needs no pacing skill.** The rider follows ERG until they cannot. A failed pace on the 20-min wastes the
   hour and is invisible in the data.
3. **It fits what the engine already does.** ERG, absolute targets, a real low cadence as the end signal
   beside the §7.4 release (§2.4). The 20-min and CP tests need a resistance/SIM block with a free-pacing UI, which crank's SIM path
   (ADR-0002 §7.9) has but has never used for a maximal effort.
4. **One short session**, and the maximal part is about a minute.

Its known weakness, the bias by the rider's anaerobic share, is accepted and shown: the result is labelled
"ramp estimate" with its band (§2.5).

**Next, not in phase 2:** a **CP assessment** (3 + 12 min, or three efforts), because the athlete model is
CP-based (`spec/ONBOARDING.md` §2 "Assessment week", §3) and [L22] recommends it. That needs its own ADR, for
the SIM-mode maximal effort and the two-parameter fit. The 20-min and 2×8 are not planned: same pacing burden
as CP, less information. **Owner question O1** (§5).

### 2.2 Protocol

| Phase | Target | Duration | Notes |
|---|---|---|---|
| Warm-up | 80 W (ERG) | 5 min | the first ramp steps continue the warm-up |
| Ramp | 100 W, then +20 W per step | 1 min per step, until the end (§2.4) | Zwift's numbers [Z1]; independent of any FTP |
| Cool-down | `Resistance { pct: 0 }` | 5 min, skippable | an engine-owned phase, not a step of the schedule (§2.3); never ERG after the ramp ends (§2.6) |

- **Fixed step and rate, not scaled by FTP.** The 0.75 factor belongs to a ramp rate. A ramp scaled to a
  rider's estimate (TR's approach) changes the rate per rider, and with it what 0.75 means. A fixed
  20 W/min keeps every result on one convention. The cost is fewer steps for a weak rider (a 120 W FTP
  fails at about the 4th step); the result uses measured power (§2.5), not step targets, so a 20 W step is
  not the FTP resolution.
- **Ceiling.** The schedule is generated up to a **top step of 700 W**, clamped to the trainer's
  `caps.max_target_w` (ADR-0002 §7.1). 700 W at failure means FTP ≈ 525 W, above any amateur. A relative
  bound such as "never above 2× the start" does not fit an absolute-watt ramp (2 × 100 W = 200 W would stop most riders
  short of failure), so the bound is absolute. A rider who completes the top step ends with the reason
  `CeilingReached` (§2.4), and the result is shown as "at least" and not offered for writing. **O2**.

### 2.3 Representation: a generated workout tree plus a dedicated assessment procedure

**The warm-up and ramp steps are a workout tree; the cool-down and the test's semantics are not.** The DSL can express those steps:
`StepNode`s with `StepEnd::Time` and `PowerTarget::Watts` (`model.rs` `PowerTarget`), so the existing compiler,
engine block timer, ERG policy and recorder ride it unchanged. The DSL **cannot** express, and should not:

- "the ride ends when the rider fails" (a `lap` step ends only on skip; adding a failure end to the grammar
  would let every workout end on a cadence drop, the opposite of §7.4's re-engage);
- "§7.4 is terminal here" (§2.6);
- "compute FTP from the ride and offer to write it".

So:

- **`crank-domain::assessment`** (new, no tokio): `RampProtocol { warmup_w, warmup_s, start_w, step_w, step_s,
  ceiling_w }` with `RampProtocol::V1` holding the constants of §2.2; `schedule(protocol, trainer_max_w) ->
  Vec<WorkoutNode>` (built directly, never parsed from text, so no `ImplausiblePower` and no FTP in the
  schedule); `ramp_result(protocol, &[RampSecond], end) -> RampResult` (§2.5), where `RampSecond { power_w:
  Option<f64>, target_w, released, paused }` is the domain's view of a recorded second (the engine maps its
  record rows onto it; the domain cannot depend on the engine). Pure functions.
- **`Workout` carries no flag.** The engine gets the assessment through its load: `RideSetup` (which holds the
  compiled `FlatBlock`s, not a tree) gains `kind: RideKind { Workout | Assessment(RampProtocol) }`, so a normal
  workout can never acquire assessment semantics by its content. The ride record stores the kind.
- **The cool-down is an engine-owned phase of `RideKind::Assessment`, not a workout step.** Today a step cannot
  command resistance: `PowerTarget` has no resistance variant, and the engine has no command that jumps from
  the ramp's end past the remaining ramp blocks. The two ways to close that gap:
  - *a new target kind `Resistance { pct }`* in the model, DSL, compiler, editor and ERG policy. Rejected: it
    widens the grammar of every workout for one test, it needs a rule under a health cap (a resistance block
    has no power bound, so `check_cap` would have to refuse it like `free`), and the jump command would still
    be needed, because the ramp's end is not a block boundary;
  - *an engine-owned phase*. **Chosen.** The schedule holds only the warm-up and the ramp blocks. When the
    ramp ends (§2.4), the engine drops the remaining ramp blocks, enters `AssessmentPhase::CoolDown`, commands
    `Resistance { pct: 0 }` (the mode of the user pause, the §7.4 release and the ride end, ADR-0002 §7.9) and
    runs a 5 min timer; when it expires, or on End, the ride finishes as usual. The rider's **Done** is a new
    `EngineCmd::EndRamp`, valid only in an assessment's ramp phase (`WrongState` otherwise); Pause in that
    phase is handled as `EndRamp` (§2.4). The phase and the end reason are fields on the ride status.
- The test is **not** a library entry the editor can open and change. It is listed with the workouts
  ("Ramp test") and started through the test card (§2.7).

### 2.4 The end

The ramp ends on the first of these. The engine owns the decision and reports `AssessmentEnd`:

| Reason | Trigger | Result |
|---|---|---|
| `Exhausted` | 8 consecutive ticks (2 s), each with a **real** cadence value < 40 rpm (a sample's cadence, or one held within its 4-tick hold, ADR-0001 §5) while the ERG target > 0 | computed |
| `RiderStopped` | the rider presses **Done** on the test view, or **Pause**, or confirms **End** during the ramp | computed |
| `CeilingReached` | the top step completes | "at least", not writable |
| `TrainerLost` | a loss is detected (8 ticks without a page-25 sample, or `Link(Lost)`) | **void** |
| `CadenceUnknown` | the §7.4 release fires without the `Exhausted` predicate holding (the cadence was unknown, expired or invalid on some of its ticks) and no loss was detected | **void** |
| `Safety` | Report symptom, the HR limit (§2.6), a medical stop raised mid-ride | void |

**Exhaustion vs dropout.** The §7.4 release on its own cannot tell them apart: the ERG policy counts an
unknown cadence as low (`policy.rs` `ErgPolicy`, `cadence.is_none_or(|c| c < RELEASE_RPM)`), so a silent
trainer is released too, a few ticks after its held cadence expires. Today only the order of the checks
(the 8-tick loss check usually fires first) keeps a dropout from looking like a release, which is a race
with about 1 s of margin. A test cannot rest on it. So:

- **`Exhausted` has its own predicate**, kept by the engine for assessment rides beside the §7.4 counter: a
  counter of consecutive ticks with a **real** cadence value below 40 rpm. A tick whose cadence is unknown,
  past its hold or invalid (page 25 `0xFF`, ADR-0001 §5 "Per-field hold") **resets** it. Only 8 such ticks
  make `Exhausted`. Unknown cadence never counts as exhaustion.
- **The §7.4 release still runs unchanged** and still takes the torque off at once: safety does not wait
  for the classification. In an assessment it is terminal (§2.6).
- **Order within a tick**, after the samples are merged: (1) the loss check: 8 ticks without a page-25
  sample, or an applied `Link(Lost)`, ends the ramp as `TrainerLost`; (2) the `Exhausted` predicate; (3) a
  §7.4 release with neither ends the ramp as `CadenceUnknown`. The first that holds wins and the ramp ends
  once. Whatever order a dropout's checks fire in, the outcome is void (`TrainerLost` or `CadenceUnknown`),
  never a result. An invalid cadence with valid power is `CadenceUnknown` and void, by this rule, not by
  the power-gap check.
  The rule is conservative: a run that mixes unknown and real low cadence before the release fires is
  `CadenceUnknown`, so an ambiguous end costs a retest, never a wrong FTP.

**After any end** (`TrainerLost` and `CeilingReached` included) the engine runs the cool-down only. The ramp
is never resumed: a reconnect after `TrainerLost` re-applies the cool-down's `Resistance { pct: 0 }`, never
an ERG target, and owes no soft start.

**Pause.** In a test, Pause ends the ramp (as on TrainerRoad [TR1]); the test view says so on the button.
A resume of a ramp would re-engage ERG at a high target with a soft start, which a maximal test must not
offer. Stopped pedalling (cadence 0 for 3 s) is preceded by the 2 s release and is `Exhausted`.
A warm-up that ends early (End, Pause, loss) gives no result and no ramp.

### 2.5 Result

- **Input:** the recorded per-second power (the record stores unfiltered per-second means, ADR-0002 §7.8),
  never the step targets, the display filter or the trainer's acknowledgements.
- **FTP = round(0.75 × MMP60)**, MMP60 = the **best gap-free 60 s window**: the highest mean over 60
  consecutive ramp seconds that all have a power value and none of which is released or paused (warm-up and
  cool-down excluded). The partial last step counts, as [TR1] says.
- **Validity** (each failure is a named reason on the result, and a void result is never offered for writing):
  1. the end is `Exhausted` or `RiderStopped`;
  2. the ramp ran **≥ 3 full steps** (reached 160 W): fewer is too short to be maximal;
  3. a gap-free window exists, and the final 3 min of the ramp have **≤ 2** null power seconds (else the
     best window could sit well before the end, which a gap hid);
  4. the best window ends within the **last 2 min** of the ramp: a maximal ramp peaks at its end;
  5. **compliance:** in the last full step before the end, the recorded power was within ±10 % of the target for ≥ 50 %
     of its seconds (the trainer actually held ERG). A trainer that never reached the targets gives a
     meaningless MMP60.
- **Not required:** heart rate. The ramp is a power test. HR, when present, is shown (peak HR) and is
  never an input to FTP. Feeding the athlete model's HRmax from the test is out of scope.
- **Uncertainty:** the stored estimate is `Estimate { value, sd, source: "test", n: 1 }` (`spec/ONBOARDING.md` §3)
  with **sd = 8 % of value**. This is a **convention and a floor**, not a measured error of this protocol:
  no study gives one. For scale only, [M22]'s 18 W sy.x is ~7 % at 250 W, for a different protocol and
  FTP definition; the true error of a single ramp is likely larger. It moves only by an amendment. **O3**.
- **Shown** on the ride summary: "Ramp test: FTP ≈ 287 W (± 23 W)", best minute 383 W, the end reason,
  peak HR, every validity check with a tick or its reason, and the current FTP with the change
  ("250 → 287 W, +15 %"). A change of more than ±30 % adds "much different from your current FTP: was
  the test maximal?". It does not block.
- **Explicit confirm.** Two buttons, **Use 287 W as my FTP** and **Keep 250 W**; nothing is written before
  the first. The confirm is a core command (`athlete_confirm_ftp { ride_id }`) that **recomputes** the result
  from the stored ride and writes the estimate with its provenance (ride id, method `ramp_v1_0.75`), so a UI
  can never write a value the core did not derive. It refuses a void result, a ride that is not a ramp test,
  or a ride already confirmed. The store and the resolution of FTP onto `RideSetup` are #247's; this ADR adds
  only the FTP estimate and the confirm, and #247 must carry an FTP field (§4).

### 2.6 Safety (ADR-0002 §7, DESIGN §5.8)

- **Start guard.** The test starts only through the core's single start command (#81), so every refusal
  holds on every path:
  - a **medical stop** refuses it;
  - a **health cap** refuses it, including the over-45 advisory cap, because a maximal test exceeds any cap
    by design (whether the advisory cap should instead allow the test after an explicit acknowledgement is
    **O5**). The engine's `check_cap` (`engine.rs` `check_cap`) already rejects a schedule above the cap
    (`CapExceededTarget`); the core refuses it earlier, with its own reason, so the UI can say why;
  - **"Unwell today"** refuses it (no planned ride that day);
  - the test card's **maximal-effort acknowledgement** must be ticked (§2.7). It is logged.
- **Medical stop and End always win.** Report symptom works as in any ride (DESIGN §5.2): `Safety` end,
  medical stop set, no result. End is never held (ADR-0002 §7.9).
- **§7.4 is terminal in a test.** After the release, the engine does **not** re-engage ERG at 60 rpm. It
  goes to the cool-down in `Resistance { pct: 0 }` (§2.3), as after every other end (§2.4). A soft start back to a failing target is exactly the
  hazard §7.4 exists to stop.
- **Cadence floor:** the §7.4 thresholds, unchanged (40 rpm, 2 s). The test adds no softer floor.
- **HR limit:** when the athlete model has an HRmax **not** from the prior (measured or self-reported), HR
  above HRmax + 5 bpm for 10 s ends the ramp as `Safety` with "heart rate above your maximum: stop and rest".
  Every trigger is logged with the HR, HRmax and its source. It is off when the medication answer switched off HR-based coaching (`spec/ONBOARDING.md` §1.5): the HR is
  not trusted then. A prior HRmax is too uncertain to stop a test on. **O4**.
- **Max target:** §2.2's ceiling, and the trainer cap. Above the trainer's `max_target_w` no step exists.
- **Warm-up:** 5 min at 80 W, and the ramp's first steps are easy. After the end: 5 min cool-down at 0 %
  resistance, skippable, and "keep spinning easy" on screen.
- **No coach.** No nudges, no live edits (DESIGN §17), no ± offset during an assessment: they would change
  the protocol. The ± keys are disabled in the test view.

### 2.7 UI (outline; the design amendment comes with the app sub-issue)

- **Test card** (Workouts): what it is, how long (~25 min), "maximal: you ride until you cannot", the
  acknowledgement tick, Start.
- **Ride view, ramp phase:** current target, the next step's target, time left in the step, cadence, and
  **Done** ("I've had enough, end the test"). Pause reads "Pause ends the test".
- **Summary:** the result block of §2.5 and its two buttons.

### 2.8 Sim and test strategy

- **Domain (pure):** the schedule never exceeds `min(ceiling, trainer max)`, starts at the protocol's watts for
  any FTP (property test over FTP), and its total length is the ceiling's; `ramp_result` against hand-made
  records: MMP60 with a partial last step, each validity failure on its own, the ±30 % warning, the sd.
- **Engine (paused time, `SimTrainer`):** each `AssessmentEnd` reason fires at the right tick; real cadence
  < 40 rpm (samples and held values) gives `Exhausted` on exactly the 8th tick; **silence after the last
  sample never gives `Exhausted`**, whichever of the loss check and the release fires first (both orders
  forced), and ends `TrainerLost` or `CadenceUnknown`; an invalid cadence with valid power gives
  `CadenceUnknown`; after every end the remaining ramp blocks are dropped and only `Resistance { 0 }` follows,
  also after a reconnect; `EndRamp` and Pause end the ramp and are `WrongState` outside it; ± and coach
  changes are refused.
- **Sim rider:** `SimRider` (`crates/crank-devices/src/sim/core.rs`) gets an exhaustion model: W′bal from its CP
  and W′ (Skiba [knowledge pack 02]); at W′bal = 0 cadence decays below 40 rpm. A seeded sim ramp then must
  yield an FTP inside a band derived from the rider's own CP/W′. This tests the pipeline end to end, not
  the physiology.
- **Core:** the start guard (stop, cap, unwell, no acknowledgement) each refused with its reason; the
  confirm recomputes, refuses void, foreign and repeated confirms, and writes provenance.
- **App:** test-backend mocks model each core refusal of the start and confirm commands (repo convention).
- **HIL (owner):** one ramp on the NEO, pedalled by the owner, checking that ERG holds at the high steps and
  that the release ends the test. A batched owner request, never assumed.
- **FMEA:** an `assessment` phase in `docs/safety/fmea.toml` (terminal release, void on loss, cap refusal).

## 3. Alternatives considered

- **20-min test first.** Better-studied factor [V18, B18], but needs pacing skill and a pacing target that a new
  rider lacks; a failed pace is undetectable. Rejected as the first test.
- **2×8.** Its FTP disagrees with lab thresholds [C3]; same pacing problem. Rejected.
- **CP test first.** The best-founded [L22, K21] and matches the athlete model, but two maximal self-paced
  efforts and a SIM-mode free-pacing UI crank has not built. Second, with its own ADR.
- **Submaximal HR test.** Validated only in well-trained men [L11]; needs HR; regression constants not
  transferable. Rejected for v1; may serve as a re-check later.
- **Ramp scaled to the current FTP** (TR's way). Changes the ramp rate per rider and needs an FTP. Rejected (§2.2).
- **A DSL keyword for "until failure".** It would put an end-on-failure rule into the general grammar,
  where §7.4 must re-engage. Rejected (§2.3).
- **A `Resistance { pct }` workout target for the cool-down.** Rejected in §2.3.
- **A fully separate procedure engine.** Duplicates ERG, block timing and recording. Rejected: the schedule
  rides the existing engine; only the end and the result are new.
- **Write FTP automatically.** A void or sub-maximal test would silently rescale every workout. Rejected: explicit confirm.

## 4. Consequences

- A new rider gets a measured FTP in one ~25 min session; the library's fixed 250 W stops being the only option.
- FTP becomes an `Estimate` with provenance. **#247 currently resolves CP, W′ and τ but has no FTP
  field**; phase 2 needs it there (sub-issue 4 below depends on #247).
- The engine gains a ride kind and a terminal release; normal workouts are unchanged (tested).
- The result is biased by rider type; the label and the band say so. A CP assessment will refine it.

**Phase 2 sub-issues, in order (not filed in this round):**

1. domain: `assessment` module: `RampProtocol::V1`, `schedule`, `RampSecond`, `ramp_result` and the validity checks (§2.2, §2.3, §2.5).
2. safety: the FMEA `assessment` phase rows (terminal release, the dropout ordering of §2.4, void on loss, cap refusal), before or alongside 4.
3. devices: `SimRider` exhaustion from W′bal (§2.8).
4. engine: `RideSetup.kind`, `RideKind::Assessment`, the `Exhausted` predicate and the end order of §2.4, `AssessmentEnd`, the engine-owned cool-down phase and the drop of the remaining ramp blocks, `EngineCmd::EndRamp`, Pause as `EndRamp`, the terminal release, no resume after any end, no ± or coach (§2.3, §2.4, §2.6).
5. core: the start command with its guard (stop, cap, unwell, acknowledgement), the result DTO, `athlete_confirm_ftp` (after #247).
6. app: test card, ramp view, summary result and confirm; bindings; design amendment (§2.7).
7. HIL: the owner-pedalled ramp on the NEO.

## 5. Open questions (owner)

| Question | Recommendation | Issue | Milestone |
|---|---|---|---|
| O1: is the ramp the default and only test for now, with a CP assessment next (and no 20-min/2×8)? | ramp first, CP next | filed on acceptance | — |
| O2: ceiling 700 W (clamped to the trainer); warm-up 5 min at 80 W, start 100 W, +20 W/min: keep Zwift's numbers? | keep Zwift's numbers and the 700 W ceiling | filed on acceptance | — |
| O3: sd = 8 % of FTP as the stated uncertainty of a ramp estimate | 8 %, stated as a convention floor | filed on acceptance | — |
| O4: the HR limit (HRmax + 5 bpm for 10 s, only with a non-prior HRmax): keep, change, or drop? | keep, and log every trigger | filed on acceptance | — |
| O5: the over-45 advisory cap refuses the test for every rider under it, shutting out a whole group. Intended, or should the test run after an explicit acknowledgement (the injury cap and medical stop still refuse)? | owner's call | filed on acceptance | — |

## 6. References (checked 2026-11-03)

- [Z1] Zwift Insider, "Determine your FTP with Zwift's new ramp test" (2019-03-01): 100 W start, +20 W/min. https://zwiftinsider.com/new-ramp-test
- [TR1] TrainerRoad support, "How to test with the Ramp Test": 1-min steps to failure, FTP = 75 % of best 1-min, ends on stop or pause, ERG default. https://support.trainerroad.com/hc/en-us/articles/360003910912-How-to-Test-With-the-Ramp-Test
- [TR2] TrainerRoad, "FTP testing tips": 20-min and 8-min tests switch ERG → slope for the effort. https://www.trainerroad.com/blog/ftp-assessment-tips/
- [M22] McGrath et al. (2022), Prediction of FTP from graded exercise test data in highly-trained individuals, *Int J Exerc Sci* 15(4):747–759 (70 triathletes; FTP = 0.95 × 20-min TT; GXT step protocol not reported). https://pmc.ncbi.nlm.nih.gov/articles/PMC9365101/
- [V18] Valenzuela et al. (2018), Is the FTP a valid surrogate of the lactate threshold?, *IJSPP* 13(10):1293–1298, doi:10.1123/ijspp.2018-0008. https://pubmed.ncbi.nlm.nih.gov/29745765/
- [B18] Borszcz et al. (2018), FTP in cyclists: validity of the concept and physiological responses, *Int J Sports Med* 39(10):737–742, doi:10.1055/s-0044-101546.
- [I19] Inglis et al. (2019), MLSS versus the 20-min FTP test in well-trained individuals: "watts" the big deal?, *IJSPP* 15(4):541. https://journals.humankinetics.com/view/journals/ijspp/15/4/article-p541.xml
- [W23] Wong et al. (2023), FTP is not a valid marker of the maximal metabolic steady state, *J Sports Sci*, doi:10.1080/02640414.2023.2176045.
- [K21] Karsten et al. (2021), Relationship between the CP test and a 20-min FTP test in cycling, *Front Physiol* 11:613151. https://pmc.ncbi.nlm.nih.gov/articles/PMC7862708/
- [L22] Leo et al. (2022), Power profiling and the power-duration relationship in cycling: a narrative review, *Eur J Appl Physiol* 122(2):301–316. https://pmc.ncbi.nlm.nih.gov/articles/PMC8783871/
- [L11] Lamberts et al. (2011), A novel submaximal cycle test to monitor fatigue and predict cycling performance, *Br J Sports Med* 45(10):797–804. https://pubmed.ncbi.nlm.nih.gov/19622525/
- [C1] Roadman Cycling, "How to do a ramp test" (coaching opinion on rider-type bias). https://roadmancycling.com/answers/how-to-do-a-ramp-test
- [C2] Road Bike Rider, "Why you don't need ERG mode for a ramp test" (one coach's anecdote). https://www.roadbikerider.com/no-erg-mode-ramp-test/
- [C3] Sanders, Taylor, Myers & Akubat, A field-based cycling test to assess predictors of endurance performance and establishing training zones (accepted manuscript, Newman University). https://newman.repository.guildhe.ac.uk/17203/1/NU0038.pdf
- In repo: `spec/knowledge-pack/02-critical-power-wbal.md`, `03-power-duration-models.md`; `spec/ONBOARDING.md`; ADR-0001 §5; ADR-0002 §7; DESIGN §5.2, §5.8, §15.2.
