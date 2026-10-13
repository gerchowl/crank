# 03 · Power–duration models (OmPD etc.) and the FTP problem

**Scope.** Which model to fit to mean-maximal power (MMP) history to get CP, W′, Pmax and predictions across 1 s – hours; what to do about FTP-centric zones (the contract currently has `pct_ftp`, `if`, `tss`, and `Workout.ftp`).
**Overall confidence:** model choice — C (few validation studies); FTP ≠ physiological threshold — B.

## Key findings

- **[C] OmPD extends CP to the whole duration range.** Omni-domain power-duration model: W′ expenditure rate bounded by Pmax, and power above the CP time-to-task-failure declining log-linearly. Fit to MMP of 9 cyclists: residuals 4 ± 1% (vs 6 ± 2% for an exponential variant), effective W′ stable between 120–1800 s; time-trial prediction error ~7 ± 5% (not different between omni variants) [@puchowicz2020]. Equation as reported in the narrative review: P(t) = (W′/t)·(1 − e^(−t·(Pmax−CP)/W′)) + CP for t ≤ CP_TTF; beyond CP_TTF subtract A·ln(t/CP_TTF) [@leo2021].
- **[B (narrative consensus)] Model choice should follow the domain of interest; avoid single-effort tests.** Recommended protocol: one ~10–15 s sprint for Pmax and ≥ 3 maximal efforts between 2 and 15 min (shortest 2–5 min, longest 12–15 min), ≥ 30 min active recovery between efforts, ideally over 2 days; fit CP/W′ with the non-linear 2-parameter model. Two-point fits always give R² = 1 and no error estimate. The authors explicitly *do not recommend* single-effort tests (8- or 20-min FTP) nor the 3-min all-out test for CP [@leo2021].
- **[C] The hyperbola is an approximation.** It fits 2–15 min but misrepresents < 2 min and > 15 min; a power-law model may be a safer pacing tool [@drake2023]. Race MMP can be modeled across durations in Grand Tours [@vinetti2022]; multi-domain lab-input models are emerging [@wahl2026]. Physiological basis of the curve's domains: [@burnley2018].
- **[B] FTP is not a physiological threshold.** Time to exhaustion at FTP (95% of 20-min power) was 50.9 ± 15.7 min (not 60), and FTP20/FTP60/IAT agreed on average but with LoA of roughly ±40–60 W [@borszcz2018]. At FTP, TTE was 33.7 ± 7.6 min; at FTP+15 W, 22 min with VO2 stabilizing below peak and end lactate 6.7 vs 9.2 mM → FTP "should not be considered a threshold marker between heavy and severe" [@wong2022]. FTP vs MLSS: not interchangeable [@borszcz2019; @lillobevia2019]. FTP ≈ LT in trained cyclists but underestimates LT in recreational ones (bias depends on fitness) [@valenzuela2018]. FTP test reliability and construct validity have also been examined in trained cyclists [@gough2025]; test–retest reliability alone does not establish physiological validity. General critique of threshold/zone methods: [@jamnick2020].
- **[B] Fatigue changes the curve** (see `01-durability.md`): the record power profile is lower after accumulated work [@mateomarch2022rpp; @leo2024].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| OmPD fit residual | ~4 ± 1% of MMP | C | [@puchowicz2020] |
| TT prediction error (omni models) | ~7 ± 5% | C | [@puchowicz2020] |
| CP fitting efforts | ≥ 3 efforts, 2–15 min; plus 10–15 s sprint for Pmax | B | [@leo2021] |
| CP_TTF (time sustainable at CP) | ~20–40 min; treat as a fitted parameter | B/C | [@leo2021; @jones2019] |
| TTE at FTP | 34–51 min (mean of two studies), large SD | B | [@wong2022; @borszcz2018] |
| FTP vs physiological thresholds | individual LoA ≈ ±40–60 W | B | [@borszcz2018] |

## Implications for the app

**Metrics to compute**
- Fit **OmPD** (or a CP-family model) to the athlete's 90-day MMP curve, using only efforts flagged maximal (self-paced / free-mode blocks), excluding ERG-held segments. Report CP, W′, Pmax, CP_TTF, and fit residuals; show the fitted curve on `mmp_curve`.
- Keep `pct_ftp` for compatibility, but compute an **internal anchor = CP** and derive FTP-equivalent only as a display convenience. Persist the anchor's provenance (test vs MMP fit vs user-entered) and age.
- `tss`/`if` are FTP-based conventions (D); keep them labeled as such (see 13).

**s1 judge features / rules**
- When the target is defined relative to FTP and the athlete's CP fit differs by > ~5%, the judge should reason on the CP-based domain ("this 105% FTP block is X% CP → severe/heavy").
- Do not `push` above a target near FTP for > ~30 min on the assumption it is "threshold" [@wong2022].

**Planner rules**
- Prefer CP-anchored domains: moderate (< VT1/LT1, or DFA-α1 > 0.75 proxy), heavy (VT1 → CP), severe (> CP), extreme (very short). Zone systems that assume FTP = MMSS are D-grade conventions.
- Suggest a structured CP test (≥ 3 efforts, 2–15 min, + sprint) every ~6–8 weeks or when the MMP fit is stale/uncertain (D for the interval; B for the protocol) [@leo2021].
- Never compute FTP from a single 20-min effort as the *only* anchor without a caveat.

