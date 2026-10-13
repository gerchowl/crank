# 12 · VLamax (maximal lactate accumulation rate)

**Scope.** Whether the planner should use "VLamax" (popular in commercial metabolic-profiling software) to classify riders or prescribe training.
**Bottom line:** the app has no blood-lactate measurement, so it cannot measure VLamax. Power-only "VLamax" estimates are unvalidated. Do not use VLamax as a planning input. **Grade of usefulness for this app: D.**

## Key findings

- **[C (expert review)] The construct is weakly defined.** Mader's original vLamax referred to muscle; the field test measures blood lactate rise over a 10–15 s all-out sprint corrected for an "alactic time". Problems: glycolytic flux is unlikely to be truly maximal, peak flux is brief, there is no criterion for reaching it, no correction for clearance or for oxidative ATP resynthesis, and the alactic-time correction is error-prone. The authors propose an uncorrected "vLapeak" instead [@wackerhage2025].
- **[A-/B] Reliability is variable.** Systematic review/meta-analysis (11 studies): ICC 0.47–0.96, CV 4.6–26.4%, pooled ICC 0.84; alactic-time estimation the least reliable (ICC −0.12 to 0.88); "small changes may fall within measurement error"; "should not be used in isolation for performance diagnostics" [@rohrs2026reliability]. Another review: ICC 0.66–0.96, moderate-to-excellent [@fernandezjarillo2025]. Cycling-specific 15-s test reliability [@harnish2023]; test-duration uncertainty [@yang2024].
- **[C] Protocol choices bias the value.** Fixed-duration sprints underestimate the instantaneous peak by ~10% (peak at ~9 s in elite sprinters) [@dunst2026]; maximal accumulation rates across 5–30 s sprints show a breakpoint, implying duration-dependence [@ozkaya2026].
- **[C] Derived "calculated MLSS" has wide individual error.** cP-MLSS from VLamax + VO2max was reliable (ICC 0.99) and not different on average from expert LT2, but LoA −39 to +29 W [@sablain2025].
- **[C] Utility beyond sprinting is unproven.** VLamax relates to sprint and strength measures; its value for predicting performances ≥ 1 min is inconclusive [@quittmann2026].
- **[D] Commercial claims.** The one study found on software-calculated VO2max from cycling power and VLamax evaluates a single commercial tool (INSCYD); authors' affiliations/conflicts were not verified in this search, and no independent validation was found [@vanschuylenbergh2026].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Test–retest ICC range | 0.47–0.96 (pooled 0.84) | A- | [@rohrs2026reliability] |
| Test–retest CV range | 4.6–26.4% | A- | [@rohrs2026reliability] |
| Duration bias of fixed 12-s test | ~−10% vs instantaneous peak | C | [@dunst2026] |
| cP-MLSS vs LT2 limits of agreement | −39 to +29 W | C | [@sablain2025] |

## Implications for the app

**Metrics to compute**
- None. Do **not** compute or display a "VLamax" from power data. If a user imports a lab VLamax, store it as a free-form note with test protocol and date.
- Use what the app *can* measure for the glycolytic/sprint end: Pmax, 10–30 s MMP, W′, OmPD fit (03), and repeatability under fatigue (01).

**s1 judge features / rules**
- None.

**Planner rules**
- Do not prescribe "lower your VLamax" / "raise your VLamax" training blocks; rider profiling should use the power-duration curve (CP, W′, Pmax) and durability instead.
- If the athlete brings VLamax-based zones from a coach/software, map them to CP/HRVT-based domains and flag discrepancies rather than adopting them blindly.

**UI hints**
- If a user asks about VLamax, explain briefly: needs blood lactate, reliability varies, value for long-event performance unproven; show W′ and sprint MMP instead.

## Controversies / uncertainty

- Proponents value VLamax for metabolic profiling and modeling (Mader model); critics note construct and measurement issues [@wackerhage2025; @rohrs2026reliability].
- Popular practice (commercial "metabolic profiles" with VLamax-based training targets) clearly outruns the published validation.

## References

<!-- REFS -->
- `dunst2026` — Dunst & Meixner (2026). Time-resolved energy-share reconstruction of individual glycolytic flux kinetics reveals duration-dependent bias in elite sprint cycling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-026-06397-1](https://doi.org/10.1007/s00421-026-06397-1) · PMID 42667324
- `fernandezjarillo2025` — Fernandez-Jarillo & Lomero-Arenas (2025). Reliability of estimating maximal glycolytic power using the maximal lactate accumulation rate (VLamax): a systematic review. *Retos*. doi:[10.47197/retos.v66.110040](https://doi.org/10.47197/retos.v66.110040) · OpenAlex W4408366418
- `harnish2023` — Harnish et al. (2023). Reliability of the 15-s Maximal Lactate Accumulation Rate (VLamax) Test for Cycling. *Physiologia*. doi:[10.3390/physiologia3040040](https://doi.org/10.3390/physiologia3040040) · OpenAlex W4388184054
- `ozkaya2026` — Ozkaya et al. (2026). Rates of maximal blood lactate accumulation in the extreme intensity exercise domain: implications for training prescription. *European Journal of Applied Physiology*. doi:[10.1007/s00421-026-06290-x](https://doi.org/10.1007/s00421-026-06290-x) · OpenAlex W7166593661
- `quittmann2026` — Quittmann (2026). Maximal lactate accumulation rate: Current evidence and future directions for exercise testing and training. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-06022-7](https://doi.org/10.1007/s00421-025-06022-7) · PMID 41171430
- `rohrs2026reliability` — Rohrs et al. (2026). Same Test, Different Stories? A Critical Look at vLamax Modality-Specific Reliability. *Journal of Science in Sport and Exercise*. doi:[10.1007/s42978-026-00371-1](https://doi.org/10.1007/s42978-026-00371-1) · OpenAlex W7167724139
- `sablain2025` — Sablain et al. (2025). Evaluating the maximal rate of lactate accumulation and estimated maximal lactate steady state in cycling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05751-z](https://doi.org/10.1007/s00421-025-05751-z) · PMID 40338332 · OpenAlex W4410215701
- `vanschuylenbergh2026` — Van Schuylenbergh et al. (2026). Validity of the Calculated VO2max from Cycling Power and VLamax. *International Journal of Sports Science and Physical Education*. doi:[10.11648/j.ijsspe.20261101.11](https://doi.org/10.11648/j.ijsspe.20261101.11) · OpenAlex W7132851274
- `wackerhage2025` — Wackerhage et al. (2025). Is the vLamax for Glycolysis What the VO2max is for Oxidative Phosphorylation?. *Sports Medicine*. doi:[10.1007/s40279-025-02259-6](https://doi.org/10.1007/s40279-025-02259-6) · PMID 40676393 · OpenAlex W4412492567
- `yang2024` — Yang et al. (2024). Uncertainty in determining the optimal test duration for maximal rate of lactate accumulation during all-out sprint cycle ergometry. *European Journal of Applied Physiology*. doi:[10.1007/s00421-024-05506-2](https://doi.org/10.1007/s00421-024-05506-2) · OpenAlex W4399139120
