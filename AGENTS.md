# Working on crank

Implement the decisions in [ADR-0001](docs/adr/0001-architecture.md) and
[ADR-0002](docs/adr/0002-ble-trainer.md). Domain implementation is tracked in
#29–#45; this scaffold contains no ride logic. `shared/` is frozen in place;
`design/prototype/` (#49) remains a separate package using the mock.

Use branches `<type>/<issue>-<slug>` and open PRs into `dev`. Before pushing,
`git fetch origin` and `git rebase origin/dev`. Use conventional commits with
an actual `Refs: #N` trailer. Never push main/dev, bypass hooks, stash, touch
spike worktrees, or terminate processes you did not start.

Releases (owner only; agents never push or merge `main`) follow [RELEASING.md](RELEASING.md); keep `## Unreleased` in
[CHANGELOG.md](CHANGELOG.md) current and bump no version by hand (`scripts/release-version.py`, the `release-versions` flake check, #428).
PR CI is Linux only; macOS runs post-merge on every push to `dev` (ADR-0001 §9.6). Before merging into `dev`, look at
`dev`'s latest macOS run: a red one needs a `bug` issue first (a runner failure, #480, is re-run once and noted on its PR;
RELEASING.md "Triage"). Merging a `release/*-rc.*` PR publishes a pre-release: leave release PRs to the owner.

Run everything in `nix develop`; first run `pnpm install --frozen-lockfile`.
Pre-commit (#456): run `prek install --allow-missing-config` once per clone (prek is pinned in the devShell). The hook lives in the
shared `.git/hooks`, so every worktree and branch runs it; the flag lets a branch without `prek.toml` still commit. `prek.toml` runs ESLint, stylelint and
Prettier on staged app files, `tsc --noEmit`, and `cargo fmt --check` on staged Rust, all offline from the devShell
toolchain. Hooks can be skipped, so CI stays the gate; `prek run --all-files` runs them on everything.
The generated hook calls prek by its `/nix/store` path (falling back to PATH), so commit from the devShell, or re-run
`prek install --allow-missing-config` after a prek bump: after a GC a commit outside `nix develop` fails with "prek: not found".
Inner loop (#371): `just test-changed [base=origin/dev]` runs nextest and doctests only for the workspace crates that
`git diff base...HEAD` (plus the working tree and untracked files) touches and their reverse dependencies
(`scripts/test-changed.py`, from `cargo metadata`; its mapping is tested by `scripts/test-test-changed.py`, the
`test-changed-test` flake check). The default for a path is safe: anything outside a workspace crate that is not on the
non-Rust list (`docs/` except `docs/hil/captures/`, which maps to crank-ble, and `docs/adr/models/fixtures/`, which maps to crank-domain; `app/` except `app/src-tauri/`; `design/`,
`shared/`, `assets/`, `issues/`, `scripts/`, `.github/`, any `*.md`, and the root files justfile, LICENSE, NOTICE,
.gitignore, .editorconfig, package.json, pnpm-*.yaml, deny*.toml) runs the whole workspace (Cargo.toml, Cargo.lock,
flake.*, rust-toolchain.toml, clippy.toml, `.cargo/`, `.config/` and anything new); a diff of non-Rust paths only runs
nothing. Renames count as their old and new path; doctests run only for selected crates with a lib target. CI's `crank-ble/replay,crank-ble/hil` flags are passed when crank-ble is selected or a selected crate depends on it directly (cargo rejects them otherwise); a selection without such an edge (e.g. crank-desktop alone) runs without them. It is not the gate: run `just check` once before pushing.
`just check` runs the hermetic `nix flake check` gate. `just test` runs nextest,
doctests, app Vitest, and the frozen shared tests. `just model` runs the executable trainer-following model of ADR-0002 §4.4.1 and checks
that the committed fixture `crates/crank-engine/tests/fixtures/following.json` is its output (the `following-model` flake
check); `just model-fixture` regenerates it. `crank-engine`'s `refines_the_model` test reads that fixture, so the Rust
machine, its reply-row table and its constants are tied to the model without running Python at test time (#218). `just lint` runs clippy and
ESLint/stylelint/Prettier (`app/stylelint.config.mjs`: colours and sizes come from `theme.css` tokens, DESIGN §18.7 rules 1 to 3, #454; an exception goes in that file with its reason, never in a stylesheet; `app/eslint.config.js` holds the other half, #455: icons only from `lucide-react`, no `<svg>` outside `src/icons/`, no literal colour or px in `style={{}}`, jsx-a11y recommended; an exception is an inline `eslint-disable` with its reason; the selectors see only a literal `style={{…}}` and a literal `<svg>`/`createElement("svg")`/`dangerouslySetInnerHTML`: a style object built elsewhere or a dynamic tag (`const Tag = "svg"`) is not caught, so review and the layout audit (#457) cover it; live numbers render through `<Num>` (`src/Num.tsx`) or the `num` class, checked by the layout audit, #457, not by a lint rule); `just fmt` formats Rust and app files. `just bindings` exports
Rust types and command wrappers to `app/src/bindings.ts`; commit that file.
The drift check must fail after an unexported contract change. The `deps-edges`
flake check checks all-feature crate boundaries and separately walks the production
feature graph, failing if expected crates are missing or e2e is enabled.
`cargo deny check` checks dependency policy; in the flake the `deny` check runs advisories offline against the
pinned `advisory-db` input (bump: `nix flake update advisory-db`; ADR-0001 §9.5). The directory name
`advisory-db-3157b0e258782691` in flake.nix is cargo-deny's hash of the default db URL.

`crank-desktop` wires `crank-core` as Tauri commands (`app/src-tauri/src/commands.rs`, one per facade method; `ApiError` is the only
error type that crosses) and manages one `Core`. Runtime configuration: `--sim` or `CRANK_SIM=1` (anything but empty, `0`, `false`)
drives the seeded device simulator instead of Bluetooth; `CRANK_DATA_DIR` sets the directory of `crank.db` and `ride.lock` (default:
the app-data directory of the bundle identifier). **Logs and raw-frame captures (#411, #414, ADR-0001 §7.1)**: the desktop main (`app/src-tauri/src/logging.rs`,
never a library crate) installs vigil (`vigil-telemetry`, tag `0.2.0`; tests that start a pipeline need nextest, one per process). Records
are OTLP/JSON Lines, one `LogsData` per line: `logs.jsonl` (records sit under `resourceLogs[].scopeLogs[].logRecords[]`) and `traces.jsonl`,
rotated into 8 MiB segments (`logs-<UTC stamp>.jsonl`). A normal launch logs to `~/Library/Logs/io.github.gerchowl.crank/`
(Tauri's `app_log_dir`; a Finder launch has no stderr) with `CRANK_LOG` as the `EnvFilter` (default `info`), and vigil prunes segments
older than 7 days, keeps at most 6 log and 2 trace segments and under 96 MiB in all; it captures nothing. The plain-text `crank.log`/`crank.log.1`
of #411 are never deleted by the app: delete them by hand. With `CRANK_CAPTURE_DIR=<dir>` (the HIL riding session, #44 Part B) the logs go to
`<dir>` with no retention and no caps (vigil deletes nothing there; disk use is unbounded), `crank_ble` is always at TRACE and `CRANK_LOG`
only adds to it (its `crank_ble…` directives are dropped with a WARN record); `CRANK_LOG_CAPTURE_RESERVED` is reserved and refuses the capture (#432).
Each trainer connection (reconnects included) writes `<dir>/trainer-<unix secs>-<n>.fecap`, separate from the logs, the `.fecap` format that `hil_fec analyse`
and `redact` read (`hil_fec analyse --target <dir>/trainer-….fecap --out <dir>`); reconnect timings are the `reconnecting trainer` and
`trainer reconnect failed` records of `<dir>/logs.jsonl` (`body.stringValue`, with `timeUnixNano`). If that directory cannot be written the app says so and exits instead of riding uncaptured.
Ending a ride flushes the log on a blocking thread, without waiting for it; quitting gives vigil what the 3 s quit cap left after the ride end (waiting for an in-flight flush only within that), and records queued past it are lost (at most the last ~1 s batch), and a quit or crash can leave `logs.jsonl` with a partial last line (readers skip it).
The core opens the `ride{id}` and `device{kind,id}` spans (`traces.jsonl`; devices are `<trainer-id>`/`<strap-id>`, never a name or serial) and ends them when the ride is released, before vigil's shutdown (#465).
Everything stays on the machine. A capture holds raw device frames: page 81 carries the trainer serial, and `redact` masks only that;
the log can carry device names in error text and its resource carries `host.name`. Redact before committing, and do not share either file unreviewed. Streams belong to the calling window's label and are dropped when it is destroyed
or its page (re)loads (a reload keeps the webview, so the old channel would not fail). The window's link channel (`links_open`,
`Channel<LinkFrame>`, ADR-0002 §5.1.4, `crates/crank-core/src/keeper/channel.rs`) follows the same rules; it works with
or without a ride, has one frame in flight until `links_ack`, and coalesces changes into the next frame (each is the
complete state, so there is no resync); it is the only way the keeper's state reaches the page: `links_intent` answers `null` (#597), its effect arrives as the next frame, and the page applies and acks every frame, with no ordering rule; `LinkFrame.seq` is the keeper state's number, used only by `links_ack`; each device has one source, from the hand-over to a load on: a device in `riding` (held by the ride) is read from `StreamFrame.links`, every other one (a strap that missed the load included) from `LinkFrame`. **Quitting ends a ride**: an exit request (last window closed, `app.exit`, Cmd+Q, whose menu item replaces the default one, on macOS only and only after it is verified as the built-in Quit, so it calls
`app.exit(0)`; Windows and Linux get no custom menu and quit through the run-event hold-back, #306) is held back, `Core::shutdown` ends the ride (end mode `Resistance{0}`, ADR-0002 §7.9) and waits at most 2.5 s for the
devices to be released, vigil shuts down within what is left of 3 s, then the app exits (`app/src-tauri/src/lifecycle.rs`). Exits that never ask (Dock Quit, logout, system shutdown:
`NSApp terminate:` gives tao only `applicationWillTerminate:`, hence `RunEvent::Exit` and no `ExitRequested`) hit a backstop that runs
the same shutdown synchronously on `Exit`; macOS may kill a terminating app a few seconds in, so the cap must stay at or below 3 s.
`just quit-test` (macOS, GUI session) terminates a real Wry app mid-ride with `NSApp terminate:` and checks the ride was recorded `Finished`.
If the core cannot open (a second crank holds `ride.lock`) a native
dialog says so and the process exits with status 1. Tauri rejects malformed arguments with a plain string, not an `ApiError`:
`app/src/api.ts` (`api()`, `toApiError`) maps it to `{code: "internal", message}`. The UI waves (#229) call the generated commands through it.
`app/src/stream/` is the window's typed stream client (#89, ADR-0001 §4.3): one `Channel<StreamFrame>` (`StreamClient`),
`StreamProvider` (mount it where a ride is loaded: `open_stream` needs one and the stream ends with it), and the hooks
`useSubscribe(metric, opts)`, `useStatus`, `useLinks`, `useStreamError` and `useCommand`. It acks every applied frame
(`stream_ack`, cumulative, one in flight) and, on `resync` or a `seq` gap (compared by equality, `seq` wraps), re-reads
`ride_status`, `device_links` and `stream_snapshot` once. A new frame lane or re-query member (the nudges lane and
`coach_decisions`, #309, gated on #243) is added in two places: `state.ts` (field, `applyFrame` branch, `apply…`) and
`Requery.run` in `client.ts`. ESLint forbids `structuredClone` in `src/stream/` (stream data is plain JSON). Tests use
`mockIPC` with `src/stream/testBackend.ts`; `app/e2e/specs/stream.e2e.mjs` is the real-desktop freeze check, run by `just e2e` only.
The ride screen (`app/src/ride/`, #96 MVP) takes one `RideSession` per `ride_load`: the loader calls `beginRide(outcome.data)` (the previous session is
disposed), `useSession()` returns it, and the view mounts `StreamProvider` + `RideTiers` around a fixed CSS grid whose changing values sit in slots that
reserve their widest form. A finished ride routes once to `history` with the ride id until the summary route exists (#106). The forced-end note of the summary keys on `RideEnded.forced` from the `ride_end` answer (`ride/forcedEnds.ts`), never on `outcome === 'aborted'` (#346). Its test backend
(`rideBackend.ts`) lists every modelled refusal with its core `file.rs` and symbol (convention below). Loss states (#100) are read from the status loss fields (#329: `trainerLostSinceS` keys "the trainer is lost", not `state`; `reconnectAttempt`, `softStartTargetW`, `lastGapS`, `reconnectedAtS`, `userPaused`; null outside Active), the links and `ergReleased` only (`lossView`); durations are `sessionS` minus a stamp, never a timer. The rider's pause is `userPaused`, so Resume follows the core's pause under a trainer loss (#327). End is never held by a pending command (#327), nor by an engine that does not answer: every other engine request is bounded (1 s) and answers `engine_unresponsive`, which the ride view shows (#351).
`just dev` starts the window on its own data dir (`./.crank-dev`, so it never shares `ride.lock` with the installed app; override with
`CRANK_DATA_DIR`; `CRANK_SIM=1 just dev` rides without hardware). `just bundle` (sage only, outside the sandbox) builds
`.#crank-app` (an unsigned, Nix-assembled `crank.app`), signs a copy inside-out with the local `nix-tcc`
identity (by its SHA-1, the identity the requirement check verifies) and swaps it atomically into
`~/Applications/crank.app` (`scripts/install-app.sh`), so the Bluetooth TCC grant survives rebuilds
(ADR-0001 §9.3, ADR-0002 §8). The install logic is tested by `scripts/test-install-app.sh` (stub
`codesign`, temp target), the `install-app` flake check on aarch64-darwin only: it uses `renamex_np`
and the recipe is macOS-only, so on Linux the script skips with a message. `just bundle` reads the
identity's SHA-1 from the keychain at run time, never creates or changes identities, and uses no
hardened runtime or App Sandbox (library validation rejects the unsigned nix-store dylibs;
distribution signing is #47). The bundle must not depend on the Nix store at runtime (#413): the `crank-app` derivation points the Darwin stdenv's libiconv load command at the system `/usr/lib/libiconv.2.dylib` (`install_name_tool`, then an ad-hoc re-sign, all before `just bundle` signs), and the `bundle-no-nix-store` flake check (aarch64-darwin, `scripts/check-bundle-no-nix-store.sh <crank.app>`, also runnable by hand on an installed app) fails if `otool` shows `/nix/store` in a load command or rpath of any Mach-O in the bundle. A new native dependency must be linked from the system or bundled under Contents/Frameworks, never left on a store path. `just e2e` (`scripts/e2e.sh`, ADR-0001 §8.3.1) builds `.#crank-desktop-e2e` and runs the WebdriverIO specs of
`app/e2e/` (in sim mode, scratch data dir per launch) in the GUI session through Alacritty IPC (`CRANK_E2E_DIRECT=1` from a GUI terminal); output goes to the
gitignored `app/e2e-out/`. `just e2e` runs the specs in two app launches and compares screenshots across them; on Linux it skips with a message.
`just screens` awaits the UI waves (#229) and fails explicitly until then.
**Design harness (#457, DESIGN §18.7):** `just design` (= `pnpm -C app exec playwright test`, in the devShell) opens every
fixture of `app/e2e/design/scenarios.ts` at 1024×768 and 1440×900 in both themes, injects `app/e2e/design/layout-audit.js`
(reads the `theme.css` scales; controls are exactly `--control-h` except the rail's `--control-h-rail`, its own height variant, DESIGN §18.4) and runs
`@axe-core/playwright` (WCAG 2.2 A/AA). `app/e2e/design/index.html?scenario=<name>` is its own Vite root
(`app/e2e/design/vite.config.ts`), never part of the app build; it mounts the real `AppRoot` on the mockIPC backends
(`rideBackend.ts`, `devicesBackend.ts`, `workoutsBackend.ts`, `historyBackend.ts`; a new screen state is a scenario entry
plus its setup in `fixture.tsx`). Nothing is measured before the fixture settles (`window.__fixture.settled()`):
every bundled font face of `theme.css` (Inter 400/600, JetBrains Mono 400/700) is loaded, else the test fails, then
two frames pass with no mock answer in flight. The faces are `font-display: swap` and fetched lazily, so without that
load a slow fetch measured the fallback font (#504); the build sandbox has no system fonts at all. Motion is off
(`reducedMotion: "reduce"`; the one transition, an opacity fade, honours it), and the viewport is fixed per project.
Every rule starts as a warning: `app/e2e/design/baseline.json` holds the per-fixture,
per-project count of each audit and axe rule, a count above it fails, a rule absent from it fails on its first violation,
and #460 ratchets the counts down. Each rule still above zero, and each count now under its baseline ("lower it"), is
printed after the run as a `design warning:` line (`app/e2e/design/warnings.ts`; the list reporter drops annotations).
A baseline key that is no fixture × project of the run fails. `just design update` rewrites the baseline from the current
counts and prunes such keys (one worker); any other mode than `check`/`update` is refused;
commit it only with the change that moved a count, and say why in the PR. The browser is Chromium from nixpkgs
(`playwright-driver.browsers`, the devShell's `PLAYWRIGHT_BROWSERS_PATH`); `@playwright/test` is pinned to the same version
and the flake asserts it. Chromium stands in for the app's WKWebView: the WebdriverIO specs (`just e2e`) still test the
real app. The `app-e2e-design` flake check runs the harness on x86_64-linux only. **Generic layout invariants (#630):** on
every fixture × project the audit holds `layout.contained` (each element's border box inside the box that lays it out:
its parent's border box in the flow, its containing block's padding box when positioned, the window when fixed; the page
itself and a scroll container bound nothing along the axis that scrolls; an inline box only across), `layout.landmarks`
(no two landmark regions side by side overlap; a dialog is its own layer) and `layout.textOverflow` (the block holding
a text run holds all its content on each non-scrolling axis; a font's half-leading overhang past its `line-height` is
not overflow). A deviation is an entry in `app/e2e/design/audit-opts.ts` `EXEMPT` with its rules, selector and reason
(a temporary one names its issue); the audit throws on one without a reason. They replace the per-component
`layout.overflowParent`, `layout.clipped` and the spec's cut-text check. Chromium (and Playwright's WebKit build) lay
some things out unlike the app's WKWebView: #630's strip drew into the key row only there. `just design-webkit`
(`app/e2e/design/webkit.ts`, macOS only, in the GUI session, with the Xcode command line tools' `swiftc`) runs the same
audit and exemptions on every fixture at the harness's sizes in the system WKWebView (`webkit/runner.swift`, off screen,
occlusion detection off so frames run; `webkit/driver.js` opens each state as the spec does) and fails on any
invariant violation; run it with any layout change. The audit reads one fixture at a time; rule 10's
cross-state comparison of the ride's region boxes is #498. **Focus walk (#458, rule 8):** after the audit, each fixture
is tabbed from the document start until focus is back on the first stop, then Shift+Tabbed back, then its skip link is
used (`app/e2e/design/focus.ts`, the key presses and rules; `focus-audit.js`, the in-page reads). Its rules share the
baseline: `focus.ring` (a stop whose focused `outline`, or `box-shadow`, is unchanged, under 2 px, off screen or under
3 : 1 against the background it is drawn on), `focus.order` (a step against the reading order: rows top to bottom, then
left to right, a later column may start higher, 4 px tolerance; Shift+Tab not the exact reverse; the skip link not first,
not onto `#main`, or the next Tab not `#main`'s first control) and `focus.trap` (focus lost to `body` other than once at
the document's edge, a stop that is no control, a cycle that never closes). Every screen passes all three today, so none
is in the baseline and a first violation fails; each failure prints the offending stops (`<selector>: <why>`, or
`<from> -> <to>: against reading order`). **Visual baselines (#459):** on Linux each fixture × project is also compared
with its committed PNG, `app/e2e/design/screenshots/linux/<project>/<fixture>.png` (`toHaveScreenshot` after `settled()`,
Playwright's default per-pixel threshold and no `maxDiffPixels`: a missing PNG or one pixel past the threshold fails,
`updateSnapshots: "none"`). Determinism: no animation, transition or caret (`toHaveScreenshot`'s `animations: "disabled"`,
`caret: "hide"`, plus the fixture's CSS reset), only the bundled fonts (the sandbox has none other), `Date` frozen at
2026-11-02T08:00Z (`page.clock.setFixedTime`; timers still run), `Math.random` seeded (mulberry32; no fixture draws from
it today, every fixture is static data), UTC and en-US. The `app-e2e-design` check on x86_64-linux runs them; the 20× loop
of that check (#459) had zero diffs. macOS skips them (another font rasteriser and compositor draw other pixels) and
prints `design skipped (N): <reason>` after the run. `just design update` renders them afresh from the flake's
x86_64-linux `design-screenshots` package (it needs an x86_64-linux host or remote builder; never render them on macOS)
into an empty directory, so a renamed fixture leaves no PNG behind; a PNG that is no Linux fixture × project fails.
**A changed baseline PNG needs the reviewer's sign-off:** commit PNGs only with the change that moved them, say why in
the PR, and the reviewer looks at each changed image (the check's failure prints the pixel count; Playwright's
`e2e-out/design` holds the expected/actual/diff images of a local Linux run). The PNGs are about 3.2 MB for 16 fixtures
× 4 projects. Screenshots of the real app will go to `docs/design/screens/` as ADR-0001 §8 specifies.

Citing Rust from app/ (comments, docs, test backends): always `<name>.rs <symbol>` with an optional ` (~N)` line hint directly after
the symbol, e.g. ``core.rs `Core::end_ride` (~1742)``; the symbol is an identifier path (`A::b`), in backticks or not. Never a bare
`core.rs:1742` (a line number goes stale when the Rust file moves, #380, #391). A bare file mention with no symbol is fine
(`crates/crank-core/src/core.rs`). Repeat the file name for each symbol (`core.rs A (~1) / core.rs B (~2)`), and never split a
citation across lines: `scripts/check-rust-citations.py` (unit-tested by `scripts/test-check-rust-citations.py`; `just config-checks`,
the `rust-citations` and `rust-citations-test` flake checks) resolves each symbol in the cited file under `crates/` or
`app/src-tauri/` (the last `::` segment must be DEFINED there: comments and strings are stripped first, then it must be an item
(`fn`/`struct`/`enum`/`trait`/`type`/`const`/`static`/`mod`/`union`/`macro_rules!`), the type or trait an `impl` header names (not its generic
arguments: cite `ApiError`, not the `X` of `From<X>`), or an enum variant or named field; so prose such as `core.rs for the details` fails too; a base name shared
by several files must pick one by defining the symbol in exactly one non-test file, else give the path) and fails on any other line
hint on a line that mentions `.rs` (`:N`, `#LN` including GitHub URL fragments, `@N`, `LN`, `line N`, `ln N`, `l. N`, `[N]`, `(N)`, `~N`; after a file name or symbol one rule closes the whole class: a number is a line hint unless one of these case-sensitive units or count nouns follows it (checked for EVERY number up to the next `.rs` mention, so `50 Hz | 1742` fails): `%`, `Hz kHz MHz rpm bpm`, `s ms us µs ns min h`, `W kW J kJ V mA dB`, `B KiB MiB byte(s) bit(s)`, and the nouns `sample frame packet tick attempt call caller call site test case file variant field item` (singular or plural; also `call-site`, `retry`/`retries`, `entry`/`entries`) (each is something a line number is not; `lines` is deliberately absent). So `at 50 Hz` and `3 call sites` pass, while `| 1742`, `— 1742`, `(Zeile 1742)`, `1742 is wrong`, `(1742 lines)`, `1742f`, `0x6D6`, `1742 S`, `1742 h.`, `B-tree`, `50 Hz, 1742`, `in 2024`, `x 3` and `50-60 Hz` fail: rephrase. A unit may be glued to the number (`50Hz`) but must end there (not before a letter, digit, `.` or `-`, so `5 s.` at a sentence end is rephrased). Separators and up to two label words may sit before each number) and on any `(~N)` that does
not directly follow a resolved symbol. Only ` (~N)` with N >= 1 may follow the symbol: `sym 12`, `sym (12)`, `sym, 12`, `(~ 12)`, `(~0)`
fail; an issue reference `#123` beside a citation is fine. Known limits (#403): an item must start its line or follow `{`, `}` or `;`; tuple-struct fields and macro-generated items (`fn $name()`) cannot be cited (fields of struct-like enum variants can); only two label words are scanned before a number (`a b c 1742` passes), a backtick ends the scan, and superscript or fraction digits are not numbers. The scanner is a small Python lexer + regexes (stdlib only, no `syn`/ctags: neither is
in the devShell and a cargo build would make a doc check heavy).

npm licences: `scripts/check-licenses.py` gates only our own `package.json` files. There is no licence gate for transitive
**dev**Dependencies (about 446 came in with WebdriverIO, #233) by decision: they belong to the e2e harness and the shared
frozen mock, run only on developer machines and CI, and are never shipped in the app. The production tree (react, react-dom,
`@tauri-apps/api` and their transitives, which Vite bundles) is checked against an SPDX allow-list (parsed with
`AND`/`OR`/`WITH` and parentheses, `scripts/check-npm-prod-licenses.py`, unit-tested by
`scripts/test-check-npm-prod-licenses.py`). The `npm-prod-licenses` flake check runs it in CI on the installed tree;
`just npm-licenses` is the same check locally after `pnpm install`. `just config-checks` also runs the unit tests and
`scripts/test-e2e-prune.sh` (the `e2e-prune` and `npm-licenses-test` flake checks).

App icons are generated from `assets/icon/icon-1024.png`: run `just icons` after changing the mark. It runs
`tauri icon` and records every hash in `app/src-tauri/icons/MANIFEST.sha256`; the `icons-drift` flake check
fails when the source or any generated icon differs from the manifest. It proves the files match the
recorded hashes, not that `tauri icon` produced them: `icon.icns` is not reproducible (two runs on the
same input differ), so regenerate-and-compare is not possible (#165).

Every pnpm lockfile update must update the `fetchPnpmDeps` hash in flake.nix.
The fetch phase explicitly prepares darwin-arm64 and linux-x64 optional packages
with `supportedArchitectures`: pnpm 12's `--force` alone no longer fetches other
platforms (https://pnpm.io/settings/dependency-resolution#forceignoresplatform).
Keep these targets aligned with the flake systems so the store hash is portable.
The fetch phase also removes empty store directories before packing: pnpm leaves
an empty `v11/links/` on macOS only, changing the archive hash without changing
cache data (#28 cross-platform build verification).
The flake uses explicit source filesets: add each new input to the appropriate
set, including migrations, fixtures, and icons read by Tauri. New public Rust
items require documentation. Clippy pedantic warnings fail the gate; unwrap
is denied outside tests. Domain cannot depend on tokio; engine cannot depend
on BLE (including transitively). UI consumes generated Rust bindings.

On shared sage use `CARGO_BUILD_JOBS=8` and a task-specific `CARGO_TARGET_DIR`. The devShell
sets neither (a job count is a host property, not a project one): export them in your own environment
before `nix develop`; `nix develop` inherits them.
`[profile.dev] debug = "line-tables-only"` keeps file:line in backtraces and debuggers and cuts a cold dev
target dir by about a quarter (#371). sccache is deliberately not wired into the devShell. Measured: with
`CARGO_TARGET_DIR` set (every agent does) a second cold build in a fresh target dir got 0 Rust cache hits, because
sccache 0.18 hashes all `CARGO_*` environment variables except a few (`CARGO_TARGET_DIR` is the one that matters here),
so each target dir gets its own keys. Passing `--target-dir` instead gave hits (+182 of about 300 in a second build, same
worktree) but only 112 s to 102 s of wall time, which is not worth changing every agent's setup. From the sccache source,
not measured: it also hashes the compile's working directory, so separate worktrees would not share either.
Numbers: PR #386.
Wrap long commands in watcher-s1 (`watcher-s1 guide`). Hardware BLE runs require
an Alacritty IPC wrapper: flock processes are denied Bluetooth because their
responsible process is sshd-keygen-wrapper. Never modify TCC. See
[TRAINER-BLE](spec/TRAINER-BLE.md). The NEO has FE-C over BLE, no FTMS, and
allows one central at a time. Batch pedalling requests for the owner.

The flake declares no nixConfig and CI uses GitHub-hosted runners (ADR-0001
§9.4); a host-level binary cache, if any, comes from that host's own Nix
configuration. Never use --accept-flake-config or change nix.conf for this
project (architect decision, #28 round 2).

Tauri ACL JSON lists contain absolute generated paths. Rust derivations run
`scripts/relocate-tauri-permissions.py` after restoring crane artifacts:
Cargo relocates its own metadata, while this hook rebases the embedded ACL
paths using Cargo's `root-output` marker and verifies every file exists.
