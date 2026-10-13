# 06 · Training intensity distribution (TID): polarized, pyramidal, threshold

**Scope.** How the planner distributes weekly time across intensity zones; how to measure TID; how strongly to believe "polarized is optimal".
**Overall confidence:** "most volume at low intensity plus some high intensity" — A/B; "polarized beats pyramidal" — not supported in recent meta-analyses (A, null result).

## Key findings

- **[B (observational)] Successful endurance athletes do mostly low-intensity training.** Elite athletes self-select ~70–90% of time below the first threshold (Z1), < 10% between thresholds (Z2), 10–20% above the second (Z3); polarized ≈ 70/10/20, pyramidal ≈ 70/20/10 [@foster2022; @stoggl2015; @sperlich2023]. Most retrospective studies describe *pyramidal* TID [@stoggl2015]; determinants of TID in elites are reviewed in [@bourgois2019].
- **[A] Polarized vs other TIDs: little or no difference in performance.**
  - IPD network meta-analysis (13 studies, 348 athletes): no difference POL vs PYR in VO2max (SMD −0.06) or TT (SMD −0.05); competitive athletes may gain VO2max more with POL, recreational with PYR [@rosenblat2025].
  - Meta-analysis (17 studies, 437): POL modestly better for VO2peak (SMD 0.24, mainly < 12-week studies and highly trained), **no difference** for TT, time to exhaustion, or VT2/LT2 power [@oliveira2024].
  - Cyclists specifically (41 studies, 797 participants): POL and non-POL comparable for VO2max and TT; no association between weekly/total volume and gains beyond a necessary volume [@cove2025].
  - Bayesian NMA: no TID model definitively better than POL; ranking probabilities favoured threshold for VO2max and HIT for TT (low certainty) [@li2026].
  - Earlier comparisons: a POL-favouring trial [@stoggl2014] and a POL-vs-threshold meta-analysis of RCTs [@rosenblat2019]; position statement "Polarized Training Is Optimal" acknowledges that experimental evidence favours pyramidal more often than polarized [@foster2022]. See also a 16-week POL vs PYR trial in runners [@filipas2022] and a scoping review across athlete levels [@riverakofler2025].
- **[B] Some high-intensity work seems obligatory.** Clamping Z1 at ~93% produced small gains [@foster2022]; HIIT is the most time-efficient VO2max stimulus (see 11).
- **[C] TID can be quantified with a polarization index.** PI = log10((Z1/Z2) × Z3 × 100) with zone fractions; PI > 2.00 a.U. = polarized [@treff2019]. TID results depend on how zones are defined (HR-time-in-zone vs session-goal vs power) [@foster2022; @rosenblat2025].
- **[C] Block periodization** of HIT is an alternative organization, meta-analysed in [@molmen2019] (effect sizes not extracted here).
- **[C] Durability link.** More time < VT1 and a shift toward polarized TID associated with better fatigued MMP in pros [@spragg2023training].

## Numbers the app can use

| Quantity | Value | Grade | Source |
|---|---|---|---|
| Z1 share (time) | ~70–90% (higher in high-volume sports like cycling) | B | [@foster2022; @stoggl2015] |
| POL template | ~70–80 / ~5–10 / ~15–20% | D (convention) | [@foster2022] |
| PYR template | ~70–80 / ~15–20 / ~5–10% | D (convention) | [@foster2022] |
| Polarization index cut-off | PI > 2.00 = polarized | C | [@treff2019] |
| Effect size POL vs others | VO2peak SMD ≈ 0.24 (short, highly trained); TT ≈ 0 | A | [@oliveira2024; @rosenblat2025] |

## Implications for the app

**Metrics to compute**
- Weekly and 4-week TID in **3 physiological zones** anchored on the athlete's thresholds (Z1 < VT1/HRVT1; Z2 VT1→CP; Z3 > CP), computed by power *and* by HR (they differ; HR lags and drifts) — show both.
- Session-goal TID (count sessions by intent) alongside time-in-zone [@foster2022].
- Polarization index [@treff2019].

**s1 judge features / rules**
- The judge enforces session *intent*: in a Z1 session, `ease_off` when power/HR/α1 indicate drift into Z2 for > ~2–3 min (C/D); "grey-zone creep" is the commonest way recreational riders lose the intended TID (D).

**Planner rules**
- Default to a **pyramidal-to-polarized** week: ≥ 75–80% of time in Z1, 1–2 (max 3) high-intensity sessions/week, and threshold/tempo time as needed by goals — do not enforce a rigid 80/20 polarized split (A: no superiority).
- Recreational/time-limited athletes (e.g., ≤ 5–6 h/week): pyramidal or threshold-inclusive designs are reasonable [@rosenblat2025]; competitive athletes may benefit from a more polarized block [@rosenblat2025; @oliveira2024].
- Prioritize consistency and adequate volume over the TID label; volume beyond a sufficient level showed no added benefit in cyclists' trials [@cove2025].

**UI hints**
- Weekly TID bar with Z1/Z2/Z3 and PI; label targets as ranges, not a single ideal.
- When the athlete asks "why not more intervals?" — explain the evidence: some high-intensity is needed; *more* is not clearly better.

## Controversies / uncertainty

