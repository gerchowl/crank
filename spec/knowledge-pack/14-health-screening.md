# 14 · Pre-participation health screening (red flags)

**Scope.** The Health check in onboarding (`spec/ONBOARDING.md` §1.5, #117), the daily re-check and the medical-stop guard (DESIGN §5.8, #81, #93). What each item is derived from, how it escalates, and how often it is asked again.
**Bottom line:** crank is **not a medical device**. It *refers*, it never *clears*. Five red-flag items and two impact items are crank's own wording, informed by PAR-Q+ and the ACSM 2015 algorithm; they are **not** the PAR-Q+ and must never be labelled as it. The hard medical stop and the daily re-check are product choices that are stricter than either source. **Grade of evidence for the screening content: A (consensus); for crank's escalation and interval: D (convention).**

## Key findings

- **[A (consensus)] PAR-Q+ is the Canadian evidence-based consensus replacement for PAR-Q/PARmed-X.** It uses a 7-item general section, follow-up questions per condition and a decision tree, built from systematic reviews with the AGREE instrument [@warburton2011consensus; @jamnik2011; @bredin2013]. A "yes" to a general item leads to the follow-up pages (and ePARmed-X+ or a qualified exercise professional), not automatically to a physician [@bredin2013].
- **[A (consensus)] ACSM 2015 screens on three factors:** current physical-activity level, presence of signs/symptoms and/or known cardiovascular, metabolic or renal disease, and desired exercise intensity. Risk-factor profiling was dropped from screening to remove unnecessary barriers; the aim is to identify people at elevated risk of exercise-related sudden cardiac death or acute MI [@riebe2015]. Only the abstract was available in the search; the symptom list and the decision boxes were not read from the primary text.
- **[A (consensus)] PAR-Q+ clearance is time-limited.** The current form (2025 edition) has the participant declare that clearance is valid for a maximum of 12 months from the date of completion and becomes invalid if their condition changes [PAR-Q+ form, see References].
- **[D] Crank's hard medical stop is stricter than the sources.** Any red-flag "yes" stops plan and coach; the sources send the person to follow-up questions or an exercise professional first. This is a deliberate product choice (DESIGN §5.8), not a literature finding.
- **[D] Crank's daily re-check of the five red flags is stricter than the sources' 12-month validity.** No source found asks for daily re-screening; it is a convention chosen because each ride is a start of exertion on a trainer with no supervisor.

## Licence of the PAR-Q+ (read this before reusing any wording)

The PAR-Q+ is © PAR-Q+ Collaboration and states: "You are encouraged to photocopy the PAR-Q+. You must use the entire questionnaire and NO changes are permitted." (2025 form, p. 4.) Consequences for crank:

1. crank **does not reproduce** PAR-Q+ item text. The table below paraphrases the *content* of each source item and cites it; the exact wording is on the form.
2. crank's items are **its own questions**. They are modified, shortened and re-ordered, so they are **not the PAR-Q+** and the UI/docs must say "informed by", never "PAR-Q+" or "PAR-Q+ screening". (`spec/ONBOARDING.md` §1.5 already says "PAR-Q-style".)
3. If the owner wants a screen that may be called PAR-Q+, it must be the unmodified full form (or ePARmed-X+ through its operators); that is a different feature. Raised as an open question in the PR.

## Item wording, derivation and deliberate differences

Crank wording is the DESIGN §5.8 table, verbatim. "Source item" paraphrases the PAR-Q+ 2025 general question number; ACSM is cited at factor level only (abstract).

| # | crank wording (DESIGN §5.8) | Source item (paraphrased) | Where crank differs, and why |
|---|---|---|---|
| R1 | Chest pain, pressure or tightness — at rest or when exercising? | PAR-Q+ Q2: pain in the chest at rest, during daily activities or when physically active. ACSM: signs/symptoms factor [@riebe2015]. | Adds "pressure or tightness" (how angina is described by laypeople); drops "during daily activities" because "at rest or when exercising" covers it for a rider. |
| R2 | Fainting, or dizziness that made you lose balance, in the last 12 months? | PAR-Q+ Q3: losing balance because of dizziness, or losing consciousness, in the last 12 months. | PAR-Q+ says to answer NO if the dizziness came from over-breathing (including vigorous exercise). **crank omits that exception**: a self-screen cannot judge the cause, so it errs towards stopping. |
| R3 | A racing or irregular heartbeat together with dizziness or breathlessness? | **No matching PAR-Q+ 2025 general item.** ACSM symptom factor covers palpitations/tachycardia as a sign/symptom class [@riebe2015] (class named in the abstract; the specific list not verified here). | crank-added item; source is the ACSM factor, not a questionnaire line. Paired with dizziness/breathlessness so common post-effort heartbeat awareness is not a red flag. |
| R4 | Unusual breathlessness at light effort? | **No matching PAR-Q+ 2025 general item.** ACSM symptom factor, as R3. | crank-added item; "unusual" and "light effort" keep normal exertional breathlessness out. |
| R5 | A heart condition a doctor told you to limit exercise for? | PAR-Q+ Q1: a heart condition or high blood pressure ever diagnosed; Q7: a doctor said only medically supervised activity. ACSM: known cardiovascular disease factor. | Narrower than Q1 (no blood pressure; needs a doctor's *limit*) and broader than Q7 (a limit, not only "supervised"). High blood pressure alone does not stop a ride. |

Impact items (not red flags; no medical stop):

| # | crank wording | Source item | Difference and why |
|---|---|---|---|
| I1 | Medication that changes your heart rate? | PAR-Q+ Q5: prescribed medication for a chronic condition. | Different purpose: the answer is a **coaching input** (HR zones/HR-vs-usual off), not a clearance question. The narrower wording asks only what crank can act on. |
| I2 | An injury that limits hard efforts? | PAR-Q+ Q6: a bone, joint or soft-tissue problem that could be made worse by more activity (follow-up page by condition). | PAR-Q+ routes by condition; crank only asks whether hard efforts are limited and responds with a **cap** (100 % of CP, no sprints). The cap level is a crank engineering convention (D); no source gives it. |

PAR-Q+ items on other chronic conditions (Q4) and the follow-up pages (arthritis, cancer, metabolic, respiratory, mental health, etc.) are **not** screened by crank; any such condition is outside what crank can judge. This is a scope statement, not a safety claim.

## Escalation

| Event | crank (DESIGN §5.8) | Source recommendation |
|---|---|---|
| Any red flag R1–R5 "yes" (onboarding, daily re-check, or symptom during a ride) | **Medical stop**: no plan, coach off and silent, no capped plan; riding only as a deliberate, logged free ride; ride start refused by the core guard (#81). Message: see a doctor before training; emergency line for chest pain now. | PAR-Q+: complete follow-up pages and/or ePARmed-X+ / qualified exercise professional [@bredin2013]. ACSM: signs/symptoms or known disease are the factors that move a person towards medical evaluation before starting or progressing [@riebe2015]. |
| Medical stop lifted | Only by **recording a clearance**: date of the check (not in the future), who cleared the rider, an explicit statement; every change logged. | PAR-Q+ clearance is the participant's declaration, valid ≤ 12 months [PAR-Q+ form]. |
| Medication "yes" | HR zones, HR-guided recovery, HR-vs-usual off; power/feel targets. | None (coaching decision). |
| Injury "yes" | Cap at 100 % of CP, no sprints. | None (coaching decision). |
| Unanswered item | Continue disabled; no plan proposed. | None (product invariant). |

## Re-screening

- **Sources:** PAR-Q+ clearance valid at most 12 months from the date completed and void on a change in condition (also "Your health changes" is a delay-and-ask trigger on the form). Temporary illness (cold, fever) and pregnancy are listed delay triggers [PAR-Q+ form]. ACSM 2015: screening depends on *current activity level* and *desired intensity*, so a change in either changes the outcome [@riebe2015] (abstract-level).
- **crank:** all five red flags again once a day before the first start (DESIGN §5.8), a symptom in a ride at once, and the periodic check-in about every 6 weeks or when goals change (`spec/ONBOARDING.md` §3).
- **Gap:** the daily re-check asks only the five red flags. A new medication or injury (I1, I2) is only caught at the periodic check-in or by the athlete editing Settings. Reported in the PR.

## Not a medical device

crank gives training guidance for healthy riders and is **not a medical device** (README of this pack). It does not diagnose, treat or clear anyone. A medical stop is a **referral**; lifting it is the rider's **dated statement** that a clinician or qualified professional cleared them, recorded as that and nothing more. No UI or copy may say crank "cleared", "approved" or "passed" a rider.

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| PAR-Q+ clearance validity | ≤ 12 months, void on change in condition | A | PAR-Q+ 2025 form |
| Dizziness look-back window | 12 months | A | PAR-Q+ 2025 form (Q3) |
| crank re-check interval | 1 day (all 5 red flags) | D | DESIGN §5.8 |
| Health cap | 100 % of CP, no sprints | D | DESIGN §5.8 |

## Implications for the app

**Planner / guard**
- Any unanswered red flag, or any "yes", means no proposal anywhere (DESIGN §5.8). The guard lives in core (#81); the UI must not re-implement it.
- Cite this entry (`14-health-screening.md`, `@riebe2015`, `@warburton2011consensus`) rather than DESIGN §5.8 from onboarding and the re-check.

**UI hints**
- Label the step "Health check", not "PAR-Q+". Say crank refers and does not clear.
- Show the source and the "not a medical device" line next to the clearance record in Settings → Health.

## Controversies / uncertainty

- **Over-referral vs under-referral.** ACSM's own rationale for the 2015 update is that the previous screening produced excessive physician referrals and a barrier to exercise [@riebe2015]; crank's stricter-than-source stop trades that barrier for caution on a home trainer without supervision. It is a product decision the owner can revisit.
- **R3/R4 have no questionnaire source.** They rest on ACSM's symptom factor and crank's judgement; wording was not validated.
- **Self-screening validity.** No evidence in this pack that a layperson's self-answers match clinical findings; the sources themselves require a declaration and follow-up routing.

## References

<!-- REFS -->
- `bredin2013` — Bredin et al. (2013). PAR-Q+ and ePARmed-X+: new risk stratification and physical activity clearance strategy for physicians and patients alike. *Canadian Family Physician* 59(3):273–277. PMID [23486800](https://pubmed.ncbi.nlm.nih.gov/23486800/)
- `jamnik2011` — Jamnik et al. (2011). Enhancing the effectiveness of clearance for physical activity participation: background and overall process. *Applied Physiology, Nutrition, and Metabolism* 36(S1):S3–S13. doi:[10.1139/h11-044](https://doi.org/10.1139/h11-044) · PMID 21800946
- `riebe2015` — Riebe et al. (2015). Updating ACSM's Recommendations for Exercise Preparticipation Health Screening. *Medicine and Science in Sports and Exercise* 47(11):2473–2479. doi:[10.1249/MSS.0000000000000664](https://doi.org/10.1249/MSS.0000000000000664) · PMID 26473759
- `warburton2011consensus` — Warburton et al. (2011). Evidence-based risk assessment and recommendations for physical activity clearance: Consensus Document 2011. *Applied Physiology, Nutrition, and Metabolism* 36(S1):S266–S298. doi:[10.1139/h11-062](https://doi.org/10.1139/h11-062) · PMID 21800945
- `warburton2011intro` — Warburton et al. (2011). Evidence-based risk assessment and recommendations for physical activity clearance: an introduction. *Applied Physiology, Nutrition, and Metabolism* 36(S1):S1–S2. doi:[10.1139/h11-060](https://doi.org/10.1139/h11-060) · PMID 21800937
- PAR-Q+ form — PAR-Q+ Collaboration. *PAR-Q+ The Physical Activity Readiness Questionnaire for Everyone*, 2025 edition (form dated 01-11-2024). © PAR-Q+ Collaboration. <https://eparmedx.com/wp-content/uploads/2025/01/PARQPlus2025Fillable.pdf>, read 2026-10-14. Not a BibTeX entry (no DOI/PMID); the licence text above is quoted from p. 4.
