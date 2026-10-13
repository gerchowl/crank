# 13 · Training load and fitness–fatigue models (Banister, CTL/ATL/TSB, ACWR)

**Scope.** How the planner should quantify load (TSS, kJ, TRIMP, sRPE), whether to use impulse-response (Banister) / CTL–ATL–TSB "form" curves, and ACWR-style spike rules.
**Overall confidence:** load must be monitored and progressed sensibly — B/D; Banister/fitness-fatigue models predicting an *individual's* performance — weak (B for the negative finding); ACWR as an injury predictor — weak/contested (A-level heterogeneity, methodological flaws); evidence is from swimming, team sports and running — little cycling-specific validation.

## Key findings

- **[B] Fitness–fatigue (impulse-response) models fit history but predict poorly.** In 11 swimmers over 61 weeks, Banister and variable dose-response models had the best fit (MAPE ~2.0–2.7% on validation data), yet "their ability to predict future performance from past data was not satisfactory for individual training planning" [@busso2023]. Bayesian analysis across datasets: the model is ill-conditioned, fitness/fatigue parameters poorly identifiable, and adding the fatigue component did not improve prediction (p > 0.40) — questioning the biological meaning of the fatigue term [@marchal2025].
- **[C] Parameters depend on modeling choices; generic constants are inappropriate.** Fitted values depend on starting values, fitting technique and the load metric; "the use of general constants should be avoided" (this applies to the fixed 42-day/7-day CTL/ATL convention) — use the model to be "data-informed rather than data-driven" [@vermeire2022]. Different load quantification methods change the model outcome [@vermeire2021]. Conceptual issues and ML hybrids: [@imbach2022]; Bayesian estimation helps but does not cure identifiability [@peng2023]; cycling field-data estimation of Banister parameters [@scarf2019]; energy-system-specific (3-D) impulse-response extensions [@kontro2026]; teaching overview of the models [@clarke2013].
- **[B] Load metrics disagree, especially across intensities.** In 11,716 pro sessions, kJ predicted sRPE, TRIMP and TSS, but TSS behaved differently at high vs low intensity factor (consistent with TSS ∝ IF²); training IF 0.59 vs racing 0.73 [@vanerp2019]. Short-term dose–record-power relationships in professional road cyclists (time, TRIMP, TSS, time in power zones) were examined with explicitly listed threats to validity [@gallo2023]. No single load measure predicted fresh or fatigued TT gains [@voet2025]. kJ ignores intensity for durability (01) [@sanchezjimenez2025].
- **[A-/B] ACWR is methodologically compromised as an injury predictor.** The coupled ACWR (acute week inside the chronic window) creates spurious correlation [@lolli2017]; a rejoinder discusses what coupling means for practical ACWR use [@windt2019]. Systematic review of team sports: most studies associate high ACWR with injury, but 14 different binning methods, coupled calculations in 95% of studies, and heterogeneity "limit the strength of recommendations" [@andrade2020]; further systematic reviews exist [@griffin2020; @maupin2020]; quality appraisals of ACWR reviews themselves vary [@macmillan2020]. A 2025 meta-analysis reports lowest injury incidence at ACWR 0.8–1.3 but urges caution due to heterogeneity and possible publication bias [@qin2025]; multilevel meta-analysis in team sports [@ding2026]. A prospective cohort examined ACWR in recreational runners [@nakaoka2021]. Evidence in cyclists is essentially absent (gap).

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Banister prediction error (swimmers) | MAPE ~2–2.7%, "not satisfactory for individual planning" | B | [@busso2023] |
| CTL/ATL time constants 42 d / 7 d | convention only; individual constants vary and are poorly identifiable | D | [@vermeire2022; @marchal2025] |
| IF typical (pros) | training 0.59, racing 0.73 | C | [@vanerp2019] |
| ACWR "sweet spot" 0.8–1.3 | reported in team sports; heterogeneous, coupled-ratio artifacts | C/D | [@qin2025; @andrade2020; @lolli2017] |

## Implications for the app

