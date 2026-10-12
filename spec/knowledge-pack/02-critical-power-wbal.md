# 02 · Critical power (CP), W′ and W′ balance (W′bal)

**Scope.** The heavy/severe boundary, the finite work capacity above it, and real-time W′ balance for the in-ride judge and the editor's "W′bal floor" validation.
**Overall confidence:** CP as the heavy/severe boundary — A/B; W′bal *depletion* bookkeeping — B; W′bal *reconstitution* constants — C (population constants are unreliable, individualize).

## Key findings

- **[A/B] CP is the best available marker of the heavy/severe boundary (maximal metabolic steady state).** Above CP, VO2 and lactate rise to max; exercise at CP is sustainable only ~20–30 min [@jones2019; @poole2016; @jones2017; @burnley2018]. Meta-analysis: CP correlates with MLSS (r = 0.77), RCP (0.80) and VT1 (0.76), but none is interchangeable — VT1 underestimates CP by ~30%, MLSS by ~11%; RCP overestimates by ~6%, VT2 by ~21% [@galanrioja2020]. CP sits ~12 W above MLSS on average (95% LoA −19 to +44 W), with larger gaps in fitter/younger riders [@micheli2025].
- **[B] W′bal (Skiba) tracks depletion of W′ and predicts exhaustion reasonably at group level.** The original model assumes exponential reconstitution with a time constant that shortens as recovery power falls further below CP (r² = 0.77 for the τ–D_CP fit) [@skiba2012]. Field validation: W′bal at self-reported exhaustion was 0.5 ± 1.3 kJ vs a minimum of 3.6 ± 2.0 kJ in non-exhausted efforts; ROC AUC 0.914 [@skiba2014validation]. Varying work/recovery durations gave only −1.6 ± 1.1 kJ average error [@skiba2014durations].
- **[B] Population τ constants mis-predict recovery in many athletes.** Elite cyclists recovered faster than Skiba-2 predicts (τ bias −112 s); fitted relation τ = 2287.2 × D_CP^−0.688 [@bartram2017]. Both SK2 and Bartram models *over*-predicted W′ recovery in amateurs; recovery power mattered more than recovery duration [@sreedhara2020]. All published τ equations failed to predict task failure in 20:10 and 60:30 protocols; individual τ recommended, and τ correlated with relative CP and LT1 [@welburn2025]. Integral vs differential forms rest on different assumptions and give different answers — implementation must be explicit [@skiba2021; @caen2022].
- **[B/C] Reconstitution is bi-exponential and slows with repeated bouts.** Group fit: fast component 50.7% (τ ≈ 21.5 s), slow component 49.3% (τ ≈ 388 s); after a second depletion the slow τ rose to ~716 s; individual τ_fast 22 ± 12 s, τ_slow 377 ± 100 s [@chorley2022]. A dynamic bi-exponential model with recovery-intensity-dependent amplitudes/τ fitted experimental data and can be individualized from CP, W′ and one extra session [@chorley2023; @chorley2020].
- **[B] Prior intermittent work reduces W′ but not CP (short term).** After 40–80 min of intermittent sprints, CP and W′ recovery rate were unchanged but W′ fell from 13.5 to 10.1–9.1 kJ; W′bal overestimated remaining W′ after 80 min [@black2026]. After long moderate work CP itself falls (see `01-durability.md`) [@norte2026cp; @jones2023].
- **[C] Environment shifts CP/W′** (e.g., hypoxia) and needs adjusted inputs [@townsend2017].
- **[C] W′bal can design intervals.** Masters cyclists training with W′bal-designed sessions (each planned to fully deplete W′) improved CP by 5–6% in 4 weeks [@galanrioja2023]; but popular HIIT models (Skiba and Coggan-type) can generate physiologically impossible sessions if parameters are wrong [@briand2022].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Sustainable duration at CP | ~20–30 min (20–40 min in some reports) | B | [@jones2019; @leo2021] |
| CP vs MLSS | CP ≈ MLSS + 12 W (LoA −19 to +44 W) | A- | [@micheli2025] |
| CP vs VT1 / MLSS / RCP / VT2 | VT1 ≈ −30%, MLSS ≈ −11%, RCP ≈ +6%, VT2 ≈ +21% vs CP | A- | [@galanrioja2020] |
| W′bal at exhaustion (field) | 0.5 ± 1.3 kJ; non-exhausted minimum 3.6 ± 2.0 kJ | B | [@skiba2014validation] |
| Elite τ_W′ (mono-exp) | τ = 2287.2 × D_CP^−0.688 s (D_CP = CP − P_rec, W) | C | [@bartram2017] |
| Bi-exponential reconstitution | ~50% fast (τ ≈ 20 s) + ~50% slow (τ ≈ 380 s; ~550–720 s after repeated bouts) | C | [@chorley2022] |
| W′ loss after 40–80 min intermittent work | −25 to −33% (CP unchanged) | C | [@black2026] |
| CP test design | ≥ 3 maximal efforts, shortest 2–5 min, longest 12–15 min, ≥ 30 min between efforts; SE: CP 2–5%, W′ < 10% | B (consensus-like review) | [@leo2021] |

