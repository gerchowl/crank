# HIL session 1, Part B runbook: control checks with a rider (#44, #412)

How to run the Part B pedalling checks of ADR-0002 §9 with `hil_fec`. This covers the ERG control
checks 6, 7 and 12. Checks 8 and 9 (SIM grades, low-cadence stall), check 5 (page 25 against CPS
`2A63`) and checks 10/11a–c (second central, dropout) follow in their own PRs (#412 parts a2, b and c). Results go to a new `docs/hil/<date>-fec.md` in the
format of [the Part A results](2026-10-14-fec.md): one "Results at a glance" row per check, which each
run prints.

## Safety: what every run does

The checks drive a real trainer under a rider, so every subcommand (`crates/crank-ble/examples/hil_fec/rider.rs`):

- connects into the release state, page 48 at 0 %, and writes nothing else until the operator presses
  Enter in the window;
- engages ERG as the engine does at a ride start (ADR-0002 §7.5): page 49 at 0 W until the first page-25
  sample with cadence above 0, then a 5 s ramp in 1 Hz steps from the 3 s power to the target;
- never commands more than 250 W or a grade steeper than 10 % (the #44 values);
- while ERG is applied, stops on the §7.4 release rule (cadence below 40 rpm for 2 s);
- stops on 2 s without page 25 (ADR-0002 §6), a dropped link, a failed write, or a rider who does not
  start pedalling within 120 s;
- on completion, on any stop, on a panic, and on SIGINT (Ctrl-C), SIGTERM or SIGHUP (the window closed),
  writes page 48 at 0 % (the ride-end mode of ADR-0002 §7.9) and a page-70 request for page 71, so the
  capture shows whether the trainer took it, then disconnects.

A run that stops early prints `STOPPED: <why> (trainer released)` and exits with status 4; its capture and
summary are still written and analysable.

## Running

Bluetooth only works from an Alacritty window on sage (AGENTS.md, TCC). Build once in `nix develop`, with
your own `CARGO_TARGET_DIR`:

```text
cargo build -p crank-ble --features hil --example hil_fec
```

Then one wrapper per check, launched with
`alacritty msg --socket "$(ls -t $TMPDIR/Alacritty-*.sock | head -1)" create-window -e /abs/path/check6.sh`.
The window shows the cues; the owner presses Enter there to start. The log file is for the agent:

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
| 12 | `hil_fec check12 --out DIR` | 90 rpm at 200 W ERG; the harness switches to `Free` (page 50 defaults, page 51 grade 0); after 10 s, cue to 70 rpm for 15 s | 1 min |

Each run writes `DIR/<check>.fecap` (both directions, the `.fecap` format) and `DIR/<check>.json` with the
procedure's outcome, the release, the identity, and `result` (below), and prints the result row.

## Results and re-analysis

`result` is computed only from the capture, so it is reproducible from a committed, redacted capture:

```text
hil_fec redact  --target RAW/check6.fecap --out docs/hil/captures
hil_fec analyse --target docs/hil/captures/check6.fecap --out docs/hil/captures   # offline
```

`analyse` keeps the reply accounting it always wrote and adds `part_b` (the same `result`) for a capture
named `check6`, `check7` or `check12`, and prints the row again.

| Check | What `result` reports (method text is in the JSON) | ADR-0002 |
|---|---|---|
| 6 | Per ERG step (a page-49 target held ≥ 10 s after another): t90 from the write to the first page-25 sample past 90 % of the step. Per cadence crossing of 75 rpm inside a hold: largest power deviation in the next 10 s and the time until power stays within ±5 % for 3 s | §7.2 (step response, "lead") |
| 7 | Over the longest stretch without any write, after its first 10 s: share of samples within ±5 %/±10 % of the target and the last 10 s mean; held = ≥ 90 % within ±10 % and last 10 s within ±5 % | §4.4 (no keep-alive) |
| 12 | Mean power below 80 rpm after `Free` against the ERG target (released = below 90 %: ERG would hold it) and whether page 71 echoed pages 50 and 51 | §7.9 (`Free` sent explicitly) |

Every result also carries `release`: whether page 48 at 0 % was the last control write and whether a
page-71 reply echoed it.
