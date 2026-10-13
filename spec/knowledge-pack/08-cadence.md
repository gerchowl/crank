# 08 · Cadence, efficiency and fatigue

**Scope.** Cadence targets in workouts (`StepNode.cadence`), cadence as a fatigue signal, and cadence as a confounder for HR / DFA-α1 / drift in ERG mode.
**Overall confidence:** acute physiology of cadence — B; training at specific cadences — C (conflicting small RCTs).

## Key findings

- **[B] Gross efficiency falls as cadence rises (at fixed submaximal power).** Within subjects, force effectiveness and gross efficiency decreased with increasing cadence around the freely chosen cadence (FCC) at ~75% VO2max [@leirdal2011]. Typical FCC in trained cyclists ≈ 92–93 rpm [@whitty2016]. The energetically optimal cadence decreases after prolonged cycling [@annaheim2010]; a power-maximizing cadence can be estimated from field data [@reed2016].
- **[B] Higher cadence increases cardiovascular and autonomic load at the same power.** 80 vs 40 rpm at 60% VO2peak for 90 min: more cardiovascular drift (HR +9 bpm, SV −9 mL, cardiac output −1 L/min) [@kounalakis2012]. 120 vs 60 rpm at 90% of anaerobic threshold: higher HR, lactate, RPE and lower DFA-α1 [@gronwald2018cadence].
- **[C] Cadence decline tracks fatigue during long steady rides.** In 60-min efforts at 75% FTP (85 observations), each rpm of cadence decline in the second half corresponded to +0.61% cardiovascular drift and +0.58% aerobic decoupling (r ≈ 0.38–0.40) [@barsumyan2026]. In monuments, top riders sustained torque and power under fatigue with minimal cadence change; torque correlated with power far more than cadence did [@javaloyes2025].
- **[C] Training at non-preferred cadences: mixed.** 12 weeks of 5×6 min at 40 rpm (moderate) gave no benefit in veteran cyclists, whereas extra freely-chosen-cadence riding did [@kristoffersen2014]. High-cadence intervals raised FCC (92→101 rpm) and GE at high cadences; low-cadence intervals improved a short TT more [@whitty2016]. Low-cadence (50–70 rpm) SIT/HIIT within a polarized program improved Pmax and VO2max more than freely chosen cadence in well-trained women [@hebisz2024]. Systematic review: high cadence + high intensity impairs neuromuscular function more, but cadence's role in fatigue onset and chronic adaptation is unclear [@mater2021].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Typical FCC (trained) | ~90–95 rpm | C | [@whitty2016] |
| HR cost of higher cadence | ~+9 bpm (80 vs 40 rpm, 90 min @ 60% VO2peak) | C | [@kounalakis2012] |
| Cadence decline as fatigue marker | 1 rpm decline ≈ +0.6% drift / decoupling | C | [@barsumyan2026] |
| Low-cadence interval range used in positive trial | 50–70 rpm (SIT/HIIT) | C | [@hebisz2024] |

## Implications for the app

**Metrics to compute**
- Per-block cadence mean/SD, **cadence drift** (2nd half vs 1st half of steady blocks), torque (P / angular velocity) for strength-endurance blocks.
- Store cadence alongside HR, α1 and decoupling so they can be *adjusted*: compare HR/α1 only at matched cadence (±5–10 rpm) (C).

**s1 judge features / rules**
- In ERG mode, power is fixed; cadence changes shift torque and HR. If HR or α1 worsens but cadence rose > ~10 rpm, attribute part of it to cadence before proposing `ease_off` (C).
- A sustained cadence decline (> ~5 rpm vs block start) in a long steady block, together with rising decoupling, is supporting evidence of fatigue (C) [@barsumyan2026].

**Planner rules**
- Default to free cadence; use cadence targets as *variation*, not as a performance necessity (C).
- Low-cadence/high-torque intervals (50–70 rpm) are an optional variant for HIIT/SIT blocks; avoid very low cadence (40 rpm) moderate work as a primary tool (C, conflicting).
- Do not change cadence prescription and intensity simultaneously when trying to compare sessions (D).

**UI hints**
- Cadence target band on the ride screen when prescribed; show torque for low-cadence blocks.
- Note on the HR/α1 widgets when cadence is outside the athlete's usual range ("readings affected by cadence").

