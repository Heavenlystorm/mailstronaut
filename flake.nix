{
  description = "Mailstronaut: Rust workspace, development environment, and checks";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
      packageFor =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "mailstronaut";
          version = "0.1.0";
          src = pkgs.lib.cleanSource self;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--workspace"
            "--all-features"
          ];
          cargoTestFlags = [
            "--workspace"
            "--all-features"
          ];
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = with pkgs; [
            libxkbcommon
            wayland
            fontconfig
            freetype
          ];
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
          default = pkgs.mkShell {
            packages = with pkgs; [
              pkg-config
              libxkbcommon
              wayland
              fontconfig
              freetype
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              nixfmt
              git
              actionlint
              shellcheck
            ];
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
