# Invoke with nix develop --impure --expr
# 'import ./experiments/p01-flatpak/shell.nix'. Keep the package/toolchain pins;
# this optional probe does not change the production package or default shell.
let
  project = builtins.getFlake ("git+file://" + toString ../..);
  system = builtins.currentSystem;
in
project.devShells.${system}.flatpak
