# Example composition, not a Mailstronaut HM module or a config-format adapter.
# Requires the user's flake to supply inputs.mailstronaut through extraSpecialArgs.
# The application begins consuming this configuration in P02.
{ inputs, pkgs, ... }:
{
  home.packages = [ inputs.mailstronaut.packages.${pkgs.stdenv.hostPlatform.system}.default ];
  xdg.configFile."mailstronaut/config.nix".source = ./config.nix;
}
