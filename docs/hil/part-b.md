# HIL session 1, Part B runbook: control checks with a rider (#44, #412)

How to run the Part B pedalling checks of ADR-0002 §9 with `hil_fec`. This covers the control checks
6, 7, 8, 9 and 12. Check 5 (page 25 against CPS `2A63`) and checks 10/11a–c (second central, dropout)
follow in their own PRs (#412 parts b and c). Results go to a new `docs/hil/<date>-fec.md` in the
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
named `check6`, `check7`, `check8`, `check9` or `check12`, and prints the row again.

| Check | What `result` reports (method text is in the JSON) | ADR-0002 |
|---|---|---|
| 6 | Per ERG step (a page-49 target held ≥ 10 s after another): t90 from the write to the first page-25 sample past 90 % of the step. Per cadence crossing of 75 rpm inside a hold: largest power deviation in the next 10 s and the time until power stays within ±5 % for 3 s | §7.2 (step response, "lead") |
| 7 | From the procedure's 150 W write (the end of the soft start) to the next write, after its first 10 s: share of samples within ±5 %/±10 % of the target and the last 10 s mean; held = ≥ 90 % within ±10 % and last 10 s within ±5 % | §4.4 (no keep-alive) |
| 8 | Per SIM state held ≥ 10 s: mean power, page-16 speed and cadence, and the road model at that speed (`v·m·g·(grade + Crr) + cw·(v + wind)²·drafting·v`, m from page 55): the ratio shows whether grade and mass apply | §7.9 (SIM, page 55) |
| 9 | How the descent ended (`stalled`, `floor`, `departed`), the stall cadence as a range, power by 5 rpm cadence bin and the lowest bin still followed (≥ 2 samples, ≥ 80 % within ±10 % of 250 W); the verdict compares the stall with the 40 rpm release: below it the release comes first, at or above it too late, across it ambiguous | §7.4 |
| 12 | Mean power below 80 rpm after `Free` against the ERG target (released = below 90 %: ERG would hold it) and whether page 71 echoed pages 50 and 51 | §7.9 (`Free` sent explicitly) |

Every result also carries `release`: whether page 48 at 0 % was the last control write and whether a
page-71 reply echoed it.

### Check 9: what counts as a stall

Check 9 measures the trainer: down to which cadence the NEO's ERG holds 250 W while the rider follows
the cue. The cue starts 10 s after the 250 W write at 90 rpm and falls 5 rpm every 10 s (0.5 rpm/s). The
live run and `analyse` apply the same rule (`partb::descent`) to the page-25 samples:

- **Following the cue:** the cadence never falls more than 9 rpm within 3 s (3 rpm/s, six times the cue's
  rate, so a 5 rpm step made over a second or two passes) and stays no more than 15 rpm above the cue
  (30 s behind it). At 3 rpm/s a power lag of up to 1 s blurs the stall cadence by at most 3 rpm, less
  than a 5 rpm bin.
- **Stall:** power below 90 % of 250 W for 2 s (the 2 s of the §7.4 rule) while the cadence still
  follows, from 1 s before the drop. The stall cadence is the range over that second. If the range spans
  40 rpm, the verdict says it is ambiguous rather than picking a side.
- **Floor:** the first sample below 25 rpm with power still held ends the run as `no stall down to …`.
- **Anything else aborts the run:** a cadence that falls faster, unavailable cadence or power, or falling behind
  the cue. The trainer is released, the outcome is `aborted: <why>`, and there is no verdict. Power and
  cadence falling together cannot be told apart from page 25. It could be a rider who stops on purpose or
  one the trainer overpowers, and neither gives a stall cadence. In both cases the abort reason gives the
  cadence it was seen at.