**Metrics to compute**
- Store **multiple load measures per session**: kJ, kJ above CP (01/02), time in physiological zones (06), TSS/IF (labelled FTP-convention), HR-TRIMP, and sRPE if the athlete rates the ride (sRPE is cheap and integrates subjective strain; see 10).
- Rolling 7/28/42-day sums and EWMA "fitness/fatigue" curves may be *displayed* as descriptive trends, with the caveat that they are not predictions.
- If an impulse-response model is offered, fit it per athlete with Bayesian priors and show uncertainty; validate it on held-out data before using it to drive decisions [@marchal2025; @peng2023].

**s1 judge features / rules**
- Not an in-ride input except as context: very high recent load (e.g., last 7 days ≫ usual) plus negative readiness → lower `push` propensity (C/D).

**Planner rules**
- Progress load gradually and keep week-to-week jumps moderate, but do **not** enforce hard ACWR thresholds as injury rules for cyclists (evidence weak, team-sport-derived, methodologically flawed) [@lolli2017; @andrade2020].
- Do not schedule tapers or "peak form" dates solely from TSB/form curves; use them as one input with readiness and recent performance [@busso2023; @vermeire2022].
- Use intensity-aware load: two sessions with equal TSS but different domain distribution are not equivalent stimuli [@vanerp2019; @sanchezjimenez2025].

**UI hints**
- Label CTL/ATL/TSB-style charts "descriptive trend (model with generic constants)". Avoid "fitness" as a literal claim.
- Show ACWR, if at all, as "load spike vs your recent normal", without injury-risk color coding.

## Controversies / uncertainty

- Proponents argue fitness–fatigue models remain useful for *reasoning* about taper and load; critics show poor identifiability and prediction [@clarke2013; @marchal2025; @imbach2022].
- ACWR debate continues (coupling, binning, causal interpretation) [@lolli2017; @windt2019; @macmillan2020].
- TSS/CTL are proprietary-origin conventions with no validation found in this search beyond the intensity-dependence finding [@vanerp2019].

## References

