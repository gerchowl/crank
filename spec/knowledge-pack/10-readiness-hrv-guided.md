# 10 · Readiness: HRV-guided training, subjective check-ins, sleep

**Scope.** How the planner should use the pre-ride check-in (sleep, legs, stress, motivation, time; free text), resting HRV (`rmssd`) if available, and recent load, to decide "as planned / easier / harder / rest".
**Overall confidence:** HRV-guided ≥ predefined for performance — small, non-significant benefit (A, mostly null); fewer non-responders — B/C; subjective measures are more sensitive to load than objective ones — A-/B; sleep loss impairs endurance — A.

## Key findings

- **[A] HRV-guided training gives at most small performance gains over predefined plans.** Meta-analysis (8 studies, 198 participants): HRV-guided training had a medium effect on submaximal physiology (g = 0.30) but small, non-significant effects on performance (g = 0.08) and VO2peak (g = 0.17); **fewer non-responders** and more positive responders; HRV-guided groups typically did *fewer* moderate/high-intensity sessions [@duking2021]. Methodological meta-analysis: superior for vagal HRV (SMD 0.50) but only small non-significant SMDs (~0.20) for VO2max, VT2 and performance [@manresarocamora2021]. Two further meta-analyses address VO2max [@granerogallegos2020] and physiological/aerobic performance [@medellin2020] (only abstracts' aims were reviewed here; not used for effect sizes).
- **[B/C] Cycling RCTs.** HRV-guided group improved peak power (+5.1%), VT2 power (+13.9%) and 40-min TT (+7.3%) while the traditional group did not, though between-group differences were not significant [@javaloyes2019]; HRV-guided vs block periodization in well-trained cyclists [@javaloyes2020]. Combining vagal HRV + resting HR + well-being scores gave the largest gains in experienced cyclists (single study) [@alfonso2025].
- **[B] Multi-marker individualization helps recreational athletes.** Adjusting load twice weekly from nocturnal HRV, perceived recovery and an HR–speed index: 10-km −6.2% vs −2.9% with predefined training, more high responders and no low responders [@nuuttila2022]. HR-based and subjective markers were compared for detecting fatigue during intensified training [@nuuttila2024].
- **[A-/B] Subjective self-reports beat objective markers for tracking training response.** Systematic review (56 studies): subjective and objective measures generally did not correlate; subjective measures responded to acute and chronic load with superior sensitivity and consistency [@saw2015]. Multidimensional decision frameworks are recommended over single signals [@rebelo2026; @buchheit2014].
- **[B] Fatigue patterns in HRV are heterogeneous.** Fatigue in elite endurance athletes "shifts and scatters" HRV in different directions [@schmitt2013]; overreaching can even speed HR recovery (see 05) [@aubry2015]. Predicting next-day HRV change and perceived recovery with machine learning has been attempted [@rothschild2024]. Intensified training in cyclists produces a time course of performance and fatigue-marker changes [@halson2002].
- **[A] Sleep loss impairs endurance performance.** Meta-analysis (31 studies): moderate impairment (SMD −0.52), larger for exercise > 30 min; partial or one night of total deprivation similar [@lopes2023]; broader physical-performance meta-analysis [@craven2022].
- **[C/D] Rule-based HRV systems are the weak point.** The main limitation is not data availability but transparent, physiologically grounded decision rules (baseline windows, smallest worthwhile change, how to weight signals) [@schaffarczyk2026; @manresarocamora2021].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| HRV-guided vs predefined, performance | g ≈ 0.08–0.20 (ns) | A | [@duking2021; @manresarocamora2021] |
| Sleep deprivation effect on endurance | SMD −0.52 (larger for > 30 min tasks) | A | [@lopes2023] |
| Typical decision cadence in successful trials | daily (cyclists) or twice weekly (runners) | B/C | [@javaloyes2019; @nuuttila2022] |
| Day-to-day submax HR noise | ~3 bpm (useful SWC reference for an HR-at-power check) | C | [@lamberts2009] |
| Common HRV baseline practice | rolling ~7-day mean vs longer baseline ± SWC (exact windows vary; no consensus) | D | [@manresarocamora2021; @schaffarczyk2026] |

## Implications for the app

**Metrics to compute**
- **Readiness vector, not a single score**: check-in items (sleep, legs, stress, motivation), resting lnRMSSD 7-day rolling vs 30–60-day baseline (if the user records it with the strap), resting HR, warm-up HR-at-power and α1-at-power vs usual (05, 04), recent load (13).
- For each component, report deviation from the athlete's own baseline in SWC units; never show population norms as "good/bad".

**s1 judge features / rules**
- The judge receives the day's readiness vector. With poor sleep or "heavy legs" and an in-ride signal of excess strain (HR above typical at power, slow HRR, α1 low in Z1) → lower the threshold for `ease_off` (C).
- Readiness alone should not trigger mid-ride `push`.

**Planner rules**
- Default plan = predefined structure; **modulate** day-to-day by readiness (shift hard session ±1–2 days, swap to Z1, shorten) rather than generating plans purely from HRV (A: small benefit; B: fewer non-responders).
- Weight **subjective check-in** at least as heavily as HRV [@saw2015]; require ≥ 2 concordant negative signals to downgrade a key session (C/D), but any single severe flag (illness note, sleep ≤ 1/5, pain) → easy/rest (D, safety).
- Poor sleep before a long or high-intensity session: shorten or reduce intensity of long efforts > 30 min first [@lopes2023].
- Treat positive deviations (HRV high, HR low) cautiously — parasympathetic saturation and overreaching produce confusing patterns [@schmitt2013; @aubry2015].

**UI hints**
- Check-in rationale shown with the proposal ("easier because: sleep 2/5 and legs 2/5; HRV within normal").
- Let the athlete override; log overrides as data for the planner.

## Controversies / uncertainty

- No consensus on HRV index, posture, timing, baseline window, or SWC; meta-analytic benefits are small and heterogeneous [@manresarocamora2021; @schaffarczyk2026].
- Proprietary "readiness scores" are not transparent and not validated as decision rules [@schaffarczyk2026].
- Most trials are short (≤ 8–15 weeks), small, and in recreational/trained but not elite cyclists.

## References

<!-- REFS -->
- `alfonso2025` — Alfonso et al. (2025). Individual training prescribed by heart rate variability, heart rate and well-being scores in experienced cyclists. *Scientific Reports*. doi:[10.1038/s41598-025-13540-z](https://doi.org/10.1038/s41598-025-13540-z) · PMID 41028151 · OpenAlex W4414628846
- `aubry2015` — Aubry et al. (2015). The Development of Functional Overreaching Is Associated with a Faster Heart Rate Recovery in Endurance Athletes. *PLoS ONE*. doi:[10.1371/journal.pone.0139754](https://doi.org/10.1371/journal.pone.0139754) · PMID 26488766 · OpenAlex W2221922668
- `buchheit2014` — Buchheit (2014). Monitoring training status with HR measures: do all roads lead to Rome?. *Frontiers in Physiology*. doi:[10.3389/fphys.2014.00073](https://doi.org/10.3389/fphys.2014.00073) · PMID 24578692 · OpenAlex W2137418727
- `craven2022` — Craven et al. (2022). Effects of Acute Sleep Loss on Physical Performance: A Systematic and Meta-Analytical Review. *Sports Medicine*. doi:[10.1007/s40279-022-01706-y](https://doi.org/10.1007/s40279-022-01706-y)
- `duking2021` — Duking et al. (2021). Monitoring and adapting endurance training on the basis of heart rate variability monitored by wearable technologies: A systematic review with meta-analysis. *Journal of Science and Medicine in Sport*. doi:[10.1016/j.jsams.2021.04.012](https://doi.org/10.1016/j.jsams.2021.04.012) · PMID 34489178 · OpenAlex W3163562619
- `granerogallegos2020` — Granero-Gallegos et al. (2020). HRV-Based Training for Improving VO2max in Endurance Athletes. A Systematic Review with Meta-Analysis. *International Journal of Environmental Research and Public Health*. doi:[10.3390/ijerph17217999](https://doi.org/10.3390/ijerph17217999) · OpenAlex W3095489224
- `halson2002` — Halson et al. (2002). Time course of performance changes and fatigue markers during intensified training in trained cyclists. *Journal of Applied Physiology*. doi:[10.1152/japplphysiol.01164.2001](https://doi.org/10.1152/japplphysiol.01164.2001) · OpenAlex W2163199203
- `javaloyes2020` — Javaloyes et al. (2020). Training Prescription Guided by Heart Rate Variability Vs. Block Periodization in Well-Trained Cyclists. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000003337](https://doi.org/10.1519/jsc.0000000000003337)
- `javaloyes2019` — Javaloyes et al. (2019). Training Prescription Guided by Heart-Rate Variability in Cycling. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2018-0122](https://doi.org/10.1123/ijspp.2018-0122) · PMID 29809080
- `lamberts2009` — Lamberts & Lambert (2009). Day-to-Day Variation in Heart Rate at Different Levels of Submaximal Exertion: Implications for Monitoring Training. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0b013e3181a2dcdc](https://doi.org/10.1519/jsc.0b013e3181a2dcdc) · PMID 19387374 · OpenAlex W2017114764
- `lopes2023` — Lopes et al. (2023). How much does sleep deprivation impair endurance performance? A systematic review and meta-analysis. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2155583](https://doi.org/10.1080/17461391.2022.2155583) · PMID 36472094
- `manresarocamora2021` — Manresa-Rocamora et al. (2021). Heart Rate Variability-Guided Training for Enhancing Cardiac-Vagal Modulation, Aerobic Fitness, and Endurance Performance: A Methodological Systematic Review with Meta-Analysis. *International Journal of Environmental Research and Public Health*. doi:[10.3390/ijerph181910299](https://doi.org/10.3390/ijerph181910299) · PMID 34639599 · OpenAlex W3203053436
- `medellin2020` — Medellin et al. (2020). Effectiveness of Training Prescription Guided by Heart Rate Variability Versus Predefined Training for Physiological and Aerobic Performance Improvements: A Systematic Review and Meta-Analysis. *Applied Sciences*. doi:[10.3390/app10238532](https://doi.org/10.3390/app10238532) · OpenAlex W3108944866
- `nuuttila2022` — Nuuttila et al. (2022). Individualized Endurance Training Based on Recovery and Training Status in Recreational Runners. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000002968](https://doi.org/10.1249/mss.0000000000002968) · PMID 35975912
- `nuuttila2024` — Nuuttila et al. (2024). Monitoring fatigue state with heart rate-based and subjective methods during intensified training in recreational runners. *European Journal of Sport Science*. doi:[10.1002/ejsc.12115](https://doi.org/10.1002/ejsc.12115) · OpenAlex W4395694725
- `rebelo2026` — Rebelo et al. (2026). Monitoring Training Effects in Athletes: A Multidimensional Framework for Decision-Making. *Sports Medicine*. doi:[10.1007/s40279-026-02417-4](https://doi.org/10.1007/s40279-026-02417-4) · OpenAlex W7135218565
- `rothschild2024` — Rothschild et al. (2024). Predicting daily recovery during long-term endurance training using machine learning analysis. *European Journal of Applied Physiology*. doi:[10.1007/s00421-024-05530-2](https://doi.org/10.1007/s00421-024-05530-2) · OpenAlex W4399860032
- `saw2015` — Saw et al. (2015). Monitoring the athlete training response: subjective self-reported measures trump commonly used objective measures: a systematic review. *British Journal of Sports Medicine*. doi:[10.1136/bjsports-2015-094758](https://doi.org/10.1136/bjsports-2015-094758) · PMID 26423706 · OpenAlex W2111075265
- `schaffarczyk2026` — Schaffarczyk & Sperlich (2026). Heart rate variability-guided endurance training: evaluating strengths, weaknesses, opportunities, and threats for load prescription and adjustment. *Frontiers in Sports and Active Living*. doi:[10.3389/fspor.2026.1858271](https://doi.org/10.3389/fspor.2026.1858271) · PMID 42724227 · OpenAlex W7204468571
- `schmitt2013` — Schmitt et al. (2013). Fatigue Shifts and Scatters Heart Rate Variability in Elite Endurance Athletes. *PLoS ONE*. doi:[10.1371/journal.pone.0071588](https://doi.org/10.1371/journal.pone.0071588) · OpenAlex W2129787090
