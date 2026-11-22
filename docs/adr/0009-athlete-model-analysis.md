# ADR-0009: Athlete model, post-ride analysis executor, ride analysis, comparison and set statistics

- **Status:** Proposed
- **Date:** 2026-11-22
- **Issue:** filed on acceptance (owner request 2026-11-22; the brief is this ADR's §1)
- **Related:** ADR-0001 §2 (crate boundaries), §4 (contract), §6 (persistence), §9.4 (lock-less writers); ADR-0002 §7.8
  (on target); ADR-0004 (threshold test, Proposed, branch `docs/486-threshold-test-decision`); ADR-0008 (DSL);
  `spec/ONBOARDING.md` §2–§5; DESIGN §5.5, §5.6, §5.9, §18.3; knowledge pack 01, 02, 03, 06, 08, 09, 13;
  #18, #106–#109, #113, #247, #257, #570, #582, #586, #599, #645, #699

## For the owner

This proposal is docs only; nothing is implemented. Each line is a decision that needs a yes or a no. The section
in brackets has the reasons and the rejected alternatives.

1. **The athlete model holds eight parameters the rider can see and set**: FTP, CP, W′, Pmax, HR max, LTHR, resting
   HR and weight, each an estimate with ± sd, source, method, date and full history (§2.1.1).
2. **Weight moves into the athlete model** with a history (for W/kg and kJ/kg trends); the single `athlete_profile.weight`
   column is copied over and dropped (§2.1.1, §2.5).
3. **A ride freezes the athlete model at load** (a snapshot stored with the ride). A change in Settings during a ride
   applies from the next ride (§2.1.4).
4. **A ride's IF, TSS and zones always use the snapshot it was ridden with**, never today's values, so editing FTP
   never rewrites history (§2.1.4, §2.2.3).
5. **Priors are explicit**: FTP 250 W ± 25 % (today's constant), CP = the FTP prior, W′ 20 kJ ± 30 %, HR max by
   Tanaka (208 − 0.7 × age) ± 10 bpm, no LTHR or resting-HR prior. **HR zones are never drawn from a prior HR max**:
   the ride shows no HR zone until HR max or LTHR is measured or typed in (§2.1.2).
6. **No silent writes to the eight parameters.** Analysis only *proposes*; the rider confirms each proposal. Derived
   baselines that set no prescription (HR at power, usual HR-recovery τ, typical distributions) update on their own,
   versioned and visible (§2.2.5).
7. **Tests and analysis calibrate FTP, CP/W′, HR max and LTHR**, each as a proposal with ± and source that the rider
   confirms (owner ruling 2026-11-22, "HR calibration wanted"): FTP from the ramp test (ADR-0004) and from CP; CP/W′
   from the CP assessment (ADR-0004 "next") and from self-paced rides; HR max from a verified ramp peak and from a
   clean ride peak; LTHR from a 30-min threshold effort (Friel's protocol, a test or a matching ride segment) and from
   steady ERG blocks at threshold. A value typed in Settings wins: it is current until the rider accepts a later
   proposal, and it supersedes every open proposal for that parameter. Resting HR and weight are typed in only
   (§2.1.1, §2.2.4).
8. **An upward FTP proposal from a valid CP fit** ("from your power-duration curve"), never downward. Recommended: yes
   (§2.2.4, row P8).
9. **Re-analysing an old ride never proposes over a newer estimate**: a proposal needs a ride newer than the
   parameter's current value (or a current value that is still the prior) (§2.2.4).
10. **ERG seconds never feed the CP fit**: in ERG the power is the target, not what the rider could do; only
    self-paced seconds (free ride, resistance, SIM, a self-paced test) count. Rides recorded before the per-second
    target existed count as self-paced only when the whole ride was (a free or route ride) (§2.2.4, §2.3.2).
11. **The record gains two per-second metrics**, the controlled target and the on-target class, so compliance and
    per-interval comparison work on stored rides, not only live (§2.3.2).
12. **Fitness / fatigue / form (CTL/ATL/TSB) is shown as a descriptive trend** with the generic 42/7-day constants and
    that label, never as a prediction (§2.3).
13. **Aerobic decoupling is shown with no verdict**: no "under 5 % is good" rule, because no study backs it (§2.3).
14. **Ride categories are derived** from the ridden power (endurance, tempo, threshold, VO2, anaerobic, recovery, test,
    free) in v1; rider-defined tags come later as their own issue (§2.4.3).
15. **History becomes the place for sets and comparison**: a filter bar (date range, workout, category, kind), a stats
    panel for the filtered set (totals, zone time, distributions, MMP envelope, a trend of a chosen metric), a tick
    column to pick rides, and a Compare view for two rides; Review links to "this workout's other rides" (§2.4).
16. **Analysis results are stored** in derived tables keyed by ride and an analysis version, and recomputed in the
    background when the version changes (§2.2.2, §2.5).
17. **ADR-0004 is amended** so its `athlete_confirm_ftp` becomes the generic proposal accept of §2.2.6 (§2.2.4).
18. **A proposal is accepted against the evidence it froze** (the rides, MMP points and analysis version it was
    computed from), so new rides never make it unacceptable; only a version bump or a deleted ride makes it stale, and
    then a fresh one is raised (§2.2.4).
19. **A gap or a pause never fails a ride's analysis**: each metric has its own gap and coverage rule, rolling windows
    never bridge a gap, and a metric below its coverage is shown as missing with the reason (§2.3.1.1).

## 1. Context

Owner, 2026-11-22: "an analysis afterwards — a runtime/executor that would update it [FTP/HR], and being able to set
it manually in the settings", "all the nerd analysis of the done workouts", "comparison of workouts"; and, folded into
part 4 the same day, statistics over a **selection** of rides or **all** rides (filter by date range, workout,
type/tags; totals, distributions, time in zone, MMP over the set, trends) and how the rider selects the set.

### 1.1 What exists today (dev at `dbf9f47b`)

| Area | State | Where |
|---|---|---|
| FTP | a constant, 250 W, written into every library workout's `Workout.ftp` | `crates/crank-core/src/library.rs` `FTP_W` |
| Athlete store | `athlete_estimate (param, value, sd, source, n, updated_at)` exists, append-only by design; **no code reads or writes it**; `athlete_profile.weight` exists, unused | `migrations/0001_initial.sql`, ADR-0001 §6 |
| Athlete inputs to the ride | #247 (open): resolve CP, W′, τ onto `RideSetup`; `compile.rs` `reserve_diagnostic` uses CP = the workout's FTP and a hardcoded W′ of 20 kJ (the 280 W in #247's text is the simulator rider's default FTP, `sim/core.rs` `SimRider`, not a compile constant) | #247 |
| HR references | `app/src/athleteHr.ts` placeholder: LTHR and HR max are always `null`, nothing sets them; the HR-zone tables live in `app/src/ride/hrZone.ts` | #586, #645 |
| Settings → Athlete model | lists HR max and LTHR as "Not set"; estimates, ± and override are #113 | DESIGN §5.9 |
| Stored ride summary | `RideSummary { duration_s, avg_power_w, np_w, avg_hr_bpm, kj, blocks[planned_s, planned_w, actual_s, actual_w] }`, written at finish | `dto.rs` `RideSummary`, `recorder.rs` `summarize` |
| TSS / IF of a ride | missing (#570); the *compiled workout* has planned TSS, IF and `time_in_zone_s` (Coggan 7 zones at 55/75/90/105/120/150 % FTP) | `compiled.rs` `CompiledWorkout`, `compile.rs` |
| Per-second record | 1 Hz `power, cadence, speed, hr, workout_s` plus `block`, `f32` in 60 s chunks, NaN = no data, an `estimated` bitmask | `record.rs` `METRICS`, `ride_chunk` |
| Controlled target per second | **not recorded**; on-target is counted live per lap only (`StreamCompliance { counted_s, within_s, excluded_s }`) | `lap.rs` `LapStats`, `stream/types.rs` `StreamCompliance` |
| Laps | computed live by the engine (#12); the `lap` table exists but **nothing writes it** | `lap.rs`, `0001_initial.sql` |
| HR recovery | per-block fit (`hr_recovery.rs`), `recovery` table exists, not yet written by the core | ADR-0001 §6 |
| Domain metrics | `normalized_power`, `histogram_from`, `heatmap_from`, `mmp_from`, `wbal_series`, HR-recovery fit, rMSSD/DFA-α1 | `crank-domain` |
| Review | name, outcome, traces of the *just ridden* session only (the core returns no stored samples), figures, planned vs ridden per block; items 1–3, 5, 6 of §5.5 are #106–#108 | DESIGN §5.5 "In direction A" |
| History | rides table (Started, Ride, Duration, Work, Outcome); athlete model card, CP trend, PD curve are #109 | DESIGN §5.6 |
| Series to the UI | live stream only; a backfill cursor and pixel downsampling are #599 | ADR-0001 §4.3.10 |
| FIT export | not built (#257) | — |
| Commands | `rides_list`, `ride_summary`; no athlete, analysis or comparison command | `app/src-tauri/src/commands.rs` |

### 1.2 Forces

- `spec/ONBOARDING.md` §3: every athlete parameter is an estimate with uncertainty and provenance; the coach is
  conservative in proportion to the uncertainty.
- **Owner rulings of 2026-11-22 on ADR-0004** (being finalised on `docs/486-threshold-test-v2`): the ramp test first,
  a CP assessment next; FTP ± 8 % as a convention floor; **HR calibration wanted**: tests and analysis estimate HR max
  and LTHR with ± and source, the rider confirms before anything is written, and manual entry in Settings wins; the
  safety stop at HR max + 5 bpm for 10 s uses a measured or self-reported HR max; a rider over 45 may test after an
  acknowledgement. This ADR matches them.
- ADR-0004 §2.5: a value that rescales every workout is never written without an explicit confirm, and the confirm
  recomputes in the core so a UI can never write a value the core did not derive. This ADR generalises that rule.
- The knowledge pack separates evidence from convention. FTP is not a physiological threshold (03); TSS/CTL are
  conventions with intensity-dependent behaviour (13, [vanerp2019]); the 5 % decoupling rule has no supporting study
  (09); ERG rides carry little maximal-power signal (01).
- ADR-0001 §6: the store crate owns the schema, migrations are forward-only, the review and analytics views are
  relational queries over rides. §9.4: only the lock holder writes ride data.
- Everything stays on the machine (ADR-0001 §7).

## 2. Decision

The four parts ship in order; each is useful when the parts before it have landed.

### 2.1 Part 1: the athlete model and manual settings

#### 2.1.1 Parameters

| Param (`AthleteParam`) | Unit | Used by | Sources allowed |
|---|---|---|---|
| `ftp` | W | % FTP → W at load, power zones, IF/TSS, generation (#699) | prior, self_report, test, rides, imported |
| `cp` | W | W′bal, coach (#247), kJ above CP | prior, self_report, test (the CP assessment), rides, imported |
| `w_prime` | J | W′bal, coach (#247) | prior, self_report, test (the CP assessment), rides, imported |
| `pmax` | W | OmPD fit (#18), MMP model display | self_report, rides, imported |
| `hr_max` | bpm | HR zones (5 zones), ADR-0004 HR safety stop (non-prior only) | prior, self_report, test (ramp peak), rides (clean peak), imported |
| `lthr` | bpm | HR zones (Friel 7, wins over HR max, DESIGN §18.3) | self_report, test (30-min effort), rides (threshold segment or steady ERG block), imported |
| `hr_rest` | bpm | display; HR reserve later | self_report, imported |
| `weight` | kg | W/kg, kJ/kg (durability §2.3) | self_report, imported |

`test` means a result of a dedicated assessment ride (ADR-0004); `rides` is ONBOARDING's name for "from analysis of
ordinary rides". Each method behind a `test` or `rides` value, with its evidence or convention status, is in §2.2.4.

The two τ of ONBOARDING §3 (`tauRecovery`, `wbalRecoveryTau`) and `hrAtPower` are **derived baselines** (§2.2.5), not
rider-set parameters. `lt1Hr`, `durability` and `economyByCadence` wait for the issues that compute them (#18).

**Estimate** (`crank-domain::athlete`, pure):

```rust
pub struct Estimate {
    pub id: i64,                 // the row; stable, referenced by rides and proposals
    pub param: AthleteParam,
    pub value: f64,
    pub sd: f64,
    pub source: Source,          // Prior | SelfReport | Test | Rides | Imported (ONBOARDING §3 names)
    pub method: String,          // "prior_v1", "manual", "ramp_v1_0.75", "peak_hr_v1", "cp_ols_v1", …
    pub ride_id: Option<String>, // the ride the value came from, if any
    pub n: u32,                  // number of efforts or rides behind it
    pub updated_at: i64,         // UTC seconds
}
```

History is append-only: the current value of a param is its latest row; a manual change, an accepted proposal or a
revert each add a row. **Revert** ("go back to 287 W from 3 March") adds a copy of the old row with source and method
kept and a new `updated_at`, so the history shows what happened.

**Plausibility bounds** (convention; a value outside is refused with `InvalidInput`, never clamped): FTP and CP 50–600 W;
W′ 2–60 kJ; Pmax 200–2500 W; HR max 120–230 bpm; LTHR 100–220 bpm and below HR max when both are set; resting HR
30–110 bpm; weight 30–200 kg; sd > 0 and ≤ 50 % of the value.

#### 2.1.2 Priors

`crank-domain::athlete::prior` holds the priors as named constants, each documented; they replace the hidden
constants (`library.rs` `FTP_W`, the CP and W′ in `compile.rs`, #247's fallbacks):

| Param | Prior | sd | Class |
|---|---|---|---|
| `ftp` | 250 W (today's constant) | 25 % | convention |
| `cp` | the FTP prior's value | 25 % | convention; on average CP sits 7 ± 13 W above a 20-min FTP [K21], too small to encode in a prior this wide |
| `w_prime` | 20 kJ (today's `compile.rs` value) | 30 % | convention |
| `hr_max` | 208 − 0.7 × age (needs the age; none without it) | 10 bpm | evidence: [T01] meta-analysis, 351 studies; individual error near ±10 bpm |
| `lthr`, `hr_rest`, `pmax`, `weight` | none | — | unknown is shown as "Not set" |

A prior is never a zone reference: the HR zone of DESIGN §18.3 reads only a non-prior `lthr` or `hr_max`, so a new
rider sees no HR zone rather than a zone that can be 10 bpm off. ADR-0004 §2.6 already refuses to stop a test on a prior
HR max for the same reason. Onboarding's self-reported numbers (ONBOARDING §1 item 3) are `self_report` rows with the
sd of their stated origin (ramp or 20-min test 8 %, app estimate 10 %, guess 20 %; convention, the 8 % is ADR-0004's
floor).

**No time decay of the value in v1.** An estimate keeps its value and sd; its age is shown, and after 6 weeks it is
drawn stale (DESIGN §5.6). ONBOARDING's half-life is a property of the CP fit window (§2.2.4 uses 90 days), not a
silent change of the stored value.

#### 2.1.3 Settings → Athlete model (#113, DESIGN §5.9)

One row per parameter: value ± sd, source and method in words ("ramp test, 3 Mar", "typed in", "not measured yet"),
age, a stale mark, and **Edit**. Edit opens the row's draft (the settings-draft primitive #90): value, optional ±
(default by param: the self-report sd of §2.1.2 "guess" origin unless the rider picks one), Save, Cancel. **History**
per row lists every value with its source and date and a **Use this value** (revert). Open proposals (§2.2.4) show at
the top of the section with Accept and Dismiss.

Core commands (one per facade method, `ApiError` only):

| Command | Does | Refuses |
|---|---|---|
| `athlete_model()` | the current estimate per param, the prior for unset ones, open proposals | — |
| `athlete_set(param, value, sd?)` | appends a `self_report` row | `InvalidInput` (bounds, `lthr ≥ hr_max`, a non-finite value) |
| `athlete_history(param)` | every row, newest first | `InvalidInput` (unknown param) |
| `athlete_revert(estimate_id)` | appends a copy | `NotFound`, `InvalidInput` (a prior row: revert to the prior is "Clear", below) |
| `athlete_clear(param)` | appends a row of source `prior` with the current prior (or marks the param unset when there is no prior) | `InvalidInput` |

`athleteHr.ts` is replaced by a read of `athlete_model()`; its signature (`useAthleteHr`) stays, as its header promises.
The DESIGN §5.9 amendment comes with the app issue.

#### 2.1.4 Where the model is read, and what a ride records

- **At `ride_load`** the core resolves an `AthleteSnapshot { ftp, cp, w_prime, pmax, hr_max, lthr, hr_rest, weight }`
  (each `Resolved { value, sd, source, estimate_id }` or unset) from the store, once.
- **Workouts stay in % FTP.** The library and stored workouts are compiled at load with the snapshot's FTP: the
  `Workout.ftp` the core passes to `compile_workout` comes from the snapshot, never from `FTP_W` (removed).
  `workout.ftp_at_creation` stays informative (the editor's preview at creation time).
- **Zones.** Power zones (Coggan 7, `compile.rs`) use the snapshot FTP; HR zones use the snapshot's non-prior LTHR,
  else HR max, through the domain tables of #645. The live ride reads its zones from the loaded ride (the snapshot),
  not from Settings, so the live label and the stored ride agree.
- **The ride stores the snapshot** (`ride.athlete_json`, §2.5) next to `workout_json` and `compiled_json`. Every
  per-ride analysis reads it; a later change to the model never changes a stored ride's IF, TSS or zone times.
- **Workout editor previews** (planned TSS, watts) use the current model, labelled with the FTP they assume.
- **#247** becomes the core half of this: it resolves CP, W′ and τ from the same snapshot onto `RideSetup.athlete`.
- Rides recorded before the snapshot existed have `athlete_json` NULL: IF/TSS fall back to the FTP inside their
  `workout_json` (`Workout.ftp`, which is what they were ridden at); HR zone times are not computed for them.

### 2.2 Part 2: the post-ride analysis executor

#### 2.2.1 Shape

- **`crank-domain::analysis`** (new, pure, no I/O): `analyse(input: &RideInput) -> RideAnalysis`, where `RideInput`
  holds the decoded per-second record, the compiled blocks, the snapshot, the ride kind and the start time. Every metric
  of §2.3 is a function here, with its validity rule. `ANALYSIS_VERSION: u32` is the version of the whole output.
- **`crank-core::analysis`** (new): the executor. One worker task, fed by an `mpsc` of ride ids, runs each analysis on
  a blocking thread (`spawn_blocking`), reads through the store's reader pool and writes through its single writer
  (ADR-0001 §6). Only the process that holds `ride.lock` runs it (the desktop); a lock-less process (CLI, MCP) never
  analyses and reads what is stored (ADR-0001 §9.4).
- **Triggers**, in priority order:
  1. **ride end**: after `finish_ride` (or the recovery that closes a crashed ride as `aborted`) commits, the ride id
     is queued first;
  2. **on demand**: `analysis_rerun(ride_id)` (a Review action, "Recompute"; also the CLI later);
  3. **backfill**: at startup, every ride with no `ride_analysis` row or one whose `version < ANALYSIS_VERSION`,
     newest first. Backfill pauses while a ride is loaded and resumes after it ends; it never delays item 1.
- **Aborted rides are analysed** (their data are intact, ADR-0001 §6) and carry the outcome; their metrics are shown
  with "ended early", and they may raise proposals (a peak HR is still a peak HR).

#### 2.2.2 Idempotence and versioning

- **Per-ride analysis** (`RideAnalysis` and its narrow tables, §2.2.3) is a pure function of (record, compiled
  blocks, snapshot, kind, record version, `ANALYSIS_VERSION`). Recomputing writes the same rows. One ride's derived
  rows are replaced in **one transaction**, so a reader never sees half an analysis.
- **Proposals are not a function of one ride**: a CP fit spans 90 days of rides, and "newer than the current
  estimate" depends on the model. A proposal therefore **freezes its inputs** in `evidence_json` when it is raised,
  and is accepted against that frozen evidence, never against today's window (§2.2.4 "Evidence and acceptance").
- `ANALYSIS_VERSION` is bumped by any change that changes an output (a formula, a validity rule, a bin width). A
  golden test pins it: a committed fixture ride (`crates/crank-domain/tests/fixtures/analysis/`) and its expected
  `RideAnalysis` JSON; changing the output without bumping the version fails the test, and the bump regenerates the
  fixture (the same pattern as the `following-model` fixture, AGENTS.md).
- A version bump re-runs the backfill; per-ride IF/TSS do not change with the model (§2.1.4), so a model change
  triggers nothing. **Set-level** results (the 90-day CP fit, derived baselines, trends) are not stored per ride; they
  are computed on read from the per-ride tables (§2.4.4) or recomputed after each analysis (baselines, §2.2.5).

#### 2.2.3 Storage

Derived tables (migration `0006`, §2.5), all keyed by `ride_id` and removable without losing anything the record holds:

| Table | Holds | Why a table |
|---|---|---|
| `ride_analysis` | `ride_id`, `version`, `status` (`ok` / `failed`), `error`, `computed_at`, `metrics_json` (the full `RideAnalysis`) | one row per ride; what Review reads |
| `ride_metric` | `(ride_id, metric) → value`, scalar metrics only (§2.3) | SUM/AVG/trend over a set in SQL |
| `ride_mmp` | `(ride_id, duration_s) → watts, start_s, self_paced` | MAX per duration over a set; the CP fit |
| `ride_zone_time` | `(ride_id, scheme, zone) → seconds`; schemes `power_coggan7`, `hr_friel7`, `hr_max5` | zone time over a set |
| `ride_hist` | `(ride_id, metric, bin_lo) → seconds`; fixed bins: power 10 W, HR 5 bpm, cadence 5 rpm | distributions over a set add up only with fixed bins |
| `lap` (exists) | per-interval stats, written by the executor (§2.3.2) | comparison per interval |

`metrics_json` and the narrow tables are written from the same `RideAnalysis` in the same transaction; the narrow
tables are projections, never a second computation.

#### 2.2.4 Proposals

A proposal is a value the analysis derived for one of the eight parameters; nothing changes until the rider accepts it.

| # | Param, source (`method`) | Fires when | Proposed value, sd | Class |
|---|---|---|---|---|
| P1 | `hr_max`, test (`ramp_peak_hr_v1`) | an assessment ride that ended `Exhausted` (ADR-0004 §2.4) with a valid result, and its HR gap-free over the last 3 min of the ramp with no step > 15 bpm between seconds | the peak 5 s mean HR, rounded; sd 3 bpm. Proposed when above the current value, **or** when the current value is the prior (a verified maximal effort beats an age formula even when lower) | an observed peak is a lower bound of HR max (definition); a ramp to exhaustion usually ends within a few bpm of it, so sd 3 bpm and the replace-the-prior rule are convention |
| P2 | `hr_max`, rides (`peak_hr_v1`) | any ride's peak 5 s mean HR exceeds the current `hr_max` (prior or not), the HR clean around the peak (no step > 15 bpm between seconds in the 30 s before it, ≥ 90 % HR coverage in that window) | the peak, rounded; sd 2 bpm | lower bound (definition); the 5 s mean and the step guard are convention against strap artefacts |
| P3 | `lthr`, test or rides (`lthr_friel30_v1`) | a 30-min self-paced continuous effort: an LTHR test ride (with the self-paced test ADR) or a ride segment of ≥ 30 min, all self-paced, VI ≤ 1.05, mean power ≥ 95 % of the snapshot FTP, ≥ 95 % HR coverage | mean HR of the last 20 min; sd 4 bpm | convention: Friel's field protocol [Fr]; no validation against a lactate threshold is cited |
| P4 | `lthr`, rides (`lthr_erg_steady_v1`) | an ERG block at 95–105 % of the snapshot FTP lasting ≥ 12 min, decoupling valid on it (§2.3.1) and below 5 %, ≥ 95 % HR coverage, HR on-kinetics excluded (first 5 min) | mean HR of the block after its first 5 min; sd 5 bpm (wider: it inherits FTP's error) | convention, weaker than P3: HR at threshold power holds only as far as FTP does (03: FTP ≠ threshold); the 5 % is a validity gate here, not a verdict |
| P5 | `cp` + `w_prime`, test (`cp_assessment_v1`) | a valid CP assessment (its own ADR, ADR-0004 "next") | the fit of its efforts, as below | evidence [L22, leo2021] |
| P6 | `cp` + `w_prime`, rides (`cp_ols_v1`) | a self-paced MMP point of this ride between 2 and 15 min exceeds the current CP model's prediction `CP + W′/t` by more than 2 sd of the prediction | CP and W′ from the 90-day fit below, each with its standard error | evidence for the model and the effort range [L22, leo2021]; the 2 sd trigger is convention |
| P7 | `ftp`, test (`ramp_v1_0.75`) | an assessment ride with a valid ramp result (ADR-0004 §2.5) | ADR-0004's value, sd 8 % (the owner's convention floor) | as ADR-0004 |
| P8 | `ftp`, rides (`ftp_from_cp_v1`) | a current or accepted CP estimate from P5 or P6 with n ≥ 3 efforts and relative se ≤ 5 % exceeds the current FTP by > 5 % | the CP, sd = the fit's se combined with the 13 W spread of [K21] | convention: CP ≈ FTP on the mean with wide LoA [K21]; only upward (owner decision 8) |

**The CP fit** (P5, P6): P6 takes, over the last 90 days of `ride_mmp` rows flagged `self_paced` (§2.3.2), the best
point per duration for durations 2–15 min; P5 takes the assessment's efforts. Both require ≥ 3 points spanning at least
2–10 min [L22] and fit `P = CP + W′ · (1/t)` by ordinary least squares (closed form; linear in CP and W′, so no
optimiser and no library is needed; standard errors from the residual variance). Two-point fits are refused (they
always fit exactly and give no error, [leo2021]). The OmPD fit with Pmax (#18) needs a nonlinear optimiser; its issue
does the library search the owner rule requires.

**Evidence and acceptance.** When a proposal is raised, `evidence_json` freezes everything its value was computed
from: the `ANALYSIS_VERSION`; the ride ids and, per ride, the inputs used (P1–P4, P7: the ride's seconds range and
its record version; P5, P6: each fit point as `(ride_id, duration_s, watts, start_s)` and the window's dates; P8: the
CP estimate id it read); and the value and sd it produced. `athlete_proposal_accept`:

1. recomputes the value from the frozen evidence with the domain function of its method (a pure function of the
   evidence, so the same inputs give the same value);
2. checks the evidence still holds: the analysis version equals the frozen one, and every frozen ride still exists
   with the same `ride_analysis.version` (P5/P6: with the same `ride_mmp` watts at each frozen point);
3. writes the estimate (method, the triggering ride id, n) and marks the proposal accepted.

A failed check means the evidence is **stale** (a version bump re-analysed the rides, or a ride was deleted): accept
refuses with `Conflict`, the proposal becomes `stale`, and the executor's re-analysis raises a fresh one if its rule
still fires. New rides, and old points ageing out of the 90-day window, do **not** make a proposal stale: it states
what it is based on ("CP 274 ± 6 W from 4 efforts, 12 Aug–3 Nov"), and the rider decides. A proposal older than 30
days says so beside its Accept button (convention), and a newer proposal for the same parameter supersedes it.

**Rules for every proposal:**

- **Never silent.** The only writers of `athlete_estimate` are `athlete_set`/`athlete_revert`/`athlete_clear`
  (self_report or prior), onboarding (prior, self_report), an intervals.icu import (ONBOARDING §2, `imported`), and
  `athlete_proposal_accept`. The store API takes an `EstimateOrigin` enum that only those paths construct; a test
  analyses a ride that triggers every proposal kind and asserts the current model is unchanged.
- **Manual entry wins.** `athlete_set`, `athlete_revert` and `athlete_clear` mark every open proposal for that
  parameter `superseded`, in the same transaction as the new row; accept also refuses (`Conflict`) a proposal raised
  before the parameter's current `updated_at`.
- **Newer only.** A proposal is raised only if the ride started after the parameter's current `updated_at`, or the
  current value is the prior. Backfilling two years of rides does not bury the rider in old peaks.
- **One open proposal per param.** A new one for the same param supersedes the open one (`superseded`).
- **Never raised twice.** Proposals are keyed `(ride_id, param, method, analysis_version)`. The executor raises none
  for a `(ride_id, param, method)` that already has an `accepted`, `dismissed` or `superseded` row at any version: a
  superseded proposal lost to a newer value or to a manual entry, and re-raising it would undo that (intended). Only a
  `stale` row lets a re-analysis at a new version raise the proposal again.
- **Shown** on that ride's Review ("Peak HR 194 bpm is above your HR max 188 bpm. Use 194 as your HR max?") and in
  Settings → Athlete model, with Accept and Dismiss; Today may show a one-line card (DESIGN amendment with the app issue).

**ADR-0004 amendment** (it is still Proposed): the ramp result is produced by this executor as P7 (and its peak HR as P1), and
`athlete_confirm_ftp { ride_id }` becomes `athlete_proposal_accept { proposal_id }` with the same guarantees.

#### 2.2.5 Derived baselines (automatic)

Values that set no prescription but feed the coach and the comparisons are recomputed after each analysis, versioned
with `ANALYSIS_VERSION`, shown read-only in Settings → Athlete model ("learned from 14 rides"), and never written to
`athlete_estimate`:

- **HR at power** (`athlete_baseline` rows `hr_at_power`, ONBOARDING `hrAtPower`): over 90 days, mean ± sd HR per 10 W
  band from steady ERG seconds after the first 10 min of a block (09: HR on-kinetics), n per band. Feeds the coach's
  `hr_vs_typical` (ADR-0001) and Review's "your usual HR for that power" (DESIGN §5.5 item 2).
- **Usual HR-recovery τ** (`usual_tau`, #247): median and spread of the last 20 recovery fits (`recovery`).
- **Typical distributions** (`typical_distribution` in `distributions.rs`): the 90-day power and HR histograms.

The rule for this split (owner decision 6): a value the rider would want to veto, because it rescales targets or zones,
is a proposal; a value that only describes the rider's recent rides is a baseline.

#### 2.2.6 Commands

| Command | Does | Refuses |
|---|---|---|
| `ride_analysis(ride_id)` | the stored `RideAnalysis` with its version and status; `pending` while queued | `NotFound` |
| `analysis_rerun(ride_id)` | queues it first | `NotFound`, `Conflict` (the ride is open) |
| `athlete_proposals()` | open proposals | — |
| `athlete_proposal_accept(proposal_id)` | **recomputes** the proposal from the stored ride at the current version; writes the estimate with method, ride id and n; marks it accepted | `NotFound`; `Conflict` if superseded, decided, or the recomputed value differs (stale proposal) |
| `athlete_proposal_dismiss(proposal_id)` | marks it dismissed | `NotFound`, `Conflict` (decided) |

#### 2.2.7 Failure handling

- **`failed` is for the ride, not a metric**: only a record that cannot be read (a chunk that does not decode, a
  store error) or a panic fails the ride. It gets `status = failed` and the error text; it is logged at WARN with the ride id and the error kind; the worker moves on. It is retried only by
  a version bump or `analysis_rerun`, never in a loop. Review shows "Analysis failed: …" with Recompute.
- **One metric's trouble never fails the ride.** A metric that is not valid for a ride (no HR, too short, not steady,
  below its coverage minimum of §2.3.1.1) or whose domain function returns an error (`MetricError`, `CompileError`) is
  stored as `invalid { reason }` for **that metric** (an error is also logged at WARN with the metric name), and every
  other metric is analysed. Never 0 or a guess; Review lists only valid metrics (DESIGN §5.5 item 5) and says why the others are missing in *Show as
  table*.
- No `unwrap`; a panic in an analysis is a bug that the blocking task contains (the `JoinError` becomes `failed`).
- Crash during an analysis: the transaction never committed, so the ride has its old rows or none; the next start's
  backfill picks it up.

### 2.3 Part 3: ride analysis on Review and History

#### 2.3.1 Metric set

##### 2.3.1.1 Seconds, pauses and gaps

The record has one row per **session** second, paused seconds included (`record.rs` `SecondRow`); a value is NaN when
no sample arrived (a dropout, a strap that lost contact, a #610-style speed gap).

- **Riding seconds** are seconds with a riding tick (`workout_s` finite). A paused second has none and is excluded from
  every metric except *duration*.
- **Per-metric valid seconds**: a riding second with a finite value of the metric's own channel (power, HR, cadence,
  speed). A gap in one channel never removes a second from another: an HR loss leaves every power metric intact.
- **No bridging.** A rolling window (NP's 30 s, MMP windows, the 5 s peak HR) never spans a gap or a pause. NP is
  computed per run of consecutive valid power seconds: each run of ≥ 30 s contributes its 30 s windows to one common
  fourth-power mean, `NP = (Σ_runs Σ_windows mean⁴ / total windows)^¼`, which calls the existing `normalized_power` on
  each run (it rejects NaN, `compile.rs` `normalized_power`) and weights each run's fourth-power mean by its window
  count; shorter runs add to average power and work, not to NP. Today's `recorder.rs` `summarize` concatenates the
  power seconds across gaps; issue 6 (§4.1) moves it onto this rule, so the stored summary and the analysis agree.
- **Coverage minimum** (convention): a power metric needs valid power on ≥ 50 % of riding seconds and ≥ 60 s; an HR
  metric ≥ 60 % (decoupling and the LTHR proposals state their own, higher, minimum); a cadence metric ≥ 50 %.
  Below it the metric is `invalid { reason: "power for 0:41 of 1:00:00" }`. Every valid metric carries its coverage, and
  Review shows it when below 100 % (DESIGN §5.5 item 1 already lists gaps per sensor).
- **W′bal across a gap**: the balance is held unchanged through gap seconds (no expenditure, no recovery; convention),
  and the minimum says how many gaps it spans.
- **Speed and distance** (route rides, ADR-0005): distance sums finite speed seconds only, never interpolates, and
  states "excludes 0:12 without speed".
- **Duration** is all recorded seconds (paused included); **riding time** excludes paused seconds; **moving time** is
  riding seconds with power > 0 or cadence > 0.

##### 2.3.1.2 Metrics

"Evidence" means a peer-reviewed study supports the metric's use as stated; "convention" means a coaching or platform
rule. Every metric excludes NaN seconds unless stated; `estimated` seconds count (ADR-0001 §6). All are per ride; set
aggregates are §2.4.4.

| Metric | Formula | Valid when | Class, source |
|---|---|---|---|
| Duration, riding time, moving time | §2.3.1.1 | always | definition |
| Work | Σ P · 1 s / 1000, kJ (exists) | any power | definition |
| kJ/kg | work / snapshot weight | weight set | definition; the durability axis (01) |
| Avg power, NP | mean; NP = (mean of 30 s rolling mean⁴)^¼ (`normalized_power`, exists) | ≥ 30 s power | convention [AC] |
| IF | NP / snapshot FTP | FTP resolved | convention [AC]; FTP ≠ threshold (03) |
| TSS | s × NP × IF / (FTP × 3600) × 100, s = valid power seconds (paused and gap seconds excluded, so a dropout never inflates it) | FTP resolved | convention [AC]; behaves differently at high vs low IF [vanerp2019] (#570 lands this) |
| VI | NP / avg power | ≥ 30 s power | convention [AC] |
| MMP curve | best gap-free mean per duration: 1, 2, 3, 5, 10, 15, 20, 30 s; 1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 30, 40, 50, 60, 75, 90, 120, 150, 180, 240 min (up to the ride's length); each point flagged `self_paced` by the rule of §2.3.2 | the window fits (gap-free, no pause) | definition; its use for CP/W′ is evidence [L22] |
| W′bal min, kJ above CP | `wbal_series` with snapshot CP, W′; Σ max(0, P − CP) | CP, W′ resolved | Skiba model [skiba2012]: evidence B (02); kJ above CP: 01 |
| Power time in zone | Coggan 7 at snapshot FTP (the `compile.rs` bounds), beside the planned `time_in_zone_s` | FTP | convention [AC] |
| HR avg, peak | mean; max 5 s mean | ≥ 60 % HR coverage | definition |
| HR time in zone | Friel 7 (% LTHR) else 5 zones (% HR max), domain tables of #645 | a non-prior reference | convention (DESIGN §18.3 sources) |
| Histograms | power 10 W, HR 5 bpm, cadence 5 rpm (`histogram_from`) | data present | definition |
| Efficiency factor (EF) | NP / avg HR over the steady part | steady part (below) | convention [Fr]; useful only as a trend at matched workouts |
| Aerobic decoupling (Pw:HR) | (EF₁ − EF₂) / EF₁ × 100 over the two halves of the steady part | steady part ≥ 20 min after the first 10 min, VI ≤ 1.05 on it, ≥ 90 % HR coverage | drift as a durability marker: evidence B/C [rothschild2025, stevenson2022] (09); cut-offs: none, **no verdict shown** (owner decision 13) |
| Cadence avg, distribution, drift | mean over pedalling seconds; histogram; 2nd vs 1st half of steady blocks, rpm | cadence present | 08: drift tracks fatigue, evidence C [barsumyan2026] |
| On target (compliance) | share of counted ERG seconds within ±5 % of the controlled target, settling excluded, per block and ride | recorded target (§2.3.2) | the one definition, DESIGN §5.5 / ADR-0002 §7.8 |
| Durability | MMP 1, 5, 20 min after 10/20/30 kJ/kg, as % of the ride's own fresh MMP | weight set, the ride crosses the checkpoint and has ≥ the duration after it; shown for self-paced points only | field durability repeatable: evidence B [mateomarch2025reliability]; checkpoints from 01 |
| HR recoveries | τ ± fit uncertainty against usual τ (exists, DESIGN §5.5 item 4) | recovery blocks | as DESIGN |
| Ride category | the **highest** zone (Coggan 7, at snapshot FTP) holding ≥ 25 % of the work-phase seconds (all riding seconds when the ride has no work phase: free and route rides), mapped to recovery (Z1), endurance (Z2), tempo (Z3), threshold (Z4), VO2 (Z5), anaerobic (Z6–7); `test` for an assessment. Free and route rides get a category too; their kind is a separate filter (§2.4.3) | valid power | convention (this ADR). "Highest zone over a share" rather than "most seconds", so a 5 × 3 min VO2 session is VO2 even with long recoveries, and an over-under set is not filed under its lower zone. Known weakness: a short hard block inside a long ride (under 25 % of it) leaves the ride "endurance", and the 25 % is arbitrary; the category is a filter aid, never a load measure |

**Trends** (set level, §2.4.4): weekly hours, kJ, TSS; time in 3 zones (06; the polarization index is evidence C
[treff2019] and is shown with its formula); CTL/ATL/TSB as exponentially weighted daily TSS with 42- and 7-day
constants: `CTL_d = CTL_{d−1} + (TSS_d − CTL_{d−1}) · (1 − e^(−1/42))` (ATL the same with 7), TSB_d = CTL_{d−1} −
ATL_{d−1}. The exact decay `1 − e^(−1/τ)` is used rather than the common `1/τ` approximation (they differ by ~1 % at
τ = 42: 0.0235 vs 0.0238), so other tools' numbers will not match to the unit. Both start at 0 on the day of the first
ride (or the first imported day) and the first 42 days are drawn "building"; a day's TSS is the sum of its rides'
TSS; a ride without TSS (no FTP) adds 0 and the chart says "n rides without TSS". Labelled "descriptive trend (model with generic constants)" (13: fitness–fatigue models fit history but
predict an individual poorly [busso2023]; generic constants are not appropriate for individuals [vermeire2022]).

**Not in this ADR**: DFA-α1 on Review (the domain has it; it needs the RR record and the guard rules of 04), OmPD,
Pmax fit, AEPF/CPV/torque, stamina and fuel (#18), HR reserve zones.

#### 2.3.2 Recording what compliance and comparison need

The record gains two per-second metrics in `record.rs` `METRICS` (no schema change: `ride_chunk.metric` is text):

- `target_w`: the controlled target at the last riding tick of the second (after easing, ± and coach changes, the
  target the rider was given); NaN outside ERG.
- `compliance`: the second's class, the same classification the live `StreamCompliance` counts: 0 excluded (settle,
  released, paused, suspect, not following), 1 counted and outside ±5 %, 2 counted and within.

The executor's per-interval stats come from one pure domain function over the record, segmented by the recorded
`block` and the workout's node path and iteration, and are written to `lap`. The engine's live lap stats (#12) must
equal that function on the same seconds: a test feeds the engine's per-second rows to the domain function and compares
it with the engine's lap output (one definition, as the following-model fixture ties the engine to its model). Rides
recorded before this change have no `target_w`: their on-target is `invalid { reason: "not recorded" }`.

**Record version and `self_paced`.** A NaN `target_w` must not be read as "outside ERG" on a ride that never recorded
the target. So `ride.record_version` (migration 0005: `INTEGER NOT NULL DEFAULT 1`, every existing ride is 1) says
which metrics the record holds; the recorder writes **2** from the change that adds `target_w` and `compliance`, and
any later record change bumps it again. A second is self-paced when:

- **record version ≥ 2**: it is a riding second and its `target_w` is NaN (no ERG target: free ride, resistance, SIM,
  a self-paced test, the ADR-0004 cool-down);
- **record version 1**: only when the ride's `kind` is `free` or `route` (no ERG anywhere in the ride). Every other
  version-1 ride, ERG workouts and their free blocks included, has **no** self-paced seconds, because the record
  cannot tell its blocks apart. Its MMP is stored and shown, flagged `self_paced = false`, and never feeds a fit,
  a durability figure or a proposal.

A version-1 ride never becomes version 2 by re-analysis; the record is what it is.

#### 2.3.3 Review and History

- **Review §5.5 item 5 "All numbers"** and **item 6 "Analytics"** (#108) read `ride_analysis(ride_id)`: the table of
  §2.3.1 (valid metrics only, each with its unit and, on hover/focus, its definition), time at power, the PD curve
  with the 90-day envelope and the CP model drawn (± band), HR time in zone, cadence distribution, durability when
  valid. Every chart has *Show as table* and keyboard inspection (#71).
- **Stored traces**: Review of any past ride draws its traces from the record through `ride_series` (§2.4.2), so the
  "a ride opened from the list shows its figures only" limit of DESIGN §5.5 ends.
- **Proposals** for the ride sit under the intent verdict (§2.2.4).
- **History §5.6**: the athlete model card reads `athlete_model()`; the CP trend reads accepted `cp` estimates; the PD
  curve reads `ride_mmp` (latest ride / 90 days / all time). The rides table gains TSS and category columns.
- FIT export (#257) reads the record, not the analysis; unchanged.

### 2.4 Part 4: comparison and statistics over a set of rides

#### 2.4.1 Two rides, and one workout across dates

- **Same workout across dates**: rides whose snapshot `workout_json` has the same workout id. Intervals align by
  (node id, iteration) when both rides have it; when node ids differ (an edited workout) and the compiled block lists
  have the same shape (same count, phases and durations), by block index; otherwise only whole-ride figures compare,
  and the view says why.
- **Per-interval table** (up to the 10 latest rides of the workout as columns, newest first): avg power, on target,
  avg HR, HR per watt, cadence, decoupling where valid, RPE (#106) when rated; each cell with its delta to the previous
  ride of the workout.
- **Overlay of two rides** (A and B): power and HR panels on one x axis of **workout time** (`workout_s`, so a pause
  does not shift the blocks), A solid, B dashed, each with its word; gaps never bridged (DESIGN §18.7 rule 17).
- **Any two rides**: MMP curves overlaid, zone distributions side by side (power and, when both have it, HR),
  the scalar table with deltas.

#### 2.4.2 Series for overlays

`ride_series(ride_id, metrics, axis: session | workout, from_s, to_s, max_points)` returns per bucket the min, max
and mean of each metric (≤ 2 000 buckets; one per second when the span is shorter), computed in the core from the
chunks. It is the same downsampling rule as #599's live trace (min/max per pixel column), so a stored ride and the
live ride draw the same vertices; #599's acceptance test (vertices equal the full series at one per second) is reused.

#### 2.4.3 Choosing the set

```rust
pub struct RideFilter {
    pub from: Option<LocalDate>,          // inclusive, the rider's local calendar
    pub to: Option<LocalDate>,
    pub workout_ids: Vec<String>,         // empty = any
    pub categories: Vec<RideCategory>,    // empty = any (§2.3.1)
    pub kinds: Vec<RideKind>,             // workout | free | assessment | route
    pub include_aborted: bool,            // default false
    pub ride_ids: Option<Vec<String>>,    // an explicit selection; the other fields are ignored when set
}
```

**Tags.** Workouts and rides have no tags today. v1 filters by the derived category (owner decision 14); rider-defined
tags are their own issue (filed on acceptance), and when they land they become one more `RideFilter` field.

**In the UI** (History, DESIGN amendment with the app issue):

- a **filter bar** above the rides table: date range presets (7 days, 28 days, 90 days, 365 days, all, custom),
  a workout picker, category chips, kind chips, "include ended early"; 32 px controls (§18.4). The filter stays while
  History is the view, as "Last opened" does (DESIGN §5.6);
- a **tick column** in the rides table; ticking rides switches the stats panel to "Selected (n)" and back with
  "Clear selection";
- **Compare** is enabled with exactly two ticked rides and opens the Compare view (`history/compare?a=…&b=…`), with
  the way back to History;
- **Review → "Other rides of this workout"** opens History filtered to that workout, and **"Compare with the previous
  ride"** opens Compare with the two.

#### 2.4.4 Statistics over the set

`rides_stats(filter) -> SetStats`, computed in the core by SQL over the derived tables of §2.2.3 (never by decoding
chunks):

- **Totals**: rides, duration, moving time, work, TSS (rides without FTP are counted and named "n without TSS").
- **Zone time**: the sum per scheme and zone. Each ride's zone time used its own snapshot, so the sum is "time in your
  zones as they were then", and the panel says so.
- **Distributions**: the sum of `ride_hist` per bin (fixed bins make this exact).
- **MMP envelope**: max per duration, with the ride and date of each best; the CP model fitted to the set's
  self-paced points when the §2.2.4 conditions hold (shown, never proposed: proposals come from rides).
- **Weekly series**: hours, kJ, TSS, zone time per ISO week.
- **Trend of a chosen metric**: one point per ride of any `ride_metric` (EF, decoupling, NP, avg HR, on target, …),
  with a 28-day rolling median line.
- **Load trend** (CTL/ATL/TSB) is always computed over **all** rides, because a filtered load is meaningless; the
  filter only marks which rides are in the set.

Performance: per ride, `ride_mmp` ~35 rows, `ride_zone_time` ≤ 19, `ride_hist` ≤ ~120, `ride_metric` ~30. Five years
of daily rides is ~2 000 rides and ~400 000 narrow rows; the aggregates are indexed `GROUP BY` queries. A 6 h ride's
analysis decodes 21 600 seconds × 8 metrics (~0.7 MB of `f32`) once; MMP over 35 durations uses prefix sums, O(n · d).

### 2.5 Data model and migrations

Two migrations, each with the part that needs it:

**`0005_athlete_model.sql`** (Part 1):

- Rebuild `athlete_estimate` (no row has ever been written by code, §1.1) with `id INTEGER PRIMARY KEY` (a rowid alias
  survives `VACUUM`), `method TEXT NOT NULL`, `ride_id TEXT REFERENCES ride(id) ON DELETE SET NULL`, `param` checked
  against the eight names, `source` against the five; copy any existing rows (method `legacy`); keep the
  `(param, updated_at DESC)` index.
- Copy a non-NULL `athlete_profile.weight` into `athlete_estimate` (`weight`, `self_report`, method `profile`) and drop
  the column (`ALTER TABLE … DROP COLUMN`, SQLite ≥ 3.35, guaranteed by the bundled `rusqlite`).
- `ALTER TABLE ride ADD COLUMN athlete_json TEXT` (the snapshot; NULL for older rides) and
  `ADD COLUMN kind TEXT CHECK (kind IN ('workout','free','assessment','route'))`, back-filled from what each row
  holds (`route_json` → route; the free-ride workout id `free-ride`, as `core.rs` maps it to a `null` `workout_id` → free;
  else workout). ADR-0004's `RideKind` writes it.
- `ALTER TABLE ride ADD COLUMN record_version INTEGER NOT NULL DEFAULT 1` (§2.3.2): every existing ride is
  version 1; the recorder keeps writing 1 until issue 5 makes it 2.
- `athlete_proposal (id INTEGER PRIMARY KEY, ride_id NOT NULL REFERENCES ride(id), param, value, sd, method,
  evidence_json NOT NULL, analysis_version INTEGER NOT NULL,
  status CHECK (status IN ('open','accepted','dismissed','superseded','stale')), created_at, decided_at,
  estimate_id REFERENCES athlete_estimate(id), UNIQUE (ride_id, param, method, analysis_version))` (the raise rules
  of §2.2.4 "Never raised twice" sit in the core, not in a constraint).

**`0006_ride_analysis.sql`** (Part 2): `ride_analysis`, `ride_metric`, `ride_mmp`, `ride_zone_time`, `ride_hist`,
`athlete_baseline (kind, key, value_json, version, computed_at, PRIMARY KEY (kind, key))`, each with
`REFERENCES ride(id)` where keyed by ride, and indexes for the set queries (`ride_metric(metric)`,
`ride_mmp(duration_s)`).

Derived tables are rebuildable from `ride` + `ride_chunk`; a store check (#166's schema check) may drop and rebuild
them, never the record or `athlete_estimate`.

### 2.6 Privacy

Everything is computed and stored on the machine. No new network access. The analysis tables are part of `crank.db`
and are exported only where rides already leave by the rider's choice (FIT export #257, intervals.icu). Logs carry ride
ids and error kinds, never metric values. What a workout-generation provider may receive is #699's decision; §2.7 lists
what this ADR makes available, not what is sent.

### 2.7 What generation (#699) and the coach (ADR-0001) consume

- **`training_context(now) -> TrainingContext`** (core, read-only, one DTO): the current snapshot (with sd and age),
  the last 7/28/42-day hours, kJ, TSS and time in 3 zones, CTL/ATL/TSB (labelled descriptive), days since the last
  ride per category, the last 10 rides' RPE, completion (outcome) and on target. #699's built-in rules and every
  provider read only this; a user endpoint gets whatever subset #699's ADR allows.
- **The coach (in-ride judge)** reads the loaded ride's snapshot (CP, W′, HR max, LTHR, #247) and the baselines of
  §2.2.5 (HR at power for `hr_vs_typical`, usual τ). It never reads a proposal.
- **Uncertainty reaches both**: a high FTP sd moves targets to the conservative side (ONBOARDING §4); the context
  carries the sd, and #699 decides how its generator uses it.

### 2.8 Tests

- **Domain (pure)**: each §2.3.1 metric against hand-made records (constant power, a step, gaps at window edges,
  `estimated` seconds); MMP equals a brute-force O(n²) reference on random records (property test); NP and the
  compiled TSS agree on a ride that rides the plan exactly; the CP OLS fit recovers CP and W′ from points generated
  by the model with noise, refuses two points and refuses a span below 2–10 min; each validity rule on its own;
  priors and bounds; the golden fixture pins `ANALYSIS_VERSION` (§2.2.2).
- **Gaps and pauses (§2.3.1.1)**, each a domain test: a 10 s power dropout in the middle of a 30 s NP window (NP equals
  the run-wise reference, never the concatenated trace, and the ride is not `failed`); a 20 s run between two gaps
  (counts for average power and work, not NP); power below the coverage minimum (power metrics `invalid`, HR metrics
  still valid); an HR loss for half the ride (HR metrics `invalid` or valid by coverage, every power metric unchanged);
  #610-style speed gaps (distance excludes them, nothing interpolated, nothing else affected); a ride with no power at
  all; paused seconds (in duration only; TSS's s and every window exclude them); W′bal held across a gap; a metric
  whose domain function returns an error is `invalid` while the rest is stored (`ok`).
- **Self-paced rule (§2.3.2)**: a version-1 ERG ride flags no MMP point self-paced; a version-1 free ride flags all;
  a version-2 ride flags exactly its NaN-target riding seconds.
- **Engine/domain tie**: the live lap stats equal the domain interval function on the same seconds (§2.3.2).
- **Store**: migrations 0005 and 0006 on a v4 database with rows (weight copied, kinds back-filled, estimates kept);
  the derived rows of one ride are replaced atomically; `EstimateOrigin` is the only way to append an estimate.
- **Core**: the executor runs on ride end, on rerun, and on backfill (version bump re-runs; a failed ride is not
  retried in a loop; backfill pauses while a ride is loaded); proposals P1–P8 fire and do not fire at their edges, never
  write an estimate, respect "newer only" and supersede; a P6 proposal is still acceptable after new rides arrive and
  after its oldest point leaves the 90-day window (accept uses the frozen evidence), and becomes `stale` (`Conflict`)
  after a version bump or a deleted ride, after which re-analysis raises a fresh one; `athlete_set` supersedes the
  parameter's open proposals; a superseded, accepted or dismissed proposal is not raised again; accept refuses stale,
  superseded and decided proposals; the snapshot is frozen at load (a Settings change mid-ride does not reach the ride); IF/TSS use the
  snapshot; a model change re-analyses nothing.
- **App**: each mock backend models every refusal of the commands it calls (repo convention, with the core
  `file.rs symbol`); Settings → Athlete model edit, revert, clear, proposals; History filter, selection, Compare;
  design-harness fixtures for each new screen state at both viewports and themes.

## 3. Alternatives considered

- **Write analysis results straight into the model** (the "Bayesian-style updating" of ONBOARDING §3 taken literally).
  Rejected: a strap spike or an ERG-limited curve would silently rescale every workout and zone; ADR-0004 already chose
  the confirm for the same reason. Baselines that set nothing do update automatically (§2.2.5).
- **Recompute IF/TSS of old rides with today's FTP.** Rejected: history would change every time FTP changes, and a
  ride's IF would no longer describe what the rider was asked to do. The ride's snapshot is the truth; the stats panel
  can still show today's zones on a set when a later issue asks for it.
- **Compute analysis on the fly on every Review open, store nothing.** Rejected: set statistics over hundreds of rides
  would decode every chunk on each query, and proposals need a stable key to be accepted against.
- **Store analysis only as JSON per ride.** Rejected for sets: SUM/MAX across rides then needs JSON parsing in SQL or
  in Rust over every ride; the narrow tables are projections of the same JSON (§2.2.3).
- **A separate analysis process or crate.** Rejected: it would need the lock rules of a second writer; the core
  already owns the store writer, and the pure part lives in `crank-domain` like every other metric.
- **Fit CP with a nonlinear optimiser now.** Rejected for v1: the linear `1/t` form of the 2-parameter model has a
  closed form and standard errors; the OmPD fit (#18) is where a library earns its place.
- **Include ERG seconds in the CP fit.** Rejected: an ERG effort is the target, not the rider's maximum; the envelope
  at those durations would bias CP low (01 "only update from blocks flagged maximal/self-paced").
- **Rider tags first.** Deferred, not rejected: derived categories cost nothing and need no curation; tags need a
  workout and ride editing surface (filed on acceptance).
- **Show a decoupling verdict (< 5 % "aerobic base OK").** Rejected: grade D, no supporting study (09).
- **Time-decay the stored value** (half-life 6 weeks). Rejected for v1: it changes targets without an event the rider
  can see. Staleness is shown; the fit window carries recency.

## 4. Consequences

- A rider can set FTP, HR references and weight today, and every workout, zone and ride reads them; the 250 W constant
  and the hidden CP/W′ go.
- Every ride carries the model it was ridden with; history is stable under model changes.
- The executor gives Review its full numbers, History its trends and the coach its baselines, from one versioned pure
  function.
- Proposals put the rider in the loop for every change that rescales targets.
- Comparison and set statistics are SQL over small derived tables, cheap at any history length.
- The record grows by two metrics (~8 bytes per second); a 3 h ride grows by ~86 kB.
- Harder: every metric change needs a version bump and a fixture regeneration (intended).
- ADR-0004 needs the small amendment of §2.2.4 before its core issue is implemented.

### 4.1 Implementation issues, in order (listed here, filed on acceptance)

**Part 1: athlete model**

1. domain: `athlete` module: `AthleteParam`, `Estimate`, `Source`, bounds, priors (§2.1.1–§2.1.2), `AthleteSnapshot`
   and resolution; HR-zone tables to the domain (#645) alongside.
2. store: migration 0005 (§2.5) and the estimate/proposal API with `EstimateOrigin`.
3. core: `athlete_model`, `athlete_set`, `athlete_history`, `athlete_revert`, `athlete_clear`; the snapshot at
   `ride_load` and in `ride.athlete_json`; FTP from the snapshot into `compile_workout` (removes `FTP_W`); #247 resolves
   from the same snapshot; bindings.
4. app: Settings → Athlete model (#113) and the read that replaces `athleteHr.ts`; DESIGN §5.9 amendment.

**Part 2: executor**

5. engine + store: record `target_w` and `compliance` per second and write `record_version = 2` (§2.3.2; the
   column comes with migration 0005), with the engine/domain lap tie test.
6. domain: `analysis` module, metrics of §2.3.1 with the gap rules of §2.3.1.1 (run-wise NP; `recorder.rs`
   `summarize` moves onto it), intervals, the self-paced rule, `ANALYSIS_VERSION`, golden fixture.
7. store + core: migration 0006, the executor (triggers, backfill, failure handling), `lap` rows, `ride_analysis`,
   `analysis_rerun`, baselines (§2.2.5).
8. domain + core: proposals P1, P2, P4, P6, P7, P8, the CP fit, frozen evidence and accept against it,
   `athlete_proposals`, accept and dismiss, manual entry superseding; the ADR-0004 amendment applied. P3 (LTHR from a
   30-min effort, the ride-segment form) lands here; its test form and P5 land with the self-paced test ADR.

**Part 3: ride analysis**

9. core: `ride_series` (§2.4.2; shares #599's downsampling).
10. app: Review All numbers and Analytics (#108), stored traces, proposals on Review; History athlete card, CP trend,
    PD curve, TSS and category columns (#109); DESIGN §5.5/§5.6 amendment.

**Part 4: comparison and sets**

11. core: `RideFilter`, `rides_stats`, same-workout interval alignment and the comparison DTOs.
12. app: History filter bar, tick column, stats panel, Compare view, Review links; DESIGN amendment.
13. core: `training_context` for #699 and the planner.

Follow-ups filed on acceptance: rider tags (§2.4.3); OmPD and Pmax fit, with the optimiser library search (into #18).

## 5. Open questions

| Question | Recommendation | Issue | Milestone |
|---|---|---|---|
| The 19 decisions of "For the owner" | as written | filed on acceptance | — |
| Should a prior HR max ever drive HR zones (with a "calibrating" badge) instead of showing none? | no (decision 5) | filed on acceptance | — |
| Should CP/W′ estimates widen their sd with age instead of only being marked stale? | not in v1 | filed on acceptance | — |
| The LTHR test ride (30-min self-paced effort) and the CP assessment: protocol, UI and safety | their own ADR after ADR-0004 (owner: CP assessment next); this ADR only defines where their results land (P3, P5) | filed on acceptance | — |

## 6. References (checked 2026-11-22)

- [AC] Allen H, Coggan A, McGregor S. *Training and Racing with a Power Meter*, 3rd ed., VeloPress, 2019: NP, IF, TSS,
  VI, the 7 power zones (definitions; convention).
- [Fr] Friel J. *The Cyclist's Training Bible*, 5th ed., VeloPress, 2018: efficiency factor and Pw:HR decoupling
  (convention).
- [T01] Tanaka H, Monahan KD, Seals DR (2001). Age-predicted maximal heart rate revisited. *J Am Coll Cardiol*
  37(1):153–156, doi:10.1016/S0735-1097(00)01054-8.
- [K21] Karsten et al. (2021), *Front Physiol* 11:613151 (as ADR-0004).
- [L22] Leo et al. (2022), *Eur J Appl Physiol* 122(2):301–316 (as ADR-0004).
- Knowledge pack (`spec/knowledge-pack/references.bib`): [leo2021], [skiba2012], [vanerp2019], [busso2023],
  [vermeire2022], [treff2019], [rothschild2025], [stevenson2022], [barsumyan2026], [mateomarch2025reliability];
  files 01, 02, 03, 06, 08, 09, 13.
- In repo: `spec/ONBOARDING.md` §2–§5; DESIGN §5.5, §5.6, §5.9, §18.3, §18.4, §18.7; ADR-0001 §4.3.10, §6, §9.4;
  ADR-0002 §7.8; ADR-0004 (branch `docs/486-threshold-test-decision`); `crates/crank-store/migrations/0001_initial.sql`,
  `0004_route.sql`; `crates/crank-engine/src/record.rs`, `lap.rs`; `crates/crank-core/src/recorder.rs`, `library.rs`,
  `dto.rs`; `crates/crank-domain/src/distributions.rs`, `wbal.rs`, `workout/compile.rs`, `workout/compiled.rs`;
  `app/src/athleteHr.ts`, `app/src/ride/hrZone.ts`.
