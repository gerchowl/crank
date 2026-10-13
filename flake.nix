{
  description = "crank production workspace (ADR-0001 §9)";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs = { self, nixpkgs, crane, rust-overlay }: let
    systems = [ "aarch64-darwin" "x86_64-linux" ];
    eachSystem = nixpkgs.lib.genAttrs systems;
    project = system: let
      pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
      inherit (pkgs) lib;
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
      source = fileset: lib.fileset.toSource { root = ./.; inherit fileset; };
      cargoFiles = craneLib.fileset.commonCargoSources ./.;
      rustFiles = lib.fileset.unions [ cargoFiles ./clippy.toml ./scripts/relocate-tauri-permissions.py ./spec/knowledge-pack/rules.md
        ./app/src-tauri/tauri.conf.json ./app/src-tauri/capabilities ./app/src-tauri/Info.plist ./app/src-tauri/icons ];
      frontendSrc = source (lib.fileset.unions [
        (lib.fileset.difference ./app ./app/src-tauri) ./shared
        ./package.json ./pnpm-lock.yaml ./pnpm-workspace.yaml ]);
      pnpmDeps = pkgs.fetchPnpmDeps {
        pname = "crank"; src = frontendSrc; pnpm = pkgs.pnpm_12;
        fetcherVersion = 4; hash = "sha256-5COScyIoYRxzEA36gwea0CUnA6nqi72MsKjltMkzclQ=";
      };
      frontend = name: command: pkgs.stdenvNoCC.mkDerivation {
        pname = name; version = "0.1.0"; src = frontendSrc;
        inherit pnpmDeps;
        nativeBuildInputs = [ pkgs.nodejs pkgs.pnpm_12 pkgs.pnpmConfigHook ];
        buildPhase = command;
        installPhase = "mkdir -p $out; cp -r app/dist/. $out/";
      };
      appDist = frontend "crank-app-dist" "pnpm -C app build";
      common = {
        pname = "crank"; version = "0.1.0"; src = source rustFiles;
        strictDeps = true;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
          pkgs.webkitgtk_4_1 pkgs.gtk3 pkgs.libsoup_3 pkgs.openssl ];
        # Rust checks share crane's default release-profile artifacts.
        cargoExtraArgs = "--workspace --locked";
      };
      cargoArtifacts = craneLib.buildDepsOnly (common // { src = source cargoFiles; });
      rustCheck = common // {
        inherit cargoArtifacts;
        nativeBuildInputs = common.nativeBuildInputs ++ [ pkgs.python3 ];
        postConfigure = "python3 scripts/relocate-tauri-permissions.py";
      };
      desktop = features: craneLib.buildPackage (rustCheck // {
        pname = "crank-desktop"; cargoExtraArgs = "--locked -p crank-desktop --bin crank-desktop --features custom-protocol${features}";
        doCheck = false;
        preBuild = ''export TAURI_CONFIG='{"build":{"frontendDist":"${appDist}"}}' '';
      });
      checks = {
        build = desktop "";
        fmt = craneLib.cargoFmt { pname = "crank"; version = "0.1.0"; src = source cargoFiles; };
        clippy = craneLib.cargoClippy (rustCheck // { cargoClippyExtraArgs = "--all-targets -- -D warnings"; });
        nextest = craneLib.cargoNextest rustCheck;
        doctest = craneLib.cargoDocTest rustCheck;
        doc = craneLib.cargoDoc (rustCheck // { RUSTDOCFLAGS = "-D warnings"; });
        deny = craneLib.cargoDeny { pname = "crank"; version = "0.1.0"; src = source cargoFiles; cargoDenyChecks = "bans licenses sources"; };
        bindings-drift = craneLib.mkCargoDerivation (rustCheck // {
          src = source (lib.fileset.unions [ rustFiles ./app/src/bindings.ts ]);
          buildPhaseCargoCommand = ''
            bindingsDir=$(mktemp -d)
            cargo run --release --locked -p crank-desktop --bin export-bindings -- "$bindingsDir/bindings.ts"
            diff -u app/src/bindings.ts "$bindingsDir/bindings.ts"
          '';
          doInstallCargoArtifacts = false; installPhase = "touch $out";
        });
        deps-edges = craneLib.mkCargoDerivation (rustCheck // {
          src = source (lib.fileset.unions [ rustFiles ./scripts/deps-edges.py ]);
          nativeBuildInputs = common.nativeBuildInputs ++ [ pkgs.python3 ];
          buildPhaseCargoCommand = ''
            productionMetadata=$(mktemp)
            cargo metadata --locked --format-version 1 --features crank-desktop/custom-protocol > "$productionMetadata"
            cargo metadata --locked --format-version 1 --all-features | python3 scripts/deps-edges.py "$productionMetadata"
          '';
          doInstallCargoArtifacts = false; installPhase = "touch $out";
        });
      } // lib.genAttrs [ "app-typecheck" "app-lint" "app-test" "shared-test" ] (name:
        (frontend name (if name == "shared-test" then "pnpm -C shared test"
          else "pnpm -C app ${lib.removePrefix "app-" name}" )).overrideAttrs {
            installPhase = "touch $out";
          });
    in {
      inherit checks;
      packages = { app-dist = appDist; crank-desktop = desktop "";
        crank-desktop-e2e = desktop ",e2e";
        crank-cli = craneLib.buildPackage (rustCheck // {
          pname = "crank-cli"; cargoExtraArgs = "--locked -p crank-cli"; doCheck = false;
        });
      };
      devShells.default = craneLib.devShell {
        inputsFrom = [ (desktop "") ];
        packages = [ toolchain pkgs.nodejs pkgs.pnpm_12 pkgs.just pkgs.cargo-nextest
          pkgs.cargo-deny pkgs.cargo-fuzz pkgs.python3 ];
        CARGO_BUILD_JOBS = "8";
      };
    };
  in {
    checks = eachSystem (system: (project system).checks);
    packages = eachSystem (system: (project system).packages);
    devShells = eachSystem (system: (project system).devShells);
  };
}
