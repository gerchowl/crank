# ADR-0007: Check 12 judges release from physics and the rider, not from frame forensics

- **Status:** Proposed
- **Date:** 2026-11-09
- **Issue:** #516 (PR #521)
- **Related:** #44 (HIL), #576, ADR-0002 §4.2, §5, §7.9, `docs/hil/part-b.md`, `docs/hil/part-b-runbook.md`

A separate ADR rather than an ADR-0002 amendment: ADR-0002 decides the trainer stack the app ships. This ADR
decides how a test tool (`hil_fec analyse`) reads its own captures. Number 0006 is taken by #580 (build cache).

## 1. The questions first

**What is check 12 for?** Part B is a supervised session. The owner rides the NEO, and `hil_fec check12`
runs ERG 200 W at 90 rpm, switches to `Free` (page 50 defaults, then page 51 grade 0), holds 90 rpm for 10 s, cues
70 rpm for 15 s and releases (`rider.rs` `check12`). The check answers one question: when crank sends `Free`,
does the NEO drop the ERG target? ERG holds 200 W when cadence falls, and a released trainer does not.

**Who writes the input?** Our stack writes it, to a file on the owner's disk, while the owner is on the bike. The
only writer is `Capture`. Since #521 it reads a monotonic clock (`MonoTime`) and appends the line under the
same lock (`Capture::record_now`, `record_aux_now`). Nobody else produces these bytes. So **a forged or replayed
frame is not a fault this tool can meet.** There is no attacker. A hand-edited file is not evidence, and only the
output of `redact` is committed. The forgeries of rounds 1 to 7 were byte sequences built by an adversarial
reviewer. If our stack ever duplicated, reordered or replayed frames, that would be a stack bug. It would be found
once, where the bytes enter the file, and the analyser would not try to out-guess it frame by frame. Section 3
lists the stack bugs that are plausible and what each one does to the verdict.

**What is the simplest rule that is right on every capture we have?** No live capture of check 12 exists yet.
The 7 committed live NEO captures are Part A runs, at 0 W and standing still, so no check 12 rule has ever run on
real data. What they do show is the transport. The table counts the page-25 broadcast frames (byte 2 `0x4E`, page byte 0
`0x19`) in each `docs/hil/captures/*.fecap` at PR #521's head `e066075`, with stamps in elapsed µs (counted 2026-11-09):

| capture | page 25 frames | interval min / median / max (ms) | update-count step 0 / 1 / 2 / 3–6 | step 0 = identical payload |
|---|---|---|---|---|
| check1 | 28 | 237.5 / 241.5 / 750.0 | 3 / 21 / 3 / 0 | 3 of 3 |
| check2 | 43 | 237.5 / 269.7 / 751.2 | 1 / 38 / 2 / 1 | 1 of 1 |
| check3 | 93 | 237.8 / 268.1 / 1260.0 | 2 / 88 / 2 / 0 | 2 of 2 |
| check4 | 31 | 236.5 / 266.2 / 750.2 | 1 / 26 / 3 / 0 | 1 of 1 |
| idle | 166 | 210.1 / 269.7 / 1259.6 | 4 / 149 / 11 / 1 | 4 of 4 |
| latency | 916 | 208.9 / 269.0 / 1767.7 | 42 / 763 / 88 / 22 | 42 of 42 |
| spacing | 488 | 210.0 / 268.7 / 1020.6 | 15 / 436 / 30 / 6 | 15 of 15 |

Page 25 arrives at about 4 Hz. Updates get lost: the count steps by up to 6. When the count holds, the NEO is
repeating its last update byte for byte. The 11 sim captures behave the same way at about 2.9 Hz: in check 12's
window, 30 page-25 frames arrive in 10.3 s (`adv-k8-bad.fecap`).

The rule that follows from this is the physical question the check asks. With cadence held near 70 rpm after
`Free`, the NEO's reported power must fall well below the ERG target. It must also have held that target at 90 rpm
before `Free`, or the run shows nothing (§2).

