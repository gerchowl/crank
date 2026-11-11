# ADR-0007: Check 12 judges release from physics and the rider, not from frame forensics

- **Status:** Accepted (owner, 2026-11-09)
- **Date:** 2026-11-09
- **Issue:** #516 (PR #521)
- **Related:** #44 (HIL), #576, ADR-0002 §4.2, §5, §7.9, `docs/hil/part-b.md`, `docs/hil/part-b-runbook.md`

This is a separate ADR rather than an ADR-0002 amendment. ADR-0002 decides the trainer stack the app ships, and
this ADR decides how a test tool (`hil_fec analyse`) reads its own captures. Number 0006 is taken by #580 (build
cache).

## 1. The questions first

**What is check 12 for?** Part B is a supervised session in which the owner rides the NEO. `hil_fec check12` runs
ERG 200 W at 90 rpm, then switches to `Free` (page 50 defaults, then page 51 grade 0). The rider holds 90 rpm,
then drops to 70 rpm on a cue, and the trainer is released (`rider.rs` `check12`). The check answers one
question: when crank sends `Free`, does the NEO drop the ERG target?

**Who writes the input?** Part B reads only captures that `hil_fec` writes. Each of their lines gets its stamp
from a monotonic clock (`MonoTime`) and is appended under the same lock (`Capture::record_now`, `record_aux_now`,
#521). There is no other writer and no attacker, so **a forged or replayed frame is not a fault this tool can
meet**. A hand-edited file is not evidence, and only the output of `redact` is committed. The forgeries of
#521's rounds 1–7 were byte sequences built by a reviewer. If our stack duplicated, reordered or replayed frames,
that would be a stack bug, found once where the bytes enter the file. The analyser does not try to out-guess it
frame by frame.

**Scope (decided): Part B checks read `hil_fec` captures only.** `Capture::record(at, …)` is still public, and it
takes a caller's stamp outside the lock. Two places use it:

- the app driver (`crank-ble/src/driver.rs:294`, `record_capture`), which writes the desktop
  `CRANK_CAPTURE_DIR/trainer-*.fecap`;
- the test helper `write_capture` (`checks.rs:1351` at e066075), which writes from records already in order.

Desktop captures therefore lack the one-lock guarantee. They stay inputs for Part A analyses (`hil_fec analyse
--target`), which are no Part B check. Converting the driver to `record_now` would change the shipped app for
a test tool's sake, so it is not done.

**What do the real captures tell us?** No live capture of check 12 exists yet. The 7 committed live NEO captures
are Part A runs, at 0 W and standing still, but they do show the transport. The table counts the page-25
broadcast frames (byte 2 `0x4E`, page byte 0 `0x19`) in each `docs/hil/captures/*.fecap` at #521's head
`e066075` (counted 2026-11-09):

| capture | page 25 frames | interval min / median / max (ms) | update-count step 0 / 1 / 2 / 3–6 | step 0 = identical payload |
|---|---|---|---|---|
| check1 | 28 | 237.5 / 241.5 / 750.0 | 3 / 21 / 3 / 0 | 3 of 3 |
| check2 | 43 | 237.5 / 269.7 / 751.2 | 1 / 38 / 2 / 1 | 1 of 1 |
| check3 | 93 | 237.8 / 268.1 / 1260.0 | 2 / 88 / 2 / 0 | 2 of 2 |
| check4 | 31 | 236.5 / 266.2 / 750.2 | 1 / 26 / 3 / 0 | 1 of 1 |
| idle | 166 | 210.1 / 269.7 / 1259.6 | 4 / 149 / 11 / 1 | 4 of 4 |
| latency | 916 | 208.9 / 269.0 / 1767.7 | 42 / 763 / 88 / 22 | 42 of 42 |
| spacing | 488 | 210.0 / 268.7 / 1020.6 | 15 / 436 / 30 / 6 | 15 of 15 |

Page 25 arrives at about 4 Hz, and updates get lost: the count steps by up to 6. When the count holds, the NEO
is repeating its last update byte for byte. The sim sends page 25 every 350 ms (`adv-k8-bad.fecap`).

**What does the physics tell us, whatever the gear?** ERG holds its target, so at 70 rpm it still gives about
200 W. A released trainer simulates a flat road, where power depends on speed: P(v) = Crr·m·g·v + ½·Kw·v³ (FE-C
page 50 defaults, Kw 0.51 kg/m; ADR-0005 §1). If the rider does not shift, speed is proportional to cadence. Going
from 90 to 70 rpm then scales the rolling term by c = 70/90 and the air term by c³. The ratio r = P₇₀ / P₉₀
therefore lies between c³ and c **in every gear** (Crr 0.004, m 85 kg). The watt figures are illustrative, for the FE-C road
model a real NEO is expected to apply. The sim's model differs (`sim.rs:474` at e066075: `0.51·v³`, no ½, speed =
cadence × 0.09 m/s, `sim.rs:461`), but it has the same form, so its r also lies between c³ and c:

