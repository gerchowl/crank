# Candidate rules for the planner and the s1 judge

Machine-readable list (YAML below). Fields:

- `id` — stable ID `R-<TOPIC>-NN`
- `scope` — `planner` (pre/post-ride design), `judge` (in-ride ease_off/hold/push/replan), `metric` (what the core must compute), `ui`
- `rule` — the candidate rule (thresholds are starting points, to be tuned on user data)
- `rationale` — one line
- `grade` — A meta-analysis/consensus · B multiple RCTs or strong cohort · C single study/mechanistic · D practitioner convention
- `basis` — `evidence` (A/B) or `heuristic` (C/D). Heuristic rules should default to the `lean` band and must be easy to disable.
- `refs` — citation keys in `references.bib`; topic file in parentheses

Conventions: "CP" = athlete's current critical power (fall back to FTP-derived estimate, flagged); "HRVT1" = HR at DFA-α1 0.75 or lab VT1; "usual" = athlete's own 90-day reference at comparable context.

```yaml
# ---------------- general / cross-cutting ----------------
- id: R-GEN-01
  scope: planner
  rule: Anchor intensity domains on CP (heavy/severe) and VT1/HRVT1 (moderate/heavy); show %FTP only as a display convenience.
  rationale: FTP is not a valid MMSS marker; CP best represents the heavy/severe boundary.
  grade: B
  basis: evidence
  refs: [wong2022, borszcz2018, jones2019, galanrioja2020]   # (03, 02)
- id: R-GEN-02
  scope: judge
  rule: Never base a `push` on a single signal; require ≥2 concordant signals (e.g., W′bal margin + HR below usual at power + no readiness flags). `ease_off` may use one strong safety signal.
  rationale: Every individual marker (HR, α1, HRR, HRV) has wide individual error and context dependence.
  grade: D
  basis: heuristic
  refs: [kaufmann2023, bosquet2008, aubry2015, schaffarczyk2026]
- id: R-GEN-03
  scope: metric
  rule: Every derived metric carries a validity flag and context (artefact %, cadence, steady-state, elapsed time, kJ/kg, fueling logged); the judge must ignore invalid metrics.
  rationale: α1, HRR τ and decoupling are strongly confounded by artefact, cadence and fatigue.
  grade: C
  basis: heuristic
  refs: [rogers2021artefact, gronwald2018cadence, kounalakis2012]

# ---------------- durability (01) ----------------
- id: R-DUR-01
  scope: planner
  rule: When goals include events > 2–3 h, schedule ~1 session/week with a quality effort after accumulated work (≥ 2 h or ~15–25 kJ/kg), with fueling.
  rationale: Durability discriminates performance; effect of such sessions on durability is unproven.
  grade: C
  basis: heuristic
  refs: [vanerp2021, leo2024, hamilton2024, spragg2023training, maunder2021]
- id: R-DUR-02
  scope: metric
  rule: Compute a fatigued power profile — MMP(10 s, 1, 5, 20 min) after 0/10/20/30/40 kJ/kg and after 0/2.5/5/7.5 kJ/kg above CP — only from maximal/self-paced efforts.
  rationale: Field durability is repeatable (SEM < 5%) and level-discriminating.
  grade: B
  basis: evidence
  refs: [mateomarch2025reliability, vanerp2021, leo2024]
- id: R-DUR-03
  scope: planner
  rule: For efforts placed after > 2 h of riding, lower targets by ~10% of fresh CP (or by the athlete's own fatigued/fresh ratio when ≥ 3 data points exist).
  rationale: CP and VT1 power fall ~6–15% after 2–3 h; individual spread is large.
  grade: B
  basis: evidence
  refs: [jones2023, norte2026cp, stevenson2022, hamilton2024]
- id: R-DUR-04
  scope: judge
  rule: Late in long rides (≥ ~20 kJ/kg or ≥ 2 h), if HR at the target exceeds fresh HRVT1 HR by > 5–10 bpm in a block intended as moderate → ease_off −3 to −5% (lean).
  rationale: HR at the moderate/heavy transition rises and its power falls with prolonged work; HR decoupling tracks VT1 loss.
  grade: C
  basis: heuristic
  refs: [stevenson2022, rothschild2025]
- id: R-DUR-05
  scope: metric
  rule: Use kJ above CP (intensity-aware) alongside total kJ as the fatigue-dose input for durability and judge context.
  rationale: kJ alone ignores intensity; high-intensity work degrades power at lower total work.
  grade: B
  basis: evidence
  refs: [sanchezjimenez2025]

# ---------------- CP / W′bal (02) ----------------
- id: R-WB-01
  scope: judge
  rule: If modeled W′bal at the end of the current work block (at current power) is predicted < 0 → propose ease_off −3 to −5% before failure. Band strong only if τ is individualized, else lean.
  rationale: W′bal discriminates exhaustion in the field (AUC 0.91) but population τ mis-predicts recovery.
  grade: B
  basis: evidence
  refs: [skiba2014validation, bartram2017, welburn2025]
- id: R-WB-02
  scope: metric
  rule: Implement W′bal in differential form with a versioned τ model; default bi-exponential (≈50% τ≈20 s, 50% τ≈380 s, slow τ lengthening after repeated bouts) or Skiba-2 prior; replace with athlete-fitted τ after ≥ 2–3 qualifying sessions; report spread across two τ models as uncertainty.
  rationale: Reconstitution is bi-exponential and individual; integral vs differential forms differ.
  grade: C
  basis: heuristic
  refs: [chorley2022, chorley2023, skiba2021, skiba2012]
- id: R-WB-03
  scope: judge
  rule: After > 40 min of intermittent high-intensity work in a ride, reduce effective W′ by ~25% for W′bal predictions.
  rationale: W′ fell 25–33% after 40–80 min intermittent exercise while CP was unchanged; W′bal overestimated remaining W′.
  grade: C
  basis: heuristic
  refs: [black2026]
- id: R-WB-04
  scope: planner
  rule: Editor validation — warn if any planned block drives W′bal < 0 under the conservative τ model; plan sets to deplete ≤ ~80–90% of W′ unless the session intent is "to failure".
  rationale: Parameter error makes popular HIIT models able to prescribe impossible sessions.
  grade: D
  basis: heuristic
  refs: [briand2022, skiba2021]
- id: R-WB-05
  scope: planner
  rule: Shorten recovery or raise recovery power only when needed; recovery *power* has a larger effect on W′ reconstitution than recovery *duration*.
  rationale: Recovery power dominated W′ recovery; reconstitution slows disproportionately at higher recovery power.
  grade: C
  basis: heuristic
  refs: [sreedhara2020, chorley2023]
- id: R-WB-06
  scope: judge
  rule: If the athlete repeatedly finishes blocks with modeled W′bal ≤ 0, emit replan/retest (CP or W′ underestimated) instead of push.
  rationale: Model inputs are wrong; pushing on a wrong model compounds error.
  grade: D
  basis: heuristic
  refs: [skiba2021, welburn2025]

# ---------------- power-duration (03) ----------------
- id: R-PD-01
  scope: metric
  rule: Fit OmPD (CP, W′, Pmax, CP_TTF + log decay) to 90-day MMP using only maximal efforts; report residuals and uncertainty; flag fits with < 3 qualifying efforts in 2–15 min.
  rationale: OmPD fits MMP with ~4% residuals and extends CP to all durations; CP needs ≥ 3 efforts.
  grade: C
  basis: heuristic
  refs: [puchowicz2020, leo2021]
- id: R-PD-02
  scope: planner
  rule: Suggest a CP test (≥ 3 maximal efforts 2–15 min + 10–15 s sprint, ≥ 30 min apart, ideally over 2 days) when the anchor is > 6–8 weeks old or uncertain; do not rely on a single 20-min FTP test as sole anchor.
  rationale: Narrative-review consensus protocol; single-effort tests lack physiological basis.
  grade: B
  basis: evidence
  refs: [leo2021, wong2022, borszcz2018]
- id: R-PD-03
  scope: judge
  rule: Do not push to hold a target near FTP for > ~30 min on the assumption it is "threshold".
  rationale: TTE at FTP 34–51 min with large SD; FTP+15 W ≈ 22 min.
  grade: B
  basis: evidence
  refs: [wong2022, borszcz2018]

# ---------------- DFA-α1 (04) ----------------
- id: R-DFA-01
  scope: metric
  rule: dfa_a1 from RR with 2-min window recomputed every 5 s, detrending + threshold artefact correction; valid only if artefact ≤ 3–5%, steady power ≥ 2 min, cadence within usual range.
  rationale: Validated computation; artefact > ~5% biases α1 (up to 19% at 6%).
  grade: C
  basis: heuristic
  refs: [rogers2022update, rogers2021artefact]
- id: R-DFA-02
  scope: judge
  rule: In a block intended as Z1/endurance, if valid α1 < 0.75 for ≥ 2 min at target power → ease_off −5% (lean).
  rationale: α1 = 0.75 approximates VT1/LT1 at group level; individual LoA ~±11–13 bpm.
  grade: B
  basis: evidence
  refs: [kaufmann2023, rogers2020, mateomarch2023dfa]
- id: R-DFA-03
  scope: judge
  rule: Do not trigger ease_off from α1 < 0.5 alone during work intervals; use only as corroboration with W′bal/HR.
  rationale: HRVT2 (0.5) agreement is weaker; α1 is least accurate in the uncorrelated range and at high intensity.
  grade: C
  basis: heuristic
  refs: [mateomarch2023dfa, schaffarczyk2022, kaufmann2023]
- id: R-DFA-04
  scope: planner
  rule: Estimate HRVT1 only from fresh-state ramps/progressive warm-ups (first 20–30 min); mark as "estimated ±10 bpm"; refine with CP and lab data when available.
  rationale: Fatigue shifts HR at α1 = 0.75 by ~−12 bpm; bias depends on fitness and sex.
  grade: C
  basis: heuristic
  refs: [vanhooren2025, sheoran2024]

# ---------------- HR recovery (05) ----------------
- id: R-HRR-01
  scope: metric
  rule: Fit HR(t) = HR∞ + A·e^(−t/τ) online with a prior from the athlete's usual τ at similar recovery power and preceding effort; widen the band until ≥ 60–90 s of data; treat τ as unstable before ~3 min.
  rationale: τ stabilizes only after ≥ 3 min; mono-exponential is poor after maximal efforts.
  grade: C
  basis: heuristic
  refs: [pierpont2000, bartelsferreira2015, bosquet2008]
- id: R-HRR-02
  scope: judge
  rule: hr_recovery step — if HR∞ ≥ target with confidence after ≥ 90 s → apply the step's onUnreachable policy.
  rationale: Engineering rule following directly from the fitted model.
  grade: D
  basis: heuristic
  refs: [pierpont2000]
- id: R-HRR-03
  scope: judge
  rule: τ > ~1.3–1.5× usual (matched recovery power/preceding effort) in ≥ 2 consecutive recoveries → supporting evidence for ease_off or dropping a rep; never use a fast HRR alone as a reason to push.
  rationale: HRR reliability is moderate; faster HRR can accompany functional overreaching.
  grade: C
  basis: heuristic
  refs: [bosquet2008, aubry2015, daanen2012]
- id: R-HRR-04
  scope: planner
  rule: Track HRR trends only from a standardized submax step reaching ~85–90% HRmax with fixed recovery power.
  rationale: Lowest day-to-day variation at 85–90% HRmax and in the first recovery minute.
  grade: C
  basis: heuristic
  refs: [lamberts2009, daanen2012]

# ---------------- intensity distribution (06) ----------------
- id: R-TID-01
  scope: planner
  rule: Plan ≥ 75–80% of weekly time below VT1/HRVT1 with 1–2 (max 3) high-intensity sessions; do not enforce a rigid polarized 80/20 split — pyramidal is equally valid.
  rationale: Meta-analyses show no performance superiority of POL over PYR; some high intensity is needed.
  grade: A
  basis: evidence
  refs: [rosenblat2025, oliveira2024, cove2025, foster2022]
- id: R-TID-02
  scope: planner
  rule: Recreational/low-volume athletes default to pyramidal; competitive athletes may run polarized blocks.
  rationale: IPD network meta-analysis subgroup effect by performance level.
  grade: B
  basis: evidence
  refs: [rosenblat2025, oliveira2024]
- id: R-TID-03
  scope: judge
  rule: Enforce session intent — in Z1 sessions ease_off when power/HR/α1 indicate drift into the heavy domain for > 2–3 min.
  rationale: Grey-zone creep erodes intended distribution.
  grade: D
  basis: heuristic
  refs: [foster2022]
- id: R-TID-04
  scope: metric
  rule: Report weekly TID in 3 physiological zones by power and by HR, plus session-goal TID and the polarization index (PI > 2.0 = polarized).
  rationale: TID depends on quantification method.
  grade: C
  basis: heuristic
  refs: [treff2019, foster2022]

# ---------------- carbohydrate (07) ----------------
- id: R-CHO-01
  scope: planner
  rule: Attach a fueling target to every workout > 75–90 min — 30–60 g/h for 1–2.5 h; up to 90 g/h glucose+fructose for > 2.5 h; 90–120 g/h only if the athlete opted in (trained, gut-tolerant).
  rationale: Duration-based guidance; 120 g/h raises exogenous oxidation and is tolerable, performance gain less certain.
  grade: A
  basis: evidence
  refs: [jeukendrup2014, morton2026, podlogar2022oxidation, hearris2022]
- id: R-CHO-02
  scope: planner
  rule: Durability-focused long sessions include fueling by default (≥ 60 g/h; 90–100+ g/h if tolerated).
  rationale: Higher CHO intake attenuates CP and TT loss after 3–4 h.
  grade: B
  basis: evidence
  refs: [mazza2026, norte2026cp]
- id: R-CHO-03
  scope: judge
  rule: In rides > 90 min with no intake logged in the last ~45 min and rising decoupling, prompt fueling and prefer hold/ease_off over push.
  rationale: Low CHO availability lowers power at the moderate/heavy transition and blunts durability.
  grade: C
  basis: heuristic
  refs: [main2026, norte2026cp, hamilton1991]
- id: R-CHO-04
  scope: ui
  rule: Fueling reminders every 15–20 min in long rides with one-tap logging (+20/30/40 g, +500 ml).
  rationale: Practical adherence aid.
  grade: D
  basis: heuristic
  refs: []

# ---------------- cadence (08) ----------------
- id: R-CAD-01
  scope: judge
  rule: Before attributing rising HR or falling α1 to fatigue, check cadence; if cadence rose > ~10 rpm vs block start, attribute part of the change to cadence and lower the band.
  rationale: Higher cadence raises HR/lactate, lowers α1 and exaggerates drift at the same power.
  grade: C
  basis: heuristic
  refs: [gronwald2018cadence, kounalakis2012]
- id: R-CAD-02
  scope: judge
  rule: A sustained cadence decline (> ~5 rpm) in a long steady block together with rising decoupling is supporting evidence of fatigue.
  rationale: 1 rpm decline ≈ +0.6% drift/decoupling.
  grade: C
  basis: heuristic
  refs: [barsumyan2026]
- id: R-CAD-03
  scope: planner
  rule: Default free cadence; offer low-cadence (50–70 rpm) HIIT/SIT as optional variation; avoid 40-rpm moderate intervals as a primary tool.
  rationale: Small, conflicting RCTs.
  grade: C
  basis: heuristic
  refs: [hebisz2024, kristoffersen2014, whitty2016, mater2021]

# ---------------- drift / decoupling (09) ----------------
- id: R-DRIFT-01
  scope: judge
  rule: If drift pushes HR above HRVT1 HR for > 3 min in a block intended as moderate → ease_off −3 to −5% with message "heat or dehydration — same watts now cost more".
  rationale: In heat/dehydration HR drift reflects a real rise in relative intensity.
  grade: B
  basis: evidence
  refs: [wingo2020, montain1992, hamilton1991]
- id: R-DRIFT-02
  scope: metric
  rule: Compute decoupling only on steady blocks, excluding the first ~10 min and non-steady segments; store cadence, duration and fluid/CHO context.
  rationale: Drift begins after ~10–20 min and is modulated by hydration, CHO, cadence.
  grade: B
  basis: evidence
  refs: [coyle2001, hamilton1991, kounalakis2012]
- id: R-DRIFT-03
  scope: ui
  rule: Do not label "decoupling > 5%" as failure; show decoupling as a trend vs the athlete's own comparable sessions.
  rationale: The 5% rule has no supporting study in this search.
  grade: D
  basis: heuristic
  refs: []
- id: R-DRIFT-04
  scope: judge
  rule: Never push early in a session because HR is low; HR lags and drift has not developed.
  rationale: Drift onset after ~10–20 min.
  grade: B
  basis: evidence
  refs: [coyle2001, wingo2012]

# ---------------- readiness (10) ----------------
- id: R-RDY-01
  scope: planner
  rule: Keep a predefined plan structure and modulate day-to-day (swap/shorten/move key session ±1–2 days) from a readiness vector; do not generate plans purely from HRV.
  rationale: HRV-guided gains are small/non-significant on average but reduce non-responders.
  grade: A
  basis: evidence
  refs: [duking2021, manresarocamora2021, nuuttila2022]
- id: R-RDY-02
  scope: planner
  rule: Weight subjective check-in (sleep, legs, stress, motivation) at least as heavily as HRV; downgrade a key session when ≥ 2 concordant negative signals; any severe flag (illness, pain, sleep ≤ 1/5) → easy/rest.
  rationale: Subjective measures track load more sensitively than objective markers.
  grade: B
  basis: evidence
  refs: [saw2015, alfonso2025]
- id: R-RDY-03
  scope: planner
  rule: After poor sleep, first shorten/reduce long continuous efforts (> 30 min) before cutting short intervals.
  rationale: Sleep deprivation impairs endurance (SMD −0.52), more for efforts > 30 min.
  grade: A
  basis: evidence
  refs: [lopes2023, craven2022]
- id: R-RDY-04
  scope: planner
  rule: Treat unusually high HRV / low HR / fast HRR with caution when combined with high load or poor subjective state (possible overreaching), not as a green light.
  rationale: Fatigue shifts and scatters HRV; overreaching can speed HRR.
  grade: C
  basis: heuristic
  refs: [schmitt2013, aubry2015]

# ---------------- intervals (11) ----------------
- id: R-INT-01
  scope: planner
  rule: Default VO2max session for trained cyclists — 3 × 13 × 30/15 s with 3 min between series; alternate with 4–6 × 4–5 min long intervals.
  rationale: Short intervals outperformed effort-matched long intervals in cyclists.
  grade: B
  basis: evidence
  refs: [ronnestad2015, ronnestad2020, ronnestad2021]
- id: R-INT-02
  scope: judge
  rule: In VO2max sets, if reps are completed with margin (W′bal clearly > 0 at rep end, HR plateau below usual, no readiness flags) → push +2–5% on the next rep.
  rationale: Self-paced maximal efforts produced several-fold more time > 90% VO2max than fixed intensity.
  grade: C
  basis: heuristic
  refs: [appelhans2025]
- id: R-INT-03
  scope: judge
  rule: If rep power cannot be held → ease_off −3–5% for remaining reps; after ≥ 2 consecutive failed reps despite easing → replan (end set or convert to threshold).
  rationale: Preserve session stimulus and avoid accumulating failure.
  grade: D
  basis: heuristic
  refs: [buchheit2013a]
- id: R-INT-04
  scope: ui
  rule: Label time above 90% HRmax as "not time at VO2max".
  rationale: Time > 90% HRmax and > 90% VO2max moved in opposite directions.
  grade: C
  basis: heuristic
  refs: [fleckenstein2025]
- id: R-INT-05
  scope: planner
  rule: Expect large individual response to HIIT blocks; re-test after 4–8 weeks and adapt rather than escalate blindly.
  rationale: Individual VO2max response −8% to +15% to an identical block.
  grade: C
  basis: heuristic
  refs: [bratlandsanda2020, milanovic2015]

# ---------------- VLamax (12) ----------------
- id: R-VLA-01
  scope: planner
  rule: Do not compute, display, or plan around "VLamax"; profile riders with CP, W′, Pmax and durability.
  rationale: Requires blood lactate; reliability variable (CV up to 26%); not to be used in isolation; power-only estimates unvalidated.
  grade: B
  basis: evidence
  refs: [rohrs2026reliability, wackerhage2025, quittmann2026]

# ---------------- training load (13) ----------------
- id: R-LOAD-01
  scope: planner
  rule: Use CTL/ATL/TSB-type curves (generic 42/7-day constants) only as descriptive trends; never schedule tapers or peaks from them alone.
  rationale: Fitness–fatigue models predict individual performance poorly; parameters poorly identifiable; generic constants discouraged.
  grade: B
  basis: evidence
  refs: [busso2023, marchal2025, vermeire2022]
- id: R-LOAD-02
  scope: planner
  rule: Do not apply ACWR thresholds (e.g., 0.8–1.3) as injury rules; limit week-to-week load jumps by convention and readiness.
  rationale: Coupling artefacts, heterogeneous binning, team-sport origin; no cycling evidence.
  grade: B
  basis: evidence
  refs: [lolli2017, andrade2020, qin2025]
- id: R-LOAD-03
  scope: metric
  rule: Store kJ, kJ above CP, zone time, TSS/IF (labelled FTP convention), HR-TRIMP and sRPE per session; never treat equal TSS as equal stimulus.
  rationale: Load metrics diverge with intensity; no single load metric predicts adaptation.
  grade: B
  basis: evidence
  refs: [vanerp2019, voet2025, sanchezjimenez2025]
```
