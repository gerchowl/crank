# ADR-0001 §9: run all recipes inside nix develop.
check:
    nix flake check -L

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
    CRANK_EXPORT_VECTORS=1 pnpm -C shared exec vitest run scripts/export-vectors.test.js

vectors-check:
    pnpm -C shared exec vitest run scripts

dev:
    pnpm install --frozen-lockfile
    pnpm -C app exec tauri dev

# Signed bundle implementation belongs to #37 (ADR-0001 §12).
bundle:
    @echo 'BLOCKED: app assembly and nix-tcc signing require #37'; exit 1

# The macOS embedded WebDriver harness is gated by #35.
e2e:
    @echo 'BLOCKED: embedded WebDriver harness requires #35'; exit 1

screens:
    @echo 'BLOCKED: screenshot matrix requires #35'; exit 1
