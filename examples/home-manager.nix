# Example composition, not a Mailstronaut HM module or a config-format adapter.
# Requires the user's flake to supply inputs.mailstronaut through extraSpecialArgs.
# The application evaluates config.nix directly with its bounded native evaluator.
# Optional mutable native settings/export are planned in docs/CONFIGURATION.md;
# this direct managed-file route does not require that future module.
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default ];
  xdg.configFile."mailstronaut/config.nix".source = ./config.nix;
}
