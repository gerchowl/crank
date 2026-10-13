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
The drift check must fail after an unexported contract change. `cargo metadata
--locked --format-version 1 | python3 scripts/deps-edges.py` checks boundaries.
`cargo deny check bans licenses sources` checks dependency policy.

`just dev` starts the placeholder window. `just bundle` is the Nix package
hook for the signing implementation in #37; #37 owns `crank-app` assembly,
installation at `~/Applications/crank.app`, and stable nix-tcc signing.
`just e2e` and `just screens` await the embedded macOS WebDriver gate #35;
they fail explicitly until it lands. Screenshots will go to
`docs/design/screens/` as ADR-0001 §8 specifies. Default Tauri icons are used
until the app icon source lands in #48; regenerate using `tauri icon` then.

Every pnpm lockfile update must update the `fetchPnpmDeps` hash in flake.nix.
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

The fleet cache is provided by host configuration (g-fleet
`modules/fleet-cache.nix`). Non-fleet machines build using cache.nixos.org
and source. The flake declares no nixConfig; never use --accept-flake-config
or change nix.conf for this project (architect decision, #28 round 2).
