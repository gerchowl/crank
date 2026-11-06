# Part B riding session: owner runbook (#44, #256)

Step by step for the owner's riding session: the app MVP flow (#256) and the `hil_fec` trainer control
checks. What each check measures and how results are computed is in [part-b.md](part-b.md); this page is
only what to do. Plan about 15 min for the app part and about 15 min of pedalling for the checks.

## 0. Before you start

- **Run everything on the `sage` host, at the desk.** Not in the sage-ws VM: it has no Bluetooth
  (`APPLE_VIRTUAL`, address NULL). `sage` is the Mac with the `nix-tcc` signing identity (`mba22` has none; see
  the last section).
- **Disk space:** `df -h /System/Volumes/Data` should show about 25 GB free. The host's disk holds the VM's
  disk image, so a full host disk fails nix builds in the VM with "No space left on device" while the VM
  reports free space. Free space on the host first (`nix-collect-garbage -d`, old VM images).
- **Checkout:** current `dev` in your crank checkout (`git pull --ff-only`); it must include #423 (check 9 trusts
  check 6 only from the same trainer and session). No checkout: `git clone https://github.com/gerchowl/crank`
  and `cd` into it and check out `dev`.
- Close the Tacx and Movesense apps on your phone: the NEO accepts one central at a time.
- Put the HR strap on and wet it. It must advertise Heart Rate (service `0x180D`).
- Every run releases the trainer by itself on completion, on any of its stop rules, on Ctrl-C and when you close
  the window. The 40 rpm rule (cadence below 40 rpm for 2 s) applies only while ERG is on: check 8 runs in SIM
  mode and check 9's descent has its own end rules (part-b.md, "Safety").

## 1. Build and install the app

In Alacritty on `sage`, at the desk (ssh sessions have no signing identity):

```sh
git pull --ff-only && nix develop -c just bundle
```

`just bundle` builds `.#crank-app`, signs it with the `nix-tcc` identity and installs it as
`~/Applications/crank.app` (AGENTS.md; `scripts/install-app.sh`). It fails with "no valid nix-tcc
codesigning identity" over ssh or on a host without the identity.

## 2. App part (steps 1–8)

Write down for each step whether it worked as expected.

1. Open `~/Applications/crank.app` from Finder (or with recording, below). When asked, **allow Bluetooth**. This
   must be the only time.
2. **Settings → Devices:** press **Scan**, then **Pair as trainer** on the NEO and **Pair as strap** on the
   strap.
3. **Workouts → Sensor check (3 min)** (Warm-up 1 min 50 %, Work 1 min 80 %, Cool-down 1 min 40 % of FTP) →
   **Ride this** → **Start**, then pedal.
