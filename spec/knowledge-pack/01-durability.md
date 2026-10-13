# 01 · Durability (fatigue resistance, "physiological resilience")

**Scope.** How much the fresh-state profile (CP, power at the moderate→heavy transition / VT1, MMP) degrades with accumulated work, how to measure it from ride data, and what moves it.
**Overall confidence:** the *phenomenon* is robust (B); the *training prescription to improve it* is weak (C/D).

## Key findings

- **[A/B] Fresh-state profiling overstates what an athlete can do late in a long ride.** Durability is "the time of onset and magnitude of deterioration in physiological-profiling characteristics over time during prolonged exercise" [@maunder2021]; it is now framed as a "fourth dimension" next to VO2max, economy and fractional utilisation [@jones2023]. Methodological consensus-style review: greater relative intensity and duration of the fatiguing protocol → larger deterioration; nutrition and environment must be controlled for repeatable measures [@hunter2025; @peeters2025].
- **[B] CP falls with prolonged work, with very large between-athlete spread.** CP fell ~10% on average after 2 h of heavy cycling, with individual changes from <1% to ~32% [@jones2023]. After 3 h at 95% GET, CP fell from 277 W to 236 W (water only) [@norte2026cp].
- **[B] The moderate→heavy boundary (VT1/LT1) shifts down in watts and *up* in heart rate.** After 2 h at 90% VT1 power: VT1 217→196 W, while HR at VT1 rose 142→151 bpm [@stevenson2022]; ventilatory markers of the transition also shift [@stevenson2024]. After 150 min moderate: VT1 −6 ± 7% and 5-min TT −9 ± 10%, and VT1 durability correlated with the TT loss (rs = 0.72) [@hamilton2024].
- **[B] Field durability (MMP after accumulated kJ/kg) discriminates performance level.** Successful pros lose less 20-/5-min (climbers) or 10-s/1-min (sprinters) MMP after 10–50 kJ/kg [@vanerp2021]; World Tour riders hold MMP until ~7.5 kJ/kg *above CP* while U23 riders drop after ≥5 kJ/kg above CP [@leo2024]; similar patterns in the fatigued record power profile [@mateomarch2022rpp], Grand Tours [@muriel2022], monuments [@javaloyes2025] and durability is proposed as an independent performance parameter [@barsumyan2025durability]. Durability has been tracked from junior to senior national-team level [@almquist2023].
- **[B] Field durability is repeatable.** In 18 pros, MMP after 0–40 kJ/kg had SEM < 5% and ICC > 0.90 (training-only data: SEM < 8%, ICC > 0.80); clear decline after ~20 kJ/kg [@mateomarch2025reliability].
- **[A-] Intensity of the prior work matters more than kJ alone.** Systematic review (21 studies): 10–20% power declines after 2.5–15 kJ/kg of *high-intensity* work, smaller declines after similar or larger low-intensity work; kJ alone is an insufficient fatigue metric [@sanchezjimenez2025]. (Counterpoint: one lab study found no difference after 15 kJ/kg moderate vs heavy [@evans2025].)
- **[B] Carbohydrate intake during the ride is the best-supported acute lever.** 100 vs 60 g/h: 15-min TT after 4 h dropped −6.4% vs −9.3% [@mazza2026]; 0/60/120 g/h gave dose-dependent CP preservation (236/257/266 W vs 277 W fresh) [@norte2026cp]; low CHO availability lowers power at the moderate→heavy transition in females [@main2026]. See `07-carbohydrate.md`.
- **[B] HR (and respiratory-rate) decoupling tracks loss of VT1 power in real time.** In 51 cyclists, change in VT1 power correlated with HR decoupling (r_rm = −0.76) and RF decoupling (r = −0.40); a model with baseline VT1, VO2peak, RF decoupling and HR-decoupling×time predicted VT1 within ~7 W [@rothschild2025].
- **[C] Who is durable?** In 10 young pros, ΔCP after 5×8 min at 105–110% CP correlated with relative peak power, VO2max, VT1/VT2 power, gross efficiency (r ≈ 0.75–0.89) and inversely with CHO oxidation at 200 W [@spragg2023physiol]. In 12 pros, ~40 kJ/kg reduced 20-min TT by −2.9% (range −8.5% to +1.1%), unrelated to classic lab markers or recent load [@valenzuela2023]. 6-h simulated road racing has been used to characterise performance and fatigue patterns in elite riders [@klaris2024].
- **[C] Trainability is uncertain.** Fatigued 2-min and 12-min MMP improved with more time < VT1 and a shift toward polarized distribution across a pro season (r ≈ 0.4) [@spragg2023training]; fatigued power profile varies more across a season than fresh. In untrained people, 10 weeks of either low- or high-intensity training improved durability similarly [@matomaki2023]. No single load metric predicted fresh or fatigued TT change in semi-pros [@voet2025].
- **[C] Sex differences** in durability are being studied in field and lab settings [@mateomarch2025sex; @pastorio2026]; data in women remain limited — do not hard-code sex-specific durability constants.

