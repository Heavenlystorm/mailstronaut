{
  description = "Mailstronaut: Rust workspace, development environment, and checks";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
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
        pkgs.rustPlatform.buildRustPackage {
          pname = "mailstronaut";
          version = "0.1.0";
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
            ];
          };
          # importCargoLock flattens names and collides on registry/git dpi 0.1.2.
          # Standard fetchCargoVendor keeps separate directories for each source.
          cargoHash = "sha256-E1C3ZafnEyzoTYbBPMm66P35kb240lhZ6TcaRmOvqck=";
          cargoBuildFlags = [
            "--workspace"
            "--all-features"
          ];
          cargoTestFlags = [
            "--workspace"
            "--all-features"
          ];
          strictDeps = true;
          # Native dependencies invoke these tools from build.rs; Cargo owns
          # the outer derivation's configure/build/check/install phases.
          dontUseCmakeConfigure = true;
          dontUseNinjaBuild = true;
          dontUseNinjaCheck = true;
          dontUseNinjaInstall = true;
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.makeWrapper
            pkgs.clang
            pkgs.cmake
            pkgs.ninja
            pkgs.python3
            pkgs.perl
            pkgs.libclang
            pkgs.llvm
          ];
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          # Include nested Cargo calls from native binding generators.
          CARGO_NET_OFFLINE = "true";
          # SpiderMonkey's xsum disables format warnings itself; keep all other
          # Nix hardening rather than making that warning conflict fatal.
          hardeningDisable = [ "format" ];
          prePatch = ''
            # cbindgen invokes Cargo metadata from inside vendored mozjs crates.
            # The setup hook resolves @vendor@ only in the app's config; resolve
            # the writable vendor copy too, for those nested Cargo invocations.
            substituteInPlace "$cargoDepsCopy/.cargo/config.toml" \
              --subst-var-by vendor "$cargoDepsCopy"
            # Published mozjs glue crates carry standalone locks with versions
            # outside this app's vendor set. Metadata must use the same pinned
            # dependencies as the real build, not resolve those extra locks.
            for crate in "$cargoDepsCopy"/source-registry-0/mozjs_{locale,properties,collator,normalizer}_glue-* \
              "$cargoDepsCopy"/source-registry-0/mozjs_unicode_bidi_ffi-*; do
              cp Cargo.lock "$crate/Cargo.lock"
            done
          '';
          preBuild = ''
            export AS="$CC"
          '';
          buildInputs = with pkgs; [
            libxkbcommon
            wayland
            fontconfig
            freetype
          ];
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
          nativeCheckInputs = [
            pkgs.clippy
            pkgs.rustfmt
            pkgs.cacert
          ];
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
            cargo fmt --all -- --check
            # Match the package profile/target so native build scripts can reuse
            # the release artifacts, and honor Nix's requested parallelism.
            cargo clippy --jobs "$NIX_BUILD_CORES" --release \
              --target ${pkgs.stdenv.hostPlatform.rust.rustcTarget} \
              --workspace --all-targets --all-features --locked --offline -- -D warnings
          '';
          meta = {
            description = "Mailstronaut planning scaffold; not yet a usable mail client";
            homepage = "https://github.com/eduardz1/mailstronaut";
            license = pkgs.lib.licenses.gpl3Only;
            mainProgram = "mailstronaut";
            platforms = systems;
          };
        };
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
                shellcheck ${self}/ci/check.sh
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
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.weston
              pkgs.xorg-server
              pkgs.xdotool
              pkgs.imagemagick
              pkgs.dbus
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
            ];
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
