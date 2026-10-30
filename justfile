# ADR-0001 §9: run all recipes inside nix develop.
check:
    nix flake check -L

# Same gate as the `public-hygiene` flake check (#122).
hygiene:
    scripts/check-public-hygiene.sh

# Same gates as the `window-config`, `window-config-test`, `install-app` (Darwin only) and
# `package-licenses` flake checks (#123, #125, #165, #170).
config-checks:
    python3 -I scripts/test-test-changed.py
    python3 -I scripts/test-relocate-tauri-permissions.py
    scripts/check-window-config.py
    scripts/check-licenses.py
    scripts/test-window-config.py
    scripts/test-install-app.sh
    scripts/test-e2e-prune.sh
    scripts/test-check-npm-prod-licenses.py

test:
    # CI's feature set (flake.nix `nextest`), so the `hil_fec` example tests run locally too.
    cargo nextest run --workspace --features crank-ble/replay,crank-ble/hil
    cargo test --workspace --doc
    pnpm -C app test
    pnpm -C shared test

# Inner loop (#371): nextest + doctests for the crates changed since `base` and their reverse
# dependencies (scripts/test-changed.py). Workspace-level files (Cargo.toml/lock, flake.*,
# rust-toolchain.toml, clippy.toml, .cargo/, .config/, any path nobody classified as non-Rust) run the
# whole workspace; a diff of non-Rust paths only runs nothing. Not a gate: run `just check` once before pushing.
test-changed base="origin/dev":
    #!/usr/bin/env bash
    set -euo pipefail
    out=$(python3 scripts/test-changed.py {{base}})
    plan=$(printf '%s\n' "$out" | sed -n 1p)
    docline=$(printf '%s\n' "$out" | sed -n 2p)
    featline=$(printf '%s\n' "$out" | sed -n 3p)
    echo "test-changed: $out"
    # Same feature set as `test` and the flake's `nextest`; the script emits the `crate/feature` flags cargo
    # accepts for the selection (crank-ble selected or a direct dependency of a selected crate, #376).
    features=(--features crank-ble/replay,crank-ble/hil)
    doc=(--workspace)
    case "$plan" in
      none) echo "test-changed: nothing to run (only non-Rust paths changed)"; exit 0 ;;
      all) sel=(--workspace) ;;
      crates\ *) sel=(); for c in ${plan#crates }; do sel+=(-p "$c"); done
        features=(); for f in ${featline#features}; do features+=(--features "$f"); done
        # Doctests need a lib target: a bin-only crate (crank-cli) makes `cargo test --doc -p` exit 101.
        doc=(); for c in ${docline#doc}; do doc+=(-p "$c"); done ;;
      *) echo "test-changed: unexpected plan '$plan'" >&2; exit 1 ;;
    esac
    # --no-tests=pass: a selected set with no tests at all (crank-cli today) is not a failure.
    cargo nextest run --no-tests=pass "${sel[@]}" ${features[@]+"${features[@]}"}
    if [ ${#doc[@]} -gt 0 ]; then cargo test "${doc[@]}" --doc; else echo "test-changed: no selected crate has a lib target; skipping doctests"; fi

# Executable model of ADR-0002 §4.4.1 (also the `following-model` flake check).
model:
    python3 docs/adr/models/trainer_following.py docs/adr/0002-ble-trainer.md crates/crank-engine/tests/fixtures/following.json

# Regenerate the fixture crank-engine's `refines_the_model` test reads (#218).
model-fixture:
    python3 docs/adr/models/trainer_following.py --emit-fixture crates/crank-engine/tests/fixtures/following.json

# FMEA validator, strict: a cited test that does not exist fails here (the `fmea` flake check
# only warns about it). `just fmea --write` re-renders the table into FMEA.md.
fmea *ARGS:
    python3 docs/safety/check_fmea.py --strict {{ARGS}}

lint:
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm -C app lint

# Production npm dependencies (bundled into the app) against the licence allow-list (#233).
npm-licenses:
    pnpm -C app licenses list --prod --json | scripts/check-npm-prod-licenses.py

fmt:
    cargo fmt --all
    pnpm -C app fmt

bindings:
    cargo run --locked -p crank-desktop --bin export-bindings

# ADR-0001 §3: regenerate frozen TypeScript reference results for Rust ports.
vectors:
    CRANK_EXPORT_VECTORS=1 pnpm -C shared exec vitest run scripts/export-vectors.test.js scripts/tree-reference.test.js

vectors-check:
    pnpm -C shared exec vitest run scripts

# Runs on its own data dir (default: ./.crank-dev, gitignored): `tauri dev` has the installed app's bundle
# identifier, so without it both would share one crank.db and one ride.lock. Override with CRANK_DATA_DIR.
dev:
    pnpm install --frozen-lockfile
    CRANK_DATA_DIR="${CRANK_DATA_DIR:-{{justfile_directory()}}/.crank-dev}" pnpm -C app exec tauri dev

# ADR-0001 §9.3 / ADR-0002 §8 (#37): assemble crank.app with Nix, install it at a stable path and sign
# it inside-out (by SHA-1, scripts/install-app.sh) with the local self-signed `nix-tcc` identity
# (g-fleet bin/nix-tcc-cert), so the TCC Bluetooth grant, keyed on the designated requirement,
# survives rebuilds. Runs on sage, outside the sandbox. No hardened runtime and no App Sandbox:
# library validation would reject the nix-store dylibs that are signed by nobody (distribution
# signing is #47). Never creates or modifies identities.
bundle:
    #!/usr/bin/env bash
    set -euo pipefail
    app="$HOME/Applications/crank.app"
    id=io.github.gerchowl.crank
    sha1=$(security find-identity -v -p codesigning | awk '/"nix-tcc"/ { print tolower($2); exit }')
    [ -n "$sha1" ] || { echo "no valid nix-tcc codesigning identity in the keychain" >&2; exit 1; }
    nix build .#crank-app --out-link result-crank-app
    scripts/install-app.sh result-crank-app/crank.app "$app" "$sha1" "$id"

# Regenerate the Tauri icon set from the /C/ mark and record its hashes (#121); `icons-drift` checks it.
icons:
    pnpm -C app exec tauri icon ../assets/icon/icon-1024.png -o src-tauri/icons
    rm -rf app/src-tauri/icons/android app/src-tauri/icons/ios app/src-tauri/icons/64x64.png
    python3 scripts/icons.py record

# ADR-0001 §8.3 (#35): build crank-desktop-e2e and run the WebdriverIO smoke spec in two app launches,
# comparing screenshots across them, in the GUI session (Alacritty IPC; CRANK_E2E_DIRECT=1 from a GUI
# terminal). Output goes to app/e2e-out (gitignored). Skips with a message on Linux.
e2e:
    scripts/e2e.sh

# macOS only, needs a GUI session: terminate a real Wry app with `NSApp terminate:` mid-ride and check
# the ride was ended (Finished, not Aborted). Cmd+Q, Dock Quit and logout all end this way.
quit-test:
    cargo run -p crank-desktop --example quit_check

screens:
    @echo 'BLOCKED: screenshot matrix awaits the UI waves (#229)'; exit 1
