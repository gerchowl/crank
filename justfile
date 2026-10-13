# ADR-0001 §9: run all recipes inside nix develop.
check:
    nix flake check -L

test:
    cargo nextest run --workspace
    cargo test --workspace --doc
    pnpm -C app test
    pnpm -C shared test

lint:
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm -C app lint

fmt:
    cargo fmt --all
    pnpm -C app fmt

bindings:
    cargo run --locked -p crank-desktop --bin export-bindings

dev:
    pnpm -C app exec tauri dev

# Signed bundle implementation belongs to #37 (ADR-0001 §12).
bundle:
    nix build .#crank-app

# The macOS embedded WebDriver harness is gated by #35.
e2e:
    @echo 'BLOCKED: embedded WebDriver harness requires #35'; exit 1

screens:
    @echo 'BLOCKED: screenshot matrix requires #35'; exit 1
