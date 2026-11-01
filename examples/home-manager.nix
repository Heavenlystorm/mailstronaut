# Example composition, not a Mailstronaut HM module or a config-format adapter.
# Requires the user's flake to supply inputs.mailstronaut through extraSpecialArgs.
# The application evaluates config.nix directly with its bounded native evaluator.
# The optional programs.mailstronaut module and mutable native settings/export are
# described in docs/CONFIGURATION.md; this direct-file route needs no module.
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default ];
  xdg.configFile."mailstronaut/config.nix".source = ./config.nix;
}