## Numbers the app can use

| Quantity | Value / range | Grade | Source |
|---|---|---|---|
| Fatigue checkpoints for "fatigued MMP" | 0, 10, 20, 30, 40 kJ/kg (pro data; decline evident from ~20 kJ/kg) | B | [@vanerp2021; @mateomarch2025reliability] |
| High-intensity work checkpoints | 0, 2.5, 5, 7.5 kJ/kg *above CP* | B/C | [@leo2024; @sanchezjimenez2025] |
| Typical decline after 2.5–15 kJ/kg high-intensity work | 10–20% | A- | [@sanchezjimenez2025] |
| CP drop after 2–3 h heavy/moderate work | mean ~10% (range <1–32%); 15% w/o CHO, ~4% with 120 g/h | B | [@jones2023; @norte2026cp] |
| VT1 power drop after 2–2.5 h moderate | ~6–10% (≈ −13 to −21 W); HR at VT1 +~9 bpm | B | [@stevenson2022; @hamilton2024] |
| Field durability reliability | SEM < 5% (all efforts), < 8% (training only) | B | [@mateomarch2025reliability] |
| VT1-power prediction from HR/RF decoupling | MAE ≈ 7 W, R² 0.95 (lab, with gas-exchange baseline) | C | [@rothschild2025] |

## Implications for the app

**Metrics to compute**
- `kj`, `kj_per_kg`, and **kJ above CP** (`∫ max(0, P − CP) dt`), running during the ride; persisted per ride.
- **Fatigued power profile**: MMP(10 s, 1, 5, 20 min) restricted to data *after* each checkpoint (0/10/20/30/40 kJ/kg and 0/2.5/5/7.5 kJ/kg above CP). Store the 90-day best per checkpoint; durability index = MMP_fatigued / MMP_fresh.
- **Within-ride decoupling** (HR vs power, see `09-cardiovascular-drift-decoupling.md`) as a real-time proxy of VT1 drift.
- Note on indoor data: most durability evidence is road data; ERG-mode indoor rides have little MMP signal unless the workout contains maximal efforts. Only update the fatigued profile from blocks flagged maximal/self-paced.

**s1 judge features / rules**
- Feed the judge `kj_per_kg`, `kj_above_cp_per_kg`, `decoupling`, `hr_vs_typical` at the current power, and CHO-logged-yes/no. Late in long rides (≥ ~20 kJ/kg or ≥ 2 h), treat the athlete's *fresh* zone boundaries as optimistic: a target that was "upper Z2" fresh may be heavy-domain now [@stevenson2022].
- If decoupling is rising and HR at a fixed target exceeds the athlete's fresh VT1 HR by > ~5–10 bpm in a block intended to be moderate → `ease_off` (lean band; heuristic C).

**Planner rules**
- For goals with long events (> 2–3 h), schedule ~1 session/week with a quality effort *after* accumulated work (e.g., after 2 h or ~15–25 kJ/kg) — heuristic (C/D); the phenomenon is B, the training effect of such sessions is unproven.
- Use the athlete's own fatigued-profile ratios when designing late-ride targets; default to −10% of fresh CP for efforts after > 2 h if no personal data (B).
- Pair long-ride durability sessions with fueling prompts (60–90 g/h; see 07).

**UI hints**
- Show "fresh vs fatigued" MMP curves on the `mmp_curve` widget (overlay per kJ/kg checkpoint).
- Post-ride: "Your 5-min power after 20 kJ/kg was 93% of fresh (90-day: 91%)".

