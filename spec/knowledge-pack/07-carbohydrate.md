# 07 · Carbohydrate (CHO) intake during exercise

**Scope.** Fueling prompts and plan annotations for indoor rides (the app cannot measure intake; it can prompt and log).
**Overall confidence:** CHO during > ~1–2 h improves performance — A; ≤ 90 g/h with multiple transportable CHO for > 2.5 h — A (guideline/consensus); 90 → 120 g/h adds *oxidation* — B, adds *performance* — C (few, mixed studies).

## Key findings

- **[A] Duration-based guidance.** ~1 h: mouth rinse or small amounts may help; 2–3 h: up to ~60 g/h of a single CHO source; > 2.5 h: ~90 g/h, which requires multiple transportable CHO (glucose + fructose) [@jeukendrup2014]. The 2016 ACSM sport-nutrition guidance referenced in a 2026 review recommends ≤ 90 g/h (multiple transportable CHO) for exercise > 2.5–3 h [@morton2026]. Advice is independent of body mass and training status, but should scale down when absolute intensity (and CHO oxidation) is low [@jeukendrup2014].
- **[B] Dose–response is curvilinear.** In 51 cyclists (2 h constant load + 20-km TT), estimated performance gains were 1–4.7% between 9 and 78 g/h, with diminishing returns above ~78 g/h (glucose:fructose:maltodextrin 1:1:1) [@smith2013].
- **[B] Multiple transportable CHO outperform glucose-only at high rates.** Glucose+fructose (2:1) at 1.8 g/min improved TT by 8% vs glucose alone [@currell2008]; mixtures reach higher exogenous oxidation (~1.2–1.3+ g/min) [@jentjens2005]; ratio affects oxidation and gut comfort [@obrien2011]; effects studied in long-distance triathlon [@rowlands2017].
- **[B/C] 120 g/h raises exogenous oxidation and is tolerable in trained athletes; performance gains are less certain.** 120 g/h (0.8:1 fructose:maltodextrin) vs 90 g/h (1:2): exogenous oxidation 1.51 vs 1.29 g/min, endogenous CHO oxidation unchanged [@podlogar2022oxidation]. 120 g/h from drink, gel, chew or mix: peak exogenous oxidation ~1.56–1.66 g/min, ~72–75% oxidation efficiency, minimal GI symptoms, no difference in capacity between formats [@hearris2022]. Elite marathoners: higher oxidation and lower O2 cost at 120 vs 60/90 g/h [@ravikanti2025] — contested methodologically [@podlogar2026critique]. Review: the upper limit "could increase from 90 to 120 g/h (at least for trained participants)"; 120–200 g/h seen in the field is *not* substantiated [@morton2026].
- **[B] CHO supports durability.** 100 vs 60 g/h during 4 h of intermittent cycling: smaller 15-min TT decline (−6.4% vs −9.3%), no difference in muscle glycogen use [@mazza2026]. 0 / 60 / 120 g/h during 3 h: dose-dependent preservation of CP (236 / 257 / 266 W vs 277 W fresh), no effect on W′ [@norte2026cp]. Low CHO availability lowers power at the moderate→heavy transition [@main2026].
- **[B] CHO and fluid also blunt cardiovascular drift** (see 09) [@hamilton1991].
- **[C] Train-low strategies** have limited performance utility on current evidence; periodize CHO to session demands [@podlogar2022horizons]. Intense prolonged exercise is CHO-dependent [@hawley2015].

## Numbers the app can use

| Session | CHO target | Grade | Source |
|---|---|---|---|
| < ~60–75 min | none required; small amount/mouth rinse optional | A/B | [@jeukendrup2014] |
| ~1–2.5 h, moderate/high intensity | 30–60 g/h (single source OK up to ~60) | A | [@jeukendrup2014] |
| > 2.5 h | up to ~90 g/h, glucose+fructose | A | [@jeukendrup2014; @morton2026] |
| > 2.5 h, trained & gut-tolerant | 90–120 g/h, ~1:0.8 glucose(maltodextrin):fructose | B (oxidation) / C (performance) | [@podlogar2022oxidation; @hearris2022; @morton2026] |
| Exogenous oxidation ceiling | ~1.3 g/min (90 g/h) to ~1.5–1.65 g/min (120 g/h) | B | [@podlogar2022oxidation; @hearris2022] |
| Diminishing returns | above ~78 g/h in a 2-h + TT protocol | B | [@smith2013] |

