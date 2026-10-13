# 09 · Cardiovascular drift and aerobic decoupling

**Scope.** `hr_drift`, `decoupling`, `hr_vs_typical` in the contract: what HR rising at constant power means, what drives it, and how the judge should use it.
**Overall confidence:** mechanism and modifiers (dehydration, heat, cadence) — B; decoupling as a durability marker — B/C; specific "%-decoupling" cut-offs — D.

## Key findings

- **[B] Drift is real, starts early and is mainly a heart-rate phenomenon with falling stroke volume.** Cardiovascular drift = progressive HR rise and stroke-volume fall beginning after ~10–20 min of prolonged moderate exercise [@coyle2001; @wingo2012]. Coyle & González-Alonso argue SV falls primarily *because* HR rises, not mainly from cutaneous blood-flow redistribution [@coyle2001]; alternative/broader mechanisms are reviewed in [@souissi2021; @coyle1998].
- **[B] Dehydration grades drift.** 2 h at 62–67% VO2max in 33 °C: body-mass loss of 1.1–4.2% produced linear increases in core temperature and HR and linear falls in SV (r ≈ 0.98–0.99) [@montain1992]. At 22 °C, 2 h at ~70% VO2max without fluid: HR +10%, SV −15%; full fluid replacement limited HR rise to +5% and maintained SV [@hamilton1991].
- **[B] In the heat, drift means *higher relative intensity*.** 15→45 min at 60% VO2max in 35 °C: HR +17% (cycling), SV −15%, and VO2max fell proportionally — the HR rise reflects a real rise in %VO2max [@wingo2020; @wingo2012].
- **[B/C] Drift and decoupling indicate loss of durability.** After 2 h moderate, HR at VT1 rose ~9 bpm while VT1 power fell ~21 W [@stevenson2022]. HR decoupling correlated with loss of VT1 power (r_rm = −0.76) and supported real-time VT1 prediction (MAE ~7 W) [@rothschild2025]. In 10 weeks of training, durability (smaller drifts, later onset at ~106→131–137 min) improved with both low- and high-intensity training in untrained people [@matomaki2023].
- **[C] Cadence affects drift.** Faster cadence exaggerates drift [@kounalakis2012]; cadence decline tracks drift and decoupling within a session [@barsumyan2026].
- **[C] Drift as a fitness-trend signal.** Machine-learning on standardized efforts used cardiovascular drift to quantify training response in cyclists [@barsumyan2025frai]. HR at fixed submax power has ~3 bpm day-to-day noise, lowest at 85–90% HRmax [@lamberts2009].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Drift onset | after ~10–20 min of steady exercise | B | [@coyle2001; @wingo2012] |
| HR rise, 2 h ~70% VO2max, no fluid (22 °C) | ~+10% (+5% with full fluid replacement) | B | [@hamilton1991] |
| HR rise, 30 min in 35 °C | ~+17–19% | B | [@wingo2020] |
| Dehydration–drift relation | linear across 1–4% body-mass loss | B | [@montain1992] |
| HR at VT1 after 2 h | +~9 bpm; VT1 power −~10% | B | [@stevenson2022] |
| Day-to-day submax HR noise | ~3 bpm | C | [@lamberts2009] |
| "Decoupling > 5% = aerobic base insufficient" | widely used rule of thumb; **no supporting study found** | D | — |

## Implications for the app

**Metrics to compute**
- `decoupling` = change in efficiency factor (power/HR, or HR at matched power) between first and second halves of a steady block/ride, excluding the first ~10 min (warm-up/HR on-kinetics) and non-steady segments; compute with cadence and ambient/indoor fan context logged where available.
- `hr_vs_typical` = HR at current power vs the athlete's 90-day model (e.g., the power→HR dynamic model of [@saupe2024], see 05), expressed in bpm and SD units.
- `hr_drift` (bpm/h at constant power) per steady block.

**s1 judge features / rules**
- Indoors (heat, often no airflow), drift is the norm. If HR drift in a block intended as moderate pushes HR above the athlete's VT1/HRVT1 HR for > ~3 min → `ease_off` −3 to −5% (C); explain "heat/dehydration: same watts now cost more" [@wingo2020; @montain1992].
- Combine decoupling with time/kJ: rising decoupling after > 90 min + no fluid/CHO logged → suggest drinking/fueling and `hold` rather than `push` [@hamilton1991].
- Never `push` intensity on the basis of a *low* HR early in a session (HR lags; drift has not developed).

**Planner rules**
- Interpret decoupling only from comparable sessions (same duration, intensity, cadence range, cooling) — a trend metric, not a single-ride verdict (D).
- Recommend fan/cooling and fluid for long indoor sessions (B, mechanism) and plan HR-capped endurance blocks with power ceilings so drift does not silently move the session into the heavy domain.

**UI hints**
- Show decoupling with context chips (duration, avg cadence, fluid logged).
- Avoid labelling "> 5% decoupling" as failure; present as trend vs the athlete's own history.

