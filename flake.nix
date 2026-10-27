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
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "accesskit-0.22.0" = "sha256-6TWK4HXc2a4XSf13/nksZoihMqZurQ7tBGY8+2DaPuY=";
              "atomicwrites-0.4.2" = "sha256-QZSuGPrJXh+svMeFWqAXoqZQxLq/WfIiamqvjJNVhxA=";
              "build_helpers-0.14.0" = "sha256-gIrq3k4+TjlIW8CVjGGzh0GOS/xu+UX/8n95gvTYpzw=";
              "clipboard_macos-0.1.0" = "sha256-WO3JFbE+6ESRAfkxrnEFeZyGuhUHLOKOVHcGQyHwoK0=";
              "cosmic-client-toolkit-0.2.0" = "sha256-FL2YkCVrJ6JWUGMDg+jJJa51oar/PWDfgPgCyjcv6gk=";
              "cosmic-freedesktop-icons-0.4.0" = "sha256-tPriTi5L0mFMHjo5xpF5cmKGHqlX3WUO7EZAgVdBpS4=";
              "cosmic-settings-daemon-0.1.0" = "sha256-LYIR+qK+hCBVV+bfVWz2jvH5fGvfNTcryKqfe5n8Gog=";
              "cryoglyph-0.1.0" = "sha256-10JUHl1ktbqLaReuiU3HPa4r2KvsoryyJoF3BFoge3U=";
              "dpi-0.1.2" = "sha256-/H+D2Szeaufte6gbGQelOcvU4JhncJ9k9XTFr0q7PTw=";
              "smithay-clipboard-0.8.0" = "sha256-GojAFRbhJcP0Rpr+v9WOivgW9x38PZdeBWTbMhkDB3A=";
              "snix-eval-0.1.0" = "sha256-5vjUm3KUaciznlUYSKFhgjyzG1ZNTArKREA0OyPdu4Q=";
              "softbuffer-0.4.1" = "sha256-9Ret/nfieBFl4yJ9TddyWsSuS7sI4QAza/TZrxYMb+I=";
            };
          };
          cargoBuildFlags = [
            "--workspace"
            "--all-features"
          ];
          cargoTestFlags = [
            "--workspace"
            "--all-features"
          ];
          strictDeps = true;
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.makeWrapper
          ];
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
          ];
          preCheck = ''
            cargo fmt --all -- --check
            cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
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
          servo-probe = pkgs.mkShell {
            inputsFrom = [ self.devShells.${system}.default ];
            packages = with pkgs; [
              clang
              cmake
              ninja
              python3
              perl
              libclang
            ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
          default = pkgs.mkShell {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibraries pkgs);
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
