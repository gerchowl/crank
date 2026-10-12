# 11 · High-intensity intervals, short intervals (30/15) and time near VO2max

**Scope.** Interval design for VO2max/high-end aerobic work: short vs long intervals, fixed vs self-paced intensity, and what the judge should do during interval sets.
**Overall confidence:** HIIT raises VO2max more than continuous training — A; 30/15-type short intervals ≥ effort-matched long intervals in cyclists — B (one lab group, several RCTs); "time > 90% VO2max" as *the* driver of adaptation — C (plausible mechanism, not proven causal).

## Key findings

- **[A] HIIT improves VO2max and slightly more than continuous training.** Meta-analysis (28 controlled trials, 723 adults, mostly non-athletes): vs no training, HIT +5.5 mL/kg/min; vs endurance training, a small additional +1.2 mL/kg/min, with longer repetitions and longer programs favouring larger gains [@milanovic2015]. Aerobic HIIT improved VO2max more than supramaximal sprint intervals [@helgerud2023]; sprint-interval training effects on VO2max are meta-analysed separately [@sloth2013].
- **[B] Short intervals (30 s on / 15 s off) beat effort-matched long intervals in cyclists.** 10 weeks, 2×/week: VO2max +8.7% vs +2.6%; moderate–large effects on 30-s, 5-min and 40-min power [@ronnestad2015]. Elite cyclists, 3 weeks (9 sessions) of 3 × 13 × 30/15 vs 4 × 5 min: peak aerobic power +3.7% vs −0.3%, 20-min power +4.7% vs −1.4% [@ronnestad2020]. One-week shock microcycle: superior physiological adaptations with short intervals [@ronnestad2021]. Acute responses to effort-matched SI vs LI in elite cyclists [@almquist2020]; higher VO2 during SI is sex-independent [@urianstad2024].
- **[B/C] Whether short intervals give more time > 90% VO2max depends on pacing.** At *fixed* matched intensity, 3 × 13 × 30/15 gave more time > 90% VO2max than 6 × 5 min; when *self-paced at maximal effort*, both gave similar time (1130 vs 1016 s), and self-pacing increased time > 90% VO2max several-fold over fixed intensity (SI 1130 vs 420 s; LI 1016 vs 178 s) [@appelhans2025]. Similar time near VO2max across different within-interval work-rate distributions [@turnes2026]; decreasing-intensity designs can increase it [@vaccari2020]. In runners, intensified 30-s intervals gave *less* time > 90% VO2max than 3-min intervals [@fleckenstein2025].
- **[B] Time > 90% HRmax is not a valid proxy for time > 90% VO2max.** It moved in the opposite direction to VO2 in a crossover study [@fleckenstein2025].
- **[C (expert review)] Programming principles.** Aim for several minutes per session in the "red zone" (≥ 90% VO2max); nine variables (work/relief intensity and duration, modality, reps, series, between-series recovery) shape cardiopulmonary, glycolytic and neuromuscular load [@buchheit2013a; @buchheit2013b].
- **[B/C] Individual response varies hugely.** A 9-day HIIT block in national-level cyclists/triathletes: individual VO2max changes from −8.2% to +14.5% [@bratlandsanda2020].
- **[C] W′bal-designed intervals** (sessions planned to fully deplete W′) improved CP 5–6% in 4 weeks in masters cyclists [@galanrioja2023]; see 02 for W′bal caveats.

## Numbers the app can use

| Protocol / quantity | Value | Grade | Source |
|---|---|---|---|
| Short-interval template | 3 × 13 × (30 s on / 15 s off), 3 min between series | B | [@ronnestad2020; @appelhans2025] |
| Long-interval comparator | 4–6 × 5 min, 2.5 min recovery | B | [@ronnestad2020; @appelhans2025] |
| HIT frequency in positive cycling trials | 2–3 sessions/week (10 wk: 2/wk; 3 wk: 3/wk) | B | [@ronnestad2015; @ronnestad2020] |
| Time > 90% VO2max, self-paced | ~17–19 min per session in well-trained cyclists | C | [@appelhans2025] |
| HIIT vs continuous VO2max gain | +1.2 mL/kg/min (small) | A | [@milanovic2015] |

## Implications for the app

**Metrics to compute**
- Per-interval: mean power, % CP, W′ used, end-of-rep HR and HR reached (% HRmax), rep-to-rep fade (power or, in ERG, cadence/HR creep); session time ≥ 90% HRmax with the explicit label "not equal to time at VO2max".
- Set-level W′bal minimum (02).

**s1 judge features / rules**
- **Self-paced > fixed** for VO2max sessions: if the athlete completes reps easily (HR plateau below usual, W′bal well above 0 at rep end, RPE/feel ok) → `push` +2–5% on the next rep (C) [@appelhans2025].
- If rep power can't be held (ERG cadence collapse, W′bal predicted < 0 before rep end) → `ease_off` −3–5% for the remaining reps rather than abandoning (C/D).
- If ≥ 2 consecutive reps fail despite easing → `replan` (end set early / convert to threshold) (D).

**Planner rules**
- Default VO2max session for trained cyclists: 30/15 short-interval sets (B); alternate with long intervals (4–6 × 4–5 min) for variety and to target different power ranges (C).
- 1–3 HIT sessions/week depending on volume and readiness, consistent with a mostly low-intensity TID (06).
- Prefer **self-paced / RPE-maximal** targets or ERG with judge-driven auto-adjust over rigidly fixed % targets when the goal is time near VO2max (C).
- Expect large individual response variation; re-evaluate after 4–8 weeks with an MMP/CP check (B/C).

