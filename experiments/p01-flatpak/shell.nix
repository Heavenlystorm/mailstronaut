# Invoke with nix develop --impure --expr
# 'import ./experiments/p01-flatpak/shell.nix'. Keep the package/toolchain pins;
# this optional probe does not change the production package or default shell.
let
  project = builtins.getFlake ("git+file://" + toString ../..);
  system = builtins.currentSystem;
  pkgs = import project.inputs.nixpkgs { inherit system; };
in
project.devShells.${system}.default.overrideAttrs (old: {
  nativeBuildInputs = old.nativeBuildInputs ++ [
    pkgs.flatpak
    pkgs.flatpak-builder
    pkgs.bubblewrap
    pkgs.curl
    pkgs.jq
    # flatpak-builder runs appstreamcli for metainfo composition. The host's copy loads its own
    # glibc beside Nix's, so the Nix build must come first on PATH.
    pkgs.appstream
  ];
})