| speed at 90 rpm | 10 km/h | 20 km/h | 30 km/h | 40 km/h | 65 km/h |
|---|---|---|---|---|---|
| P₉₀ / P₇₀ (W) | 15 / 10 | 62 / 35 | 175 / 91 | 387 / 193 | 1561 / 753 |
| r at 70/90 rpm | 0.66 | 0.56 | 0.52 | 0.50 | 0.48 |
| r at the worst band edges (75/85 rpm) | 0.81 | 0.75 | 0.72 | 0.71 | 0.69 |

Even at a standstill the limit is c = 75/85 = 0.88. Under ERG, r stays near 1.

**What does the rider tell us?** Whether the pedals went light. Compared with the 90 rpm part under `Free`, 70 rpm
on a flat road is lighter in every gear if the rider does not shift, and 70 rpm under ERG 200 W is heavier per stroke. The rider's answer is
an independent channel that no frame can reach.

**Which of #521's rules go?** Every rule that defended against byte sequences our stack does not produce. The
table, rule by rule, is in the PR #587 body.

## 2. Decision: the check 12 rule

**Procedure (`rider.rs` `check12`):**

1. ERG 200 W at 90 rpm.
2. `Free`, with the cue **"stay in this gear from now until the question"**, then hold 90 rpm for 15 s (was 10 s).
3. Stamp the cue `# 12 70rpm <µs>` and hold 70 rpm for 15 s.
4. Ask: **"Compared with the 90 rpm part right after the switch, did pedalling at 70 rpm get clearly easier? (y =
   easier / n = not easier / s = I shifted gear)"**. Record the answer with the existing answer note (`rider.rs`
   `answer`), which stamps it.
5. Write the closing release.

The runbook says the same: stay in this gear from the switch until the question, and answer `s` if you shifted.
No speed guard checks it; the owner rejected one as extra machinery (2026-11-09).