## Implications for the app

**Metrics to compute**
- Planned **CHO budget** per workout (g/h × duration) derived from duration and planned intensity; logged intake (tap "+30 g" buttons) → `cho_g_per_h` actual.
- Estimate CHO oxidation demand only coarsely (kJ and intensity); do not present it as precise.

**s1 judge features / rules**
- Feed the judge "CHO logged in last 30–45 min (yes/no, g)". In rides > 90 min, if no intake logged and the athlete is in late intervals with rising decoupling → message suggests fueling and biases toward `hold`/`ease_off` rather than `push` (C/D).

**Planner rules**
- Every planned workout > 75–90 min carries a fueling note with a g/h target per the table; > 2.5 h defaults to 60–90 g/h; offer 90–120 g/h only if the athlete has opted in (trained, has practiced high intake) (B/C).
- For durability-focused long rides (see 01), the default should *include* fueling unless the athlete explicitly chooses a low-CHO session; do not prescribe train-low as a performance strategy by default [@podlogar2022horizons].
- Pre-event: practice race fueling in training (D).

**UI hints**
- Fueling reminder every 15–20 min in long rides (D), big tap targets for "+20/30/40 g" and "+500 ml".
- Show g/h achieved vs planned in the post-ride summary.

## Controversies / uncertainty

- Performance benefit of 120 vs 90 g/h is not established in well-powered cycling trials; most evidence is oxidation/tolerance and durability-type outcomes [@morton2026; @podlogar2026critique].
- Optimal fructose ratio (1:2 vs 1:0.8) and individual GI tolerance vary [@obrien2011; @podlogar2022oxidation].
- Evidence is predominantly male; female data are emerging [@main2026; @morton2026].
- "Gut training" is widely practiced; it was not covered by this search (gap).

## References

