# 04 · DFA-α1 (fractal HRV) as an intensity / threshold / fatigue marker

**Scope.** Using RR intervals from the chest strap to estimate the aerobic threshold (HRVT1), possibly the second threshold (HRVT2), and in-ride internal intensity / fatigue (`dfa_a1`, `rr` in the contract).
**Overall confidence:** HRVT1 ≈ VT1/LT1 at group level — A-/B; HRVT2 ≈ VT2 — C; real-time use for fatigue/durability — C; individual-level agreement — limited (wide LoA).

## Key findings

- **[A-] DFA-α1 = 0.75 approximates the first threshold at group level, but with wide individual limits of agreement.** Systematic review (27 studies, 461 participants): HR at HRVT_low vs VT_low bias 1 bpm (LoA −10.9 to +12.8 bpm); vs LT_low 2.7 bpm (LoA −20.4 to +25.8 bpm); evidence mainly in healthy males [@kaufmann2023]. Original derivation and validations: [@rogers2020; @rogers2021cardiac; @rogers2022triathletes; @gronwald2020; @gronwald2019review]. Elite cyclists: α1 = 0.75 vs LT1 not different (r = 0.85 power, 0.66 HR) [@mateomarch2023dfa]. Women: HRVT1 vs VT1 HR bias −4.7 bpm (LoA −20 to +10.6 bpm) [@schaffarczyk2023].
- **[C] α1 = 0.5 ≈ second threshold is less secure.** Proposed as HRVT2 [@rogers2021ant]; in elite cyclists α1 = 0.5 differed significantly from LT2 (power p = 0.04; HR p = 0.02) despite r = 0.93 [@mateomarch2023dfa]; in women HRVT2 vs VT2 bias 0.5 bpm (LoA −10.8 to +11.8 bpm) [@schaffarczyk2023]. Review: HRVT_high "needs further empirical evaluation" [@kaufmann2023].
- **[B] Reliability is good, bias depends on fitness and sex.** HR at α-thresholds CV < 6%; α1 overestimates thresholds in low-fit and underestimates in high-fit people (VO2max > 55 mL/kg/min); males +2.4 bpm vs females −1.3 bpm bias at the first threshold [@sheoran2024]. Between-day reliability in running acceptable [@vanhooren2023].
- **[B/C] Computation details matter.** Standard practice: Kubios-style detrending, **2-min window, recomputed every 5 s**, threshold artefact correction [@rogers2022update]. Artefact: 1–3% missed beats → negligible bias; 6% → up to 19% proportional bias at low α1 values; HR at HRVT shifted < 1 bpm [@rogers2021artefact; @rogers2022update]. Device: Polar H7 HRVT ~4 bpm lower than ECG [@rogers2021artefact]; Polar H10 RR/HR near-identical to ECG, but α1 bias/LoA widen at high intensity (LoA up to +58/−41%) [@schaffarczyk2022]. Values in the uncorrelated range (< ~0.75) are the least accurate.
- **[C] α1 is affected by non-intensity factors.** Higher cadence (120 vs 60 rpm) lowered α1 and raised HR/lactate at the same power [@gronwald2018cadence]; hypoxia [@gronwald2019hypoxia]; temperature, breathing pattern, hydration, nutrition and illness also modulate it [@rogers2022update].
- **[C] Fatigue shifts the α1–intensity relationship.** In a second ramp after a first, HR at α1 = 0.75 was 12.3 bpm lower than gas-exchange VT1 (LoA −30 to +6 bpm) — thresholds are only valid in a non-fatigued state, which also opens fatigue-based uses [@vanhooren2025]. During 20 min at HRVT1 speed α1 stayed ~0.86 with small drift; at HRVT2 speed α1 fell 13–36% and 6/21 athletes could not finish [@gronwald2024]. Durability/“session deterioration of α1 per watt” is proposed but unvalidated [@rogers2022update; @gronwald2021case].
- **[C] Everyday workout data** (power vs α1 regression) has been explored for zone estimation without a formal ramp (exploratory) [@andriolo2024].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| HRVT1 (aerobic threshold proxy) | α1 = 0.75 | A-/B | [@kaufmann2023; @rogers2020] |
| HRVT2 (proposed) | α1 = 0.50 | C | [@rogers2021ant; @mateomarch2023dfa] |
| Window / update | 2-min window, recompute every 5 s (or ~200 beats) | C (convention from validation studies) | [@rogers2022update] |
| Artefact tolerance | ≤ 3% negligible; ≥ 5–6% → discard / low confidence | C | [@rogers2021artefact] |
| Individual LoA HRVT1 vs VT1 | about ±11–13 bpm | A- | [@kaufmann2023] |
| Fatigue shift of HR at α1 = 0.75 | ~−12 bpm in 2nd ramp | C | [@vanhooren2025] |
| α1 at 20 min HRVT1 / HRVT2 steady state | ~0.86 / ~0.60 (falling 13–36%) | C | [@gronwald2024] |