**What does the rider tell us?** Whether the pedals went light. A rider feels the change from ERG 200 W at 70 rpm
to flat road at 70 rpm at once. That is an independent channel, and a forged frame cannot reach it. The check asks
the question and records the answer. An honest disagreement between the rider and the power gives no result.

**Which of the seven rounds' rules are unnecessary?** All that defended against byte sequences our stack does not
produce:

| round | rule | under this ADR |
|---|---|---|
| 1 | one writer, stamp and append under one lock (`record_now`) | **keep**: this is the capture-time guarantee |
| 1 | stamps never go back in file order; ties follow file order | **keep**: one check of the writer, cheap |
| 1 | one silence threshold for the live guard and `analyse` | keep (live-guard parity, not freshness) |
| 2 | a record identical to another at the same µs is contradictory | delete |
| 2 | per-check grammar: token alphabet, anchored shape regexes, exact control program | delete for check 12 (§4 for the others) |
| 2 | saturating reply accounting (a panic) | keep (a bug fix) |
| 4 | the vocabulary tokens `u q e ?`, other tx outside the grammar | delete |
| 4 | a malformed line is no result | keep |
| 4 | `PowerLag::Contradictory` (check 6 breaks its grammar) | delete with the grammar; `NotMeasured` stays |
| 5 | page 25 count back is contradictory | delete |
| 5 | page 16 counters back or moving alone are contradictory | delete; keep "elapsed time held = repeat" for check 8 |
| 5 | CPS crank counters back are contradictory | delete |
| 5/6 | a page-71 status answers the request just before it, naming the last command before that request | simplify: an echo is a status after the write's request, naming its page and bytes (informational in check 12) |
| 6 | page 25 freshness as a pair (count, accumulated power); count held while power moves is contradictory; 0 W exemption | delete |
| 6 | CPS byte-identical duplicate dropped | delete (not seen, and harmless to a mean) |
| 7 | accumulated-power step ≤ count step × 4094 W | delete |

The accumulated power (page 25 bytes 3–4) is no longer read at all. It has never been seen moving on the NEO
(part-b.md, "accumulated power at 0 W only"), and the round-6 0 W exemption existed only because of that.

## 2. Decision: the check 12 rule (one page)

**Procedure change (`rider.rs` `check12`).** The procedure stamps the 70 rpm cue (`# 12 70rpm <µs>`). After the
15 s hold, while the trainer is still in `Free`, it asks **"Since the switch, did pedalling at 70 rpm get clearly
easier? (y/n)"** and records the answer with the existing answer note (`rider.rs` `answer`, `check10::Question`).
Then it writes the closing release. A run with no answer did not complete.

