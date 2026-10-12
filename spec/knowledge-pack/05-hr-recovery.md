# 05 · Heart-rate recovery (HRR) kinetics

**Scope.** Fitting HR decay during recovery blocks (`hr_recovery` widget, `StepEnd.kind = "hr_recovery"`, `HrRecoveryState.fit = {hrInf, amp, tauS, usualTauS}`), and what τ / HRR changes mean.
**Overall confidence:** exponential fit as a *descriptive* tool after submaximal work — B/C; HRR as a fatigue/readiness marker — C, and its direction is ambiguous.

## Key findings

- **[C] Mono-exponential fits are reasonable after submaximal work, poor after maximal work, and need ≥ 3 min of data.** τ stabilized only after ≥ 3 min of post-exercise monitoring; after maximal exercise τ changed by −13% with a 5-s shift in fit onset and residuals were non-random; after submaximal exercise τ was stable (−4%) [@pierpont2000]. After resistance exercise ≥ 360–420 s were needed for a stable τ [@bartelsferreira2015].
- **[C] Typical values (healthy, passive/seated recovery).** After 30 min at 70% VO2peak (middle-aged men): HRR at 60 s ≈ 22–25 bpm, HRRτ ≈ 107–120 s (SD up to ~100 s); metaboreflex activation slowed the slow phase [@pecanha2015]. HR keeps falling and HRV keeps rising for > 30 min after moderate exercise; early-recovery HR decline relates to early-recovery HRV (parasympathetic reactivation) rather than resting HRV [@javorka2002].
- **[B/C] Recovery mode changes HRR.** Active (pedaling) recovery slowed HRR vs inactive seated recovery; supine was fastest [@barak2011]. In-ride recovery blocks are active, so τ depends on recovery power — compare τ only at matched recovery power.
- **[B/C] Reliability is moderate.** Raw HR during recovery SEM ≈ 8%; ΔHR at 1 min SEM ≈ 20%; exponential parameters ICC 0.43–0.88, SEM 5.7–21.4% depending on intensity; "raw HR is the desired method" [@bosquet2008]. Day-to-day HR variation is lowest when the preceding work elicits ~85–90% HRmax (~3 bpm), and lowest in the first minute of recovery [@lamberts2009].
- **[B] HRR relates to training status, but needs standardization.** Systematic review: most longitudinal studies show HRR improving with training status; confounded by age, temperature, preceding intensity and duration [@daanen2012]. HIIT-containing programs improved acute HRR (+8–11%) where high-volume low-intensity did not [@stoggl2017].
- **[B/C] Faster HRR can mean *fatigue*, not fitness.** Functionally overreached triathletes showed **faster** HRR (+8 ± 5 bpm) with lower lactate/catecholamines and worse performance [@aubry2015]; effects of functional overreaching on autonomic HR regulation have also been studied [@bellenger2017]. Interpretation requires context (training phase, perceived fatigue, performance) [@buchheit2014; @schneider2018].
- **[C] Field estimation is possible.** First-order HR-from-power ODE and Wiener-type models fitted to field rides yield a time constant usable as an HRR proxy and can track it automatically [@saupe2024]. Reliability of recovery HRV within a standardized submaximal cycle test and its relation to training status have been examined in trained-to-elite cyclists [@lamberts2024].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Model | HR(t) = HR∞ + A·e^(−t/τ) after submax work; avoid relying on τ after maximal efforts | C | [@pierpont2000] |
| Minimum data for stable τ | ≥ 3 min (≥ 6–7 min in some protocols) | C | [@pierpont2000; @bartelsferreira2015] |
| Typical HRRτ (passive, after 30 min @ 70% VO2peak) | ~110–120 s (wide SD) | C | [@pecanha2015] |
| HRR60 after moderate work | ~20–25 bpm | C | [@pecanha2015] |
| Measurement noise | τ SEM 6–21%; ΔHR60 SEM ~20%; day-to-day submax HR ~3 bpm | B/C | [@bosquet2008; @lamberts2009] |
| Overreaching signal | HRR faster by ~8 bpm while performance falls | C | [@aubry2015] |

## Implications for the app

**Metrics to compute**
- Online fit of HR∞, A, τ with Bayesian/robust least squares; prior = athlete's `usualTauS` *at similar recovery power and preceding intensity*; publish predicted curve + band, `readyInS` (lo–hi), and `unreachable` if HR∞ ≥ target (already in contract).
- Within the first ~30–60 s the prediction is mostly prior-driven; widen the band until ≥ 60–90 s of data, and treat τ as unstable until ~3 min [@pierpont2000].
- Store per-block τ with context (preceding block power/duration, recovery power, cadence, elapsed time, kJ) for trend analysis; compare like with like.
- Optionally also store the power→HR dynamic model time constant from whole rides [@saupe2024].

**s1 judge features / rules**
- `hr_recovery` step: if `unreachable` (HR∞ ≥ target) with confidence after ≥ 90 s → apply `onUnreachable` (`lower_power` / `cap` / `ask_coach`) — mechanistic, fine as engineering rule (D).
- τ markedly slower than usual (e.g., > 1.3–1.5× usual τ at matched context) within a session → supporting evidence for `ease_off` or fewer remaining reps (heuristic D; the threshold is not evidence-based).
- Do **not** interpret faster-than-usual HRR as readiness to `push` on its own; overreaching can speed HRR [@aubry2015]. Require concordant evidence (performance at target, RPE, check-in).

**Planner rules**
- Use HR-gated recoveries mainly for safety/consistency between reps, not as a fitness test.
- Track HRR trends only from a **standardized** situation (e.g., the same submax warm-up step reaching ~85–90% HRmax, then fixed recovery power) [@lamberts2009; @daanen2012].