*Implementation note (D):* the widely implemented Skiba-2 exponential τ function (τ decreasing with D_CP) originates in [@skiba2012]; the exact coefficients are in the paper body, which was not retrievable in full text during this search. Treat any population τ formula as a default prior to be replaced by the athlete's fitted τ.

## Implications for the app

**Metrics to compute**
- `wbal_pct` using the **differential** form with an explicit, versioned τ model: default = bi-exponential (Chorley) or Skiba-2 prior; replace with athlete-fitted τ once ≥ 2–3 sessions with near-exhaustion intermittent efforts exist [@welburn2025; @chorley2023].
- Expose model uncertainty: compute W′bal under 2 τ models (e.g., Skiba-2 vs Bartram/bi-exp) and report the spread as a band.
- Track **W′ expended above CP per block** and **kJ above CP** (input to durability, 01).
- Fit CP/W′ from MMP with the rules in `03-power-duration-models.md`; flag CP estimates older than ~6–8 weeks or fitted from < 3 qualifying efforts (D).

**s1 judge features / rules**
- During work intervals in ERG, if predicted W′bal at block end (current power held) < ~0 → propose `ease_off` (−3 to −5%) *before* failure; band `strong` only if τ is individualized, otherwise `lean`.
- Lower the effective W′ by ~25% after > 40 min of intermittent high-intensity work in the same ride, and lower CP after long rides (see 01) [@black2026; @norte2026cp].
- If the athlete repeatedly completes blocks with W′bal ≤ 0 by model → model is wrong (CP/W′ too low or τ too slow): flag `replan`/retest rather than `push`.

**Planner rules**
- Prescribe work-interval power as % CP (not % FTP) where CP is known; keep total planned W′ depletion per set ≤ 100% of W′ with an uncertainty margin (e.g., plan to ~80–90%; D).
- For recovery-interval design, recovery power matters more than duration: lower recovery power speeds W′ reconstitution [@sreedhara2020; @chorley2023].
- Editor validation "W′bal floor": warn if any planned block drives modeled W′bal < 0 under the *conservative* τ model.

**UI hints**
- `wbal_pct` bar with an uncertainty band; label "model estimate".
- Show "W′bal reached 4% — you were close to your limit" post-ride; prompt CP retest if the athlete exceeds model limits.

## Controversies / uncertainty

- Whether CP vs MLSS is *the* MMSS is debated; the CP camp argues MLSS underestimates MMSS [@jones2019], meta-analyses show non-interchangeable values with wide LoA [@galanrioja2020; @micheli2025].
- The hyperbolic CP model fits 2–15 min well but misrepresents short and long durations; a power-law alternative has been argued [@drake2023]. W′ is a model parameter, not a physiologically measured "tank".
- Reconstitution τ depends on recovery power, prior bouts, training status and possibly fatigue; mono-exponential population τ is a known weak point [@bartram2017; @sreedhara2020; @welburn2025; @caen2022].
- 3-min all-out tests overestimate the severe-domain relationship and are not recommended for CP [@leo2021].

## References

