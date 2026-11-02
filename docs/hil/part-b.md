# HIL session 1, Part B runbook: control checks with a rider (#44, #412)

How to run the Part B pedalling checks of ADR-0002 §9 with `hil_fec`. The owner's step-by-step session
script (app flow plus these checks) is [part-b-runbook.md](part-b-runbook.md). This covers check 5 (page 25
against the Cycling Power measurement `2A63`) and the control checks 6, 7, 8, 9 and 12. Checks
10/11a–c (second central, dropout) follow in their own PR (#412 part c). Results go to a new `docs/hil/<date>-fec.md` in the
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

A run that stops early prints `STOPPED: <why> (trainer released)` and exits with status 4; its capture and
summary are still written and analysable, but give no result (below).

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

Each run writes `DIR/<check>.fecap` (both directions, the `.fecap` format) and `DIR/<check>.json` with the
procedure's outcome, the release, the identity, and `result` (below), and prints the result row.

## Results and re-analysis

`result` is computed only from the capture, so it is reproducible from a committed, redacted capture.
After the link is closed, `hil_fec` appends the procedure's outcome as the capture's last line,
`# hil_fec procedure: completed` (or `aborted: …`, `interrupted: …`, `failed: …`). Only a completed
procedure gives a result: for any other outcome, or a capture without that line, the row reads
`no result: procedure …` and `held`/`released`/`verdict` are null.

```text
hil_fec redact  --target RAW/check6.fecap --out docs/hil/captures
hil_fec analyse --target docs/hil/captures/check6.fecap --out docs/hil/captures   # offline
```

`analyse` keeps the reply accounting it always wrote and adds `part_b` (the same `result`) for a capture
named `check5`, `check6`, `check7`, `check8`, `check9` or `check12`, and prints the row again.

| Check | What `result` reports (method text is in the JSON) | ADR-0002 |
|---|---|---|
| 5 | Page-25 arrival rate, and page-25 power and cadence against `2A63` over the settled holds ([below](#check-5-page-25-against-cps-2a63)): seconds compared, mean difference, largest difference; verdict `agree` or `disagree: …` | §5 (telemetry source), §9 |
| 6 | Per ERG step (a page-49 target held ≥ 10 s after another): t90 from the write to the first page-25 sample past 90 % of the step. Per cadence crossing of 75 rpm inside a hold: largest power deviation in the next 10 s and the time until power stays within ±5 % for 3 s | §7.2 (step response, "lead") |
| 7 | From the procedure's 150 W write (the end of the soft start) to the next write, after its first 10 s: share of samples within ±5 %/±10 % of the target and the last 10 s mean; held = ≥ 90 % within ±10 % and last 10 s within ±5 % | §4.4 (no keep-alive) |
| 8 | Per SIM state held ≥ 10 s: mean power, page-16 speed and cadence, and the road model at that speed (`v·m·g·(grade + Crr) + cw·(v + wind)²·drafting·v`, m from page 55): the ratio shows whether grade and mass apply | §7.9 (SIM, page 55) |
| 9 | How the descent ended (`stalled`, `floor`, `departed`), the cadence at which power fell below 90 % and the lowest cadence it was settled at or above 90 % (the two bounds below), power by 5 rpm cadence bin and the lowest bin still followed (≥ 2 samples, ≥ 80 % within ±10 % of 250 W); the verdict: the release comes first, too late, or ambiguous | §7.4 |
| 12 | Mean power below 80 rpm after `Free` against the ERG target (released = below 90 %: ERG would hold it) and whether page 71 echoed pages 50 and 51 | §7.9 (`Free` sent explicitly) |

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
`redact` keeps them as they are. A device without `2A63` ends the run before anything is written:
`BLOCKED: check 5 needs Cycling Power 2A63 on the NEO`.

**Time alignment.** Both streams are stamped on arrival by the same process on the same monotonic
clock, so there is no offset between them to estimate. What remains is each stream's own jitter and
that `2A63` is slower than page 25, so the comparison avoids transients instead of correcting them:
only the **settled windows** are used, each page-49 target held ≥ 10 s (check 6's definition of a hold)
after its first 10 s. Under ERG the power is steady there, so a few hundred milliseconds of misalignment
are not mistaken for disagreement. Read back from the capture's own tx writes, they are the same live and offline.

**Power.** For every whole capture second inside a window that has at least one sample of each stream:
the mean page-25 power minus the mean `2A63` power. Page 25 power is the instantaneous 12-bit field;
`2A63` is the signed 16-bit instantaneous power. The tolerance is the ADR's: every compared second
within **±1 W** (exactly ±1 W passes), of at least **20** (`MIN_SECONDS`; fewer is `insufficient`, a
verdict needs a ride, not a glimpse). The result also gives the mean difference (`bias_w`), the
largest, and how many seconds were within the tolerance.

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