**UI hints**
- Workout editor summary: show both "% FTP" and "% CP / domain" with a tooltip explaining they differ.
- MMP widget: today vs 90-day vs all-time plus the model fit and its uncertainty band.

## Controversies / uncertainty

- Hyperbolic vs power-law vs omni models: no model is validated across all populations; prediction errors of ~5–8% are typical [@puchowicz2020; @drake2023].
- Field-derived MMP depends on whether the athlete actually rode maximally at each duration; ERG-dominated indoor histories underpopulate the curve.
- FTP remains popular because it is simple and reliable; reliability does not imply validity as a domain boundary [@wong2022].

## References

<!-- REFS -->
- `borszcz2018` — Borszcz et al. (2018). Functional Threshold Power in Cyclists: Validity of the Concept and Physiological Responses. *International Journal of Sports Medicine*. doi:[10.1055/s-0044-101546](https://doi.org/10.1055/s-0044-101546) · PMID 29801189 · OpenAlex W2803585003
- `borszcz2019` — Borszcz et al. (2019). Is the Functional Threshold Power Interchangeable With the Maximal Lactate Steady State in Trained Cyclists?. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2018-0572](https://doi.org/10.1123/ijspp.2018-0572)
- `burnley2018` — Burnley & Jones (2018). Power-duration relationship: Physiology, fatigue, and the limits of human performance. *European Journal of Sport Science*. doi:[10.1080/17461391.2016.1249524](https://doi.org/10.1080/17461391.2016.1249524)
- `drake2023` — Drake et al. (2023). Modelling human endurance: power laws vs critical power. *European Journal of Applied Physiology*. doi:[10.1007/s00421-023-05274-5](https://doi.org/10.1007/s00421-023-05274-5) · PMID 37563307 · OpenAlex W4385737482
- `gough2025` — Gough et al. (2025). The reliability and construct validity of the functional threshold power test in trained cyclists. *Journal of Science and Cycling*. doi:[10.28985/1425.jsc.06](https://doi.org/10.28985/1425.jsc.06) · OpenAlex W7110826118
- `jamnick2020` — Jamnick et al. (2020). An Examination and Critique of Current Methods to Determine Exercise Intensity. *Sports Medicine*. doi:[10.1007/s40279-020-01322-8](https://doi.org/10.1007/s40279-020-01322-8)
- `jones2019` — Jones et al. (2019). The maximal metabolic steady state: redefining the 'gold standard'. *Physiological Reports*. doi:[10.14814/phy2.14098](https://doi.org/10.14814/phy2.14098) · PMID 31124324 · OpenAlex W2945126412
- `leo2024` — Leo et al. (2024). The Influence of High-Intensity Work on the Record Power Profile of Under-23, Pro Team, and World Tour Cyclists. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2023-0451](https://doi.org/10.1123/ijspp.2023-0451) · PMID 38531349
- `leo2021` — Leo et al. (2021). Power profiling and the power-duration relationship in cycling: a narrative review. *European Journal of Applied Physiology*. doi:[10.1007/s00421-021-04833-y](https://doi.org/10.1007/s00421-021-04833-y) · OpenAlex W3209937259
- `lillobevia2019` — Lillo-Bevia et al. (2019). Is the Functional Threshold Power a Valid Metric to Estimate the Maximal Lactate Steady State in Cyclists?. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000003403](https://doi.org/10.1519/jsc.0000000000003403) · OpenAlex W2984106587
- `mateomarch2022rpp` — Mateo-March et al. (2022). The Record Power Profile of Male Professional Cyclists: Fatigue Matters. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2021-0403](https://doi.org/10.1123/ijspp.2021-0403)
- `puchowicz2020` — Puchowicz et al. (2020). Development and field validation of an omni-domain power-duration model. *Journal of Sports Sciences*. doi:[10.1080/02640414.2020.1735609](https://doi.org/10.1080/02640414.2020.1735609) · PMID 32131692 · OpenAlex W3009761812
- `valenzuela2018` — Valenzuela et al. (2018). Is the Functional Threshold Power a Valid Surrogate of the Lactate Threshold?. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2018-0008](https://doi.org/10.1123/ijspp.2018-0008) · PMID 29745765 · OpenAlex W2802629842
- `vinetti2022` — Vinetti et al. (2022). Modeling the Power-Duration Relationship in Professional Cyclists During the Giro d'Italia. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000004350](https://doi.org/10.1519/jsc.0000000000004350) · OpenAlex W4293795272
- `wahl2026` — Wahl & Ji (2026). From diagnostics to prediction: development and validation of a multi-domain power-duration model. *European Journal of Applied Physiology*. doi:[10.1007/s00421-026-06142-8](https://doi.org/10.1007/s00421-026-06142-8) · OpenAlex W7133912119
- `wong2022` — Wong et al. (2022). Functional threshold power is not a valid marker of the maximal metabolic steady state. *Journal of Sports Sciences*. doi:[10.1080/02640414.2023.2176045](https://doi.org/10.1080/02640414.2023.2176045) · PMID 36803419 · OpenAlex W4321367666
