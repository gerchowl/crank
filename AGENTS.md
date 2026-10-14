# Working on crank

Implement the decisions in [ADR-0001](docs/adr/0001-architecture.md) and
[ADR-0002](docs/adr/0002-ble-trainer.md). Domain implementation is tracked in
#29–#45; this scaffold contains no ride logic. `shared/` is frozen in place;
`design/prototype/` (#49) remains a separate package using the mock.

Use branches `<type>/<issue>-<slug>` and open PRs into `dev`. Before pushing,
`git fetch origin` and `git rebase origin/dev`. Use conventional commits with
an actual `Refs: #N` trailer. Never push main/dev, bypass hooks, stash, touch
spike worktrees, or terminate processes you did not start.

Run everything in `nix develop`; first run `pnpm install --frozen-lockfile`.
`just check` runs the hermetic `nix flake check` gate. `just test` runs nextest,
doctests, app Vitest, and the frozen shared tests. `just lint` runs clippy and
ESLint/Prettier; `just fmt` formats Rust and app files. `just bindings` exports
Rust types and command wrappers to `app/src/bindings.ts`; commit that file.
The drift check must fail after an unexported contract change. The `deps-edges`
flake check checks all-feature crate boundaries and separately walks the production
feature graph, failing if expected crates are missing or e2e is enabled.
`cargo deny check` checks dependency policy; in the flake the `deny` check runs advisories offline against the
pinned `advisory-db` input (bump: `nix flake update advisory-db`; ADR-0001 §9.5). The directory name
`advisory-db-3157b0e258782691` in flake.nix is cargo-deny's hash of the default db URL.

`just dev` starts the placeholder window. `just bundle` (sage only, outside the sandbox) builds
`.#crank-app` (an unsigned, Nix-assembled `crank.app`), signs a copy inside-out with the local `nix-tcc`
identity (by its SHA-1, the identity the requirement check verifies) and swaps it atomically into
`~/Applications/crank.app` (`scripts/install-app.sh`), so the Bluetooth TCC grant survives rebuilds
(ADR-0001 §9.3, ADR-0002 §8). The install logic is tested by `scripts/test-install-app.sh` (stub
`codesign`, temp target), the `install-app` flake check on aarch64-darwin only: it uses `renamex_np`
and the recipe is macOS-only, so on Linux the script skips with a message. `just bundle` reads the
identity's SHA-1 from the keychain at run time, never creates or changes identities, and uses no
hardened runtime or App Sandbox (library validation rejects the unsigned nix-store dylibs;
distribution signing is #47). `just e2e` (`scripts/e2e.sh`, ADR-0001 §8.3.1) builds `.#crank-desktop-e2e` and runs the WebdriverIO specs of
`app/e2e/` in the GUI session through Alacritty IPC (`CRANK_E2E_DIRECT=1` from a GUI terminal); output goes to the
gitignored `app/e2e-out/`. `just e2e` runs the specs in two app launches and compares screenshots across them; on Linux it skips with a message.
`just screens` awaits the UI waves (#229) and fails explicitly until then. Playwright browsers are excluded from the devShell until #35
provides the harness. Screenshots will go to `docs/design/screens/` as ADR-0001 §8 specifies.

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

On shared sage use `CARGO_BUILD_JOBS=8` and a task-specific `CARGO_TARGET_DIR`.
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