4. During the ride: **Pause** and **Resume**, **Skip block** once, then **End**, then **End ride**.
5. **History:** is the ride there? Do power, cadence and heart rate look plausible?
6. **Grant check (#44):** quit crank fully, run `nix develop -c just bundle` again, relaunch the same way as
   in step 1 (when recording, with the same `CRANK_CAPTURE_DIR=$CAP open …` line: a Finder launch drops the
   variable, and `open` ignores it on an app that is still running). Expect **no** Bluetooth prompt. Use the bundle for this, not `hil_fec` or any CLI.
7. Revoke Bluetooth once (System Settings → Privacy & Security → Bluetooth → crank off) and relaunch the same way (quit fully first). Devices
   should say **Bluetooth access is denied** and how to fix it; then switch it back on (#43).
8. Ride a few minutes more and look for: the **HR lost** banner (strap off for about 10 s, then back on);
   whether the numbers are readable from the saddle; anything that looks wrong.

**Recommended: record the ride.** Start the app from a terminal instead of Finder in step 1:

```sh
CAP=~/crank-captures/$(date +%F)-app
CRANK_CAPTURE_DIR=$CAP open ~/Applications/crank.app
```

With `CRANK_CAPTURE_DIR` the app writes its log to `$CAP/logs.jsonl` (INFO, and `crank_ble` at TRACE) and one
`trainer-<unix secs>-<n>.fecap` per trainer connection (AGENTS.md; `app/src-tauri/src/logging.rs`). If the
directory cannot be opened the app says so and exits rather than riding unrecorded. Without it the log is
`~/Library/Logs/io.github.gerchowl.crank/logs.jsonl` (8 MiB segments, rotated to `logs-<UTC stamp>.jsonl`). The
log is OTLP/JSON Lines, one `LogsData` per line, so read it with `jq`; `fromjson?` skips a partial last line
left by a quit or crash:

```sh
jq -cR 'fromjson? | .resourceLogs[].scopeLogs[].logRecords[] | [.timeUnixNano, .body.stringValue]' "$CAP/logs.jsonl"
```

(The plain-text `crank.log` of older builds is no longer written.) Captures hold raw device frames (the trainer serial):
redact before committing, share nothing unreviewed.

## 3. `hil_fec` checks

Build once, in `nix develop` (use your own `CARGO_TARGET_DIR` on a shared host):

```sh
nix develop -c cargo build -p crank-ble --features hil --example hil_fec
OUT=~/crank-captures/$(date +%F)-partb
mkdir -p "$OUT"
```

Run the checks **in this order (5, 6, 7, 8, 9, 12, then 10, 11a, 11b and 11c), with the same `--out`**, from an Alacritty window on `sage`. The binary is
`${CARGO_TARGET_DIR:-target}/debug/examples/hil_fec`:

| # | Command | You do | About |
|---|---|---|---|
| 5 | `hil_fec check5 --out "$OUT"` | 90 rpm through ERG 150 W then 200 W (30 s each); steady cadence is all that is asked | 1.5 min |
| 6 | `hil_fec check6 --out "$OUT"` | 90 rpm through ERG 100 → 150 → 250 → 100 → 200 W, then 60 rpm and back | 2.5 min |
| 7 | `hil_fec check7 --out "$OUT"` | 90 rpm at 150 W; the harness stays silent for 40 s | 1 min |
| 8 | `hil_fec check8 --out "$OUT"` | Easy gear, steady; grades 0 / 5 / 5 / 10 %, shift down when steep | 2.5 min |
| 9 | `hil_fec check9 --out "$OUT"` | 90 rpm at 250 W, then follow the cue down about 5 rpm every 10 s; keep turning | up to 3 min |
| 12 | `hil_fec check12 --out "$OUT"` | 90 rpm at 200 W; it switches to Free; at the cue go to 70 rpm | 1 min |

- Each run waits until you type **`y`** (or `yes`) and Enter in its window. Anything else aborts.
- Follow the on-screen cues. A run that stops early exits with status 4 and gives no result. Read its last line:
  `STOPPED: … (trainer released)` means the release was confirmed. `STOPPED: … (release NOT confirmed: …)` means
  the trainer may still hold its target: stop pedalling; if it still resists, unplug it and wait until its lights
  go out.
- A run that completes but whose release was not confirmed (expected after an 11a or 11b drop: the link is down)
  ends on `DONE, but the release was NOT confirmed: …` with the same instruction, and exits with status 5; its
  result stands.
- Exit status: 0 done, 1 error, 2 bad arguments, 3 blocked (the owner must act), 4 stopped early (no result),
  5 completed with the release NOT confirmed.
- Each run refuses to overwrite its own `.fecap` (`File::create_new`): re-run a stopped check into a **new**
  `--out` directory. For a stopped check 9 that directory has no `check6.fecap`, so first re-run check 6 into
  it, or copy `check6.fecap` over from the old one (valid for the same trainer within 2 h, `SESSION_BOUND_S`).
- **Check 9 depends on check 6.** It reads `check6.fecap` from the same `--out`, so run check 6 first, into
  that directory, on the **same trainer** and in the same session (page-81 serial match, check 6 ended at most
  2 h before check 9; #423). Otherwise check 9's "release comes first" verdict reads "not measured". If
  check 9 runs in a different directory, check 6 must be there too.
- Check 9: lower your cadence smoothly as the cue says. Stopping abruptly is safe but gives no result. At
  250 W and a falling cadence it gets hard: keep turning until the run ends.
- Each run prints one results row. Send them all, or just the `$OUT` folder name.

### Capture integrity (#516)

`hil_fec analyse` reads a capture as one timeline: the file order is the order the run wrote it, the stamps must
not go back, no record appears twice, lines with the same µs count in file order, and each check's events and
control writes must be exactly its procedure's (cue, answers, connect, drop, releases, targets). A capture that breaks any of this reads `no result: contradictory: …`
or `unknown: contradictory: …`, never a full row (`part-b.md`, Capture integrity). Still commit only the output of
`redact`, and do not edit a `.fecap` by hand: an edited capture gives no result or a different one.

### Check 10: the phone as a second central (no pedalling)

Run it after check 12, with the same `--out`, while the trainer is powered. Have your phone at hand with
the Tacx app installed but **not** connected to the NEO.

```sh
hil_fec check10 --out "$OUT"
```

1. Type **`y`** and Enter when asked. Stay off the pedals.
2. At the cue, open the Tacx app on the phone and look for the NEO.
3. When the window asks *Does the phone list the NEO…*, type **`seen`**, **`not-seen`** or **`unsure`**
   and Enter.
4. Only if you typed `seen`: try to connect the phone to the NEO and wait about 30 s. When asked *What
   did the connect do?*, type **`connected`**, **`failed`** (an error), **`stuck`** (still
   "connecting") or **`not-tried`** and Enter. Anything else is asked again. After `not-seen` or
   `unsure` this question is not asked.
5. Leave the phone alone. The window stays open for at least 60 s in all, then the run releases the
   trainer and prints the row. Answer within 3 min of the cue, or the run stops without a result.
6. Afterwards, cancel the connect in the Tacx app and close the app before the next run.

### Handle reuse (#468, no pedalling)

This measures the assumption of ADR-0002 §5.1.2: does a connect find the device by adapter lookup (`via=lookup`) or
does it have to scan (`via=scan`)? Each run is a fresh process that scans like Settings → Devices, waits
`--idle-secs`, connects once through the app's own drivers (`first`), releases, and connects again at once
(`after_disconnect`). It writes no mode or target page (a trainer connect writes page 55, the rider configuration, as pairing does). Run it per device, with the trainer free (no ride, phone apps
closed) and the strap worn: `hil_fec reuse --scan-secs 5 --idle-secs 0 --target <id> --out "$OUT/reuse-now"` gives
"right after a Devices scan", and the same with `--idle-secs 60` and `--out "$OUT/reuse-60s"` gives "after that scan
stopped"; both give "after a disconnect". The ids of the paired devices are listed by
`sqlite3 ~/Library/Application\ Support/io.github.gerchowl.crank/crank.db 'select kind, platform_id from device'`;
without `--target` the first trainer seen is taken. Each run prints and writes `reuse.json` with `via` and `ms` per connect.
A recorded app session also logs each connect's phases with their times (the `link phase` and `ready_total` records
of `logs.jsonl`, from `crank_core::links`; read them with the `jq` line of section 2 and `grep 'link phase\|ready_total'`). The architect records the results in the HIL doc and rewords §5.1.2 if
the lookup fails.

### Check 11a: unplug while pedalling

Run it after check 10, with the same `--out`, **with the trainer plugged in**. Have the mains plug within
reach from the saddle.

```sh
hil_fec check11a --out "$OUT"
```

1. Type **`y`** and Enter when asked, then ride at about 90 rpm through the 100 W soft start and 20 s.
2. At the cue *pull the trainer's mains plug now*, keep pedalling at about 90 rpm and pull the plug.
3. When asked *Is the trainer's mains plug out now?*, type **`unplugged`** (or **`cannot`**, which stops the run)
   and Enter, within 3 min. Keep riding at about 90 rpm for 30 s more; the run releases the trainer and prints
   the row.
4. Leave the trainer **unplugged** for check 11b. If the link drops after the unplug, that is the finding: the
   run stops, still answer the question if it is asked, and say so to the architect. If the trainer only goes
   silent, the cue *the trainer went silent* comes; keep pedalling easily while the harness waits up to 32 s for
   the link to drop. If that cue says *the release was NOT confirmed*, stop pedalling instead.
5. Read the last line. *done: trainer released* means the release was confirmed. *done, but the release was NOT
   confirmed* (expected after a drop: the link is down) means the trainer may still hold 100 W: stop pedalling.
   It is unplugged, so it shuts down once you stop; wait until its lights go out before the next run.

### Check 11b: the unplugged stop (pedalling)

Run it after check 11a, with the same `--out`. **The trainer stays unplugged from mains** (unplug it now if it is
not); get on and pedal easily so that it is powered (its lights on).

```sh
hil_fec check11b --out "$OUT"
```

1. Type **`y`** and Enter when asked, then ride at about 90 rpm through the 100 W soft start and 20 s.
2. At the cue *stop pedalling and wait until its lights go out*, stop and wait. The trainer is already
   released. Within 90 s the lights go out and the link drops; if it does not, the run stops (is the
   trainer unplugged?).
3. At the cue *the link dropped: pedal again*, pedal easily. The trainer wakes, and the harness reconnects
   within 60 s.
4. It releases the trainer first and checks it is the same trainer (if not, it stops). Then it asks *had
   the trainer's lights gone out?*: type **`out`**, **`on`** or **`unsure`** and Enter.
5. It asks again before it applies anything: type **`y`** and Enter, then ride at about 90 rpm through the
   100 W soft start and 20 s. The run releases the trainer and prints the row.
6. Stay off the pedals and leave the trainer unplugged for check 11c.

### Check 11c: the unplugged cold start (pedalling)

Run it right after check 11b, still unplugged. **Before you start, wait until the trainer is off (its lights
out)**, and stay off the pedals.

```sh
hil_fec check11c --out "$OUT"
```

1. When asked whether the trainer is unplugged and off, type **`y`** and Enter.
2. At the cue *start pedalling the unplugged trainer now*, pedal easily at about 90 rpm. The harness waits up
   to 60 s for the trainer to advertise, then connects (30 s at most). If either runs out, the run stops; nothing
   was written to the trainer.
3. Once connected it releases the trainer and asks before it applies anything: type **`y`** and Enter, keep
   riding at about 90 rpm through the 100 W soft start.
4. At the cue *hold about 60 rpm*, hold 60 rpm in an easy gear for 20 s, then at the next cue 90 rpm for 20 s.
   The run releases the trainer and prints the row.
5. Plug the trainer back in afterwards.

The times are counted from the start cue, so they include the moment the cue takes to reach you: start as soon
as you see it.

## 4. Afterwards

Tell the architect "done", with notes and the `$OUT` and app capture folder names. They will redact the
captures into `docs/hil/captures`, run `hil_fec analyse` on the raw captures first, write
`docs/hil/<date>-fec.md` and file whatever broke.

**Watch for and report:**

- Did the Mac sleep or dim during the ride (#51)?
- Is the FTP of 250 W, fixed for now (#247, `crates/crank-core/src/library.rs`), sensible for the Sensor check?
- Did heart rate show up in the app at all? The HR path has never run on hardware.
- #419 item 2: on CoreBluetooth, is the release frame (page 48 at 0 %) ordered after an abandoned, timed-out
  write? The simulator cannot show it. Note any `STOPPED:` line that mentions a write, with the time; the
  architect reads the order from the capture and records it on #419.

## If you must ride on `mba22`

`mba22` has no crank checkout, no app and no `nix-tcc` identity yet. You would need to create the identity
there (g-fleet `bin/nix-tcc-cert`, a keychain step only you can do), clone crank, do a full build (about 20–40
min) and grant Bluetooth to Alacritty for `hil_fec`. Prefer `sage`.