**Reading.** The capture must be a timeline: stamps never go back and every line is a record (#521, kept). Its
control writes must equal check 12's fixed program, compared item by item (§4). The check reads:

- **updates**: the page-25 frames whose update event count (page byte 1) differs from the page 25 before them. A
  held count is a repeat (§1 table: in 68 of 68 cases the payload is identical).
- **settle = 5 s.** The sim's power lag is first order with τ = 0.8 s (`sim.rs:163` at e066075), so 5 s is more
  than 5τ = 4 s, which reaches 99.3 %. The NEO's own lag is not measured yet. Check 6 measures it in the same
  session, and its t90 must be ≤ 2.3 s (`part-b.md`, check 9's conditions), so 5 s is also more than twice that.
- **`erg`**: the median power of the updates in [page-50 write − 5 s, page-50 write). The rider is at 90 rpm under
  ERG.
- **`free90`**: the median power of the updates in [page-51 write + settle, cue) at 85–95 rpm.
- **`free70`**: the median power of the updates in [t₇₀ + settle, answer stamp) at 65–75 rpm. The question has no
  stamp of its own: it is asked at the end of the 15 s hold, and the window ends at the answer note's stamp. t₇₀ is the first
  update in that band after the cue. The band itself leaves out the rider's change of cadence.
- **`r`** = `free70` / `free90`.

**Verdict:**

| condition (in this order) | result |
|---|---|
| no timeline; the program differs; the run is not completed; the cue or the answer is missing | no result |
| `erg` outside 180–220 W (ERG never held, so a release cannot be seen) | no result |
| fewer than 15 updates in `free90` or `free70` (fewer than 2 per second; about 37 are expected at 269 ms) | no result |
| the answer is `s` | no result |
| r ≤ 0.85 **and** the answer is `y` | **PASS** (`released: true`) |
| r ≥ 0.95 **and** the answer is `n` | **FAIL** (`released: false`) |
| r in (0.85, 0.95), or power and rider disagree | no result, with r and the answer reported |

**Margins.**

- **Released:** r ≤ 0.81 at any speed from 10 to 65 km/h and any cadence in the bands, and ≤ 0.88 at the limit of
  zero speed. So it is never ≥ 0.95, never FAIL, and PASS in every realistic gear.
- **Held:** r ≈ 1. Only an ERG that sags by more than 15 % from 90 to 70 rpm could read ≤ 0.85. The rider then
  feels it heavier and answers `n`, which gives no result, never PASS. Check 9 measures the NEO's ERG holding
  down to its stall in the same session.
- The median means that a minority of wrong frames cannot move the result.

**#521's falsifier fixtures under this rule.** Evaluated by hand, with the cue at `free` + 10 s as those
captures were made, and before the question existed:

- **Forgeries change nothing.** `adv-k8-forged`, `b1-gtail-forged` and `b1-tail-insert-k8` give the same medians
  as their honest bases (`erg` 200 W, `free90` 298 W, `free70` 195 W, r = 0.654). The copies are standstill frames
  at 0 rpm, outside both bands.
- **The bases do not hold.** Their `holding` transform rewrote only the frames below 80 rpm to 195 W, so at 90
  rpm they still report the sim's road power of 298 W: ERG is not held there. Under this rule they read released
  by power. They give no result for lack of an answer.
- **A new fixture.** The non-releasing fixture for the new tests holds about 200 W in both `Free` phases (§3).

**No change to the capture format.** A sequence number from our writer adds nothing to the stamp taken under the
lock: it would number a duplicated notification as faithfully as any other line. Only the trainer's own count
recognises upstream duplicates, and the update rule already uses it.

## 3. Fault model and properties

| fault | real? | effect |
|---|---|---|
| lost page 25 (a lost notification; the NEO skips updates) | yes: count steps 2–6, gaps up to 1.77 s (§1) | fewer updates; too few gives no result; the silence guard aborts at 2 s |
| the NEO repeats its last update | yes: 68 repeats, all identical | dropped by the update rule |
| pump stall: queued frames appended µs apart | yes, by the writer's design (#521) | none: no rule reads the spacing |
| duplicate delivery of a notification | not observed; would be a stack bug | an identical frame with a held count is a repeat |
| reorder, or a stamp going back | impossible in a `hil_fec` capture; the pre-#521 Part A captures show it | no result |
| any gear | yes | none: the ratio does not depend on the gear (§1) |
| the rider shifts gear during the check | possible | reported: the rider answers `s`, no result; unreported: out of model (the don't-shift rule, runbook and part-b.md) |
| the rider stops pedalling or leaves a band | yes (#576: 0 rpm counted as "below 80") | outside the band; too few updates gives no result |
| ERG sags at 70 rpm, or the power sensor is stuck | not observed | r out of its range or the rider disagrees: no result |
| the NEO never fills accumulated power | likely (only 0 W observed) | not read |
| forged, replayed or hand-edited bytes; desktop captures | outside this tool's input (§1) | out of scope |

**Properties, tested by a generator.**

- **The base captures.** The sim's check-12 capture is releasing. A non-releasing one is made from it with ERG
  200 W held through both `Free` phases. Both have the new stamps and an answer.
- **The operators**, applied in any sequence:
  - `drop(rx frame)`;
  - `repeat(page 25, same bytes, next slot)`;
  - `stall(compress a run of stamps to 1 µs steps)`;
  - `cadence(move a span of updates to 0 rpm or out of the band)`;
  - **`gear(g)`**: speed per rpm for the whole check, from 0.03 to 0.2 m/s per rpm (10–65 km/h at 90 rpm). The
    sim's road power follows it, and ERG ignores it. This needs a `sim.rs` change: today speed is fixed at
    `cadence * 0.09` (`sim.rs:461`) under the road model `0.51·v³` (`sim.rs:474`), and the plan needs a speed per rpm;
  - `answer(y | n | s | none)`.
- **The properties:**
  - a releasing capture is never FAIL, **assuming no unreported gear shift** (a shift to a harder gear between the
    switch and the question can raise r; this is the don't-shift rule, not a guard);
  - a non-releasing capture is never PASS;
  - when r and the answer disagree, the result is no result.
- **The falsifier fixtures** (§2) are kept as fixed examples, not as operators: forged input, expected never PASS.

## 4. The other checks

The shared freshness layer and the token grammars are deleted. Every check finds what its procedure wrote, and
gives no result when that is missing.

- **The control program stays.** It is a fixed list of the expected control writes per check and per way of
  ending (`Step`, `follows`, the programs of the `GRAMMAR_*` constants, with the soft start expanded to ERG 0 W
  and six 1 Hz steps). It is compared with the capture's writes item by item. Check 6's "exactly the four
  `CHECK6_W` steps, else no result and `NotMeasured`" is this comparison. Check 9 relies on it through
  `PowerLag::of_check6`, and check 12's program is `Release, SoftStart(200), Wind, Track(0), Release`. Only the
  token alphabet (`tags`, `answers`, `records`) and the shape regexes go. `PowerLag::Contradictory` goes too: a
  check 6 that breaks its program is `NotMeasured`.
- **Check 5** reads CPS power readings as they come. `cps::parse` keeps pairing crank events for cadence.
- **Check 8** keeps "page 16 elapsed time held = repeat" and "≥ 3 fresh speeds per segment, else no result". The
  NEO reading 0/0 is real (part-b.md, "page 16 at standstill only").
- **Checks 10 and 11a/b/c** keep the order conditions they compute from: the cue, the answer, the drop, the
  reconnect and the closing release, each found once and in order, else no result. Check 10 also keeps its 20 ms
  advert merge.
- **Updates** (the count changed) are the one freshness notion, used by checks 5, 7, 9 and 12.

## 5. Implementation plan

**`timeline.rs`:**

- Delete `counters`, `Fresh`, `Break`, `MOST_W`, `fresh_cps` and the same-µs duplicate check.
- Reduce `fresh_samples` to "count unchanged = repeat" and `fresh_speeds` to "elapsed time unchanged = repeat".
- In `Grammar`, delete `tags`, `answers`, `records` and `shapes`, and keep `programs` with `Step` and `follows`.
- Keep `parse` (the order and malformed-line checks), the typed views, `drops`, `closing_release`, `index_at` and
  `checks_read_typed_events_only`.

**`partb.rs`:** rewrite `check12` to §2. **`rider.rs`:** `check12` holds 15 s, gives the don't-shift cue, stamps the 70 rpm
cue and asks the question. **`sim.rs`:** a `Plan` field for speed per rpm, replacing the fixed `0.09` at `sim.rs:461`, for the
`gear(g)` operator (§3).

**Tests to delete:**

- `check12_a_stale_page25_copy_with_the_same_count_is_no_result`
- `check12_a_standstill_copy_past_half_the_accumulated_power_is_no_result`
- `check12_a_standstill_copy_a_rollover_back_reads_as_its_update`
- `page25_counters_back_at_half_their_range_and_move_together`
- `page25_accumulated_power_gains_at_most_4094_w_per_update_and_moves_with_the_count`
- `page16_backs_at_half_its_range_and_its_distance_never_moves_alone`
- the counter half of `cps_readings_between_crank_events_count_and_crank_data_never_goes_back`
- `check5_a_stale_cps_copy_is_no_result`
- `check7_stale_page_25_copies_are_no_result`
- the copy, stale and insert families of `mutated_captures_never_gain_or_change_a_full_row`, which keeps its
  program-breaking families

**Tests to keep:**

- `check12_free_releases_erg`, with a `y` answer
- the order and malformed-line tests
- the program tests, including `the_soft_start_is_erg_0_then_six_steps_on_the_line_to_its_target`
- the fail-closed tests of checks 5, 6, 8 and 9
- the 11a/b/c order tests
- `the_live_neo_captures_are_timelines`

**Tests to add:** the §3 generator, one table test per verdict row, and the runbook text.

**Docs.** In part-b.md, replace "Freshness from the trainer's own counters", the shape half of "Each check's
records follow its grammar", and the three page-25/accumulated-power known limits with §3's table. Keep "plausible
retiming", restated as "hand-edited and desktop captures are out of scope". §2 resolves #576's items: the cadence
bands, and no accumulated-power reading after a drop.

## 6. Alternatives considered

- **Keep hardening freshness** (count step bounded by rate × Δt, accumulated-power bounds, runs of consistent
  updates). Rejected: it defends against an attacker that does not exist, and adds no evidence of a release.
- **An absolute bound (power < 0.9 × target at 70 rpm).** Rejected: it depends on the gear. A released NEO at
  about 40 km/h at 70 rpm needs about 390 W.
- **Cross-check against CPS power.** Rejected as a requirement, because it would need a paired power meter.
  Check 5 already validates page 25 against CPS in the session.
- **A writer sequence number, or `record_now` in the app driver.** Rejected (§1, §2).
- **Power only, no rider question.** Rejected: a stuck sensor or a sagging ERG would then decide alone.

## 7. Open questions

| Question | Issue | Milestone |
|---|---|---|
| Calibrate the r thresholds (0.85 / 0.95), the bands and the 5 s settle on the first live check 12 | #44 | Part B session 1 |