<!-- REFS -->
- `andrade2020` — Andrade et al. (2020). Is the Acute: Chronic Workload Ratio (ACWR) Associated with Risk of Time-Loss Injury in Professional Team Sports? A Systematic Review of Methodology, Variables and Injury Risk in Practical Situations. *Sports Medicine*. doi:[10.1007/s40279-020-01308-6](https://doi.org/10.1007/s40279-020-01308-6) · PMID 32572824
- `busso2023` — Busso & Chalencon (2023). Validity and Accuracy of Impulse-Response Models for Modeling and Predicting Training Effects on Performance of Swimmers. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000003139](https://doi.org/10.1249/mss.0000000000003139) · PMID 36791017 · OpenAlex W4320856000
- `clarke2013` — Clarke & Skiba (2013). Rationale and resources for teaching the mathematical modeling of athletic training and performance. *Advances in Physiology Education*. doi:[10.1152/advan.00078.2011](https://doi.org/10.1152/advan.00078.2011) · OpenAlex W2161774578
- `ding2026` — Ding et al. (2026). Acute:chronic workload ratio and load management for team sports: a multilevel meta-analysis. *Frontiers in Public Health*. doi:[10.3389/fpubh.2026.1896651](https://doi.org/10.3389/fpubh.2026.1896651)
- `gallo2023` — Gallo et al. (2023). Relationships between training dose and record power outputs in professional road cyclists: insights and threats to validity. *Biology of Sport*. doi:[10.5114/biolsport.2023.114284](https://doi.org/10.5114/biolsport.2023.114284)
- `griffin2020` — Griffin et al. (2020). The Association Between the Acute:Chronic Workload Ratio and Injury and its Application in Team Sports: A Systematic Review. *Sports Medicine*. doi:[10.1007/s40279-019-01218-2](https://doi.org/10.1007/s40279-019-01218-2)
- `imbach2022` — Imbach et al. (2022). The Use of Fitness-Fatigue Models for Sport Performance Modelling: Conceptual Issues and Contributions from Machine-Learning. *Sports Medicine - Open*. doi:[10.1186/s40798-022-00426-x](https://doi.org/10.1186/s40798-022-00426-x) · PMID 35239054
- `kontro2026` — Kontro et al. (2026). The three-dimensional impulse-response model: Modeling the training process in accordance with energy system-specific adaptation. *PLoS ONE*. doi:[10.1371/journal.pone.0341721](https://doi.org/10.1371/journal.pone.0341721) · OpenAlex W7128165520
- `lolli2017` — Lolli et al. (2017). Mathematical coupling causes spurious correlation within the conventional acute-to-chronic workload ratio calculations. *British Journal of Sports Medicine*. doi:[10.1136/bjsports-2017-098110](https://doi.org/10.1136/bjsports-2017-098110) · PMID 29101104 · OpenAlex W2766293842
- `macmillan2020` — MacMillan et al. (2020). Variability in the Study Quality Appraisals Reported in Systematic Reviews on the Acute:Chronic Workload Ratio and Injury Risk. *Sports Medicine*. doi:[10.1007/s40279-020-01333-5](https://doi.org/10.1007/s40279-020-01333-5) · OpenAlex W3049647644
- `marchal2025` — Marchal et al. (2025). Statistical flaws of the fitness-fatigue sports performance prediction model. *Scientific Reports*. doi:[10.1038/s41598-025-88153-7](https://doi.org/10.1038/s41598-025-88153-7) · PMID 39881202 · OpenAlex W4406954482
- `maupin2020` — Maupin et al. (2020). The Relationship Between Acute: Chronic Workload Ratios and Injury Risk in Sports: A Systematic Review. *Open Access Journal of Sports Medicine*. doi:[10.2147/oajsm.s231405](https://doi.org/10.2147/oajsm.s231405)
- `nakaoka2021` — Nakaoka et al. (2021). The Association Between the Acute:Chronic Workload Ratio and Running-Related Injuries in Dutch Runners: A Prospective Cohort Study. *Sports Medicine*. doi:[10.1007/s40279-021-01483-0](https://doi.org/10.1007/s40279-021-01483-0) · OpenAlex W3164408299
- `peng2023` — Peng et al. (2023). Bayesian inference of the impulse-response model of athlete training and performance. *International Journal of Performance Analysis in Sport*. doi:[10.1080/24748668.2023.2268480](https://doi.org/10.1080/24748668.2023.2268480) · OpenAlex W4387578642
- `qin2025` — Qin et al. (2025). Acute to chronic workload ratio (ACWR) for predicting sports injury risk: a systematic review and meta-analysis. *BMC Sports Science, Medicine and Rehabilitation*. doi:[10.1186/s13102-025-01332-x](https://doi.org/10.1186/s13102-025-01332-x) · PMID 41029871
- `sanchezjimenez2025` — Sanchez-Jimenez et al. (2025). Is intensity the most important factor in determining the amount of prior work accumulated that affects cyclists' acute durability? A systematic review. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05885-0](https://doi.org/10.1007/s00421-025-05885-0) · PMID 40613880 · OpenAlex W4412017080
- `scarf2019` — Scarf et al. (2019). Modelling the effect of training on performance in road cycling: estimation of the Banister model parameters using field data. *arXiv*. doi:[10.48550/arxiv.1902.02061](https://doi.org/10.48550/arxiv.1902.02061) · OpenAlex W2914778391
- `vanerp2019` — van Erp et al. (2019). The Influence of Exercise Intensity on the Association Between Kilojoules Spent and Various Training Loads in Professional Cycling. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2018-0877](https://doi.org/10.1123/ijspp.2018-0877) · PMID 30958053
- `vermeire2022` — Vermeire et al. (2022). The Fitness-Fatigue Model: What's in the Numbers?. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2021-0494](https://doi.org/10.1123/ijspp.2021-0494) · PMID 35320776 · OpenAlex W4220965683
- `vermeire2021` — Vermeire et al. (2021). The Influence of Different Training Load Quantification Methods on the Fitness-Fatigue Model. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2020-0662](https://doi.org/10.1123/ijspp.2020-0662) · OpenAlex W3123388430
- `voet2025` — Voet et al. (2025). Training Characteristics Related to (the Changes in) Durability in Semiprofessional Cyclists. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2024-0321](https://doi.org/10.1123/ijspp.2024-0321) · PMID 40054482
- `windt2019` — Windt & Gabbett (2019). Is it all for naught? What does mathematical coupling mean for acute:chronic workload ratios?. *British Journal of Sports Medicine*. doi:[10.1136/bjsports-2017-098925](https://doi.org/10.1136/bjsports-2017-098925) · OpenAlex W2807130647
