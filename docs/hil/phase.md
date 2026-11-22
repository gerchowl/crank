# `hil_fec phase`: page-16 suppression against the page-71 request's phase (#610)

**Why.** The NEO 55624 (fw 0.7.4) stops broadcasting FE-C page 16 (speed, distance) for 22–25 s while
crank's page-70 request for page 71 keeps a particular relation to the page-16 slot. In the owner's
ride of 2026-11-08 this blanked speed for 21–22 s every ~132 s (#610). The diagnosis is in
[#610](https://github.com/gerchowl/crank/issues/610#issuecomment-7079270551) and in
ADR-0002 §4.4, amendment "phase-locked page-71 request" (proposed). This check measures the
offsets after page 16 at which a request is safe. The production poll (PR 2 of #610) is then locked
to one of them.

**What it does** (`crates/crank-ble/examples/hil_fec/phase.rs`). It connects as the other Part A
checks do and writes no control page. For each offset `D` of 0, 50, …, 950 ms, plus 175, 225, …,
425 ms near the edges `latency.fecap` showed (26 offsets), it writes one page-70 request for page
71 `D` after each received page 16. While no page 16 arrives, it writes once per 1 s, still `D`
after the last page-16 slot (`PhaseLock`, the same fallback the amendment proposes). Each offset
runs for 40 s. In `latency.fecap` the outages began 20.8–23.8 s after the request took the relation
that preceded them, so 40 s is 1.7 × the longest onset. Each offset's start is stamped in the
capture as `# phase <µs> <offset ms, u16 LE>`. The first 3 s of each offset are not counted.

**Duration:** 26 × 40 s = 17 min 20 s, plus up to 30 s scan and connect: **about 18 minutes**.
No pedalling. The trainer must be awake (mains plugged in) and no other app may be connected (phone
and Tacx app off). Only one central at a time.

## Running

Bluetooth only works from an Alacritty window on the Mac Studio (AGENTS.md, TCC). Build once in
`nix develop`, with your own `CARGO_TARGET_DIR`:

```text
cargo build -p crank-ble --features hil --example hil_fec
```

Wrapper `phase.sh`:

```sh
#!/bin/bash
# phase.sh: the #610 phase sweep; output to a log the agent reads.
OUT=/abs/path/to/raw-captures/phase-$(date +%Y%m%d-%H%M)   # outside the repository: raw captures carry the serial
mkdir -p "$OUT"
"$CARGO_TARGET_DIR/debug/examples/hil_fec" phase --out "$OUT" 2>&1 | tee "$OUT/phase.log"
echo "exit ${PIPESTATUS[0]}"; sleep 30
```

Launch it with
`alacritty msg --socket "$(ls -t $TMPDIR/Alacritty-*.sock | head -1)" create-window -e /abs/path/phase.sh`.
The window prints `phase: offset <D> ms for 40 s` per step. Leave the trainer alone until it ends.

## Result

`<out>/phase.json` holds, for each offset (`steps`): requests, page-71 replies and page-16 frames
counted after the settle time, the longest page-16 gap (`max_page16_gap_ms`) and a verdict. An
offset is **safe** when page 16 kept arriving with no gap over 2.5 s (more than two missing page-16
periods); otherwise **unsafe**. `safe_bands_ms` lists the runs of adjacent safe offsets, widest
first. The last line printed before `wrote …` is the verdict:

- **PASS:** `PASS: safe band A–B ms after page 16 (centre C ms); …`, where the widest band is at least
  150 ms wide. That leaves 75 ms to either edge, against BLE delivery quantised to the ~30 ms
  connection interval. PR 2 sets the amendment's offset to `C`. It also re-derives the reply-gap
  distribution and `T_unknown` from this capture before the amendment is accepted.
- **FAIL:** no band that wide, or none at all. The amendment as drafted does not hold; report the
  steps table on #610 for a design decision.

The same JSON carries `outages` (page-16 gaps of 6 s or more, with the request offsets at their start
and end) and `free_running_bins` (requests binned by 50 ms of offset, for captures without steps).
`hil_fec analyse --target <capture> --out DIR` recomputes the same analysis offline for any capture.
It goes under `page16_phase` in `<stem>.analysis.json`, and the verdict line is printed when the
capture has sweep steps.

Redact before committing (`hil_fec redact --target <out>/phase.fecap --out docs/hil/captures`; page 81
carries the serial) and commit `phase.fecap` and `phase.json` next to the Part A captures.

## Provisional band from `latency.fecap` (free-running 1 Hz polls, #44 Part A)

`phase::tests` assert these on the committed capture:

- Page-16 period 0.99 s (the median gap between 0.8 and 1.2 s). Two outages: 25.23 s from
  108.62 s (ERG) and 22.23 s from 240.62 s (SIM). The request offsets inside them run from
  192–193 ms to 393–424 ms (extrapolated from the last page-16 slot).
- Unsafe bins (< 50 % of requests followed by page 16 within 1.5 s): 250–400 ms, the outage band;
  900–950 ms, where the reply takes page 16's slot and page 16 thins out to about one per 2 s.
- Safe bins (≥ 90 %): 650–750 ms. The capture covers only the offsets the request happened to sit
  on (plateaus near 200, 470, 500, 720 and 980 ms), so the sweep is what decides.
