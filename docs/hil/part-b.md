# HIL session 1, Part B runbook: control checks with a rider (#44, #412)

How to run the Part B pedalling checks of ADR-0002 §9 with `hil_fec`. The owner's step-by-step session
script (app flow plus these checks) is [part-b-runbook.md](part-b-runbook.md). This covers check 5 (page 25
against the Cycling Power measurement `2A63`), the control checks 6, 7, 8, 9 and 12, and check 10
(a second central while connected; [below](#check-10-a-second-central-while-connected)) and checks 11a–c
with the trainer unplugged ([11a](#check-11a-unplug-while-pedalling),
[11b](#check-11b-the-unplugged-stop-dropout-and-reconnect), [11c](#check-11c-the-unplugged-cold-start)).
Results go to a new `docs/hil/<date>-fec.md` in the
format of [the Part A results](2026-10-14-fec.md): one "Results at a glance" row per check, which each
run prints.

## Safety: what every run does

The checks drive a real trainer under a rider, so every subcommand (`crates/crank-ble/examples/hil_fec/rider.rs`):

- connects into the release state, page 48 at 0 %, and writes nothing else until the operator types
  `y` (or `yes`) and Enter in the window; anything else, or closed input, aborts;
- engages ERG as the engine does at a ride start (ADR-0002 §7.5): page 49 at 0 W until the first page-25
  sample with cadence above 0, then a 5 s ramp in 1 Hz steps from the 3 s power to the target;
- never commands more than 250 W or a grade steeper than 10 % (the #44 values);
- while ERG is applied, stops on the §7.4 release rule (cadence below 40 rpm for 2 s), except during check 9's
  descent, which is the test of that threshold and ends instead when the trainer lets go, at the first sample
  below 25 rpm, or (aborted) as soon as the rider leaves the cued descent (below; within 160 s in any case);
- stops on 2 s without page 25 (ADR-0002 §6), a dropped link, a failed write, a write not acknowledged
  within 2 s, or a rider who does not start pedalling within 120 s; these rules keep running while a write
  is pending;
- on completion, on any stop, on a panic, and on SIGINT (Ctrl-C), SIGTERM or SIGHUP (the window closed),
  writes page 48 at 0 % (the ride-end mode of ADR-0002 §7.9) and a page-70 request for page 71, so the
  capture shows whether the trainer took it, then disconnects. Each of these two writes is bounded at
  2 s, so a stuck link cannot hold the exit.

A run that stops early prints `STOPPED: <why> (trainer released)` when its release was confirmed, else
`STOPPED: <why> (release NOT confirmed: <error>; the trainer may still hold its target. Stop pedalling; …)`
(`rider::stopped`, from the release's actual outcome, as the final cue), and exits with status 4; a check 11c that
stops before the link wrote nothing (`… (nothing was written to the trainer)`). Its capture and summary are still
written and analysable, but give no result (below). A run that completes with its release NOT confirmed (after
an 11a or 11b drop) ends on `DONE, but the release was NOT confirmed: <error>; …` and exits with status 5
(`checks::Outcome::lines`; the exit codes are in `main.rs`).

## Running

Bluetooth only works from an Alacritty window on sage (AGENTS.md, TCC). Build once in `nix develop`, with
your own `CARGO_TARGET_DIR`:

```text
cargo build -p crank-ble --features hil --example hil_fec
```

Then one wrapper per check, launched with
`alacritty msg --socket "$(ls -t $TMPDIR/Alacritty-*.sock | head -1)" create-window -e /abs/path/check6.sh`.
The window shows the cues; the owner types `y` and Enter there to start. The log file is for the agent:

```sh
#!/bin/bash
# check6.sh: run one Part B check, keep the window's output in a log.
OUT=/abs/path/to/raw-captures   # outside the repository: raw captures carry the serial
"$CARGO_TARGET_DIR/debug/examples/hil_fec" check6 --out "$OUT" 2>&1 | tee "$OUT/check6.log"
echo "exit ${PIPESTATUS[0]}"; sleep 30
```

The capture is created with `File::create_new`, so each run needs an `--out` directory without that
check's `.fecap`.

| Check | Invocation | Rider | About |
|---|---|---|---|
| 5 | `hil_fec check5 --out DIR` | 90 rpm through ERG 150 W (30 s), then 200 W (30 s); steady cadence is all that is asked. Also subscribes read-only to `2A63`; the run ends (released) if none arrives 5 s into the ride | 1.5 min |
| 6 | `hil_fec check6 --out DIR` | 90 rpm through ERG 100 → 150 → 250 → 100 → 200 W (20 s each), then at 200 W cues 60 rpm and back to 90 rpm (20 s each) | 2.5 min |
| 7 | `hil_fec check7 --out DIR` | 90 rpm at 150 W; the harness writes nothing for 40 s (no keep-alive, no page-70 poll) | 1 min |
| 8 | `hil_fec check8 --out DIR` | Easy gear, steady effort; SIM 0 %, 5 %, 5 % with page 55 rider mass 100 kg, 10 % with 75 kg (30 s each); shift down on the steep part | 2.5 min |
| 9 | `hil_fec check9 --out DIR` | 90 rpm at 250 W ERG, then lower cadence smoothly, about 5 rpm every 10 s, and keep turning until the run ends. Stopping early is safe (the trainer is released) but aborts the run without a result | up to 3 min |
| 12 | `hil_fec check12 --out DIR` | 90 rpm at 200 W ERG; the harness switches to `Free` (page 50 defaults, page 51 grade 0); after 10 s, cue to 70 rpm for 15 s | 1 min |
| 10 | `hil_fec check10 --out DIR` | No pedalling. With the phone, try to connect to the NEO while `hil_fec` holds it, then type two answers in the window (below) | 1–3 min |
| 11a | `hil_fec check11a --out DIR` | Trainer **plugged in**. 90 rpm at ERG 100 W (20 s); at the cue pull the mains plug and keep pedalling; answer `unplugged`; 30 s more (below) | 1.5 min |
| 11b | `hil_fec check11b --out DIR` | Trainer **unplugged**. 90 rpm at ERG 100 W (20 s); released, stop and wait until the lights go out; pedal again to wake it; answer the lights question and type `y`; 90 rpm at 100 W again (20 s) (below) | 2–4 min |
| 11c | `hil_fec check11c --out DIR` | Trainer **unplugged and off**. Type `y`, then at the cue start pedalling; the harness connects once it advertises; type `y`; soft start to 100 W, 20 s at 60 rpm, 20 s at 90 rpm (below) | 2 min |

Each run writes `DIR/<check>.fecap` (both directions, the `.fecap` format) and `DIR/<check>.json` with the
procedure's outcome, the release, the identity, and `result` (below), and prints the result row.

## Results and re-analysis

`result` is computed only from the capture, so it is reproducible from a committed, redacted capture.
After the link is closed, `hil_fec` appends the procedure's outcome as the capture's last line,
`# hil_fec procedure: completed` (or `aborted: …`, `interrupted: …`, `failed: …`). Only a completed
procedure gives a result: for any other outcome, or a capture without that line, the row reads
`no result: procedure …` and `held`/`released`/`verdict` are null. A capture with a **malformed control
write** (a tx page 48–51 or 55 with a field at its "invalid" value, or a bad checksum) gives no result
either, whatever its outcome: the harness encodes only valid fields, so that write is not its procedure's.
The row reads `no result: malformed control write (page N at t s)` and `invalid` names it. A tx record that is
not a 13-byte ANT frame at all counts the same (`Control::NotAFrame`, `not an ANT frame`). `partb::controls`
lists such a write as `Control::Malformed(page)`: it is never the release, and it ends an ERG hold (checks 5,
6 and 9). A capture **cut off** before its end gives no result the same way: every run writes the release last
(recorded before the write, a failed one too), so a capture whose last control write is not page 48 at 0 %, or
is the opening release, reads `no result: the capture is cut off: no release write ends it` (`partb::cut_off`),
offline as live.

```text
hil_fec redact  --target RAW/check6.fecap --out docs/hil/captures
hil_fec analyse --target docs/hil/captures/check6.fecap --out docs/hil/captures   # offline
```

`analyse` keeps the reply accounting it always wrote and adds `part_b` (the same `result`) for a capture
named `check5`, `check6`, `check7`, `check8`, `check9`, `check10`, `check11b` or `check12`, and prints the row again.

| Check | What `result` reports (method text is in the JSON) | ADR-0002 |
|---|---|---|
| 5 | Page-25 arrival rate, and page-25 power and cadence against `2A63` over the settled holds ([below](#check-5-page-25-against-cps-2a63)): seconds compared, mean difference, largest difference; verdict `agree` or `disagree: …` | §5 (telemetry source), §9 |
| 6 | Per ERG step (a page-49 target held ≥ 10 s after another): t90 from the write to the first page-25 sample past 90 % of the step. Per cadence crossing of 75 rpm inside a hold: largest power deviation in the next 10 s and the time until power stays within ±5 % for 3 s | §7.2 (step response, "lead") |
| 7 | From the procedure's 150 W write (the end of the soft start) to the next write, after its first 10 s: share of samples within ±5 %/±10 % of the target and the last 10 s mean; held = ≥ 90 % within ±10 % and last 10 s within ±5 % | §4.4 (no keep-alive) |
| 8 | Per SIM state held ≥ 10 s: mean power, page-16 speed and cadence, and the road model at that speed (`v·m·g·(grade + Crr) + cw·(v + wind)²·drafting·v`, m from page 55): the ratio shows whether grade and mass apply | §7.9 (SIM, page 55) |
| 9 | How the descent ended (`stalled`, `floor`, `departed`), the cadence at which power fell below 90 % and the lowest cadence it was settled at or above 90 % (the two bounds below), power by 5 rpm cadence bin and the lowest bin still followed (≥ 2 samples, ≥ 80 % within ±10 % of 250 W); the verdict: the release comes first, too late, or ambiguous | §7.4 |
| 12 | Mean power below 80 rpm after `Free` against the ERG target (released = below 90 %: ERG would hold it) and whether page 71 echoed pages 50 and 51 | §7.9 (`Free` sent explicitly) |
| 10 | Whether a second central (the phone) could connect while `hil_fec` held the link, whether the NEO kept advertising (the scan beside the connection and the operator), and whether `hil_fec`'s link held ([below](#check-10-a-second-central-while-connected)) | §5 (single central) |
| 11b | When the link dropped after the stop cue and how long the trainer ran on; the reconnect phase by phase; what the trainer reported after it, whether the first write was the release, and how the re-engaged ERG target was held; the cause from the lights answer ([below](#check-11b-the-unplugged-stop-dropout-and-reconnect)) | §6 (dropout, reconnect) |
| 11a | Whether the link dropped after the unplug; page 25 before and after it (FE states, target-limit codes, status bits, mean power) ([below](#check-11a-unplug-while-pedalling)) | §6 |
| 11c | Seconds from the start cue to the advertisement, the connect, the first page 25 and the first ERG echo; whether ERG 100 W is honoured at 60 and at 90 rpm ([below](#check-11c-the-unplugged-cold-start)) | §7.5 (FM-START-07) |

Every result also carries `release`: whether page 48 at 0 % was the last control write and whether a
page-71 reply echoed it.

### Check 9: what counts as a stall

Check 9 measures the trainer: down to which cadence the NEO's ERG holds 250 W while the rider follows
the cue. The cue starts 10 s after the first 250 W write at 90 rpm and falls 5 rpm every 10 s
(0.5 rpm/s). When the rider already makes 250 W or more at the start, the soft start writes 250 W six
times, 1 s apart. The live run and `analyse` both count from the first of these writes
(`partb::descent_target_at`), and the cue then comes 5 s after the soft start ends. Both apply the same
rule (`partb::descent`) to the page-25 samples:

- **Following the cue:** the cadence never falls more than 9 rpm within 3 s, and stays within 15 rpm of
  the cue, above or below. 3 rpm/s is six times the cue's rate, so a 5 rpm step made over a second or two
  passes.
- **Stall:** power below 90 % of 250 W for 2 s (the 2 s of the §7.4 rule), with the cadence still
  following from 1 s before the drop.
- **Floor:** the first sample below 25 rpm with power still held ends the descent.
- **Anything else aborts the run:** a faster fall, power and cadence falling together, unavailable cadence
  or power, or leaving the 15 rpm band. The trainer is released, the outcome is `aborted: <why>`, and there
  is no verdict. Power and cadence falling together cannot be told apart from page 25: it could be a rider
  who stops on purpose or one the trainer overpowers, so neither gives a stall cadence. The abort reason
  gives the cadence.

**Verdict.** The verdict needs evidence on the correct side of 40 rpm and never guesses. It is built from
two bounds on the cadence `c*` below which the trainer can no longer hold 90 %:

- **Lower bound, independent of lag:** power below 90 % at a sample with cadence `c` means `c* > c − 0.5`
  (cadence is rounded to 1 rpm). While cadence falls, a lagging power reads high, never low. "Too late"
  needs such a sample above 40 rpm.
- **Upper bound, needs a stated lag:** power at or above 90 % throughout a stretch of samples that spans
  5 s, with samples at most 0.5 s apart and cadences up to `N`, means `c* ≤ N + 2`. The 2 rpm cover the
  0.5 rpm of rounding and up to 1.5 rpm of movement between samples at 3 rpm/s.
  - This assumes page-25 power lags the trainer like a first-order lag with a time constant of at most
    1 s. Five time constants settle within e⁻⁵ (under 1 %) of a step, so the reading cannot still be
    carried by the higher power from before.
  - The assumption is checked on the trainer by check 6: every ERG step's `t90` must be at most 2.3 s
    (a first-order lag reaches 90 % after 2.303 time constants; delay before the response only adds to
    `t90`). See the next section.
  - "Comes first" needs `N + 2 < 40`.

Otherwise the verdict is `ambiguous: …` with both bounds. If both bounds hold at once (power not monotone
in cadence), it is `inconsistent: …`.

The resolution has a cost: a trainer that lets go at 38 rpm is safe but reads ambiguous, because a 5 s
settled hold at 0.5 rpm/s covers 2.5 rpm. A trainer that holds 250 W down to the low 30s resolves.

**Check 9 depends on check 6.** "The release comes first" is only sound within the lag bound, so check 9
reads `check6.fecap` from the same directory: the same `--out` for a live run, or the directory of the
capture for `hil_fec analyse`. Run check 6 before check 9, with the same `--out`. The result's
`power_lag` field shows what was used:

- **Check 6 completed and every step's `t90` ≤ 2.3 s:** the verdicts stand as above.
- **Check 6 must be this trainer's, from this session.** Both captures must carry the same page-81 serial
  (each run requests page 81 once at its start, a read-only request) and a `# hil_fec finished: <unix s>`
  line (written after the outcome line), and check 6 must have ended no more than 2 h before check 9
  (`SESSION_BOUND_S`: the runbook's checks and rests, not another day's setup) and not after it. A
  different, missing, unspecified or conflicting serial, a missing stamp, a check 6 outside the bound, or
  a `check6.fecap` that cannot be read (any error other than "not found") reads as not measured, with the
  reason in `power_lag.within`; never as a measurement. A redacted capture (serial `FF FF FF FF`) is
  therefore never a trainer match: run `analyse` on the raw captures, then `redact`.
- **No completed check 6** (missing, aborted, or no outcome line): "comes first" reads `… the release comes
  first only if check 6 t90 ≤ 2.3 s (not measured)`. Re-run `analyse` once check 6 is in the directory.
- **A step's `t90` > 2.3 s, or one that never reaches 90 %:** the verdict reads `not valid: power response
  too slow (t90 = X s)`. Only "too late" still stands, because its lower bound needs no lag bound. With a
  slow trainer, a lagging reading at 36–37 rpm can still show 250 W after the trainer has let go above
  40 rpm. The sim test `check9_first_needs_check_6_within_the_power_lag_bound` shows this with a 2 s lag:
  without check 6 it reported "comes first".

### Check 5: page 25 against CPS 2A63

ADR-0002 §5 keeps page 25 as the telemetry source on the condition that, in HIL check 5, "page 25
arrives at ≥ 2 Hz and its per-second mean power agrees with 2A63 within ±1 W; otherwise the ADR is
amended to use 2A63 for telemetry." `check5` measures exactly that, on a ride at two ERG targets
(150 W, then 200 W, 30 s each, 90 rpm), and adds cadence.

**Capture.** `hil_fec` subscribes to `2A63` on the same connection (read-only: a CCCD write, nothing to
the trainer) and records each notification in the `.fecap`, in processing order with the FE-C frames, as
a comment line `# cps <elapsed_us> <hex bytes>` on the same clock (`Capture::record_aux`). The format
stays compatible: every reader skips comment lines (the `.fecap` README says so), so old captures and
tools read new files as before; only `hil_fec` reads the `# cps` lines. They carry no serial, so
`redact` keeps them as they are. A device without `2A63`, or a subscription not completed within 2 s, ends the run with an error
(`check 5 needs Cycling Power 2A63 on the NEO: …`) before anything is applied; the trainer is
released (page 48 at 0 %) first, bounded at 2 s.

**Time alignment.** Both streams are stamped on arrival by the same process on the same monotonic
clock, so there is no offset between them to estimate. What remains is each stream's own jitter and
that `2A63` is slower than page 25, so the comparison avoids transients instead of correcting them:
only the **settled windows** are used, each page-49 target held ≥ 10 s (check 6's definition of a hold)
after its first 10 s. Under ERG the power is steady there, so a few hundred milliseconds of misalignment
are not mistaken for disagreement. Read back from the capture's own tx writes, they are the same live and offline.

**Power.** For every whole capture second inside a window that has at least one sample of each stream:
the mean page-25 power minus the mean `2A63` power. Page 25 power is the instantaneous 12-bit field;
`2A63` is the signed 16-bit instantaneous power. The tolerance is the ADR's: every compared second
within **±1 W** (exactly ±1 W passes). The result also gives the mean difference (`bias_w`), the
largest, and how many seconds were within the tolerance.

**Coverage.** A good average must not hide a target that was never compared, so the verdict first
checks, for **each** ERG target of the procedure (one settled window per target, `rider::CHECK5_W`),
in **continuous time**, not in capture seconds (a notification near a second boundary must not cost a
whole second):

- the window is at least **10 s** long;
- page 25 (samples with a power) has no gap above **2 s** inside the window, the window's edges
  included, i.e. the first sample is at most 2 s after the start and the last at most 2 s before the
  end (2 s is the silence at which the harness itself stops the ride, ADR-0002 §6);
- the page-25 samples that carry a **cadence** (not 0xFF) have no gap above **2 s** either, by the same
  rule: cadence present in one frame of a window is not coverage;
- `2A63` has no gap above **3 s** by the same rule (three missed notifications at its nominal 1 Hz);
- cadence comes only from crank-revolution pairs whose **both** notifications lie inside the window,
  and those pairs leave no stretch of the window uncovered by more than **3 s** (so a pair reaching back
  before the window, over a stretch without crank data, is not used);
  one pair may itself span up to 32 s inside the window (`PAIR_MAX_US`; notifications without a new crank
  event add nothing), which is fine: the counters are cumulative, so revolutions over event time stay
  exact whatever lies between the two notifications;
- at least one second of the window holds a sample of each stream, so a power difference exists.

Anything else is `insufficient: <target> (<window>): <which stream, how long>`, naming the stream that is
missing (page-25 power, page-25 cadence, 2A63 power, 2A63 crank data). A gap exactly at its
bound is within it. More than **10 %** of the `2A63` notifications malformed (shorter than their flags
say) is `insufficient` too (exactly 10 % passes), since those notifications are silently missing. The
per-second power comparison itself still bins by capture second, but a second that lacks a sample of
one stream is skipped, not failed: coverage is decided by the gaps. The sim tests sweep the 2A63 phase
(0–1 s) × jitter (σ 0–100 ms) × drops (0–3 %): a matching run agrees in at least 99 % of them, and a
run with a 4 s hole in 2A63 or a 2.6 s hole in page 25 never does.

**Rate.** Page-25 samples over the whole capture: (n − 1) / span must be ≥ **2 Hz** (the ADR's figure),
and the largest gap is reported.

**Cadence.** `2A63` carries cumulative crank revolutions and the last crank event time (1/1024 s), both
unsigned 16 bit. Between two notifications that carry a new event, cadence is
60 · Δrevolutions / Δevent time, with both differences taken modulo 2¹⁶ (the event time wraps every 64 s,
the revolutions every 65 536): a wrap is invisible to the subtraction. Notifications without a new event
(same event time) add a power reading only. A pair is rejected, and counted, when its notifications are
more than 32 s apart (a whole event-time wrap could hide in the gap) or the cadence is above 255 rpm.
Per window, the `2A63` cadence is the revolutions over the event time of all its pairs, and it is
compared with the mean page-25 cadence of the window: the difference must be within **±1 rpm**.
ADR-0002 sets no cadence figure; this one is derived: page 25 rounds to 1 rpm (±0.5) and the two streams
are sampled at different instants within a window (±0.5). The revolution count is exact, so the
derivation adds no quantisation error. A `2A63` without crank-revolution data cannot confirm cadence:
`insufficient: no 2A63 crank-revolution data`.

**Verdict.** `agree` when the rate, every compared second's power and every window's cadence meet the
tolerances, else `disagree: …` naming each that failed. Other verdicts: `no CPS: …` (no decodable `2A63`
notification in the capture), `insufficient: …`. As for every Part B check, only a completed run gives a
verdict (`partb::analyse` nulls it otherwise), and the live row and `hil_fec analyse` on the capture are
computed by the same function from the same capture. A `disagree` is the ADR's trigger to amend §5 to use
`2A63`: the owner decides, with the numbers in the JSON.

### Check 10: a second central while connected

ADR-0002 §5 assumes the NEO takes one central at a time: while crank is connected no other app can
control it. Part A's short session (#44, 2026-10-14) saw the phone still list the NEO while connected,
and its connect hang at "connecting". `check10` repeats this with a structured, re-analysable record.

**Procedure** (`rider.rs` `check10`). The run connects into the release state (page 48 at 0 %) and,
after the operator's `y`, writes nothing more: no rider, nothing applied. It then opens a **window**
and holds the link through it:

- a scan runs beside the connection (`Session::adverts`, the same `BtleplugAdvertisements` scan used to
  find the trainer) and every advertisement it reports is recorded, flagged as this trainer's or another
  peripheral's;
- the cue asks the operator to open the Tacx app on the phone, look for the NEO and try to connect;
- the questions take a typed answer, re-asked until the answer is one of the accepted words:
  - *Does the phone list the NEO while hil_fec is connected?* `seen` / `not-seen` / `unsure`
  - only after `seen` (`check10::follow_up`; a phone that did not list the NEO has nothing to connect
    to): *Try to connect the phone to the NEO and give it 30 s. What did the connect do?* `connected` /
    `failed` / `stuck` / `not-tried`
- the window lasts at least **60 s** from its start, answers or not, so the scan covers a connected
  stretch (`check10::MIN_WINDOW`), and the answers must be in within **180 s** of its start
  (`check10::ANSWER_WAIT`); otherwise the run is aborted (`no answer to … within 180 s`), and closed
  input aborts it too.

While the window is open, the guards run every 100 ms on a fixed tick, whatever the scan delivers: a
dropped link fails the run (`link dropped while the phone was trying to connect (second central)`),
page 25 silent for 2 s aborts it (`no page 25 for 2 s while the phone was trying to connect: the second
central may have taken the trainer`; the idle NEO sends page 25 at about 2.8 Hz, Part A), and the
trainer is released as on every other path. A run that ends this way gives no verdict. Both messages
name the phone, since it is the likely cause: a phone that takes the trainer either drops crank's link
or leaves it up with page 25 stopped.

**Completed** means the window ran to its close with the answers it asked recorded; only then is there a
verdict (`advertising`, `second_central` and `verdict` are null otherwise, as for every Part B check).

**Capture.** The events are tagged comment lines in the `.fecap` (`Capture::record_aux`), so `analyse`
reproduces the row from the file: `# check10 <us> 01|02` (window opened, closed), `# scan <us> 01|00|02`
(scan started, ended by itself, stopped by the harness), `# adv <us> 01|00` (an advertisement of the
trainer, of another peripheral) and `# answer-listed|answer-connect <us> <ASCII>`. The scan opens
before the window and stops after it, so its stamps bracket the window.
Readers that skip comments read the file as before. The lines carry no serial; `redact` keeps them.

**Result** (`check10.rs` `check10`), from the window's events only:

- **advertising while connected:**
  - `continues: the scan saw N advertisements of the trainer, longest gap X s` when the scan reported at
    least one inside the window. The gap is measured in continuous time, the window's edges included.
  - `continues: the phone listed it; the scan saw none (…)` on the phone's `seen` alone.
  - `not seen: …` only when both observations agree: the phone's `not-seen`, and a scan that ran over
    the whole window and reported other peripherals (so it was working) but none of the trainer. The
    scan ran over the whole window only when both its stamps are present, it opened at or before the
    window's open, stopped at or after its close and did not end by itself (`check10::Scan::of`).
    `not-seen` with any other scan (not started, ended by itself, started late, stopped early, no close
    line) or one that reported nobody is `unknown: the phone did not list it, but the scan …, so it
    cannot confirm that`.
  - Otherwise `unknown: …`, with the scan's state (as above) and how many advertisements of other peripherals it reported. A scan that saw nothing
    of the trainer is not evidence that advertising stopped: a host may not report a peripheral it is
    connected to (CoreBluetooth is not documented either way), and a scan that reports other peripherals
    only shows that it was working.
- **second central**, a decision table over the two answers (`check10::second_central`); only a phone
  that listed the NEO can have been accepted or refused:

  | listed | connect | second central |
  |---|---|---|
  | `seen` | `connected` | `accepted: the phone connected while hil_fec held the link` |
  | `seen` | `failed` | `refused: the phone's connect failed` |
  | `seen` | `stuck` | `refused: the phone's connect never completed` |
  | `seen` | `not-tried` | `unknown: the phone listed the NEO but did not try to connect` |
  | `seen` | missing or unrecognised | `unknown: <reason> to whether the phone connected` |
  | `not-seen` | not asked | `unknown: the phone did not find the NEO, so it could not try to connect` |
  | `unsure` | not asked | `unknown: the operator was unsure the phone listed the NEO, so no connect was tried` |
  | `not-seen` or `unsure` | recorded anyway (an edited capture) | `unknown: contradictory answers: …` |
  | missing or unrecognised | any | `unknown: <reason> to whether the phone listed the NEO` |

  So `accepted` and `refused` always come with advertising `continues`.
- **hil_fec link:** `held` when page 25 has no gap above 2 s inside the window (edges included, ADR-0002
  §6), else `lost (page 25 silent for X s)`.

The row reads `second central …; advertising while connected …; hil_fec link …`. A `connected` answer
contradicts ADR-0002 §5 and is the trigger to amend it.

### Check 11b: the unplugged stop (dropout and reconnect)

The NEO has no battery. Unplugged, the rider's pedalling powers it, and it shuts down shortly after
pedalling stops (#44, owner hint). Stopping until the lights go out is therefore the realistic dropout: the
link drops, and pedalling again brings the trainer back. `check11b` measures that with the trainer unplugged
from the start ([check 11a](#check-11a-unplug-while-pedalling) unplugs it while pedalling).

**Procedure** (`rider.rs` `check11b`):

1. ERG at **100 W** by soft start, 20 s at 90 rpm, under the usual guards.
2. The release (page 48 at 0 %), then the cue to stop and wait until the lights go out. A stopping rider under
   ERG is what the §7.4 rule releases, so the release comes first. From here page-25 silence is expected, so
   only the link counts: it must drop within **90 s** of the cue (`check11::DROP_WAIT`), otherwise the run is
   aborted (`the link did not drop within 90 s of the stop cue (is the trainer unplugged?)`).
3. Once the drop is seen, the cue to pedal again (the trainer needs it to boot), and a scan for the trainer.
   The reconnect must reach its first page 25 within **60 s** of the drop (`check11::RECONNECT_WAIT`);
   each phase is stamped (below).
4. **No silent resume.** The first write after the reconnect is the release (page 48 at 0 %), before
   anything else. Then a page-81 request: the serial must match the one from before the drop
   (`check11::same_trainer`; the live connect also compares the DIS serials), otherwise the run is aborted,
   released (`a different trainer answered after the reconnect …`). Then the operator answers *When the
   link dropped, had the trainer's lights gone out?* (`out` / `on` / `unsure`) and must type `y` (stamped)
   before ERG is engaged again, by soft start, as at a ride start (ADR-0002 §6 step 2). 20 s at 90 rpm, then
   the usual release. The guards watch the new link from its first page 25.

**Completed** means the drop was seen and either the trainer came back and held the target for 20 s, or the
60 s bound ran out: "no reconnect within 60 s" is a result, not a failure (the final release then fails with
`not connected`, which the summary shows). A reconnect that fails (the scan ends, the connect fails) fails
the run with its cause, stamped `07`. No `y` after the reconnect, no answer, another trainer, or a guard stop
gives no result.

**Capture.** `# check11b <us> 01|02` are the stop cue (the trainer already released) and the operator's `y`
after the reconnect. The link events are `# link <us> <code>`: `01` the drop seen, `02` the trainer
advertised again, `03` connected (live, the adapter's `DeviceConnected` event, stamped while the connect runs
and up to 1 s after, never later, so connect and discovery are timed apart), `04` services discovered and
page 25 subscribed, `05` the first page 25 after the reconnect, `06` the bound ran out, `07` the reconnect
failed. The lights answer is `# answer-lights`.

**Validation** (`check11::validate`, the same for the live row and `analyse`). The stamps must follow the
procedure, or the row is `unknown: <the breach>`:

- the link events as a state machine: `01` first; then `02`, `03` (optional: the adapter's event is not
  guaranteed), `04`, `05` in that order; `06` or `07` may end it after any of them; `05`, `06` and `07`
  exclude each other and end the sequence; never back in time;
- the drop within 90 s of the stop cue (plus 0.2 s for the 100 ms poll); `05` within 60 s of the drop; `06`
  at the 60 s bound: not before it, and at most 1 s after it (the procedure's timer fires at the bound);
- no control write but the release from the stop cue to the `y`, or to the end of the capture without one:
  `unknown: <write> at <t> s after the stop cue, before the confirmed resume`. Every tx page 48–51 and 55
  counts, a malformed one too (`Malformed(51)`); only page 48 at exactly 0 % is the release;
- the `y` only after `05` and after the release that follows `05`;
- after `05`, one page-81 serial on each side of the drop, the same (a redacted capture compares its
  redacted serials, which are alike; the live run compared the real ones).

Duplicate or out-of-order stamps of check 10's window and scan are `unknown: contradictory …` the same way
(`frames::stamps`).

**Result** (`check11.rs` `check11b`):

- **drop:** seconds from the stop cue to the drop, and how long after the cue the trainer's last page 25
  came (how long it ran on, unpowered).
- **reconnect:** seconds from the drop to the first page 25, with each phase from the drop (seen,
  connected, discovered); a phase without its stamp is `unknown`. Or `none within 60 s of the drop (…)`.
- **after the reconnect:** the FE state, target-limit code and power of the trainer's first page 25 (it
  comes before any write: what the trainer is doing on its own after the power cycle), whether the first
  write after it was page 48 at 0 % (the no-silent-resume rule), and the mean power from 5 s after the
  re-engaged target to the release.
- **cause** (decision table over the lights answer): `out` → `power-off (the lights went out)`; `on` →
  `unknown: the link dropped with the lights still on, not a power-off`; `unsure` → `unknown: the operator
  was unsure about the lights`; missing or unrecognised → `unknown` with the reason.

The row reads `dropout <cause>; detected … after the stop cue (…); reconnect …; after it …`.

### Check 11a: unplug while pedalling

Pulling the plug while the flywheel turns should keep the trainer, and its link, alive on the rider's power
(#44). `check11a` records whether it does and what page 25 says about it.

**Procedure** (`rider.rs` `check11a`): ERG at **100 W** by soft start and 20 s at 90 rpm under the usual guards;
the cue to pull the plug while pedalling on (stamped `# check11a <us> 01`); the question *Is the trainer's mains
plug out now?* (`unplugged`, `cannot`; within 3 min, the guards running, the answer stamped
`# answer-unplugged`); 30 s more at 100 W; the release. `cannot` aborts the run, released. Nothing is written
from the cue to the release.

- **A drop is the finding**, not a failure: it is stamped `# link <us> 01`, the question is asked (unguarded,
  the link is down) if it is still unanswered, and the run completes. Its release then fails with
  `not connected`, as in 11b; the trainer, if it is still powered, holds 100 W until the rider stops.
- Page 25 silent for 2 s is the guard's stop, but a powered-off trainer goes silent before its link is reported
  down: the link drops only when the BLE supervision timeout runs out, at most **32 s** (the Core Specification's
  bound on `connSupervisionTimeout`, Vol 6 Part B §4.5.2; Apple's guideline ranges have differed between
  releases). So the harness sends the release at once (bounded, as every release), cues *the trainer went
  silent* (or, when that release failed or hung, *the trainer went silent and the release was NOT confirmed*,
  with the instruction to stop pedalling, and the error is added to the abort reason) and waits up to 32 s for
  the drop: a drop is the
  finding as above; no drop aborts with `no page 25 after the unplug cue, and the link did not drop within 32 s
  (the BLE supervision-timeout maximum)`.
- The final cue says whether the release was confirmed: `done: trainer released` or `done, but the release was
  NOT confirmed: the trainer may still hold its target …` (every check; after an 11a or 11b drop it is the
  expected one).

**Validation** (`check11ac::check11a`, live and offline), else `unknown: <the breach>`:

- the unplug cue once; at most one `link 01`, not before the cue, and no trainer frame at or after its µs (a drop
  is only a drop when the frames end there, and the stamp is written when the drop is reported, after the last
  frame; tx frames are recorded even when the write fails, so only rx counts);
- one answer, after the cue (two recorded answers are contradictory);
- with a drop: the drop at or before the closing release (a drop stamped after it is contradictory); a release
  before the drop only as the silence guard's: at least `rider::SILENCE_US` (2 s, the live guard's constant)
  after the last page-25 frame, and no control write from it to the drop (a write at the drop's µs is after it);
- ERG 100 W in force: the last write before the before window (15 s before the cue);
- no control write at all from that window's start to the first release, and that release after the cue: a
  release at or before the cue (its µs included) changes the target the before window is read under. The after
  window ends at that release (the silence guard's comes early). No release after the window start means the
  capture is cut off: `unknown`, as a live run always records its release;
- evidence, from check 5's floor of 2 page-25 samples a second: at least 30 samples in the 15 s before the cue;
  without a drop, the answer before the release and at least 40 samples over at least 20 s after it (the
  procedure rides 30 s after the answer, 25 s once the first 5 s are left out; 5 s of slack for the answer's and
  the release's timing). **Unplugged** is a decision table: `unplugged` → a result; `cannot` →
`unknown: the operator could not unplug the trainer`; missing or unrecognised → `unknown` with the reason.

**Result:** the link (`no link drop in X s unplugged` or `link dropped X s after the unplug cue`) and two
windows of page 25: **before**, the 15 s up to the cue; **after**, from 5 s after the answer to the release
(or the drop). Each gives its sample count, the FE states, target-limit codes and status bits seen, the mean
power and its share within ±10 % of the target, and the mean cadence. The row reads `<link>; page 25 before →
after: FE states …, target limits …, status bits …, mean power … W (ERG 100 W)`.

### Check 11c: the unplugged cold start

Unplugged and off, the NEO powers up only when the rider pedals. `check11c` times the cold start from the cue
to the first ERG echo, and asks whether ERG targets are honoured at low cadence; if they are not, the following
machine should report `not_following`, not a fault (#44, FMEA FM-START-07, Q7).

**Procedure.** Before the link (`rider.rs` `cold_start`; `checks.rs` opens the capture first): the operator
confirms the trainer is unplugged and off; the start cue (stamped `# check11c <us> 01`); a scan for an FE-C
advertisement, at most **60 s** (`# link <us> 02` when seen); the connect, at most **30 s** (`# link <us> 04`
when connected, discovered and subscribed). Nothing can be written before the link, so a bound that runs out
aborts (`# link 06`), and a scan that ends or a connect that fails fails the run (`# link 07`); nothing is
released because nothing was applied. Then as every run: the release, the page-81 request, the operator's
`y`; then `check11c`: page 49 at 0 W and a page-70 request for page 71 (its echo), the soft start to **100 W**,
the cue to **60 rpm** (stamped `02`) and 20 s, the cue to **90 rpm** (`03`) and 20 s, the release. The times
count from the cue, so they include the moment the cue takes to reach the rider.

**Validation** (`check11ac::check11c`, live and offline): the start cue, then `02` within 60 s of it and `04`
within 30 s of `02` (each plus 0.5 s for the stamp), once each and in order, nothing else on `link`; the cues
in order; no frame before the advertisement; the first control write the release, and a release ends the run (none:
the capture is cut off); else `unknown: <the breach>`.

**Result:**

- **times:** from the start cue, `advertising`, `connected`, the first page 25, and the first ERG echo (the
  first page 71 naming page 49 after the first page-49 write), with the echo's delay after its write.
- **60 rpm / 90 rpm** (decision table), page 25 from 5 s after each cue to the next cue or the release. A write
  inside the window, or a target in force other than ERG 100 W, → `unknown`; fewer than 10 samples, no
  cadence, or a mean cadence more than 10 rpm from the cue → `unknown`; at least 80 % within ±10 % of 100 W →
  `honoured`; otherwise `not honoured, not_following` with the mean power, the share and the target-limit codes
  seen.

The row reads `from the start cue: advertising +… s, connected +… s, first page 25 +… s, first ERG echo +… s
(… s after its page 49); ERG 100 W at 60 rpm …, at 90 rpm …` (`no ERG echo (…)` when none came).