<!-- REFS -->
- `bartram2017` — Bartram et al. (2017). Accuracy of W' Recovery Kinetics in High Performance Cyclists - Modeling Intermittent Work Capacity. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2017-0034](https://doi.org/10.1123/ijspp.2017-0034) · PMID 29035607 · OpenAlex W2766256141
- `black2026` — Black et al. (2026). Dynamics of the Power-Duration Relationship during Intermittent Team Sports Exercise. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000003929](https://doi.org/10.1249/mss.0000000000003929) · PMID 41430780 · OpenAlex W7116850743
- `briand2022` — Briand et al. (2022). Can Popular High-Intensity Interval Training (HIIT) Models Lead to Impossible Training Sessions?. *Sports*. doi:[10.3390/sports10010010](https://doi.org/10.3390/sports10010010) · OpenAlex W4205700576
- `burnley2018` — Burnley & Jones (2018). Power-duration relationship: Physiology, fatigue, and the limits of human performance. *European Journal of Sport Science*. doi:[10.1080/17461391.2016.1249524](https://doi.org/10.1080/17461391.2016.1249524)
- `caen2022` — Caen (2022). W' recovery during intermittent exercise: current limitations and future challenges of predictive models. *Journal of Science and Cycling*. OpenAlex W7218319225
- `chorley2022` — Chorley et al. (2022). Bi-exponential modelling of W' reconstitution kinetics in trained cyclists. *European Journal of Applied Physiology*. doi:[10.1007/s00421-021-04874-3](https://doi.org/10.1007/s00421-021-04874-3) · PMID 34921345
- `chorley2020` — Chorley & Lamb (2020). The Application of Critical Power, the Work Capacity above Critical Power (W'), and its Reconstitution: A Narrative Review of Current Evidence and Implications for Cycling Training Prescription. *Sports*. doi:[10.3390/sports8090123](https://doi.org/10.3390/sports8090123) · PMID 32899777
- `chorley2023` — Chorley et al. (2023). A dynamic model of the bi-exponential reconstitution and expenditure of W' in trained cyclists. *European Journal of Sport Science*. doi:[10.1080/17461391.2023.2238679](https://doi.org/10.1080/17461391.2023.2238679) · PMID 37470470
- `drake2023` — Drake et al. (2023). Modelling human endurance: power laws vs critical power. *European Journal of Applied Physiology*. doi:[10.1007/s00421-023-05274-5](https://doi.org/10.1007/s00421-023-05274-5) · PMID 37563307 · OpenAlex W4385737482
- `galanrioja2020` — Galan-Rioja et al. (2020). Relative Proximity of Critical Power and Metabolic/Ventilatory Thresholds: Systematic Review and Meta-Analysis. *Sports Medicine*. doi:[10.1007/s40279-020-01314-8](https://doi.org/10.1007/s40279-020-01314-8) · PMID 32613479 · OpenAlex W3040654852
- `galanrioja2023` — Galan-Rioja et al. (2023). Utility of the W'BAL model in training programme design for masters cyclists. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2142675](https://doi.org/10.1080/17461391.2022.2142675) · PMID 36310098
- `jones2023` — Jones (2023). The fourth dimension: physiological resilience as an independent determinant of endurance exercise performance. *The Journal of Physiology*. doi:[10.1113/jp284205](https://doi.org/10.1113/jp284205) · PMID 37606604 · OpenAlex W4386047001
- `jones2019` — Jones et al. (2019). The maximal metabolic steady state: redefining the 'gold standard'. *Physiological Reports*. doi:[10.14814/phy2.14098](https://doi.org/10.14814/phy2.14098) · PMID 31124324 · OpenAlex W2945126412
- `jones2017` — Jones & Vanhatalo (2017). The 'Critical Power' Concept: Applications to Sports Performance with a Focus on Intermittent High-Intensity Exercise. *Sports Medicine*. doi:[10.1007/s40279-017-0688-0](https://doi.org/10.1007/s40279-017-0688-0) · OpenAlex W2598090454
- `leo2021` — Leo et al. (2021). Power profiling and the power-duration relationship in cycling: a narrative review. *European Journal of Applied Physiology*. doi:[10.1007/s00421-021-04833-y](https://doi.org/10.1007/s00421-021-04833-y) · OpenAlex W3209937259
- `micheli2025` — Micheli et al. (2025). Analysis of the factors influencing the proximity and agreement between critical power and maximal lactate steady state: a systematic review and meta-analyses. *PeerJ*. doi:[10.7717/peerj.19060](https://doi.org/10.7717/peerj.19060) · PMID 40124604 · OpenAlex W4408578419
- `norte2026cp` — Norte et al. (2026). Graded Carbohydrate Ingestion up to 120 g/h Attenuates the Reduction in Critical Power Following 3 h of Moderate-Intensity Exercise in a Dose-Dependent Manner. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.70326](https://doi.org/10.1111/sms.70326) · PMID 42322010
- `poole2016` — Poole et al. (2016). Critical Power. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000000939](https://doi.org/10.1249/mss.0000000000000939) · OpenAlex W2340239240
- `skiba2012` — Skiba et al. (2012). Modeling the Expenditure and Reconstitution of Work Capacity above Critical Power. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0b013e3182517a80](https://doi.org/10.1249/mss.0b013e3182517a80) · PMID 22382171 · OpenAlex W2083253761
- `skiba2021` — Skiba & Clarke (2021). The W' Balance Model: Mathematical and Methodological Considerations. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2021-0205](https://doi.org/10.1123/ijspp.2021-0205) · PMID 34686611 · OpenAlex W3205104371
- `skiba2014validation` — Skiba et al. (2014). Validation of a Novel Intermittent W' Model for Cycling Using Field Data. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2013-0471](https://doi.org/10.1123/ijspp.2013-0471) · PMID 24509723 · OpenAlex W2166095874
- `skiba2014durations` — Skiba et al. (2014). Effect of Work and Recovery Durations on W' Reconstitution during Intermittent Exercise. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000000226](https://doi.org/10.1249/mss.0000000000000226) · PMID 24492634 · OpenAlex W2333040772
- `sreedhara2020` — Sreedhara et al. (2020). Modeling the Recovery of W' in the Moderate to Heavy Exercise Intensity Domain. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000002425](https://doi.org/10.1249/mss.0000000000002425) · PMID 32555021 · OpenAlex W3035830584
- `townsend2017` — Townsend et al. (2017). Prediction of Critical Power and W' in Hypoxia: Application to Work-Balance Modelling. *Frontiers in Physiology*. doi:[10.3389/fphys.2017.00180](https://doi.org/10.3389/fphys.2017.00180) · OpenAlex W2602163053
- `welburn2025` — Welburn et al. (2025). W' reconstitution modelling during intermittent exercise performed to task failure. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05912-0](https://doi.org/10.1007/s00421-025-05912-0) · PMID 40788399 · OpenAlex W4413099925
