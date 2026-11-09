# ADR-0006: Build speed: Nix layout and binary caches

- **Status:** Proposed
- **Date:** 2026-11-09
- **Issue:** #580
- **Related:** ADR-0001 §9 (Nix, CI, distribution; this ADR amends §9.4 and §9.6), `flake.nix`,
  `.github/workflows/{ci,release,prepare-release}.yml`, RELEASING.md; g-fleet's k3s lane design (ADR-032, "the
  k3s doc" below) and runner pool design ("the pool design"), g-fleet#469, #730, #837, #858,
  #376, #1020, #759, #949

## 1. Context

The owner (#580, 2026-11-08) wants crank's build "completely optimised": use the caches we have, and a layout
in which a small edit is cheap. Today:

- **Layout (`flake.nix`).** crane. `buildDepsOnly` (`cargoArtifacts`, pname `crank-deps`) compiles the
  third-party dependencies once per `Cargo.lock` in the release profile (the default; no `[profile.release]`,
  so opt-level 3, no LTO, 16 codegen units). Every other Rust derivation inherits it and compiles the
  workspace crates itself: `build` (crank-desktop), `build-e2e` (crank-desktop with `e2e`, another vendor
  directory), `crank-cli`, `clippy`, `nextest`, `doctest`, `doc`, `engine-no-features`, `bindings-drift`
  and `scales-parity` (`cargo run --release`). `crank-app` is a `runCommand` that copies desktop and cli into a
  bundle and re-signs them ad hoc.
- **Caches.** (1) The fleet attic, private network: owner Macs substitute from it; nothing in crank pushes to
  it. (2) `gerchowl.cachix.org`, a substituter on owner Macs; nothing pushes crank to it. (3) An R2 bucket
  the owner could create. (4) The GitHub Actions cache through `magic-nix-cache`: 11 732 entries,
  10 719 851 401 bytes (`gh api repos/gerchowl/crank/actions/cache/usage`, 2026-11-08), at the 10 GB
  per-repository limit. (5) sccache, the RUSTC_WRAPPER on owner Macs outside Nix (not usable inside a
  derivation).
- **CI.** GitHub-hosted runners (`ubuntu-latest` on every PR, `macos-latest` after every merge into `dev`
  and on release branches, ADR-0001 §9.6). They cannot reach the fleet attic. Release dry runs: run
  42104438450 (2026-11-05), build job 27.5 min of which the macOS gate 24.6 min; run 42529294719 (2026-11-08),
  build job 36.8 min of which the gate 34.5 min (step times from `gh api repos/gerchowl/crank/actions/jobs/<id>`).
- **Owner decision (2026-11-08, on this ADR's first round).** CI and Release move to the fleet's self-hosted
  runners, next to the agents, with **the fleet attic as the one shared cache**: under sustained load that is
  cheaper than a paid cloud cache. No Cachix push, no R2 bucket, no paid cache. Most CI runs on the host's
  aarch64-linux k3s cluster; a local macOS VM does only what needs macOS. This ADR designs that within the
  fleet's existing runner design (g-fleet's k3s lane design, ADR-032, and its runner pool design), which it
  aligns with and does not fork.

### 1.1 Measurements

All numbers below come from commands run for this ADR on 2026-11-08/09; the commands, exit codes and raw
outputs are listed in the PR. The tool is `scripts/nix-build-profile.py`: it reads
`nix … --log-format internal-json -v` on stdin and prints the wall time, each derivation built with its own
wall time, and how often each workspace crate was compiled (cargo's `Compiling|Checking|Documenting
crank-<name> v… (` lines; dependencies come from the vendor directory and are not counted).

**Host and method.** The owner's Mac Studio (Apple Silicon), shared with other agents, under the repository's
host rules: `--cores 2 --max-jobs 1`, local builds only (`--option builders ""`), one heavy job at a time. Two
cores make the absolute times long; read them as relative to each other. "Cold" means every crank
derivation is new: a scratch clone adds an environment variable `CRANK_MEASURE_SALT` to `common` and to the
frontend derivations (never committed), so `crank-deps`, the workspace builds, the checks and `app-dist`
rebuild while the toolchain, vendored crates and npm store still come from caches. "Warm" means the same
salt after a one-line edit: a comment appended to `crates/crank-core/src/lib.rs` (Rust) or
`app/src/main.tsx` (frontend only).

**Table 1. CI today: what the caches served** (recent GitHub-hosted jobs, logs read with
`gh api repos/gerchowl/crank/actions/jobs/<id>/logs`; "throttled" counts the line "GitHub Actions Cache
throttled Magic Nix Cache", "magic" and "cache.nixos.org" count `copying path … from` that cache, "deps built" counts
`building '…-crank-deps-0.1.0.drv'`).

| Run | Job | Minutes | throttled | magic | cache.nixos.org | deps built | `crank-deps` drv |
|---|---|---|---|---|---|---|---|
| 42524855940 (push dev) | macos | 35 | 14 | 1 | 509 | 1 | `5y71fsk9…` |
| 42521374993 (push dev) | macos | 35 | 13 | 0 | 510 | 1 | `5y71fsk9…` |
| 42516467689 (push dev) | macos | 35 | 1 | 528 | 254 | 1 | `5ccdapvr…` |
| 42491266517 (push dev) | macos | 32 | 0 | 528 | 241 | 1 | `5ccdapvr…` |
| 42524855940 (push dev) | linux | 29 | 1 | 1206 | 496 | 1 | `dz6fahsm…` |
| 42521374993 (push dev) | linux | 19 | 0 | 1787 | 451 | 1 | `dz6fahsm…` |
| 42516467689 (push dev) | linux | 26 | 1 | 1435 | 513 | 1 | `ff4pk72q…` |
| 42491266517 (push dev) | linux | 21 | 0 | 1425 | 513 | 1 | `ff4pk72q…` |
| 42523038097 (PR) | linux | 19 | 5 | 1471 | 496 | 1 | — |
| 42520663713 (PR) | linux | 27 | 74 | 106 | 721 | 1 | — |

Every job built `crank-deps`, including the four where the previous run on the same platform had built the
same derivation (the pairs with one hash): the cache had seen it and did not serve it. Neither
`gerchowl.cachix.org` nor the fleet attic holds any crank output (`nix path-info --store <cache>` on the
`crank-deps`, `crank-desktop`, `crank-clippy` and `crank-nextest` outputs of the current `dev`: not found in
either). In the macOS job of run 42524855940 the longest derivations were crank-desktop (both builds, 865 s
between first and last log line), crank-nextest (818 s), crank-deps (337 s), crank-doctest (295 s) and the
`crank` mkCargoDerivation checks (269 s); per workspace crate, the job compiled, checked or documented each one 7
to 14 times (`Compiling|Checking|Documenting crank-… v` lines; that count includes `crank-deps`' dummies).

**Table 2. Local builds, baseline layout** (profile as above; `crank-app` steps salt `a2`, flake-check steps salt
`b2`, so the two series do not share derivations).

| Step | Exit | Wall | Derivations built | Workspace compiles |
|---|---|---|---|---|
| cold | 0 | 2222 s | crank-deps 1367 s, crank-desktop 256 s, crank-app-dist 110 s, crank-cli 67 s, crank-app 1 s | crank-deps 36 (3 dummy passes × 12 crates), crank-desktop 10, crank-cli 11 |
| warm, one line in crank-core | 0 | 379 s | crank-desktop 262 s, crank-cli 84 s, crank-app 4 s | 21: every crate crank-desktop and crank-cli depend on, twice, where 4 crates (crank-core, crank-mcp, crank-cli, crank-desktop) depend on crank-core |
| warm, one line in `app/src/main.tsx` | 0 | 2055 s | crank-app-dist 1220 s, crank-desktop 208 s, crank-app 1 s | 10: the desktop embeds the frontend bundle (`TAURI_CONFIG` names `appDist`), so a frontend-only edit recompiles every crate crank-desktop depends on |
| `nix flake check --keep-going`, cold | 0 | 4010 s | 24 built; the longest: crank-nextest 784 s, crank-desktop (`build`, `build-e2e`) 605 s and 520 s, crank-deps 434 s, vectors-drift 422 s, app-lint 113 s | 114 outside `crank-deps` |
| the same, after one line in crank-core | 0 | 1146 s | 18 built: crank-desktop 342 s and 108 s, crank-nextest 297 s, crank-doctest 74 s, the `crank` checks 73 + 40 + 31 s, crank-cli 60 s, crank-doc 27 s, crank-clippy 26 s | 103: every Rust check recompiles the workspace; only `crank-deps` is reused |
| the same, after one line in `app/src/main.tsx` | 0 | 1469 s | 15 built: crank-desktop 449 s and 204 s, the frontend checks 21–149 s each | 21: both desktop builds (10 each), for the embedded bundle, and crank-domain once more in a `crank` check |

The `crank-app-dist` row is inflated by the host: its `pnpm install` alone took 4 min 30 s ("Done in 4m 30.2s",
814 packages) and vite 8.25 s, where the cold build of the same derivation took 110 s, and on GitHub-hosted runners
each of the 11 frontend derivations installs in 1.4–14.3 s (run 42541163301). The repeated installs are therefore
not worth a shared `node_modules` derivation; the embedded bundle is the layout cost a frontend edit pays.

From 2026-11-09 00:56 to about 03:20 the host's job wrapper refused every heavy job (a shared build volume fell
below its 25 GiB free floor, `heavy: REFUSED: … has 24 GiB free (< 25 GiB, g-fleet#952 rule 4)`), so the layout
prototypes were measured on GitHub-hosted runners, which need no lock (Tables 3, 4); the local rows after the warm
crank-core edit ran once the wrapper admitted jobs again. An earlier attempt at the cold build hung for 20 min with
cargo idle at 0.6 s of CPU and no log; it was interrupted and re-run with a new salt (g-fleet#1460).

**Table 3. `nix flake check -L`, baseline against the shared-layer prototypes** (workflow-dispatched CI on
branches `docs/580-base-measure`, `docs/580-p1-measure`, `docs/580-p1r-measure`, all at `dev` 2de24c5 plus the
prototype; never merged). Hosted runners have 4 cores and build 4 derivations at once, so a derivation's span
overlaps others and the job time varies by minutes between identical runs; the compile counts are exact. The `linux` jobs are
x86_64 (`ubuntu-latest`), the `macos` jobs Apple Silicon (`macos-latest`). "Workspace
events" are cargo's `Compiling|Checking|Documenting crank-…` lines outside the deps layers. P1 = shared layer in the
dev profile with its own dev deps layer; P1r = the same in the release profile on the existing deps layer.

| Run | Job | Result | Minutes | Workspace events | nextest | test-suite run time (nextest `Summary`) |
|---|---|---|---|---|---|---|
| baseline 42541163301 | linux | success | 21.6 | 92 | compiles 12 crates | 36.7 s |
| baseline 42541163301 | macos | success | 35.2 | 103 | compiles 12 crates, 1242 s span | 200.9 s |
| P1 42545971862 | linux | success | 23.2 | 78 | compiles nothing | 181.2 s |
| P1 42541165442 | linux | success | 27.6 | 76 | compiles nothing | 249.6 s |
| P1r 42546036516 | linux | success | 19.0 | 83 | compiles nothing | 47.4 s |
| P1 42541165442 | macos | failure: `Missing cached Tauri permission` (relocation across two layers) | — | — | — | — |
| P1r 42546036516 | macos | failure: `sim_parity` `NotFound` (`CARGO_MANIFEST_DIR` of the layer's build dir) | — | 87 | — | 600 of 1103 run, 1 failed |
| P1 42545971862 | macos | failure: the same `sim_parity` `NotFound`, in the dev profile | — | — | — | test 596 of 1103 failed |

In the same runs: `crank-bins` (both binaries, one cargo invocation) compiled 12 workspace crates where `build` and
`crank-cli` compiled 10 + 11; the dev deps layer took 425–571 s cold on top of the release one (275–848 s).

**Table 4. A one-line crank-core edit of crank-cli, crane against per-crate** (workflow-dispatched on
`docs/580-p2-measure`, run 42546576393, `ubuntu-24.04-arm` (aarch64-linux) and `macos-latest`, 4 cores, `nix build --max-jobs auto`; the edit is the
same appended comment, applied by `postPatch` in a second package so both builds run in one job and the edit
reuses everything the first built). The crate2nix tree is crate2nix 0.14.1's `Cargo.nix` for the workspace with
the same toolchain.

| Layout | Cold | After the edit | Rebuilt after the edit |
|---|---|---|---|
| crane (today) | 337 s (crank-deps and crank-cli) | **38 s** | 1 derivation, 11 workspace crates |
| crate2nix + buildRustCrate | 114 s (168 crate derivations for crank-cli only) | **17 s** | 3 derivations: crank-core 14.0 s, crank-cli 1.0 s, crank-mcp 0.8 s |
| crane (today), `macos-latest` | 396 s | **64 s** | 1 derivation, 11 workspace crates |
| crate2nix, `macos-latest` | 161 s (156 crate derivations) | **25 s** | 3 derivations: crank-core 16.6 s, crank-mcp 2.4 s, crank-cli 2.2 s |

An earlier run of the same job (42545784637) measured crane 316 s cold and 35 s after the edit; its crate2nix leg
failed on `rusqlite_migration`'s build script until the override of §2.1.

Sizes for the cache (`nix path-info -sS`): `crank-deps` 348 105 168 bytes (a zstd tarball), closure 1 036 981 704
bytes, of which 828 780 672 bytes in 435 paths are not on cache.nixos.org (the deps tarball and the vendored crate
sources it references); the Rust toolchain adds 804 MB of NAR per system not on cache.nixos.org;
`crank-desktop` 19.6 MB, `crank-app` 20.3 MB, `crank-cli` 0.4 MB.

## 2. Decision

### 2.1 Layout: a small edit rebuilds only what depends on it

Measured (§1.1, Tables 2–4). Today, a one-line edit anywhere recompiles every workspace crate in every Rust derivation:
21 compiles for `crank-app` and 92 (Linux) or 103 (macOS) compile, check and document events for the flake check
outside the deps layer. Two changes, in order:

**L1, now, crane (Recommended first step).**

- **One build for both binaries.** `crank-bins` = one `buildPackage` with `-p crank-desktop -p crank-cli --bin
  crank-desktop --bin crank` (`--bin` filters across packages, so both names are needed: a prototype with one
  `--bin` silently built only the desktop). `crank-app` and the `build` check use it. The shared crates compile once
  instead of twice: 12 compile events on macOS instead of 21 (Table 3). `crank-core` has no features, so the feature
  set of each binary is unchanged; `deps-edges` keeps checking the production graph.
- **One test derivation that builds and runs.** `rust-tests` builds every workspace target once, in the release
  profile with CI's feature set (`--workspace --all-targets --features crank-ble/replay,crank-ble/hil`), then runs
  in the same derivation: nextest, the doctests (with the same features), `bindings-drift`'s `export-bindings`,
  `scales-parity`'s Rust half and `engine-no-features`' `cargo check -p crank-engine`. It replaces five derivations
  that each recompiled the workspace. On Linux the prototype's nextest and doctest compiled nothing (Table 3).
  - **Build and run in one derivation.** The prototype split them (a `cargoBuild` layer inherited by the five
    checks) and that is unsafe on macOS: Nix builds each derivation there in a fresh random directory
    (`/nix/var/nix/builds/nix-<pid>-<random>`; on Linux always `/build`). (1) Tauri's ACL lists carry absolute
    paths that `scripts/relocate-tauri-permissions.py` rebases from the path in `root-output`, which names the first
    layer only, so a second generation failed with `Missing cached Tauri permission`. (2) Test binaries bake
    `env!("CARGO_MANIFEST_DIR")` at compile time (8 uses in 7 files), so `sim_parity` failed with `NotFound` in the
    derivation that ran it. Both pass on Linux by luck of the fixed path. One derivation per compile-and-run has
    neither problem.
  - **Release profile, not dev.** Prototyped in the dev profile, the suite ran in 181.2 s (run 42545971862) and
    249.6 s (run 42541165442), 4.9× to 6.8× the release profile's 36.7 s (run 42541163301) on the same hosted Linux
    runner class (nextest `Summary`, Table 3), which spends the compile time it saves. The dev profile
    would also need a second deps layer (425–571 s cold, Table 3, and in the attic about as much again as the release
    layer's 348 MB per system).
- **Kept separate.** `clippy` (clippy-driver's metadata cannot be shared with rustc's), `doc` (rustdoc), `build-e2e`
  (the `e2e` feature and its own vendor directory), `fmt` and `deny`.

**L2, next, per-crate derivations for the shipped binaries (Recommended target).** `crank-bins` built with
crate2nix and nixpkgs' `buildRustCrate`: one derivation per crate (third-party and workspace), so an edit
rebuilds the edited crate and its reverse dependencies, and the attic caches each crate separately, so branches
share everything they did not touch. Measured for crank-cli on aarch64-linux (Table 4): a one-line crank-core
edit took **17 s, rebuilding 3 derivations** (crank-core 14 s, crank-mcp, crank-cli), against **38 s and 11
workspace crates** with crane, and on macOS **25 s against 64 s**; a cold build 114 s against crane's 337 s (crane's deps layer builds the whole
workspace's dependencies, Tauri included, so that comparison flatters crate2nix). Costs, stated:

- a generated `Cargo.nix` (22 674 lines, 970 KB) and `crate-hashes.json` committed, regenerated with every
  `Cargo.lock` change, and a drift check that fails when they disagree;
- a crate override per non-standard build script: the prototype needed one (`rusqlite_migration` reads
  `CARGO_PKG_README`, which `buildRustCrate` leaves empty unless told);
- crank-desktop's Tauri build script (the `TAURI_CONFIG` frontend path, the ACL files) is **not yet proven** under
  `buildRustCrate`;
- `buildRustCrate` compiles with `-C codegen-units=1` (seen in its log); cargo's release default is 16, so the
  shipped binaries would be built differently from the tested ones (§2.2 keeps the default until that is measured);
- checks keep needing cargo, so the flake has two Rust build systems: crate2nix for what ships, crane for what is
  checked.

The embedded frontend makes the same point from the other side: a frontend-only edit costs a full desktop
workspace compile today (208 s for `crank-app`, 653 s for the two desktop builds of the flake check, Table 2); per
crate it would rebuild crank-desktop alone, whose build script takes the new bundle.

L2 goes ahead only if its sub-issue shows crank-desktop and crank-app building per-crate on both systems and a
crank-core edit to `crank-app` at no more than half of L1's time; otherwise L1 stays. A crate at the bottom of the
graph (`crank-domain`) rebuilds nearly everything in either layout; the gain is for edits higher up, where most
work happens (crank-core, crank-desktop, the app).

### 2.2 Release profile: unchanged

The shipped binaries keep cargo's default release profile (opt-level 3, no LTO, 16 codegen units). LTO and
`codegen-units = 1` make every release link slower in exchange for a smaller or faster binary, and crank's hot
path is a 4 Hz BLE loop and a webview, neither CPU-bound. A release-profile change is its own decision, measured
on binary size and start-up time. The dev profile keeps `debug = "line-tables-only"` (#371).

### 2.3 Runner topology

The fleet's CI design is g-fleet ADR-032 (the k3s doc) and the runner pool (the pool design). crank uses it as designed:

- **The Linux lane.** One-shot JIT runner pods on the host's aarch64-linux k3s guest, created per queued job by
  the runner pool, the fleet's single dispatcher (k3s doc, "Layout inside the VM", the "Runner engine" bullet: the pool
  is the JIT dispatcher, not ARC). The owner's message of 2026-11-08 names ARC runner scale sets; the fleet replaced
  ARC on this cluster on 2026-10-25/26 because ARC cannot filter `workflow_job` events by event name, ref or head
  repository, which a public repository needs (ADR-032 "Why a broker, not ARC"). crank's jobs request exactly one
  role label, `crank-linux-<s|m|l>` (2/4, 4/8, 8/16 CPU/GiB; limits equal requests), in the `runners-crank`
  namespace that the lane's PR 7 adds (g-fleet#858). The pool's own label names include the host's name, which
  stays out of this public repository: the mapping from crank's role labels to the pool's lanes and sizes lives in
  crank's row of the pool's policy table in g-fleet (g-fleet#1445). Pods are admitted by Kueue (one ClusterQueue,
  10 CPU / 30 GiB, `lane_max` 5), bind-mount the guest's `/nix` read-only and its nix-daemon socket, and
  substitute from the fleet attic. Horizontal scaling is the pool's job: as many pods as the queue and the
  budget admit.
- **What runs on Linux.** Everything that is not about macOS: the Rust checks (clippy, nextest, doctest, doc,
  `engine-no-features`, `bindings-drift`, `deps-edges`, `scales-parity`), `fmt`, `deny`, `deny-fuzz`, every
  frontend check, the design harness with its visual baselines, and all the script checks. These move from
  x86_64-linux to **aarch64-linux**, which the flake does not build today: `systems` gains `aarch64-linux`,
  pnpm's `supportedArchitectures` gains `linux-arm64` (the fetch hash changes), and the visual baselines are
  rendered once on aarch64-linux (Chromium's rasteriser is not promised to match across architectures) and
  reviewed image by image like any baseline change (AGENTS.md, #459). x86_64-linux keeps evaluating but is no
  longer built by CI; nothing x86_64-linux ships (Linux bundles are #41).
- **What stays macOS, and why.** Only what a Darwin build exercises: `build` and `build-e2e` (crank-desktop
  with the macOS code of `lifecycle.rs` and `display.rs`, the only two `cfg(target_os)` files, and the
  CoreBluetooth backend of `btleplug`), `crank-cli`, `crank-app` with `bundle-no-nix-store` (#413, `otool` on
  Mach-O files), `install-app` (`renamex_np`), and `clippy` and `rust-tests` (nextest) for the Darwin `cfg` code
  and the Darwin stdenv's linker. Everything else is platform-independent and runs once, on Linux. The flake's
  `checks.aarch64-darwin` shrinks to that set (#602).
- **Where macOS work runs: the macOS guest as a Nix builder, not a runner VM (Recommended; this departs from
  the owner's topology of a macOS tart VM as Release builder, so it is the owner's decision, #615).** Every item in that
  list is a Nix derivation for `aarch64-darwin`. It needs a Darwin *builder*, not a GitHub job on macOS. The
  host's macOS compute guest is already the fleet's aarch64-darwin remote builder; the Linux guest's nix-daemon
  gets it as a builder for `aarch64-darwin`, and the crank job on the Linux lane runs
  `nix build .#checks.aarch64-darwin.<…>` like any other build. The outputs come back into the Linux guest's
  store and are pushed to the attic from there (§2.5). This uses **no additional macOS VM**: the host's two
  macOS guest slots are taken by the agent workspace and the compute guest (g-fleet's tart
  guest module, "THE 2-GUEST LIMIT"), which is why the per-job macOS runner VMs are off
  and why the pool's macOS lane (role label `crank-macos-<size>`; its tart backend still unbuilt, "wave 2") would have
  no slot to start in. The release packaging that today calls `ditto`, `plutil` and `shasum` on a macOS runner
  moves into a derivation and Python (`crank-app-zip`: `zip -X -r`, the plist read with `plistlib`), so the
  Release job itself runs on Linux too.
- **When a macOS runner VM becomes necessary.** Developer ID signing and notarization (#47, #426) need the signing
  identity in a keychain and `notarytool`, which no Nix build provides. That is the first job that needs a macOS
  runner, and it goes to the pool's macOS lane once a guest slot is free (the pool's "Day-1 configuration":
  `max_vms = 1`, 2 only after the per-repo tart runners retire). It is out of scope here; #426 owns it.

### 2.4 Self-hosted runners on a public repository

ADR-0001 §9.4 ("self-hosted runners must never serve a public repository") is amended: they serve crank for
**trusted events only**, the exception the fleet already made for flock (g-fleet#837) and specified for crank
(g-fleet#858). How, exactly:

- **Fork PRs never run on them.** Four independent layers, each failing closed:
  1. **Workflows.** No job that can be started by `pull_request` or `pull_request_target` requests a
     self-hosted role label (`crank-linux-*`, `crank-macos-*`). The self-hosted jobs live in workflows whose triggers are `push` (to `dev`, `main`),
     `merge_group` (base `dev`) and `workflow_dispatch` (refs `dev` and `release/*`). There is no expression that
     picks the runner from event data: a job is self-hosted by its workflow's triggers, so a fork cannot reach one
     through a crafted event.
  2. **Pool discovery.** The pool reads each queued job's run (`GET /repos/{o}/{r}/actions/runs/{run_id}`) and
     admits it only if the event, the base ref and `head_repository == repository` match crank's row; a
     `pull_request` from crank is refused at discovery (k3s doc, "How the pool reads the trigger", "Head-repo
     rule"; PR 7 gate "a fork PR is logged and discarded without creating a Pod").
  3. **Pre-job hook.** `ACTIONS_RUNNER_HOOK_JOB_STARTED` in the runner image re-checks the same policy against
     `$GITHUB_EVENT_PATH` before step 1 and fails closed on a mismatch or an unknown event.
  4. **Watcher.** The pool watches `workflow_job in_progress` and kills a pod whose acquired job fails the policy
     (ADR-032 "Runner theft defence in depth").
  Plus the repository setting: fork PR workflows from outside collaborators need approval for **all** outside
  collaborators, and secrets are never sent to fork PR workflows (§2.7).
- **`workflow_dispatch` is a new event for the pool's rows.** crank's Prepare Release dispatches CI on
  `release/<version>` (a push made with `GITHUB_TOKEN` starts no workflow), and Release dry runs are dispatches.
  Dispatching needs write access, so it is as trusted as a push; the row allows it on `dev` and `release/*` only.
  The pool's policy has no `workflow_dispatch` row today: g-fleet#1445.
- **The PR gate becomes a merge queue (Recommended).** Pull requests are the only event a public repository
  cannot run on these runners, and they are where crank's required check runs today. `dev` gets a merge queue:
  the required `CI Summary` runs on `merge_group` on the Linux lane (with the Darwin set, through the builder) and
  decides the merge; on `pull_request` a short GitHub-hosted job (`nix flake check --no-build` on
  `ubuntu-24.04-arm`, plus `fmt`) gives early feedback and reports `CI Summary` so the PR can be queued. Trust in
  `merge_group` rests on who can enqueue: only users with write access, and only after the PR's checks passed
  (ADR-032: "Trust in merge_group rests on the base branch's branch-protection review-before-enqueue rule").
  A fork PR must therefore be reviewed before a maintainer enqueues it; an agent never enqueues a fork PR. The
  post-merge macOS run of ADR-0001 §9.6 becomes unnecessary: the Darwin set is part of the queue's gate, so a PR
  can no longer break the macOS build and land green.
- **Isolation between jobs.** One job per pod, one-shot JIT runner, deleted after the job; `emptyDir` work,
  home and `/tmp` (2 GiB); uid/gid 2000 with no `/etc/passwd` entry, so the pod is never a nix-trusted user;
  read-only root filesystem, all capabilities dropped, `RuntimeDefault` seccomp; the volume allow-list and the
  cluster-wide hostPath guard; a default-deny ingress and an egress policy that reaches only DNS, the attic,
  Loki, Prometheus and the public internet; and `runners-crank` is its own namespace, so crank pods cannot reach
  other repositories' pods (k3s doc, "Pod security — allow-list, not denylist", "Pod egress policy"). What is
  shared: the Linux guest's nix store and daemon. A pod's builds are sandboxed on NixOS, and an untrusted client
  can add to the store only what it builds. Darwin builds on the compute guest run without a sandbox (Nix's
  default on macOS); they are reachable only by trusted events. **Residual risk, stated:** agents on the
  workspace guest are root-equivalent on the Linux guest through Nix trust and the builder key (g-fleet#1020).
  Until #1020 is fixed, the store that CI pods read and that the attic is pushed from is as trustworthy as every
  agent on the host; the attic push design (§2.5) inherits that.
- **Concurrency with agent builds.** Pods are bounded by Kueue (10 CPU / 30 GiB). Their `nix build`s run in the
  Linux guest's `nix-daemon.service` (`MemoryMax=8G`, `CPUWeight=50`, `max-jobs` 8, cores 2), **outside** the
  Kueue budget and shared with the agents' aarch64-linux remote builds (k3s doc, "Nix in pods": pod builds compete with
  the agents' builds, not with Kueue); Darwin builds share the compute guest's builder with the agents' Darwin builds. Nothing queues
  across the two; sizing is g-fleet#845. The repository's `heavy` wrapper governs agent jobs on the workspace
  guest only. Cached builds are the mitigation that matters: with the layout of §2.1 and the attic, a merge
  rebuilds the changed crates, not the workspace.
- **When the runners are offline.** A job for a self-hosted role label waits in GitHub's queue (24 h, then it fails); a
  merge queue with no runner blocks merges, it never merges unchecked. Fallback, chosen by the owner with one
  repository variable: `CRANK_RUNNERS=github` makes the workflows request `ubuntu-24.04-arm` (free for public
  repositories since 2025-08-07, the same aarch64-linux as the lane, so the same visual baselines) and
  `macos-latest` for the Darwin set, with only cache.nixos.org (no attic, no Actions cache); `CRANK_RUNNERS=self-hosted`
  (the default) is the lane. The variable is read in `runs-on` from `vars`, which a fork cannot set. Times on the
  fallback are today's (§1.1), not the lane's. An outage of the attic alone slows the lane, it does not stop it:
  Nix skips an unreachable substituter and builds (attic's known wedge, g-fleet#759).

### 2.5 The fleet attic: one cache, three writers, one rule

The attic cache `fleet` serves aarch64-linux and aarch64-darwin outputs side by side: a binary cache is keyed by
store path hash, not by system, so one cache and one public key cover both. What gets pushed, when, and by whom:

| Source | What | When | Who holds the push credential |
|---|---|---|---|
| CI on the Linux lane | the closure of the job's declared outputs: `crank-deps`, `crank-bins` (with L2: each crate's derivation), the check outputs, `crank-app` and `crank-app-zip` for aarch64-darwin (they come back from the Darwin builder into the Linux guest's store) | after a job **succeeds**, for `push` to `dev`/`main`, `merge_group` on `dev` and the Release workflow; never for `pull_request` or `workflow_dispatch` on `release/*` dry runs before merge | the runner pool, the trusted dispatcher (g-fleet#469 decision 4: "Builds are pushed to attic by the dispatcher, not by the job … signed with a key that jobs never see"). The pods never see it. The push from the Linux guest's store is g-fleet#1447 |
| Owner builds on the owner's Macs | the same set, from `just cache-push` (`nix build --print-out-paths` of `.#crank-app` and the Rust checks, piped to `attic push`) | when the owner wants a local build shared | the owner's attic login on the infra-domain Macs (g-fleet `modules/home/common.nix`, `attic_push_token`); the recipe never reads, prints or stores it, and refuses when no login is configured |
| Agent builds through `heavy` | nothing | never | none: agents only substitute. Their inputs are unreviewed branches, and a PR branch is cheaper to rebuild than to pollute the attic with (k3s doc, "Nix in pods") |

A Nix `post-build-hook` on any host was rejected: it runs synchronously in the daemon after every build, for every
project and every requester (agents included), and needs the credential readable by the daemon. The attic's public
key is already trusted on the owner's Macs and in the Linux guest.

### 2.6 GitHub-hosted runners: the fallback; `magic-nix-cache`: dropped

GitHub-hosted runners stay, as the fallback of §2.4 and for the short `pull_request` job. `magic-nix-cache` is
removed everywhere, including the fallback: in 10 of 10 recent jobs `crank-deps` was built, also where the
previous run on the same platform had built the same derivation; the Actions cache throttled it (`HTTP error 418`,
"GitHub Actions Cache throttled Magic Nix Cache") in 7 of 10 jobs, and on macOS it served 0 or 1 path in 2 of 4
(§1.1). The repository's Actions cache is full (10.7 GB of 10 GB). On the fallback it would cost a full cache and
buy a few hundred vendored crate sources that cache.nixos.org and crates.io serve anyway.

### 2.7 Owner actions

No agent creates, reads or stores a secret. The owner:

1. **Merge queue on `dev`.** Settings → Rules → the ruleset protecting `dev` (or branch protection): enable
   "Require merge queue", merge method "Create a merge commit" (Prepare Release requires merge commits), build
   concurrency 1, status check `CI Summary` required. Agents then land PRs with `gh pr merge --auto` instead of a
   direct merge.
2. **Fork PR policy.** Settings → Actions → General → "Fork pull request workflows from outside collaborators":
   **"Require approval for all outside collaborators"**; "Send secrets to workflows from fork pull requests" stays
   off; "Send write tokens" stays off.
3. **Repository variable.** `gh variable set CRANK_RUNNERS -R gerchowl/crank --body self-hosted` once the lane serves
   crank; set it to `github` to fall back (§2.4).
4. **g-fleet decisions.** Merge the g-fleet changes of §2.9 (crank's row with `workflow_dispatch`, the Darwin
   builder for the Linux guest, the pool's trusted attic push) when their PRs come; g-fleet merges are the
   owner's.
5. **Release verification.** Accept that release provenance names a self-hosted runner (§4): installers drop
   `--deny-self-hosted-runners` and keep `--signer-workflow` and `--source-ref`.
6. **Attic push from a Mac.** Run `just cache-push` from an infra-domain Mac after a local build worth sharing.

No Cachix token, no R2 bucket and no runner registration token are needed: the fleet's CI GitHub App is
already installed on crank (g-fleet#858).

### 2.8 Implementation order

Each sub-issue's acceptance is tied to a number measured here.

| # | Issue | What | Waits for | Target (baseline) |
|---|---|---|---|---|
| 1 | #601 | L1: `crank-bins` and `rust-tests`; the profiler's test | — | Linux workspace events ≤ 83 (92); local warm `crank-app` ≤ 290 s (379 s) |
| 2 | #602 | aarch64-linux system, Darwin check set, `just check` on both systems | — | macOS job ≤ 20 min (35.2 min, run 42541163301) |
| 3 | #606 | `just cache-push` (owner) | — | a second Mac builds nothing |
| 4 | #603 | the gate on the lane through a merge queue, attic only, hosted fallback, `magic-nix-cache` removed | g-fleet#858, #1445, #1446, #1447; owner actions 1–3 | `crank-deps` substituted, not built (built in 10 of 10 jobs, Table 1); gate ≤ 12 min (21.6 / 35.2 min, run 42541163301) |
| 5 | #605 | Release on the lane: `crank-app-zip`, provenance | #603, #604 and #615 (owner), g-fleet#1445, #1446 | release build job ≤ 15 min (27.5 min, run 42104438450; 36.8 min, run 42529294719) |
| 6 | #607 | L2: per-crate shipped binaries, go/no-go | #601 | warm `crank-app` ≤ 50 % of L1 |

1–3 speed up every build today and do not wait for g-fleet.

### 2.9 g-fleet issues

Filed for this ADR (none of them changes g-fleet; each is the owner's to merge):

| Issue | What crank needs | Blocks |
|---|---|---|
| g-fleet#1445 | `workflow_dispatch` rows (refs `dev`, `release/*`) in the pool's policy, hook and gate; crank's three rows | Release and Prepare Release on the lane |
| g-fleet#1446 | the Darwin builder for the Linux guest's nix-daemon, so a lane job builds `aarch64-darwin` derivations | the Darwin set on the lane |
| g-fleet#1447 | the pool's trusted attic push from the Linux guest's store (credential held by a system service, never by pods) | attic hits for CI-built outputs |
| g-fleet#1448 | the macOS lane's trusted-events layers for public repositories | #426's signing job only |

Already open and required: g-fleet#858 (crank's enrolment, the lane's PR 7), #730 (lane PRs 5d, 5e, 6, 7), #1020
(agent Nix trust on the Linux guest), #845 (nix-daemon sizing). g-fleet#376 (the fleet's private runner VM and the
attic) does not serve crank: the fleet-runner-onboard skill refuses public repositories.

## 3. Alternatives considered

### 3.1 One shared `cargoArtifacts` layer inherited by separate check derivations

crane's chaining pattern (`cargoArtifacts = <a cargoBuild>`), prototyped as P1: it removes the duplicate compiles
within one revision on Linux, but fails on macOS for the two path reasons of §2.1, and it never makes an edit
cheaper across revisions (any edit changes the layer). Taken in its safe form (one derivation builds and runs).

### 3.2 crane per member, layered by dependency order

A chain of crane derivations in topological order, each built on the previous one with the crates above it replaced
by dummies. Rejected: cargo decides a path crate's freshness from modification times, and in the Nix store every
source and every inherited artifact has the same mtime (crane installs artifacts with `--mtime=@1`), so a dummy from
a lower layer can be taken as fresh for the real source in a higher one; plus the macOS path hazards of §2.1.

### 3.3 The dev profile for checks

Measured slower (§2.1): tests ran 4.9× to 6.8× longer (runs 42545971862, 42541165442), and it needs a second
deps layer.

### 3.4 sccache inside the build

Not possible in the sandbox without an impure cache directory, and measured as not worth wiring even outside it
(AGENTS.md, PR #386: 0 hits across target dirs, 112 s → 102 s at best). The k3s lane has no sccache either (k3s doc,
"Caches").

### 3.5 Cachix, R2, attic on R2 (rejected by the owner, 2026-11-08)

- **Cachix `gerchowl`** (this ADR's first-round recommendation): 5 GB free with LRU eviction, then plans without
  a published price. Rejected: under sustained load the fleet attic on owned hardware is cheaper, and it is next
  to the runners.
- **An R2 bucket as a Nix binary cache:** cheap storage ($0.015/GB-month, egress free), but garbage collection,
  signing-key custody and a public endpoint all become the owner's. Rejected for the same reason, and because the
  self-hosted runners do not need a public cache.
- **attic with R2 storage:** needs a public attic server. Rejected: no public endpoint is wanted.

### 3.6 Keep `magic-nix-cache` on the fallback

Rejected, measured (§2.6).

### 3.7 ARC runner scale sets on the k3s cluster

Named in the owner's message. Rejected by the fleet's ADR-032 on 2026-10-25/26 for this cluster, and crank follows
it: ARC cannot filter `workflow_job` by event, ref or head repository, which is the trust boundary for a public
repository; the pool's JIT pods do (k3s doc, "Runner engine"; "Divergence from ADR-030": the fleet's other ARC
installation stays for private repositories).

### 3.8 A macOS runner VM for the Darwin gate and the release build (the owner's topology; pending #615)

The owner's topology of 2026-11-08: a local macOS tart VM is the final Release builder and runs the macOS-only
checks, with the same caches. Its advantages are real: one VM per job, destroyed after, and a release built on
macOS by the job itself. The pool's macOS lane (one-shot tart clones, role label `crank-macos-<size>`) for the Darwin set. Not recommended for now, the owner decides (#615): the
host's two macOS guest slots are taken, so it would wait for a slot that never frees; its tart backend is
unbuilt; and every Darwin item is a Nix derivation a builder can build. It is the right home for #426's signing
job. A persistent runner inside the compute guest was rejected outright: a long-lived runner on a public
repository keeps state between jobs, and the fleet-runner-onboard skill and the pool refuse public repositories
for it.

### 3.9 Keep `pull_request` as the gate, self-hosted only after merge

Keeps today's flow with no merge queue. Rejected: the gate would stay on GitHub-hosted runners (the pool refuses
`pull_request` on a public repository), so the self-hosted lane would only re-run post-merge, and a PR could still
break the Darwin build and land green (ADR-0001 §9.6's known risk).

## 4. Consequences

- **Amends ADR-0001 §9.4 and §9.6.** Self-hosted runners serve crank for trusted events only (§2.4). The required
  gate moves from `pull_request` (GitHub-hosted x86_64-linux) to `merge_group` (the lane, aarch64-linux plus the
  Darwin set), and the post-merge macOS run is folded into the queue's gate. ADR-0001 gets an amendment note
  pointing here when the first implementation PR lands.
- **Release provenance changes.** The attestation will name a self-hosted runner. `release.yml`'s own check and
  the installer command in RELEASING.md drop `--deny-self-hosted-runners` and keep `--signer-workflow`
  `…/release.yml` and `--source-ref`. The trust an installer places in the bytes moves from GitHub's runner fleet
  to the owner's host and to everyone with Nix trust on it (g-fleet#1020). That is a weaker statement than today's,
  and RELEASING.md says so. Until Developer ID signing (#426) there is no other signature to lean on.
- **Release bytes can come from the attic.** A release build substitutes what trusted CI already built for the
  same revision. The attestation still binds the zip to the Release run that packaged it, not to a build from
  source.
- **The repository's process changes.** Agents land PRs through the merge queue (`gh pr merge --auto`), and a red
  queue run removes the PR from the queue instead of turning `dev` red. AGENTS.md and RELEASING.md change with
  the implementation.
- **Local `just check` grows to two systems.** It builds `.#checks.aarch64-darwin` (the Darwin set) and
  `.#checks.aarch64-linux` (everything) through the host's builders, the same targets as CI: "local CI is CI".
- **CI stops depending on hosted-runner availability** for its normal path (the macOS acquisition problem of
  #480), and starts depending on the lane, the pool, the Darwin builder and the attic. The fallback (§2.4) keeps
  merges possible when they are down, at today's speed.
- **x86_64-linux stops being built by CI.** It still evaluates. If a Linux bundle (#41) ships for x86_64, its
  build gets its own job then.
- **Blocked on g-fleet.** Nothing in §2.3–§2.5 can be switched on before g-fleet#858 (lane PR 7, after 5d and 5e),
  #1445, #1446 and #1447 land. The layout (§2.1) and the aarch64-linux flake work do not wait for g-fleet and
  speed up every build, local and CI, at once.

## 5. Open questions

| Question | Issue | Milestone |
|---|---|---|
| The owner named ARC; the fleet's lane is the runner pool, not ARC. This ADR follows the fleet (§3.7). Confirm, or reopen ADR-032 in g-fleet | #608 | — |
| Release provenance from a self-hosted runner before #426, or keep Release on GitHub-hosted until signing lands? | #604 | — |
| Does L2 (per-crate) pass its go/no-go, crank-desktop's Tauri build script included? | #607 | — |
| macOS work on the compute guest as a Nix builder (recommended: no free guest slot, no tart backend yet, every macOS item is a derivation; cost: unsandboxed Darwin builds shared with agents, a builder key on the Linux guest) or the owner's macOS runner VM (cleaner one-VM-per-job isolation; needs a freed guest slot, the pool's tart backend and g-fleet#1448)? | #615 | — |

## 6. References

- crane, rev `bd63bf1d0a0f6dcacd7549c714a671ee63b0db6a` (`flake.lock`): `lib/buildDepsOnly.nix`,
  `lib/setupHooks/{inheritCargoArtifactsHook,installCargoArtifactsHook,configureCargoCommonVarsHook}.sh`,
  `docs/introduction/sequential-builds.md` (read 2026-11-08).
- crate2nix 0.14.1 (nixpkgs), `crate2nix generate`; nixpkgs `buildRustCrate`.
- Cachix: <https://www.cachix.org/pricing>, <https://docs.cachix.org/garbage-collection>; Cloudflare R2 pricing:
  <https://developers.cloudflare.com/r2/pricing/> (both checked 2026-11-08, for §3.5).
- GitHub arm64 hosted runners for public repositories, GA:
  <https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/>.
- g-fleet (private; read 2026-11-09 at `cec902ce`): the k3s lane design (ADR-032; sections cited by name), the
  runner pool design (§1 "Model", "Day-1 configuration", §10), the tart guest module (header), issues #469 (decision 4), #730, #837, #858, #845, #1020, #759; claude-config `skills/fleet-runner-onboard`.
- magic-nix-cache, rate limits and HTTP 418: <https://github.com/DeterminateSystems/magic-nix-cache> (checked 2026-11-08).
- GitHub Actions cache above 10 GB: <https://github.blog/changelog/2025-11-20-github-actions-cache-size-can-now-exceed-10-gb-per-repository/>.
- CI runs measured: 42524855940, 42523038097, 42521374993, 42520663713, 42516467689, 42491266517.
- In-repo: `flake.nix`, `.github/workflows/{ci,release}.yml`, `scripts/nix-build-profile.py`, AGENTS.md (sccache,
  PR #386), ADR-0001 §9.3, §9.4, §9.6.