## Controversies / uncertainty

- Optimal cadence depends on the criterion (metabolic cost, power, neuromuscular fatigue); no single target is evidence-based [@leirdal2011; @mater2021].
- Cadence training RCTs are small, short and partly in specific populations (veterans, women) with opposite results [@kristoffersen2014; @hebisz2024; @whitty2016].

## References

<!-- REFS -->
- `annaheim2010` — Annaheim et al. (2010). The energetically optimal cadence decreases after prolonged cycling exercise. *European Journal of Applied Physiology*. doi:[10.1007/s00421-010-1431-9](https://doi.org/10.1007/s00421-010-1431-9) · OpenAlex W2060189011
- `barsumyan2026` — Barsumyan et al. (2026). The relationship between cadence decline, cardiovascular drift and aerobic decoupling as a marker of fatigue in well trained cyclists. *BMC Sports Science, Medicine and Rehabilitation*. doi:[10.1186/s13102-026-01678-w](https://doi.org/10.1186/s13102-026-01678-w) · PMID 41923151 · OpenAlex W7147353069
- `gronwald2018cadence` — Gronwald et al. (2018). Non-linear dynamics of cardiac autonomic activity during cycling exercise with varied cadence. *Human Movement Science*. doi:[10.1016/j.humov.2018.06.013](https://doi.org/10.1016/j.humov.2018.06.013) · PMID 29966866
- `hebisz2024` — Hebisz & Hebisz (2024). Greater improvement in aerobic capacity after a polarized training program including cycling interval training at low cadence (50-70 RPM) than freely chosen cadence (above 80 RPM). *PLoS ONE*. doi:[10.1371/journal.pone.0311833](https://doi.org/10.1371/journal.pone.0311833) · PMID 39536034 · OpenAlex W4404317964
- `javaloyes2025` — Javaloyes et al. (2025). The Role of Cadence and Torque in Fatigue-Related Power Output Decline in Cycling's Grand Monuments. *Sports*. doi:[10.3390/sports13110406](https://doi.org/10.3390/sports13110406) · PMID 41295789 · OpenAlex W4416230172
- `kounalakis2012` — Kounalakis & Geladas (2012). Cardiovascular drift and cerebral and muscle tissue oxygenation during prolonged cycling at different pedalling cadences. *Applied Physiology, Nutrition, and Metabolism*. doi:[10.1139/h2012-011](https://doi.org/10.1139/h2012-011) · PMID 22509808
- `kristoffersen2014` — Kristoffersen et al. (2014). Low cadence interval training at moderate intensity does not improve cycling performance in highly trained veteran cyclists. *Frontiers in Physiology*. doi:[10.3389/fphys.2014.00034](https://doi.org/10.3389/fphys.2014.00034) · PMID 24550843 · OpenAlex W2083652634
- `leirdal2011` — Leirdal & Ettema (2011). The relationship between cadence, pedalling technique and gross efficiency in cycling. *European Journal of Applied Physiology*. doi:[10.1007/s00421-011-1914-3](https://doi.org/10.1007/s00421-011-1914-3) · PMID 21437606 · OpenAlex W1976653497
- `mater2021` — Mater et al. (2021). Effect of Cycling Cadence on Neuromuscular Function: A Systematic Review of Acute and Chronic Alterations. *International Journal of Environmental Research and Public Health*. doi:[10.3390/ijerph18157912](https://doi.org/10.3390/ijerph18157912) · PMID 34360206 · OpenAlex W3183820523
- `reed2016` — Reed et al. (2016). Determining optimal cadence for an individual road cyclist from field data. *European Journal of Sport Science*. doi:[10.1080/17461391.2016.1146336](https://doi.org/10.1080/17461391.2016.1146336) · OpenAlex W2279754108
- `whitty2016` — Whitty et al. (2016). The effect of low- vs high-cadence interval training on the freely chosen cadence and performance in endurance-trained cyclists. *Applied Physiology, Nutrition, and Metabolism*. doi:[10.1139/apnm-2015-0562](https://doi.org/10.1139/apnm-2015-0562) · PMID 27175601 · OpenAlex W2284568593
