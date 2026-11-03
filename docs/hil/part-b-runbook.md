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

With `CRANK_CAPTURE_DIR` the app writes `$CAP/crank.log` (INFO, and `crank-ble` at TRACE) and one
`trainer-<unix secs>-<n>.fecap` per trainer connection (AGENTS.md; `app/src-tauri/src/logging.rs`). If the
directory cannot be opened the app says so and exits rather than riding unrecorded. Without it the log is
`~/Library/Logs/io.github.gerchowl.crank/crank.log`. Captures hold raw device frames (the trainer serial):
redact before committing, share nothing unreviewed.

## 3. `hil_fec` checks

Build once, in `nix develop` (use your own `CARGO_TARGET_DIR` on a shared host):

```sh
nix develop -c cargo build -p crank-ble --features hil --example hil_fec
OUT=~/crank-captures/$(date +%F)-partb
mkdir -p "$OUT"
```

Run the checks **in this order (5, 6, 7, 8, 9, 12, then 10), with the same `--out`**, from an Alacritty window on `sage`. The binary is
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
- Follow the on-screen cues. A run that stops early prints `STOPPED: … (trainer released)` and exits with
  status 4: that is safe, but it gives no result.
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

### Still to come (#412 parts c2 and c3)

- **Checks 11a–c** (dropout: unplug while pedalling, unplugged stop, unplugged cold start):
  `TBD: command and procedure (#412 parts c2 and c3)`. They need their own session.

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