## Controversies / uncertainty

- No standard protocol: kJ/kg vs kJ/kg above CP vs time; absolute vs relative intensity; lab vs field [@hunter2025; @peeters2025].
- Whether durability is a *trainable independent* quality or a by-product of aerobic fitness, efficiency and fueling is unresolved [@spragg2023physiol; @valenzuela2023; @matomaki2023].
- Most field data come from male professionals with road power data; generalization to recreational indoor riders is extrapolation.
- kJ ignores intensity; intensity-weighted metrics are proposed but not validated [@sanchezjimenez2025].

## References

<!-- REFS -->
- `almquist2023` — Almquist et al. (2023). Development of Cycling Performance Variables and Durability in Female and Male National Team Cyclists: From Junior to Senior. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000003232](https://doi.org/10.1249/mss.0000000000003232) · OpenAlex W4378953685
- `barsumyan2025durability` — Barsumyan et al. (2025). Durability as an independent parameter of endurance performance in cycling. *BMC Sports Science, Medicine and Rehabilitation*. doi:[10.1186/s13102-025-01238-8](https://doi.org/10.1186/s13102-025-01238-8) · OpenAlex W4412151865
- `evans2025` — Evans et al. (2025). Durability of the Major Determinants of Endurance Performance Is Not Different After 15 kJ/kg of Moderate or Heavy Exercise. *European Journal of Sport Science*. doi:[10.1002/ejsc.70039](https://doi.org/10.1002/ejsc.70039) · OpenAlex W4413140304
- `hamilton2024` — Hamilton et al. (2024). Durability of the moderate-to-heavy-intensity transition is related to the effects of prolonged exercise on severe-intensity performance. *European Journal of Applied Physiology*. doi:[10.1007/s00421-024-05459-6](https://doi.org/10.1007/s00421-024-05459-6) · PMID 38546844
- `hunter2025` — Hunter et al. (2025). Durability as an index of endurance exercise performance: Methodological considerations. *Experimental Physiology*. doi:[10.1113/ep092120](https://doi.org/10.1113/ep092120) · PMID 40150840 · OpenAlex W4408956908
- `javaloyes2025` — Javaloyes et al. (2025). The Role of Cadence and Torque in Fatigue-Related Power Output Decline in Cycling's Grand Monuments. *Sports*. doi:[10.3390/sports13110406](https://doi.org/10.3390/sports13110406) · PMID 41295789 · OpenAlex W4416230172
- `jones2023` — Jones (2023). The fourth dimension: physiological resilience as an independent determinant of endurance exercise performance. *The Journal of Physiology*. doi:[10.1113/jp284205](https://doi.org/10.1113/jp284205) · PMID 37606604 · OpenAlex W4386047001
- `klaris2024` — Klaris et al. (2024). Performance and Fatigue Patterns in Elite Cyclists During 6 h of Simulated Road Racing. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.14699](https://doi.org/10.1111/sms.14699) · OpenAlex W4400688613
- `leo2024` — Leo et al. (2024). The Influence of High-Intensity Work on the Record Power Profile of Under-23, Pro Team, and World Tour Cyclists. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2023-0451](https://doi.org/10.1123/ijspp.2023-0451) · PMID 38531349
- `main2026` — Main et al. (2026). Low carbohydrate availability reduces power output at the moderate-to-heavy transition, impairs efficiency, and increases median power frequency during cycling in females. *European Journal of Applied Physiology*. doi:[10.1007/s00421-026-06260-3](https://doi.org/10.1007/s00421-026-06260-3) · OpenAlex W7163123181
- `mateomarch2025reliability` — Mateo-March et al. (2025). Reliability of the durability concept in professional cyclists: a field-based study. *International Journal of Sports Medicine*. doi:[10.1055/a-2555-8961](https://doi.org/10.1055/a-2555-8961) · PMID 40373793
- `mateomarch2025sex` — Mateo-March et al. (2025). Sex differences in durability: A field-based study in professional cyclists. *Journal of Science and Medicine in Sport*. doi:[10.1016/j.jsams.2025.02.009](https://doi.org/10.1016/j.jsams.2025.02.009)
- `mateomarch2022rpp` — Mateo-March et al. (2022). The Record Power Profile of Male Professional Cyclists: Fatigue Matters. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2021-0403](https://doi.org/10.1123/ijspp.2021-0403)
- `matomaki2023` — Matomaki et al. (2023). Durability is improved by both low and high intensity endurance training. *Frontiers in Physiology*. doi:[10.3389/fphys.2023.1128111](https://doi.org/10.3389/fphys.2023.1128111) · PMID 36875044 · OpenAlex W4321101608
- `maunder2021` — Maunder et al. (2021). The Importance of 'Durability' in the Physiological Profiling of Endurance Athletes. *Sports Medicine*. doi:[10.1007/s40279-021-01459-0](https://doi.org/10.1007/s40279-021-01459-0)
- `mazza2026` — Mazza et al. (2026). Carbohydrate Intake of 100 vs. 60 g/h During Prolonged Cycling Improves Durability Without Altering Muscle Glycogen Utilization in Male Endurance Athletes. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000004180](https://doi.org/10.1249/mss.0000000000004180) · PMID 42841583
- `muriel2022` — Muriel et al. (2022). Durability and repeatability of professional cyclists during a Grand Tour. *European Journal of Sport Science*. doi:[10.1080/17461391.2021.1987528](https://doi.org/10.1080/17461391.2021.1987528)
- `norte2026cp` — Norte et al. (2026). Graded Carbohydrate Ingestion up to 120 g/h Attenuates the Reduction in Critical Power Following 3 h of Moderate-Intensity Exercise in a Dose-Dependent Manner. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.70326](https://doi.org/10.1111/sms.70326) · PMID 42322010
- `pastorio2026` — Pastorio et al. (2026). Sex Differences in Durability Following Heavy Intensity Cycling Exercise in Trained Athletes. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.70273](https://doi.org/10.1111/sms.70273) · OpenAlex W4413855057
- `peeters2025` — Peeters et al. (2025). What is a cycling race simulation anyway: a review on protocols to assess durability in cycling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05725-1](https://doi.org/10.1007/s00421-025-05725-1) · OpenAlex W4407567765
- `rothschild2025` — Rothschild et al. (2025). Durability of the moderate-to-heavy intensity transition can be predicted using readily available markers of physiological decoupling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05815-0](https://doi.org/10.1007/s00421-025-05815-0) · PMID 40402269
- `sanchezjimenez2025` — Sanchez-Jimenez et al. (2025). Is intensity the most important factor in determining the amount of prior work accumulated that affects cyclists' acute durability? A systematic review. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05885-0](https://doi.org/10.1007/s00421-025-05885-0) · PMID 40613880 · OpenAlex W4412017080
- `spragg2023physiol` — Spragg et al. (2023). The Relationship between Physiological Characteristics and Durability in Male Professional Cyclists. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000003024](https://doi.org/10.1249/mss.0000000000003024) · PMID 35977108
- `spragg2023training` — Spragg et al. (2023). The relationship between training characteristics and durability in professional cyclists across a competitive season. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2049886](https://doi.org/10.1080/17461391.2022.2049886) · PMID 35239466
- `stevenson2022` — Stevenson et al. (2022). Prolonged cycling reduces power output at the moderate-to-heavy intensity transition. *European Journal of Applied Physiology*. doi:[10.1007/s00421-022-05036-9](https://doi.org/10.1007/s00421-022-05036-9) · PMID 36127418
- `stevenson2024` — Stevenson et al. (2024). Prolonged exercise shifts ventilatory parameters at the moderate-to-heavy intensity transition. *European Journal of Applied Physiology*. doi:[10.1007/s00421-023-05285-2](https://doi.org/10.1007/s00421-023-05285-2)
- `valenzuela2023` — Valenzuela et al. (2023). Durability in Professional Cyclists: A Field Study. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2022-0202](https://doi.org/10.1123/ijspp.2022-0202) · PMID 36521188
- `vanerp2021` — van Erp et al. (2021). Maintaining Power Output with Accumulating Levels of Work Done Is a Key Determinant for Success in Professional Cycling. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000002656](https://doi.org/10.1249/mss.0000000000002656) · PMID 33731651
- `voet2025` — Voet et al. (2025). Training Characteristics Related to (the Changes in) Durability in Semiprofessional Cyclists. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2024-0321](https://doi.org/10.1123/ijspp.2024-0321) · PMID 40054482