## Implications for the app

**Metrics to compute**
- `dfa_a1`: from RR, 2-min window, 5-s grid; publish together with **artefact %** and a validity flag (valid if artefact ≤ 3–5%, HR stable, cadence within the athlete's normal range).
- **HRVT1 estimation** from ramps or progressive warm-ups: regress α1 vs power and HR, read crossing at 0.75; store HR and power at HRVT1 with CI; re-estimate only in *fresh* state (first 20–30 min) [@vanhooren2025].
- **Session α1 drift per watt** (α1 at matched power late vs early) as an *exploratory* durability marker (C).

**s1 judge features / rules**
- Use α1 primarily to **guard low-intensity blocks**: if a block intended as Z1/endurance shows valid α1 < 0.75 sustained ≥ 2 min at target power → `ease_off` −5% (lean) [@rogers2020; @gronwald2024].
- Do **not** use α1 < 0.5 alone to trigger `ease_off` in work intervals (expected there); use it as supporting evidence with W′bal and HR drift.
- Suppress α1-based rules when artefact > 5%, during cadence > ~100 rpm or rapid power changes (< 2 min since step), and in the first ~5 min of a ride (C/D).
- Late in a long ride, a fall in α1 at unchanged power plus HR decoupling is supporting evidence for durability loss (C).

**Planner rules**
- If no lab thresholds exist, initialize moderate/heavy boundary from HRVT1 (α1 = 0.75) and refine with CP; mark as "estimated, ±10 bpm".
- Optional "α1 ramp check" in warm-ups (e.g., 3 × 3-min steps) to track HRVT1 over weeks — fresh state only (C/D).

**UI hints**
- `dfa_a1` widget with 0.75 / 0.5 reference lines, artefact badge, and greyed-out invalid segments.
- Explain to users that α1 is an *estimate*; individual error can exceed ±10 bpm.

## Controversies / uncertainty

- Agreement is good at group level; individual LoA are wide, especially for HRVT2 and in women/low-fit groups (data sparse) [@kaufmann2023; @sheoran2024].
- Strong dependence on preprocessing, device RR precision, and artefact handling; results from one software pipeline may not transfer [@rogers2022update; @schaffarczyk2022].
- Much of the literature comes from a small group of authors; independent replication is growing but still limited.
- Fatigue-based uses (durability, readiness from warm-up α1) are plausible but not validated against outcomes [@vanhooren2025; @rogers2022update].

## References

<!-- REFS -->
- `andriolo2024` — Andriolo et al. (2024). Relationship of Cycling Power and Non-Linear Heart Rate Variability from Everyday Workout Data: Potential for Intensity Zone Estimation and Monitoring. *Sensors*. doi:[10.3390/s24144468](https://doi.org/10.3390/s24144468)
- `gronwald2021case` — Gronwald et al. (2021). Real-Time Estimation of Aerobic Threshold and Exercise Intensity Distribution Using Fractal Correlation Properties of Heart Rate Variability: A Single-Case Field Application in a Former Olympic Triathlete. *Frontiers in Sports and Active Living*. doi:[10.3389/fspor.2021.668812](https://doi.org/10.3389/fspor.2021.668812) · OpenAlex W3166335414
- `gronwald2019review` — Gronwald & Hoos (2019). Correlation properties of heart rate variability during endurance exercise: A systematic review. *Annals of Noninvasive Electrocardiology*. doi:[10.1111/anec.12697](https://doi.org/10.1111/anec.12697) · OpenAlex W2972463369
- `gronwald2019hypoxia` — Gronwald et al. (2019). Effects of Acute Normobaric Hypoxia on Non-linear Dynamics of Cardiac Autonomic Activity During Constant Workload Cycling Exercise. *Frontiers in Physiology*. doi:[10.3389/fphys.2019.00999](https://doi.org/10.3389/fphys.2019.00999)
- `gronwald2024` — Gronwald et al. (2024). Correlation properties of heart rate variability for exercise prescription during prolonged running at constant speeds: A randomized cross-over trial. *European Journal of Sport Science*. doi:[10.1002/ejsc.12175](https://doi.org/10.1002/ejsc.12175) · PMID 39300759 · OpenAlex W4402693444
- `gronwald2018cadence` — Gronwald et al. (2018). Non-linear dynamics of cardiac autonomic activity during cycling exercise with varied cadence. *Human Movement Science*. doi:[10.1016/j.humov.2018.06.013](https://doi.org/10.1016/j.humov.2018.06.013) · PMID 29966866
- `gronwald2020` — Gronwald et al. (2020). Fractal Correlation Properties of Heart Rate Variability: A New Biomarker for Intensity Distribution in Endurance Exercise and Training Prescription?. *Frontiers in Physiology*. doi:[10.3389/fphys.2020.550572](https://doi.org/10.3389/fphys.2020.550572) · OpenAlex W3087321042
- `kaufmann2023` — Kaufmann et al. (2023). Heart Rate Variability-Derived Thresholds for Exercise Intensity Prescription in Endurance Sports: A Systematic Review of Interrelations and Agreement with Different Ventilatory and Blood Lactate Thresholds. *Sports Medicine - Open*. doi:[10.1186/s40798-023-00607-2](https://doi.org/10.1186/s40798-023-00607-2) · PMID 37462761 · OpenAlex W4384626208
- `mateomarch2023dfa` — Mateo-March et al. (2023). Validity of detrended fluctuation analysis of heart rate variability to determine intensity thresholds in elite cyclists. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2047228](https://doi.org/10.1080/17461391.2022.2047228) · PMID 35238695
- `rogers2022triathletes` — Rogers et al. (2022). An Index of Non-Linear HRV as a Proxy of the Aerobic Threshold Based on Blood Lactate Concentration in Elite Triathletes. *Sports*. doi:[10.3390/sports10020025](https://doi.org/10.3390/sports10020025) · OpenAlex W4212983271
- `rogers2020` — Rogers et al. (2020). A New Detection Method Defining the Aerobic Threshold for Endurance Exercise and Training Prescription Based on Fractal Correlation Properties of Heart Rate Variability. *Frontiers in Physiology*. doi:[10.3389/fphys.2020.596567](https://doi.org/10.3389/fphys.2020.596567) · OpenAlex W3123545861
- `rogers2021ant` — Rogers et al. (2021). Detection of the Anaerobic Threshold in Endurance Sports: Validation of a New Method Using Correlation Properties of Heart Rate Variability. *Journal of Functional Morphology and Kinesiology*. doi:[10.3390/jfmk6020038](https://doi.org/10.3390/jfmk6020038) · OpenAlex W3159046206
- `rogers2021artefact` — Rogers et al. (2021). Influence of Artefact Correction and Recording Device Type on the Practical Application of a Non-Linear Heart Rate Variability Biomarker for Aerobic Threshold Determination. *Sensors*. doi:[10.3390/s21030821](https://doi.org/10.3390/s21030821)
- `rogers2022update` — Rogers & Gronwald (2022). Fractal Correlation Properties of Heart Rate Variability as a Biomarker for Intensity Distribution and Training Prescription in Endurance Exercise: An Update. *Frontiers in Physiology*. doi:[10.3389/fphys.2022.879071](https://doi.org/10.3389/fphys.2022.879071) · OpenAlex W4229372143
- `rogers2021cardiac` — Rogers et al. (2021). Aerobic Threshold Identification in a Cardiac Disease Population Based on Correlation Properties of Heart Rate Variability. *Journal of Clinical Medicine*. doi:[10.3390/jcm10184075](https://doi.org/10.3390/jcm10184075) · OpenAlex W3198099088
- `schaffarczyk2022` — Schaffarczyk et al. (2022). Validity of the Polar H10 Sensor for Heart Rate Variability Analysis during Resting State and Incremental Exercise in Recreational Men and Women. *Sensors*. doi:[10.3390/s22176536](https://doi.org/10.3390/s22176536) · PMID 36081005 · OpenAlex W4293715556
- `schaffarczyk2023` — Schaffarczyk et al. (2023). Validation of a non-linear index of heart rate variability to determine aerobic and anaerobic thresholds during incremental cycling exercise in women. *European Journal of Applied Physiology*. doi:[10.1007/s00421-022-05050-x](https://doi.org/10.1007/s00421-022-05050-x) · PMID 36269394 · OpenAlex W4307035996
- `sheoran2024` — Sheoran et al. (2024). Exercise intensity measurement using fractal analysis of heart rate variability: Reliability, agreement and influence of sex and cardiorespiratory fitness. *Journal of Sports Sciences*. doi:[10.1080/02640414.2024.2421691](https://doi.org/10.1080/02640414.2024.2421691) · PMID 39488502 · OpenAlex W4404001830
- `vanhooren2023` — Van Hooren et al. (2023). The Between-Day Reliability of Correlation Properties of Heart Rate Variability During Running. *Applied Psychophysiology and Biofeedback*. doi:[10.1007/s10484-023-09599-x](https://doi.org/10.1007/s10484-023-09599-x) · OpenAlex W4385382517
- `vanhooren2025` — Van Hooren et al. (2025). Correlation properties of heart rate variability to assess the first ventilatory threshold and fatigue in runners. *Journal of Sports Sciences*. doi:[10.1080/02640414.2023.2277034](https://doi.org/10.1080/02640414.2023.2277034) · PMID 37916488 · OpenAlex W4388224140
