# A real Home Manager activation of the optional module, not the stubbed evaluation in
# home-manager-check.nix. Home Manager is pinned by source hash here, so flake.lock is unchanged.
# ci/home-manager-activation.sh supplies the disposable home directory through
# MAILSTRONAUT_HM_HOME, because activation must run with HOME equal to homeDirectory.
{
  self,
  pkgs,
  appearance ? "dark",
  mutable ? false,
}:
let
  homeManagerSource = builtins.fetchTarball {
    url = "https://github.com/nix-community/home-manager/archive/095fef2f265c3c0da4fe315efba8f8a04495ab34.tar.gz";
    sha256 = "sha256-X6xa6sGA7RnJQuZGbFMC+K+KToWAlBDCCjJXfTwEPsw=";
  };
  homeManager = import "${homeManagerSource}/lib" { inherit (pkgs) lib; };
  configuration = homeManager.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeManagerModules.default
      {
        home = {
          # Home Manager refuses activation unless this matches the running user.
          username = builtins.getEnv "USER";
          homeDirectory = builtins.getEnv "MAILSTRONAUT_HM_HOME";
          stateVersion = "26.05";
        };
        programs.mailstronaut = {
          enable = true;
          settings = {
            inherit appearance;
            accounts = [
              {
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
              }
            ];
          };
          mutableSettings = mutable;
        };
      }
    ];
  };
in
configuration.activationPackage