<!-- REFS -->
- `currell2008` — Currell & Jeukendrup (2008). Superior endurance performance with ingestion of multiple transportable carbohydrates. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0b013e31815adf19](https://doi.org/10.1249/mss.0b013e31815adf19) · PMID 18202575 · OpenAlex W2138450863
- `hamilton1991` — Hamilton et al. (1991). Fluid replacement and glucose infusion during exercise prevent cardiovascular drift. *Journal of Applied Physiology*. doi:[10.1152/jappl.1991.71.3.871](https://doi.org/10.1152/jappl.1991.71.3.871) · PMID 1757323 · OpenAlex W1837083581
- `hawley2015` — Hawley & Leckey (2015). Carbohydrate Dependence During Prolonged, Intense Endurance Exercise. *Sports Medicine*. doi:[10.1007/s40279-015-0400-1](https://doi.org/10.1007/s40279-015-0400-1) · OpenAlex W2125438466
- `hearris2022` — Hearris et al. (2022). 13C-glucose-fructose labeling reveals comparable exogenous CHO oxidation during exercise when consuming 120 g/h in fluid, gel, jelly chew, or coingestion. *Journal of Applied Physiology*. doi:[10.1152/japplphysiol.00091.2022](https://doi.org/10.1152/japplphysiol.00091.2022) · PMID 35446596
- `jentjens2005` — Jentjens & Jeukendrup (2005). High rates of exogenous carbohydrate oxidation from a mixture of glucose and fructose ingested during prolonged cycling exercise. *British Journal of Nutrition*. doi:[10.1079/bjn20041368](https://doi.org/10.1079/bjn20041368) · OpenAlex W2142308193
- `jeukendrup2014` — Jeukendrup (2014). A Step Towards Personalized Sports Nutrition: Carbohydrate Intake During Exercise. *Sports Medicine*. doi:[10.1007/s40279-014-0148-z](https://doi.org/10.1007/s40279-014-0148-z) · PMID 24065337 · OpenAlex W2000960085
- `main2026` — Main et al. (2026). Low carbohydrate availability reduces power output at the moderate-to-heavy transition, impairs efficiency, and increases median power frequency during cycling in females. *European Journal of Applied Physiology*. doi:[10.1007/s00421-026-06260-3](https://doi.org/10.1007/s00421-026-06260-3) · OpenAlex W7163123181
- `mazza2026` — Mazza et al. (2026). Carbohydrate Intake of 100 vs. 60 g/h During Prolonged Cycling Improves Durability Without Altering Muscle Glycogen Utilization in Male Endurance Athletes. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000004180](https://doi.org/10.1249/mss.0000000000004180) · PMID 42841583
- `morton2026` — Morton et al. (2026). From Metabolism to Medals: Contemporary Perspectives and Revisiting Carbohydrate Guidelines for Fueling Endurance Athletes during Exercise. *The Journal of Nutrition*. doi:[10.1016/j.tjnut.2026.101442](https://doi.org/10.1016/j.tjnut.2026.101442) · PMID 41759826
- `norte2026cp` — Norte et al. (2026). Graded Carbohydrate Ingestion up to 120 g/h Attenuates the Reduction in Critical Power Following 3 h of Moderate-Intensity Exercise in a Dose-Dependent Manner. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.70326](https://doi.org/10.1111/sms.70326) · PMID 42322010
- `obrien2011` — O'Brien & Rowlands (2011). Fructose-maltodextrin ratio in a carbohydrate-electrolyte solution differentially affects exogenous carbohydrate oxidation rate, gut comfort, and performance. *American Journal of Physiology - Gastrointestinal and Liver Physiology*. doi:[10.1152/ajpgi.00419.2010](https://doi.org/10.1152/ajpgi.00419.2010)
- `podlogar2022oxidation` — Podlogar et al. (2022). Increased exogenous but unaltered endogenous carbohydrate oxidation with combined fructose-maltodextrin ingested at 120 g/h versus 90 g/h at different ratios. *European Journal of Applied Physiology*. doi:[10.1007/s00421-022-05019-w](https://doi.org/10.1007/s00421-022-05019-w) · PMID 35951130 · OpenAlex W4291021389
- `podlogar2026critique` — Podlogar & Rowlands (2026). Does 120 g/h carbohydrate ingestion really confer a metabolic advantage in elite marathoners? Methodological concerns and alternative interpretations. *Journal of Applied Physiology*. doi:[10.1152/japplphysiol.00303.2026](https://doi.org/10.1152/japplphysiol.00303.2026) · PMID 42284230 · OpenAlex W7164523166
- `podlogar2022horizons` — Podlogar & Wallis (2022). New Horizons in Carbohydrate Research and Application for Endurance Athletes. *Sports Medicine*. doi:[10.1007/s40279-022-01757-1](https://doi.org/10.1007/s40279-022-01757-1) · PMID 36173597 · OpenAlex W4297964698
- `ravikanti2025` — Ravikanti et al. (2025). 13C-labelled glucose-fructose show greater exogenous and whole-body CHO oxidation and lower O2 cost of running at 120 versus 60 and 90 g/h in elite male marathoners. *Journal of Applied Physiology*. doi:[10.1152/japplphysiol.00665.2025](https://doi.org/10.1152/japplphysiol.00665.2025)
- `rowlands2017` — Rowlands & Houltham (2017). Multiple-Transportable Carbohydrate Effect on Long-Distance Triathlon Performance. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000001278](https://doi.org/10.1249/mss.0000000000001278) · OpenAlex W2597474893
- `smith2013` — Smith et al. (2013). Curvilinear dose-response relationship of carbohydrate (0-120 g/h) and performance. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0b013e31827205d1](https://doi.org/10.1249/mss.0b013e31827205d1) · PMID 22968309
