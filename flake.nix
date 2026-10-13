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
      cargoFiles = lib.fileset.difference (craneLib.fileset.commonCargoSources ./.) ./crates/crank-fec/fuzz;
      rustFiles = lib.fileset.unions [ cargoFiles ./crates/crank-fec/tests/fixtures ./clippy.toml ./crates/crank-domain/tests/vectors ./scripts/relocate-tauri-permissions.py ./spec/knowledge-pack/rules.md
        ./app/src-tauri/tauri.conf.json ./app/src-tauri/capabilities ./app/src-tauri/Info.plist ./app/src-tauri/icons ];
      frontendSrc = source (lib.fileset.unions [
        (lib.fileset.difference ./app ./app/src-tauri) ./shared
        ./crates/crank-domain/tests/vectors
        ./package.json ./pnpm-lock.yaml ./pnpm-workspace.yaml ]);
      pnpmDeps = pkgs.fetchPnpmDeps {
        pname = "crank"; src = frontendSrc; pnpm = pkgs.pnpm_12;
        # pnpm 12 --force still filters optional dependencies by host platform.
        # Fetch both supported targets so one hash works on macOS and Linux.
        prePnpmInstall = ''pnpm config set --location=project --json supportedArchitectures '["darwin-arm64","linux-x64"]' '';
        # pnpm leaves an empty v11/links directory on macOS but not Linux.
        # Empty store directories carry no cache data; normalize before packing.
        postInstall = ''find "$storePath" -type d -empty -delete'';
        fetcherVersion = 4; hash = "sha256-iEoUtgzaQ8Zz7/R5OoZGuXh0p7APj5hsL2IE0Xnk8VA=";
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
      cli = craneLib.buildPackage (rustCheck // {
        pname = "crank-cli"; cargoExtraArgs = "--locked -p crank-cli"; doCheck = false;
      });
      # ADR-0001 §9.3: file layout only. Tauri's bundler and codesign do not run in the
      # sandbox; `just bundle` copies this to ~/Applications and signs it with nix-tcc.
      crankApp = pkgs.runCommand "crank-app" { } ''
        app=$out/crank.app/Contents
        mkdir -p $app/MacOS $app/Resources
        install -m755 ${desktop ""}/bin/crank-desktop $app/MacOS/crank-desktop
        install -m755 ${cli}/bin/crank $app/MacOS/crank
        install -m644 ${./app/src-tauri/icons/icon.icns} $app/Resources/icon.icns
        install -m644 ${./app/src-tauri/Info.plist} $app/Info.plist
      '';
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
        scales-parity = craneLib.mkCargoDerivation (rustCheck // {
          src = source (lib.fileset.unions [ rustFiles
            (lib.fileset.difference ./app ./app/src-tauri) ./shared
            ./package.json ./pnpm-lock.yaml ./pnpm-workspace.yaml ]);
          inherit pnpmDeps;
          nativeBuildInputs = rustCheck.nativeBuildInputs ++ [ pkgs.nodejs pkgs.pnpm_12 pkgs.pnpmConfigHook ];
          buildPhaseCargoCommand = ''
            cargo run --release --locked -p crank-domain --example export_scale_edges > "$TMPDIR/rust-edges.json"
            CRANK_SCALE_EDGES="$TMPDIR/rust-edges.json" pnpm -C app exec vitest run tests/scales-parity.test.ts
          '';
          doInstallCargoArtifacts = false; installPhase = "touch $out";
        });
        # The Tauri icon set must be the output of `just icons` for the committed /C/ mark (#121).
        icons-drift = pkgs.runCommand "icons-drift" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          cp -r ${source (lib.fileset.unions [ ./scripts/icons.py ./assets/icon/icon-1024.png ./app/src-tauri/icons ])} src
          cd src && python3 -I scripts/icons.py check
          touch $out
        '';
        # Executable model of the trainer-following machine (ADR-0002 §4.4.1, #54).
        following-model = pkgs.runCommand "following-model" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python3 ${./docs/adr/models/trainer_following.py} ${./docs/adr/0002-ble-trainer.md}
          touch $out
        '';
      } // lib.genAttrs [ "app-typecheck" "app-lint" "app-test" "shared-test" "vectors-drift" ] (name:
        (frontend name (if name == "shared-test" then "pnpm -C shared test"
          else if name == "vectors-drift" then "pnpm -C shared exec vitest run scripts"
          else "pnpm -C app ${lib.removePrefix "app-" name}" )).overrideAttrs {
            installPhase = "touch $out";
          });
    in {
      inherit checks;
      packages = { app-dist = appDist; crank-desktop = desktop "";
        crank-desktop-e2e = desktop ",e2e";
        crank-cli = cli;
      } // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin { crank-app = crankApp; };
      devShells.fuzz = pkgs.mkShell {
        packages = [ (pkgs.rust-bin.selectLatestNightlyWith (t: t.minimal.override {
          extensions = [ "rust-src" ];
        })) pkgs.cargo-fuzz ];
        CARGO_BUILD_JOBS = "8";
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
