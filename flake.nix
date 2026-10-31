{
  description = "Mailstronaut: Rust workspace, development environment, and checks";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane/692f7e9ef2ece8125b466f66f2af532b3edaed0d";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      debugToolchainFor =
        pkgs:
        pkgs.rust-bin.selectLatestNightlyWith (
          toolchain:
          toolchain.default.override {
            extensions = [
              "rustc-codegen-cranelift-preview"
              "rust-src"
              "rust-analyzer"
            ];
          }
        );
      runtimeLibraries =
        pkgs: with pkgs; [
          wayland
          libxkbcommon
          fontconfig
          freetype
          libGL
          vulkan-loader
          libx11
          libxcursor
          libxi
          libxrandr
          libxcb
          openssl
          zstd
        ];
      packageFor =
        pkgs:
        let
          craneLib = crane.mkLib pkgs;
          pname = "mailstronaut";
          version = "0.1.0";
          target = pkgs.stdenv.hostPlatform.rust.rustcTarget;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            # Exclude the nightly-only .cargo development backend overrides;
            # this package uses the pinned stable LLVM toolchain.
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./README.md
              ./LICENSE
              ./crates
              ./assets
              ./examples
              ./docs/CONFIGURATION.md
              ./experiments/p02-accounts/credentials.sh
            ];
          };
          # fetchCargoVendor hashes its staging output. Keep its source limited
          # to Cargo.lock so application edits do not change dependency inputs.
          cargoHash = "sha256-cH8i/Y4P8Ms72wArUgp5d/+XY6vGoZ4GSUUPFSzpNRs=";
          cargoDeps =
            (pkgs.rustPlatform.fetchCargoVendor {
              inherit pname version;
              hash = cargoHash;
              # Immutable upstream source cache; avoid requiring the Snix forge
              # during every clean CI build. Cargo's revision remains unchanged.
              nativeBuildInputs = [
                pkgs.gitMinimal
                pkgs.xz
              ];
              preBuild = ''
                mkdir -p "$TMPDIR/snix-source"
                tar -xJf ${./third-party/snix-f393b22.git.tar.xz} -C "$TMPDIR/snix-source"
                snix_repo="$TMPDIR/snix-source/repository"
                test "$(git --git-dir="$snix_repo" rev-parse HEAD)" = f393b22aa9b8e0e74c9e8d67f15454409299e510
                test "$(git --git-dir="$snix_repo" rev-parse HEAD^{tree})" = 45d3546fc8de18b759530d4d324812fc6e3272de
                export GIT_CONFIG_GLOBAL="$TMPDIR/mailstronaut-gitconfig"
                git config --file "$GIT_CONFIG_GLOBAL" protocol.file.allow always
                git config --file "$GIT_CONFIG_GLOBAL" "url.file://$snix_repo.insteadOf" https://git.snix.dev/snix/snix.git
              '';
              src = pkgs.lib.fileset.toSource {
                root = ./.;
                fileset = ./Cargo.lock;
              };
            }).overrideAttrs
              (old: {
                # Crane reads root config.toml. Keep sources at one immutable store
                # path across builders: CI's unsandboxed temporary paths differ.
                buildCommand = old.buildCommand + ''
                  substitute "$out/.cargo/config.toml" "$out/config.toml" \
                    --subst-var-by vendor "$out"
                '';
              });
          common = {
            inherit pname version src;
            cargoVendorDir = cargoDeps;
            cargoExtraArgs = "--workspace --all-features --locked --offline --target ${target}";
            strictDeps = true;
            dontUseCmakeConfigure = true;
            dontUseNinjaBuild = true;
            dontUseNinjaCheck = true;
            dontUseNinjaInstall = true;
            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
              clang
              cmake
              ninja
              python3
              perl
              libclang
              llvm
              clippy
              rustfmt
              cacert
            ];
            buildInputs = with pkgs; [
              libxkbcommon
              wayland
              fontconfig
              freetype
              openssl
              zstd
              libxcb
            ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            CARGO_NET_OFFLINE = "true";
            CARGO_CACHE_AUTO_CLEAN_FREQUENCY = "never";
          };
          cargoArtifacts = craneLib.buildDepsOnly common;
        in
        craneLib.buildPackage (
          common
          // {
            inherit cargoArtifacts;
            passthru = { inherit cargoDeps cargoArtifacts cargoHash; };
            nativeCheckInputs = [
              pkgs.dbus
              pkgs.gnome-keyring
            ];
            preBuild = ''
              # Reject source/lint failures before linking the large executable.
              cargo fmt --all -- --check
              cargo clippy --jobs "$NIX_BUILD_CORES" --release \
                --target ${pkgs.stdenv.hostPlatform.rust.rustcTarget} \
                --workspace --all-targets --all-features --locked --offline -- -D warnings
            '';
            postInstall = ''
              install -Dm444 ${
                pkgs.makeDesktopItem {
                  name = builtins.readFile ./assets/app-id;
                  desktopName = "Mailstronaut";
                  exec = "mailstronaut";
                }
              }/share/applications/*.desktop -t "$out/share/applications"
            '';
            postFixup = ''
              wrapProgram "$out/bin/mailstronaut" --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)}
            '';
            preCheck = ''
              export XDG_RUNTIME_DIR="$NIX_BUILD_TOP/renderer-runtime"
              export XDG_CACHE_HOME="$NIX_BUILD_TOP/renderer-cache"
              mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CACHE_HOME"
              chmod 700 "$XDG_RUNTIME_DIR"
              export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)}:''${LD_LIBRARY_PATH:-}"
              export FONTCONFIG_FILE="${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}"
            '';
            postCheck = ''
              # Reuse the just-tested workspace feature graph. Building only the
              # credential target again changes unified features and recompiles
              # dependencies even though its test executable already exists.
              cargo test --release --workspace --all-features --locked --offline \
                --target ${target} --no-run --message-format=json \
                > "$NIX_BUILD_TOP/private-fixtures.json"
              fixture_path() {
                python3 -c '
              import json, sys
              records = [json.loads(line) for line in open(sys.argv[1])]
              paths = [r["executable"] for r in records if r.get("target", {}).get("name") == sys.argv[2] and r.get("executable") and r.get("profile", {}).get("test")]
              assert len(paths) == 1, "expected one compiled test fixture"
              print(paths[0])
              ' "$NIX_BUILD_TOP/private-fixtures.json" "$1"
              }
              credential_fixture="$(fixture_path credentials)"
              oauth_fixture="$(fixture_path oauth)"
              account_fixture="$(fixture_path mailstronaut)"
              MAILSTRONAUT_CREDENTIAL_FIXTURE="$credential_fixture" \
                MAILSTRONAUT_ACCOUNT_ACTION_FIXTURE="$account_fixture" \
                bash experiments/p02-accounts/credentials.sh
              MAILSTRONAUT_CREDENTIAL_FIXTURE="$credential_fixture" \
                MAILSTRONAUT_OAUTH_FIXTURE="$oauth_fixture" \
                bash experiments/p02-accounts/credentials.sh
            '';
            meta = {
              description = "Native Wayland email client built with GPUI Kit and Blitz; development prototype";
              homepage = "https://github.com/eduardz1/mailstronaut";
              license = pkgs.lib.licenses.gpl3Only;
              mainProgram = "mailstronaut";
              platforms = systems;
            };
          }
        );
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          mailstronaut = packageFor pkgs;
          default = self.packages.${system}.mailstronaut;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          workspace = self.packages.${system}.mailstronaut;
          nix-format = pkgs.runCommand "mailstronaut-nix-format" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
            nixfmt --check ${self}/flake.nix ${self}/examples/config.nix ${self}/examples/home-manager.nix
            touch "$out"
          '';
          ci-lint =
            pkgs.runCommand "mailstronaut-ci-lint"
              {
                nativeBuildInputs = [
                  pkgs.actionlint
                  pkgs.shellcheck
                ];
              }
              ''
                actionlint ${self}/.github/workflows/ci.yml
                shellcheck ${self}/ci/check.sh ${self}/ci/nix-check.sh \
                  ${self}/experiments/p01-gpui/wayland.sh \
                  ${self}/experiments/p02-accounts/credentials.sh \
                  ${self}/experiments/p02-accounts/gui.sh \
                  ${self}/experiments/p06-search/reproduce.sh \
                  ${self}/experiments/p06-search/run-release.sh
                touch "$out"
              '';
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          # Lance/Prost generates protocol bindings during the isolated search probe.
          search-probe = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.protobuf
              pkgs.strace
            ];
          });
          # Test-only independent model oracle; never part of the Rust application.
          search-reference = self.devShells.${system}.search-probe.overrideAttrs (_old: {
            MAILSTRONAUT_REFERENCE_PYTHON = "${
              pkgs.python3.withPackages (python: [
                python.torch
                python.transformers
                python.numpy
                python.safetensors
              ])
            }/bin/python3";
          });
          # Optional isolated-window acceptance tools; retain every development
          # toolchain/environment setting without relying on inputsFrom.
          gui-test = self.devShells.${system}.default.overrideAttrs (old: {
            LIBGL_DRIVERS_PATH = "${pkgs.mesa}/lib/dri";
            __EGL_VENDOR_LIBRARY_FILENAMES = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
            MAILSTRONAUT_TEST_VULKAN_ICD = "${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${pkgs.stdenv.hostPlatform.parsed.cpu.name}.json";
            FONTCONFIG_FILE = "${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}";
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.sway-unwrapped
              pkgs.grim
              pkgs.wtype
              (pkgs.wlrctl.overrideAttrs (old: {
                # Keep a stable pointer capability for the isolated headless
                # fixture. The opt-in test tool mode does not patch the app.
                patches = (old.patches or [ ]) ++ [ ./experiments/p01-gpui/wlrctl-headless.patch ];
              }))
              pkgs.wl-clipboard
              pkgs.imagemagick
              pkgs.dbus
              pkgs.zenity
              pkgs.gnome-keyring
              pkgs.libsecret
            ];
          });
          default = pkgs.mkShell {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibraries pkgs);
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            shellHook = ''
              export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-$(nproc)}"
              export CMAKE_BUILD_PARALLEL_LEVEL="$CARGO_BUILD_JOBS"
              export MAKEFLAGS="-j$CARGO_BUILD_JOBS"
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)}:$LD_LIBRARY_PATH"
            '';
            CARGO_UNSTABLE_CODEGEN_BACKEND = "true";
            CARGO_UNSTABLE_PROFILE_RUSTFLAGS = "true";
            CARGO_PROFILE_DEV_CODEGEN_BACKEND = "cranelift";
            CARGO_PROFILE_TEST_CODEGEN_BACKEND = "cranelift";
            CARGO_PROFILE_RELEASE_CODEGEN_BACKEND = "llvm";
            CARGO_PROFILE_DEV_RUSTFLAGS = "-Clinker=${pkgs.clang}/bin/clang -Clink-arg=--ld-path=${pkgs.wild}/bin/wild";
            CARGO_PROFILE_TEST_RUSTFLAGS = "-Clinker=${pkgs.clang}/bin/clang -Clink-arg=--ld-path=${pkgs.wild}/bin/wild";
            packages = with pkgs; [
              (debugToolchainFor pkgs)
              clang
              wild
              cmake
              ninja
              python3
              perl
              libclang
              llvm
              pkg-config
              libxkbcommon
              wayland
              fontconfig
              freetype
              openssl
              zstd
              libxcb
              nixfmt
              git
              actionlint
              shellcheck
              time
              openssl
            ];
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
