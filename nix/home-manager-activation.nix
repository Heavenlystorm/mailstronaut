# A real Home Manager activation of the optional module, not the stubbed evaluation in
# home-manager-check.nix. Home Manager is pinned by source hash here, so flake.lock is unchanged.
# ci/home-manager-activation.sh supplies the disposable home directory through
# MAILSTRONAUT_HM_HOME, because activation must run with HOME equal to homeDirectory.
#
# Each argument is a Nix expression, so the script can declare, omit or break a setting:
#   appearance          the declared appearance (omitted when declareAppearance is false)
#   mutable             whether the configuration is an application-owned, writable file
#   accounts            the declared account list; null means one synthetic account
#   settings            an exported native declaration, or null to use fixture settings
{
  self,
  pkgs,
  appearance ? "dark",
  mutable ? false,
  declareAppearance ? true,
  accounts ? null,
  settings ? null,
  startOnLogin ? false,
  trackGenerations ? false,
}:
let
  homeManagerSource = builtins.fetchTarball {
    url = "https://github.com/nix-community/home-manager/archive/095fef2f265c3c0da4fe315efba8f8a04495ab34.tar.gz";
    sha256 = "sha256-X6xa6sGA7RnJQuZGbFMC+K+KToWAlBDCCjJXfTwEPsw=";
  };
  homeManager = import "${homeManagerSource}/lib" { inherit (pkgs) lib; };
  syntheticAccount = {
    id = "declared";
    provider = "manual";
    address = "declared@example.test";
    displayName = "Declared";
    userName = "declared";
    imap = {
      host = "imap.example.test";
      port = 993;
      security = "tls";
    };
    smtp = {
      host = "smtp.example.test";
      port = 587;
      security = "starttls";
    };
    auth = {
      kind = "password";
      secretRef = "declared-password";
    };
  };
  configuration = homeManager.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeManagerModules.default
      {
        nix.assumeXdg = true;
        home = {
          # This disposable activation owns no legacy roots under /nix/var/nix.
          activationGenerateGcRoot = trackGenerations;
          # Home Manager refuses activation unless this matches the running user.
          username = builtins.getEnv "USER";
          homeDirectory = builtins.getEnv "MAILSTRONAUT_HM_HOME";
          stateVersion = "26.05";
        };
        programs.mailstronaut = {
          enable = true;
          inherit startOnLogin;
          settings =
            if settings != null then
              settings
            else
              (if declareAppearance then { inherit appearance; } else { })
              // {
                accounts = if accounts == null then [ syntheticAccount ] else accounts;
              };
          mutableSettings = mutable;
        };
      }
    ];
  };
in
configuration.activationPackage
