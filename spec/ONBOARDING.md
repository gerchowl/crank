# crank — first run, cold start & the athlete model

Principle: **every athlete parameter is an estimate with uncertainty and provenance.** Onboarding gives
priors; imported history and rides refine them; the coach is conservative in proportion to uncertainty.

## 1. First-run onboarding (V0, once; ~3 min, skippable per section)
1. **About you**: age, sex (optional, affects priors), weight, resting HR (or "measure for me"), max HR if known.
2. **Background ("mileage")**: years riding; typical hours or km per week over the last 3 months;
   longest ride recently; structured training before? (none / some / lots).
3. **Current numbers, if known**: FTP/CP and from what (ramp test, 20-min test, app estimate,
   guess) with a date. Unknown is fine.
4. **Goals**: event and date (optional), focus (endurance / climbing / crits / general fitness / weight),
   days per week and time per session available, preferred ride types.
5. **Health and safety**: short PAR-Q-style screen (heart issues, chest pain, dizziness, medication, injuries).
   A red flag (chest pain, fainting/dizziness, racing or irregular heartbeat, unusual breathlessness, a heart
   condition a doctor limited exercise for) → **medical stop**: no plan, coach off, riding only as a deliberate,
   logged free ride (not even that when an injury cap is also active: a free ride has no targets, so the cap
   cannot be held), lifted only by a dated clearance the rider records (DESIGN §5.8). crank refers, it never
   clears. Medication and injury answers are not red flags and do different things: a medication that changes
   heart rate switches off HR-based coaching (HR zones, HR-guided recoveries, HR-vs-usual; no power cap); an
   injury that limits hard efforts caps intensity at threshold with no sprints.
   Item wording, sources and escalation: `spec/knowledge-pack/14-health-screening.md`.
6. **Equipment**: trainer and HR strap pairing test, crank length, connectors (intervals.icu, later).

## 2. Baseline paths (pick the best available, in this order)
| Path | Gives | Confidence |
|---|---|---|
| **Imported history** (intervals.icu, ≥ 6 weeks with power) | PD curve → CP/W′/OmPD, load history, durability, HR-at-power, typical cadence | high from day 1 |
| **Assessment week** (guided, 2–3 short sessions) | CP/W′ from 2–3 maximal efforts (e.g. 3′ + 12′ on separate days), HR max/recovery τ, DFA-α1 check on an easy ride | high after ~1 week |
| **Passive learning** (just ride; the planner adds short max efforts at the end of normal workouts) | fits converge over ~2–4 weeks | medium, improving |
| **Questionnaire only** | priors from age/volume/experience + any self-reported FTP | low → conservative |

The user chooses between the assessment week and passive learning. Imported data skips both.

## 3. Athlete model (contract addition)
```ts
interface Estimate { value: number; sd: number; source: "prior" | "self_report" | "imported" | "test" | "rides"; updatedAt: string; n: number }
interface AthleteModel {
  cp: Estimate; wPrime: Estimate; pmax: Estimate;          // OmPD
  hrMax: Estimate; hrRest: Estimate; lt1Hr?: Estimate;      // lt1 from DFA-α1 (guard only)
  tauRecovery: Estimate;                                    // HR recovery τ at reference conditions
  wbalRecoveryTau: Estimate;                                // per-athlete W′ reconstitution
  hrAtPower: { watts: number; hr: Estimate }[];             // "typical HR" curve
  economyByCadence?: { rpm: number; hrPerW: Estimate }[];
  durability?: Estimate;                                    // % CP retained after 2000 kJ
  weeklyLoad: { week: string; kj: number; hours: number }[];
}
```
- **Bayesian-style updating**: each ride updates estimates; old data decays (half-life ~6 weeks for CP).
- **Break and illness detection**: a gap > 10 days or a sudden HR-at-power shift → widen the uncertainty
  and plan conservatively (a "return" block).
- **Periodic re-check-in** every ~6 weeks, or when goals change: goals, availability, health.

## 4. Coach behaviour by confidence
- **Uncertainty-aware targets**: with high CP sd, intervals target the conservative side, and ramps include
  "how did that feel" taps that update the estimate.
- **s1 features unlock per baseline**: `hr_vs_typical` only once ≥ N minutes exist at that power band;
  until then the absolute rules from the knowledge pack apply (HR caps, DFA-α1 guard, RPE).
- **The UI shows confidence**: e.g. "CP 265 ± 20 W (from 4 rides)", with the band shown in the PD plot,
  and a "calibrating" badge on metrics still learning.
- **Progressive unlock**: auto-push and aggressive replans only after the baseline reaches medium confidence.

## 5. Views
- **V0 Onboarding** (wizard, sections 1–6, resumable).
- **V10 Settings → Athlete model**: every estimate with value ± sd, source, last update, and a manual override.
- **V1 Today** shows a calibration progress card until the baseline is complete.
