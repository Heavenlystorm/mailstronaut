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
          cargoHash = "sha256-EbgPgrRKV8i6lLA2ZkTaxgtg5AouuN4J6mFDf8Abitw=";
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit pname version;
            hash = cargoHash;
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = ./Cargo.lock;
            };
          };
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
            ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            CARGO_NET_OFFLINE = "true";
            CARGO_CACHE_AUTO_CLEAN_FREQUENCY = "never";
            # SpiderMonkey xsum disables format warnings; retain other hardening.
            hardeningDisable = [ "format" ];
            postPatch = ''
              # Nested mozjs metadata requires writable vendor sources. Retain
              # source timestamps: Crane normalizes restored artifacts to epoch 1.
              cp -r --preserve=timestamps "$cargoVendorDir" "$NIX_BUILD_TOP/vendor"
              chmod -R u+w "$NIX_BUILD_TOP/vendor"
              export cargoVendorDir="$NIX_BUILD_TOP/vendor"
              substituteInPlace "$cargoVendorDir/.cargo/config.toml" \
                --subst-var-by vendor "$cargoVendorDir"
              # Crane consumes root config.toml; nested Cargo uses .cargo/config.toml.
              cp --preserve=timestamps "$cargoVendorDir/.cargo/config.toml" "$cargoVendorDir/config.toml"
              for crate in "$cargoVendorDir"/source-registry-0/mozjs_{locale,properties,collator,normalizer}_glue-* \
                "$cargoVendorDir"/source-registry-0/mozjs_unicode_bidi_ffi-*; do
                cp --preserve=timestamps Cargo.lock "$crate/Cargo.lock"
              done
            '';
            preBuild = ''
              export AS="$CC"
            '';
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
            preBuild = common.preBuild + ''
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
              # Servo initializes its verifier even for memory-backed documents.
              export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)}:''${LD_LIBRARY_PATH:-}"
              export LIBGL_ALWAYS_SOFTWARE=1
              export EGL_PLATFORM=surfaceless
              export __EGL_VENDOR_LIBRARY_FILENAMES="${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json"
              export LIBGL_DRIVERS_PATH="${pkgs.mesa}/lib/dri"
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
              paths = [r["executable"] for r in records if r.get("target", {}).get("name") == sys.argv[2] and r.get("executable")]
              assert len(paths) == 1, "expected one compiled credential fixture"
              print(paths[0])
              ' "$NIX_BUILD_TOP/private-fixtures.json" "$1"
              }
              credential_fixture="$(fixture_path credentials)"
              oauth_fixture="$(fixture_path oauth)"
              MAILSTRONAUT_CREDENTIAL_FIXTURE="$credential_fixture" \
                bash experiments/p02-accounts/credentials.sh
              MAILSTRONAUT_CREDENTIAL_FIXTURE="$credential_fixture" \
                MAILSTRONAUT_OAUTH_FIXTURE="$oauth_fixture" \
                bash experiments/p02-accounts/credentials.sh
            '';
            meta = {
              description = "Local email client for COSMIC; development prototype";
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
                shellcheck ${self}/ci/check.sh \
                  ${self}/experiments/p02-accounts/credentials.sh \
                  ${self}/experiments/p02-accounts/gui.sh \
                  ${self}/experiments/p04-inbox/gui.sh
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
          # Retain the experiment's entry point without a second environment.
          servo-probe = self.devShells.${system}.default;
          # Optional isolated-window acceptance tools; retain every development
          # toolchain/environment setting without relying on inputsFrom.
          gui-test = self.devShells.${system}.default.overrideAttrs (old: {
            MAILSTRONAUT_TEST_THEME_DATA = "${pkgs.cosmic-settings}/share";
            MAILSTRONAUT_TEST_AT_SPI_BUS = "${pkgs.at-spi2-core}/libexec/at-spi-bus-launcher";
            GI_TYPELIB_PATH = "${pkgs.at-spi2-core}/lib/girepository-1.0:${pkgs.glib.out}/lib/girepository-1.0:${pkgs.gobject-introspection}/lib/girepository-1.0";
            MAILSTRONAUT_TEST_AT_SPI_PYTHON = "${
              pkgs.python3.withPackages (python: [
                python.pyatspi
                python.pygobject3
              ])
            }/bin/python3";
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.weston
              pkgs.xorg-server
              pkgs.xdotool
              pkgs.imagemagick
              pkgs.dbus
              pkgs.xclip
              pkgs.zenity
              pkgs.gnome-keyring
              pkgs.libsecret
            ];
          });
          default = pkgs.mkShell {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibraries pkgs);
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            shellHook = ''
              export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-$(nproc)}"
              export CMAKE_BUILD_PARALLEL_LEVEL="$CARGO_BUILD_JOBS"
              export MAKEFLAGS="-j$CARGO_BUILD_JOBS"
              export AS="$CC"
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)}:$LD_LIBRARY_PATH"
              NIX_HARDENING_ENABLE=" ''${NIX_HARDENING_ENABLE:-} "
              export NIX_HARDENING_ENABLE=''${NIX_HARDENING_ENABLE// format / }
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