**Reading.** The capture must be a timeline: stamps non-decreasing and every line a record (kept from #521).
Otherwise there is no result. Check 12 reads:

- `target`: the last page-49 write (200 W), then the page-50 write and the page-51 grade-0 write after it. `free`
  is the stamp of the later of the two. If any of these, the cue stamp, the answer or the closing release is
  missing, there is no result.
- **updates**: the page-25 frames whose update event count (page byte 1) differs from the page 25 just before
  them. A held count is the NEO repeating itself (§1 table, step 0 = identical payload in 68 of 68 cases).
- `before` = median power of the updates in [`free` − 5 s, `free`) (rider at 90 rpm under ERG).
- `after` = median power of the updates in [cue + 5 s, closing release) with cadence 60–79 rpm (page byte 2).
  The 5 s skip covers the change of cadence and the trainer's power lag. The sim's τ is 0.8 s
  (`sim.rs` `power_tau_s`), and the largest first echo of a SIM value on the NEO was 4.21 s (ADR-0002 §4.4.1,
  `T_unknown`).
- `n` = the number of those updates.

**Verdict.**

| condition (in this order) | result |
|---|---|
| no timeline, run not completed, a required write, stamp or answer missing | no result |
| `before` outside 180–220 W (ERG never held the target, so a release cannot be seen) | no result |
| `n` < 20 (fewer than 2 updates/s over the 10 s window; nominal ≈ 37 at 269 ms) | no result |
| `after` < 0.9 × target **and** the answer is yes | **PASS** (`released: true`) |
| `after` ≥ 0.9 × target **and** the answer is no | **FAIL** (`released: false`) |
| power and rider disagree | no result, both reported |

Median, not mean: a minority of wrong frames cannot move it, and nothing needs to be told apart. The 90 % bound is
the existing one ("ERG would hold it"). A released NEO at 70 rpm in a gear that gives about 23 km/h needs about
½·0.51·6.5³ + 0.004·85·9.81·6.5 ≈ 92 W (the page-50 default Kw 0.51 kg/m with FE-C's ½, ADR-0005 §1). That leaves
88 W of margin below 180 W. The sim's road model omits the ½ and gives about 149 W (its CPS in
`adv-k8-bad.fecap` at 77–86 s, 0x0095), still below 180 W.

**Against the falsifier repros, without any freshness rule** (the falsifier artefacts of #521 rounds 6–7,
evaluated by hand with the cue at `free` + 10 s, as the procedure runs it): the non-releasing captures `adv-k8-bad`, `b1-gtail-bad` and `b1-tail-bad` give `after`
195 W, n 30. Their forged versions `adv-k8-forged` (round 7, count jumps of 7–10 and a 0 W chain), `b1-gtail-forged`
(round 7 tail) and `b1-tail-insert-k8` (round 6 insert) all give `after` 195 W, n 25–30, so **not released**. The
copies are standstill frames at 0 rpm, outside the cadence band. Round 6's substitution of every frame after
`Free` by a standstill copy leaves only 0 rpm copies and the frames without one, so the result is no result or not
released. None of them reaches PASS, and PASS would also need the rider's yes.

**No change to the capture format.** A writer sequence number would add nothing to the stamp taken under the
lock. It would number a duplicated notification as faithfully as any other line. Upstream duplicates can only be
recognised by the trainer's own counter, and the update-count rule above already does that.

## 3. Fault model

| fault | real? | effect under §2 |
|---|---|---|
| lost page 25 (BLE notification lost, NEO skips updates) | yes: count steps 2–6, gaps up to 1.77 s (§1) | fewer updates; n < 20 gives no result; the silence guard aborts at 2 s |
| NEO repeats its last update | yes: 68 repeats, all byte-identical | dropped by the count rule |
| pump stall: queued frames appended µs apart | yes, by design of the writer (#521) | none: no rule reads spacing |
| duplicate delivery of one notification | not observed; would be a stack bug | an identical frame with a held count is dropped as a repeat |
| reorder, stamp going back | impossible past the one-lock writer; Part A captures predating it show it | no result (`records out of time order`) |
| clock jump | the clock is monotonic (`MonoTime`, tokio `Instant`) | out of scope |
| rider stops pedalling or leaves 60–79 rpm | yes (#576: 0 rpm was counted as "below 80") | outside the band; too few updates gives no result |
| rider in a big gear (released power near 180 W) | possible | power says held, rider says easier: no result. Runbook: stay in a middle gear |
| stuck power sensor (NEO reports a frozen value) | not observed | the rider disagrees: no result |
| NEO never fills accumulated power | likely (only 0 W observed) | not read |
| forged, replayed or hand-edited bytes | not a fault of this tool (§1) | out of scope; only `redact` output is committed |

Every in-model doubt resolves to no result. None of them can yield PASS for a trainer that holds 200 W, because
PASS needs both the median below 180 W in the cadence band and the rider's yes.

**Test generator (replaces round 7's battery for check 12).** Start from the sim check-12 capture, and from it
made non-releasing (`holding`, 195 W after `Free`). Apply any sequence of the in-model operators:
`drop(rx frame)`, `repeat(page 25, same bytes, next slot)`, `stall(compress a run of stamps to 1 µs steps)`,
`cadence(set a span of updates to 0 rpm or out of the band)`, `answer(y|n|none)`. Two properties: a non-releasing
capture is never PASS, and a releasing capture is never FAIL. The three falsifier repros are kept as fixed
examples (forged input, expected: not PASS), not as operators, because they are outside the model.

## 4. The other checks

The freshness and grammar layer is shared, so its deletion reaches checks 5–11. The same principle applies:
each check finds what its procedure wrote and gives no result when that is missing. It never adds rules against
bytes our stack does not write.

- **Check 5** (page 25 against CPS): reads CPS power readings as they come. `cps::parse` keeps pairing crank events
  for cadence. No counter contradictions.
- **Checks 6 and 9**: keep fail-closed ("exactly the procedure's steps, else `NotMeasured` and no result").
  `PowerLag::Contradictory` goes with the grammar.
- **Check 8**: keeps "page 16 elapsed time held = repeat" and "≥ 3 fresh speeds per segment, else no result". The
  NEO reading 0/0 is a real fault (part-b.md, "page 16 at standstill only").
- **Checks 10 and 11a/b/c**: keep the order conditions their results are computed from: the cue, the answer, the
  drop, the reconnect and the closing release, each found once and in procedure order, else no result. They keep
  check 10's 20 ms advert merge, which is a real scanner behaviour. The regex grammars and exact programs are
  deleted.
- **Updates** (count changed) stay the one freshness notion, used by checks 5, 7, 9 and 12.

## 5. Implementation plan

Delete from `timeline.rs`: `counters`, `Fresh`, `Break`, `MOST_W`, the bodies of `fresh_samples` (it becomes "count
unchanged = repeat"), `fresh_speeds` (it becomes "elapsed unchanged = repeat") and `fresh_cps` (removed), the
same-µs duplicate check, and `Grammar` with its shapes and programs (`follows`, `Step`, the `GRAMMAR_*`
constants). Keep `parse` (order and malformed-line checks), the typed views, `drops`, `closing_release`,
`index_at` and `checks_read_typed_events_only`.

In `partb.rs`, `check12` is rewritten to §2. In `rider.rs`, `check12` stamps the cue and asks the question.

Delete these tests: `check12_a_stale_page25_copy_with_the_same_count_is_no_result`,
`check12_a_standstill_copy_past_half_the_accumulated_power_is_no_result`,
`check12_a_standstill_copy_a_rollover_back_reads_as_its_update`,
`page25_counters_back_at_half_their_range_and_move_together`,
`page25_accumulated_power_gains_at_most_4094_w_per_update_and_moves_with_the_count`,
`page16_backs_at_half_its_range_and_its_distance_never_moves_alone`,
`cps_readings_between_crank_events_count_and_crank_data_never_goes_back` (the counter half),
`check5_a_stale_cps_copy_is_no_result`, `check7_stale_page_25_copies_are_no_result`, and the copy, stale and
insert families of `mutated_captures_never_gain_or_change_a_full_row`. Keep `check12_free_releases_erg` (plus a
yes answer), the order and malformed-line tests, the fail-closed tests of checks 5, 6, 8 and 9, the 11a/b/c order
tests, `the_live_neo_captures_are_timelines`, and the falsifier repros as fixed examples.

Add these tests: the §3 generator; one table test per verdict row; and the runbook text ("stay in a middle gear;
answer y if pedalling got clearly easier").

Docs: replace part-b.md's "Freshness from the trainer's own counters", "Each check's records follow its grammar"
and the three page-25/accumulated-power known limits with §3's table. Keep "plausible retiming", restated as
"hand-edited captures are out of scope". #576's items are resolved by §2: the cadence band, and no accumulated-power
reading after a drop.

## 6. Alternatives considered

- **Keep hardening freshness** (count step bounded by rate × Δt, accumulated-power bounds, runs of consistent
  updates). Rejected: it defends against an attacker that does not exist. Each rule was a patch against the last
  forgery, and none of them adds evidence that the trainer released.
- **Cross-check against CPS power** (an independent meter). Rejected as a requirement, because it makes check 12
  depend on a power meter being paired. Check 5 already validates page 25 against CPS in the same session.
- **A writer sequence number in the capture format.** Rejected (§2): it detects nothing that the stamp under the
  lock does not.
- **No rider question, power only.** Rejected: a stuck sensor or a big gear would then decide the verdict alone,
  and asking costs one keypress.

## 7. Open questions

| Question | Issue | Milestone |
|---|---|---|
| Calibrate the 90 % bound and the 60–79 rpm band on the first live check 12 | #44 | Part B session 1 |