**UI hints**
- `hr_recovery` widget: show "ready in ≈ m:ss (lo–hi)", τ vs usual τ with a note "comparable only at similar recovery power".
- Post-ride τ-per-block chart (UX spec) should mark blocks whose preceding effort was maximal (τ less meaningful).

## Controversies / uncertainty

- Direction ambiguity: faster HRR may reflect improved fitness *or* functional overreaching [@daanen2012; @aubry2015].
- Mono- vs multi-component kinetics: early (vagal, ~first 30–60 s) and slow (metaboreflex/thermal/catecholamine) phases differ [@pecanha2015; @javorka2002]; a single τ blends them.
- Most data are passive-recovery lab studies; active in-ride recoveries on a trainer are under-studied (gap).
- Typical τ values for trained cyclists during active recovery between intervals were not found in this search.

## References

<!-- REFS -->
- `aubry2015` — Aubry et al. (2015). The Development of Functional Overreaching Is Associated with a Faster Heart Rate Recovery in Endurance Athletes. *PLoS ONE*. doi:[10.1371/journal.pone.0139754](https://doi.org/10.1371/journal.pone.0139754) · PMID 26488766 · OpenAlex W2221922668
- `barak2011` — Barak et al. (2011). Heart rate recovery after submaximal exercise in four different recovery protocols in male athletes and non-athletes. *Journal of Sports Science and Medicine*. PMID 24149885 · OpenAlex W2137315967
- `bartelsferreira2015` — Bartels-Ferreira et al. (2015). Can a first-order exponential decay model fit heart rate recovery after resistance exercise?. *Clinical Physiology and Functional Imaging*. doi:[10.1111/cpf.12132](https://doi.org/10.1111/cpf.12132) · PMID 24494748 · OpenAlex W1972020448
- `bellenger2017` — Bellenger et al. (2017). The effect of functional overreaching on parameters of autonomic heart rate regulation. *European Journal of Applied Physiology*. doi:[10.1007/s00421-017-3549-5](https://doi.org/10.1007/s00421-017-3549-5)
- `bosquet2008` — Bosquet et al. (2008). Reliability of postexercise heart rate recovery. *International Journal of Sports Medicine*. doi:[10.1055/s-2007-965162](https://doi.org/10.1055/s-2007-965162) · PMID 17614018 · OpenAlex W1966239604
- `buchheit2014` — Buchheit (2014). Monitoring training status with HR measures: do all roads lead to Rome?. *Frontiers in Physiology*. doi:[10.3389/fphys.2014.00073](https://doi.org/10.3389/fphys.2014.00073) · PMID 24578692 · OpenAlex W2137418727
- `daanen2012` — Daanen et al. (2012). A systematic review on heart-rate recovery to monitor changes in training status in athletes. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.7.3.251](https://doi.org/10.1123/ijspp.7.3.251) · PMID 22357753 · OpenAlex W2114975892
- `javorka2002` — Javorka et al. (2002). Heart rate recovery after exercise: relations to heart rate variability and complexity. *Brazilian Journal of Medical and Biological Research*. doi:[10.1590/s0100-879x2002000800018](https://doi.org/10.1590/s0100-879x2002000800018) · PMID 12185393 · OpenAlex W2164651630
- `lamberts2024` — Lamberts et al. (2024). Reliability of recovery heart rate variability measurements as part of the Lamberts Submaximal Cycle Test and the relationship with training status in trained to elite cyclists. *European Journal of Applied Physiology*. doi:[10.1007/s00421-023-05385-z](https://doi.org/10.1007/s00421-023-05385-z)
- `lamberts2009` — Lamberts & Lambert (2009). Day-to-Day Variation in Heart Rate at Different Levels of Submaximal Exertion: Implications for Monitoring Training. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0b013e3181a2dcdc](https://doi.org/10.1519/jsc.0b013e3181a2dcdc) · PMID 19387374 · OpenAlex W2017114764
- `pecanha2015` — Pecanha et al. (2015). Metaboreflex Activation Delays Heart Rate Recovery after Aerobic Exercise. *The FASEB Journal*. doi:[10.1096/fasebj.29.1_supplement.1054.4](https://doi.org/10.1096/fasebj.29.1_supplement.1054.4) · OpenAlex W1648790336
- `pierpont2000` — Pierpont et al. (2000). Heart rate recovery post-exercise as an index of parasympathetic activity. *Journal of the Autonomic Nervous System*. doi:[10.1016/s0165-1838(00)00090-4](https://doi.org/10.1016/s0165-1838(00)00090-4) · PMID 10785283 · OpenAlex W1979967239
- `saupe2024` — Saupe (2024). Estimation of Heart Rate Recovery from Field Rides. *Journal of Science and Cycling*. OpenAlex W7217666987
- `schneider2018` — Schneider et al. (2018). Heart Rate Monitoring in Team Sports - A Conceptual Framework for Contextualizing Heart Rate Measures for Training and Recovery Prescription. *Frontiers in Physiology*. doi:[10.3389/fphys.2018.00639](https://doi.org/10.3389/fphys.2018.00639) · OpenAlex W2801918947
- `stoggl2017` — Stoggl & Bjorklund (2017). High Intensity Interval Training Leads to Greater Improvements in Acute Heart Rate Recovery and Anaerobic Power as High Volume Low Intensity Training. *Frontiers in Physiology*. doi:[10.3389/fphys.2017.00562](https://doi.org/10.3389/fphys.2017.00562) · PMID 28824457 · OpenAlex W2741716073