**UI hints**
- Interval page: rep counter, per-rep W′ used, fade indicator.
- Explain "% HRmax time" vs "VO2 time" caveat in the post-ride review.

## Controversies / uncertainty

- Evidence for 30/15 superiority comes largely from one research group; independent replications in cyclists are limited.
- Causality of "time near VO2max" for adaptation is assumed, not demonstrated [@buchheit2013a].
- Fixed-percentage prescriptions depend on the accuracy of the anchor (MAP/CP/FTP) — errors propagate (02, 03).

## References

<!-- REFS -->
- `almquist2020` — Almquist et al. (2020). Systemic and muscular responses to effort-matched short intervals and long intervals in elite cyclists. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.13672](https://doi.org/10.1111/sms.13672)
- `appelhans2025` — Appelhans et al. (2025). Multiple Short Intervals Induce Longer Time Above 90% of Maximal Oxygen Consumption Than Long Intervals When Matched by Similar Fixed Intensity, But Not During Self-Paced Cycling Ergometry. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2024-0115](https://doi.org/10.1123/ijspp.2024-0115) · PMID 40328438
- `bratlandsanda2020` — Bratland-Sanda (2020). Large Inter-individual Differences in Responses to a Block of High Intensity Aerobic Interval Training: a Case Series in National-level Cyclists and Triathletes. *International Journal of Exercise Science*. doi:[10.70252/otac6224](https://doi.org/10.70252/otac6224) · PMID 32509110 · OpenAlex W3009831526
- `buchheit2013a` — Buchheit & Laursen (2013). High-intensity interval training, solutions to the programming puzzle: Part I: cardiopulmonary emphasis. *Sports Medicine*. doi:[10.1007/s40279-013-0029-x](https://doi.org/10.1007/s40279-013-0029-x) · PMID 23539308
- `buchheit2013b` — Buchheit & Laursen (2013). High-intensity interval training, solutions to the programming puzzle. Part II: anaerobic energy, neuromuscular load and practical applications. *Sports Medicine*. doi:[10.1007/s40279-013-0066-5](https://doi.org/10.1007/s40279-013-0066-5)
- `fleckenstein2025` — Fleckenstein et al. (2025). Faster intervals, faster recoveries - intensified short VO2max running intervals are inferior to traditional long intervals in terms of time spent above 90% VO2max. *Frontiers in Sports and Active Living*. doi:[10.3389/fspor.2024.1507957](https://doi.org/10.3389/fspor.2024.1507957) · PMID 39835194 · OpenAlex W4406082950
- `galanrioja2023` — Galan-Rioja et al. (2023). Utility of the W'BAL model in training programme design for masters cyclists. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2142675](https://doi.org/10.1080/17461391.2022.2142675) · PMID 36310098
- `helgerud2023` — Helgerud et al. (2023). Aerobic high-intensity intervals improve VO2max more than supramaximal sprint intervals in females, similar to males. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.14470](https://doi.org/10.1111/sms.14470) · OpenAlex W4386084057
- `milanovic2015` — Milanovic et al. (2015). Effectiveness of High-Intensity Interval Training (HIT) and Continuous Endurance Training for VO2max Improvements: A Systematic Review and Meta-Analysis of Controlled Trials. *Sports Medicine*. doi:[10.1007/s40279-015-0365-0](https://doi.org/10.1007/s40279-015-0365-0) · PMID 26243014 · OpenAlex W1558792767
- `ronnestad2015` — Rønnestad et al. (2015). Short intervals induce superior training adaptations compared with long intervals in cyclists - an effort-matched approach. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.12165](https://doi.org/10.1111/sms.12165) · PMID 24382021
- `ronnestad2020` — Rønnestad et al. (2020). Superior performance improvements in elite cyclists following short-interval vs effort-matched long-interval training. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.13627](https://doi.org/10.1111/sms.13627) · PMID 31977120 · OpenAlex W3001686330
- `ronnestad2021` — Rønnestad et al. (2021). Superior Physiological Adaptations After a Microcycle of Short Intervals Versus Long Intervals in Cyclists. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2020-0647](https://doi.org/10.1123/ijspp.2020-0647) · OpenAlex W3136935079
- `sloth2013` — Sloth et al. (2013). Effects of sprint interval training on VO2max and aerobic exercise performance: A systematic review and meta-analysis. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.12092](https://doi.org/10.1111/sms.12092)
- `turnes2026` — Turnes et al. (2026). Different work rate distributions elicit similar time spent near VO2max during short-duration interval sessions. *Acta Gymnica*. doi:[10.5507/ag.2026.003](https://doi.org/10.5507/ag.2026.003) · OpenAlex W7134918424
- `urianstad2024` — Urianstad et al. (2024). The higher oxygen consumption during multiple short intervals is sex-independent and not influenced by skeletal muscle characteristics in well-trained cyclists. *European Journal of Sport Science*. doi:[10.1002/ejsc.12214](https://doi.org/10.1002/ejsc.12214) · OpenAlex W4403653471
- `vaccari2020` — Vaccari et al. (2020). High-intensity decreasing interval training (HIDIT) increases time above 90% VO2peak. *European Journal of Applied Physiology*. doi:[10.1007/s00421-020-04463-w](https://doi.org/10.1007/s00421-020-04463-w)