- Definitions: HR-time-in-zone systematically inflates Z2 vs session-goal counting; studies use different anchors (LT, VT, HR %max, RPE) [@foster2022; @rosenblat2025].
- Most RCTs are short (≤ 12–16 weeks), small and in runners; cycling-specific long-term data are scarce [@cove2025].
- Popular "polarized = optimal for everyone" messaging outruns the experimental evidence (A-grade null findings).

## References

<!-- REFS -->
- `bourgois2019` — Bourgois et al. (2019). Perspectives and Determinants for Training-Intensity Distribution in Elite Endurance Athletes. *International Journal of Sports Physiology and Performance*. doi:[10.1123/ijspp.2018-0722](https://doi.org/10.1123/ijspp.2018-0722) · OpenAlex W2971586016
- `cove2025` — Cove et al. (2025). The effect of training distribution, duration, and volume on VO2max and performance in trained cyclists: A systematic review, multilevel meta-analysis, and multivariate meta-regression. *Journal of Science and Medicine in Sport*. doi:[10.1016/j.jsams.2024.12.005](https://doi.org/10.1016/j.jsams.2024.12.005) · PMID 39788807 · OpenAlex W4405651548
- `filipas2022` — Filipas et al. (2022). Effects of 16 weeks of pyramidal and polarized training intensity distributions in well-trained endurance runners. *Scandinavian Journal of Medicine and Science in Sports*. doi:[10.1111/sms.14101](https://doi.org/10.1111/sms.14101) · OpenAlex W3213367177
- `foster2022` — Foster et al. (2022). Polarized Training Is Optimal for Endurance Athletes. *Medicine and Science in Sports and Exercise*. doi:[10.1249/mss.0000000000002871](https://doi.org/10.1249/mss.0000000000002871) · PMID 35136001 · OpenAlex W4211102970
- `li2026` — Li et al. (2026). Effects of Different Training-Intensity Distribution Models on Maximal Oxygen Uptake and Time-Trial Performance in Endurance Athletes: A Bayesian Network Meta-Analysis. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000005415](https://doi.org/10.1519/jsc.0000000000005415) · PMID 42171506 · OpenAlex W7162117707
- `molmen2019` — Mølmen et al. (2019). Block periodization of endurance training - a systematic review and meta-analysis. *Open Access Journal of Sports Medicine*. doi:[10.2147/oajsm.s180408](https://doi.org/10.2147/oajsm.s180408) · OpenAlex W2980855242
- `riverakofler2025` — Rivera-Kofler et al. (2025). Effects of Polarized Training vs. Other Training Intensity Distribution Models on Physiological Variables and Endurance Performance in Different-Level Endurance Athletes: A Scoping Review. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000005033](https://doi.org/10.1519/jsc.0000000000005033) · OpenAlex W4405732314
- `rosenblat2019` — Rosenblat et al. (2019). Polarized vs. Threshold Training Intensity Distribution on Endurance Sport Performance: A Systematic Review and Meta-Analysis of Randomized Controlled Trials. *Journal of Strength and Conditioning Research*. doi:[10.1519/jsc.0000000000002618](https://doi.org/10.1519/jsc.0000000000002618) · OpenAlex W2806257935
- `rosenblat2025` — Rosenblat et al. (2025). Which Training Intensity Distribution Intervention will Produce the Greatest Improvements in Maximal Oxygen Uptake and Time-Trial Performance in Endurance Athletes? A Systematic Review and Network Meta-analysis of Individual Participant Data. *Sports Medicine*. doi:[10.1007/s40279-024-02149-3](https://doi.org/10.1007/s40279-024-02149-3) · PMID 39888556
- `oliveira2024` — Silva Oliveira et al. (2024). Comparison of Polarized Versus Other Types of Endurance Training Intensity Distribution on Athletes' Endurance Performance: A Systematic Review with Meta-analysis. *Sports Medicine*. doi:[10.1007/s40279-024-02034-z](https://doi.org/10.1007/s40279-024-02034-z) · PMID 38717713 · OpenAlex W4396748031
- `sperlich2023` — Sperlich et al. (2023). The proportional distribution of training by elite endurance athletes at different intensities during different phases of the season. *Frontiers in Sports and Active Living*. doi:[10.3389/fspor.2023.1258585](https://doi.org/10.3389/fspor.2023.1258585) · OpenAlex W4388189845
- `spragg2023training` — Spragg et al. (2023). The relationship between training characteristics and durability in professional cyclists across a competitive season. *European Journal of Sport Science*. doi:[10.1080/17461391.2022.2049886](https://doi.org/10.1080/17461391.2022.2049886) · PMID 35239466
- `stoggl2014` — Stoggl & Sperlich (2014). Polarized training has greater impact on key endurance variables than threshold, high intensity, or high volume training. *Frontiers in Physiology*. doi:[10.3389/fphys.2014.00033](https://doi.org/10.3389/fphys.2014.00033) · OpenAlex W2117675876
- `stoggl2015` — Stoggl & Sperlich (2015). The training intensity distribution among well-trained and elite endurance athletes. *Frontiers in Physiology*. doi:[10.3389/fphys.2015.00295](https://doi.org/10.3389/fphys.2015.00295) · OpenAlex W1881383198
- `treff2019` — Treff et al. (2019). The Polarization-Index: A Simple Calculation to Distinguish Polarized From Non-polarized Training Intensity Distributions. *Frontiers in Physiology*. doi:[10.3389/fphys.2019.00707](https://doi.org/10.3389/fphys.2019.00707) · PMID 31249533 · OpenAlex W2951390760
