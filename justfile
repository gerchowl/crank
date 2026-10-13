# ADR-0001 §9: run all recipes inside nix develop.
check:
    nix flake check -L

# Same gate as the `public-hygiene` flake check (#122).
hygiene:
    scripts/check-public-hygiene.sh

# Same gates as the `window-config`, `window-config-test`, `install-app` (Darwin only) and
# `package-licenses` flake checks (#123, #125, #165, #170).
config-checks:
    scripts/check-window-config.py
    scripts/check-licenses.py
    scripts/test-window-config.py
    scripts/test-install-app.sh

test:
    cargo nextest run --workspace
    cargo test --workspace --doc
    pnpm -C app test
    pnpm -C shared test

# Executable model of ADR-0002 §4.4.1 (also the `following-model` flake check).
model:
    python3 docs/adr/models/trainer_following.py

lint:
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm -C app lint

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

dev:
    pnpm install --frozen-lockfile
    pnpm -C app exec tauri dev

# ADR-0001 §9.3 / ADR-0002 §8 (#37): assemble crank.app with Nix, install it at a stable path and sign
# it inside-out (by SHA-1, scripts/install-app.sh) with the local self-signed `nix-tcc` identity (g-fleet bin/nix-tcc-cert), so the TCC
# Bluetooth grant, keyed on the designated requirement, survives rebuilds. Runs on sage, outside the
# sandbox. No hardened runtime and no App Sandbox: library validation would reject the nix-store dylibs
# that are signed by nobody (distribution signing is #47). Never creates or modifies identities.
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

# The macOS embedded WebDriver harness is gated by #35.
e2e:
    @echo 'BLOCKED: embedded WebDriver harness requires #35'; exit 1

screens:
    @echo 'BLOCKED: screenshot matrix requires #35'; exit 1