## Controversies / uncertainty

- Mechanism (HR-driven SV fall vs thermal redistribution) is debated [@coyle2001; @souissi2021].
- No validated decoupling thresholds for "aerobic readiness"; the 5% rule is practitioner convention (D).
- Lab studies mostly use fixed fans and controlled ambient conditions; home trainer environments vary widely (gap).

## References

<!-- REFS -->
- `barsumyan2025frai` — Barsumyan et al. (2025). Quantifying training response in cycling based on cardiovascular drift using machine learning. *Frontiers in Artificial Intelligence*. doi:[10.3389/frai.2025.1623384](https://doi.org/10.3389/frai.2025.1623384) · OpenAlex W4412041705
- `barsumyan2026` — Barsumyan et al. (2026). The relationship between cadence decline, cardiovascular drift and aerobic decoupling as a marker of fatigue in well trained cyclists. *BMC Sports Science, Medicine and Rehabilitation*. doi:[10.1186/s13102-026-01678-w](https://doi.org/10.1186/s13102-026-01678-w) · PMID 41923151 · OpenAlex W7147353069
- `coyle1998` — Coyle (1998). Cardiovascular drift during prolonged exercise and the effects of dehydration. *International Journal of Sports Medicine*. doi:[10.1055/s-2007-971975](https://doi.org/10.1055/s-2007-971975) · OpenAlex W2027029075
- `coyle2001` — Coyle & Gonzalez-Alonso (2001). Cardiovascular drift during prolonged exercise: new perspectives. *Exercise and Sport Sciences Reviews*. doi:[10.1097/00003677-200104000-00009](https://doi.org/10.1097/00003677-200104000-00009) · PMID 11337829 · OpenAlex W2041950757
- `hamilton1991` — Hamilton et al. (1991). Fluid replacement and glucose infusion during exercise prevent cardiovascular drift. *Journal of Applied Physiology*. doi:[10.1152/jappl.1991.71.3.871](https://doi.org/10.1152/jappl.1991.71.3.871) · PMID 1757323 · OpenAlex W1837083581
- `kounalakis2012` — Kounalakis & Geladas (2012). Cardiovascular drift and cerebral and muscle tissue oxygenation during prolonged cycling at different pedalling cadences. *Applied Physiology, Nutrition, and Metabolism*. doi:[10.1139/h2012-011](https://doi.org/10.1139/h2012-011) · PMID 22509808
- `lamberts2009` — Lamberts & Lambert (2009). Day-to-Day Variation in Heart Rate at Different Levels of Submaximal Exertion: Implications for Monitoring Training. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0b013e3181a2dcdc](https://doi.org/10.1519/jsc.0b013e3181a2dcdc) · PMID 19387374 · OpenAlex W2017114764
- `matomaki2023` — Matomaki et al. (2023). Durability is improved by both low and high intensity endurance training. *Frontiers in Physiology*. doi:[10.3389/fphys.2023.1128111](https://doi.org/10.3389/fphys.2023.1128111) · PMID 36875044 · OpenAlex W4321101608
- `montain1992` — Montain & Coyle (1992). Influence of graded dehydration on hyperthermia and cardiovascular drift during exercise. *Journal of Applied Physiology*. doi:[10.1152/jappl.1992.73.4.1340](https://doi.org/10.1152/jappl.1992.73.4.1340) · PMID 1447078 · OpenAlex W1910883221
- `rothschild2025` — Rothschild et al. (2025). Durability of the moderate-to-heavy intensity transition can be predicted using readily available markers of physiological decoupling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-025-05815-0](https://doi.org/10.1007/s00421-025-05815-0) · PMID 40402269
- `saupe2024` — Saupe (2024). Estimation of Heart Rate Recovery from Field Rides. *Journal of Science and Cycling*. OpenAlex W7217666987
- `souissi2021` — Souissi et al. (2021). A new perspective on cardiovascular drift during prolonged exercise. *Life Sciences*. doi:[10.1016/j.lfs.2021.120109](https://doi.org/10.1016/j.lfs.2021.120109) · OpenAlex W3210091546
- `stevenson2022` — Stevenson et al. (2022). Prolonged cycling reduces power output at the moderate-to-heavy intensity transition. *European Journal of Applied Physiology*. doi:[10.1007/s00421-022-05036-9](https://doi.org/10.1007/s00421-022-05036-9) · PMID 36127418
- `wingo2012` — Wingo et al. (2012). Cardiovascular drift during heat stress: implications for exercise prescription. *Exercise and Sport Sciences Reviews*. doi:[10.1097/jes.0b013e31824c43af](https://doi.org/10.1097/jes.0b013e31824c43af) · PMID 22410803 · OpenAlex W2059810365
- `wingo2020` — Wingo et al. (2020). Cardiovascular Drift and Maximal Oxygen Uptake during Running and Cycling in the Heat. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000002324](https://doi.org/10.1249/mss.0000000000002324) · PMID 32102057
