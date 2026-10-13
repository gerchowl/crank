# crank knowledge pack (v1)

Evidence base for the **LLM planner** (designs and rewrites workouts from history, readiness and the pre-ride check-in) and the **System-1 judge** (in-ride `ease_off | hold | push | replan` nudges with `strong | lean | uncertain` bands). Domain: indoor cycling on a Tacx NEO (ERG / sim / free) with a chest strap delivering RR intervals.

- **Literature search date:** scitadel searches run 2026-10-12 (UTC); targeted follow-up searches (PubMed/OpenAlex) and paper retrieval 2026-10-12/13. Pack compiled 2026-10-13.
- **Sources:** only items returned by scitadel searches (PubMed, OpenAlex). Every citation has a DOI, PMID or OpenAlex ID in `references.bib`. Where only an abstract (or only the aim of an abstract) was available, claims were limited to what the abstract states; nothing was taken from memory.
- **Not a medical device.** Rules are training heuristics for healthy athletes.

## Files

| File | Topic | Main consumers |
|---|---|---|
| `01-durability.md` | Fatigue resistance, fatigued power profile, kJ/kg checkpoints | planner, judge, metrics |
| `02-critical-power-wbal.md` | CP as the heavy/severe boundary, W′, W′bal reconstitution τ | judge, editor validation, metrics |
| `03-power-duration-models.md` | OmPD and other PD models; FTP critique; CP testing | metrics, planner |
| `04-dfa-alpha1.md` | DFA-α1 thresholds (0.75 / 0.5), computation, artefacts, fatigue | judge, metrics |
| `05-hr-recovery.md` | HR recovery kinetics, τ fitting, interpretation pitfalls | `hr_recovery` widget/step, judge |
| `06-intensity-distribution.md` | Polarized vs pyramidal vs threshold TID | planner |
| `07-carbohydrate.md` | CHO g/h by duration, 90 vs 120 g/h, durability | planner, ride UI |
| `08-cadence.md` | Cadence, efficiency, cadence as confounder and fatigue sign | judge, planner |
| `09-cardiovascular-drift-decoupling.md` | Drift mechanisms, decoupling as durability proxy | judge, metrics |
| `10-readiness-hrv-guided.md` | HRV-guided training, subjective check-ins, sleep | planner (check-in → proposal) |
| `11-intervals-vo2max.md` | 30/15 short intervals, time near VO2max, self-pacing | planner, judge |
| `12-vlamax.md` | Why the app should not use VLamax | planner |
| `13-training-load-models.md` | Banister/fitness–fatigue, CTL/ATL/TSB, ACWR critique | planner, analytics UI |
| `rules.md` | 53 candidate rules (YAML) with grade, basis and refs | planner + judge implementation |
| `references.bib` | BibTeX for every cited item | — |

Each topic file has: key findings (graded, cited) · numbers the app can use · implications (metrics / s1 judge / planner / UI) · controversies & uncertainty · references.

## Evidence grades

| Grade | Meaning | How the app should treat it |
|---|---|---|
| **A** | Meta-analysis, systematic review with pooled estimate, or consensus/guideline | May drive defaults; still individualize |
| **B** | Multiple RCTs or a strong/consistent cohort (incl. good field cohorts) | May drive defaults and `strong` bands where measurement is reliable |
| **C** | Single study, small trials with conflicting results, or mechanistic reasoning | Heuristic: `lean` band at most; easy to disable; log outcomes |
| **D** | Practitioner convention, engineering choice, or no supporting study found | Heuristic: explicit opt-in or UI-only; must be labelled as convention |

Modifiers like "A-" mean a systematic review without pooled effect, or a meta-analysis with serious heterogeneity. `rules.md` marks every rule as `evidence` (A/B) or `heuristic` (C/D).

## How the planner should use the pack

1. **Anchor intensities physiologically.** Use CP (heavy/severe) and VT1/HRVT1 (moderate/heavy) as domain anchors; keep %FTP for display only (02, 03). Note on the contract: `Workout.ftp`, `pct_ftp`, `if`, `tss` are conventions; carry an internal CP anchor with provenance and age.
2. **Weekly structure:** mostly low intensity, 1–2 (max 3) high-intensity sessions, pyramidal or polarized — no rigid 80/20 dogma (06). Short-interval 30/15 sets as the default VO2max session for trained riders (11).
3. **Day-to-day:** start from the planned session and modulate with the readiness vector; subjective check-in weighs at least as much as HRV; sleep loss → cut long continuous efforts first (10).
4. **Long rides:** fueling target always attached (07); durability work only for long-event goals, with fatigued-profile targets (01).
5. **Do not** plan from VLamax (12), ACWR thresholds or CTL/TSB "form" alone (13).
6. **Rationale text:** cite the rule IDs and the grade in the proposal rationale so the athlete sees *why* (e.g., "R-RDY-02 (B): sleep 2/5 and legs 2/5 → easier").

## How the s1 judge should use the pack

- **Inputs per decision** (all with validity flags): power vs target, `wbal_pct` (+ model spread), HR vs usual at this power (`hr_vs_typical`), `hr_drift`/`decoupling`, `dfa_a1` + artefact %, cadence and its change, HR-recovery fit (τ vs usual, `unreachable`), elapsed time, kJ/kg and kJ above CP, CHO/fluid logged, today's readiness vector, block intent (phase).
- **Asymmetry:** `ease_off` may follow one strong *safety-relevant* signal (W′bal heading below 0, HR above HRVT1 in a Z1 block); `push` needs ≥ 2 concordant signals and no readiness flags (R-GEN-02).
- **Band calibration:** rules graded C/D should cap at `lean`; `strong` requires an A/B rule *and* a valid, individualized measurement (e.g., individually fitted W′ τ).
- **Confounders first:** check cadence change, artefact, heat/drift and time-since-step before attributing a signal to fatigue (R-GEN-03, R-CAD-01).
- **Log everything** (decision, inputs, accepted/declined, outcome) so heuristic thresholds can be tuned per athlete — the literature cannot supply individual thresholds.

## Biggest places where popular practice outruns evidence

- **FTP-centric zones:** FTP is neither the MMSS nor a 60-min power for many riders (TTE 34–51 min; LoA ±40–60 W) (03).
- **Rigid polarized 80/20:** recent meta-analyses find no performance superiority over pyramidal (06).
- **VLamax-based profiling:** construct and reliability problems; app cannot measure it (12).
- **ACWR sweet spot / CTL-ATL-TSB:** coupled ratios, team-sport origin, poorly identifiable fitness–fatigue parameters (13).
- **"Decoupling < 5%" and "HRV readiness scores":** conventions without validation (09, 10).
- **W′bal with population τ:** depletion is reasonable, recovery constants are not (02).

## Main gaps (where evidence is thin)

- Durability *training* interventions (what improves it) — almost none (01).
- Real-time in-ride decision rules (what to change, by how much, when) — no trials; the judge is necessarily heuristic and must learn per athlete.
- HR recovery during *active* in-ride recoveries on a trainer; typical τ in trained cyclists (05).
- DFA-α1 in women, older/low-fit riders, and in fatigue/durability use (04).
- Cycling-specific load–injury and load–performance models (13).
- Long-term (> 16 weeks) TID and HIIT trials in cyclists (06, 11); 120 g/h CHO performance trials (07); gut training (not searched).

## Regenerating

`references.bib` and the reference lists were generated from a scratch reference table (key, authors, year, title, journal, DOI/PMID/OpenAlex) assembled from scitadel results; cited keys are validated against that table. When adding a claim, add the paper via scitadel first and record its identifiers.
