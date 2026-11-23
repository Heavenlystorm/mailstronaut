# Optional Home Manager integration. It manages one native Nix file: `settings` is the
# same data the application reads, so no option mirrors a Rust field and no other
# configuration format exists. docs/CONFIGURATION.md describes the ownership contract.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.mailstronaut;
  configPath = "${config.xdg.configHome}/mailstronaut/config.nix";
  systemdConfigPath = lib.replaceStrings [ "%" "$" ] [ "%%" "$$" ] (
    lib.escape [ "\\" "\"" ] configPath
  );
  native =
    managed:
    pkgs.writeText "mailstronaut-config.nix" (
      (if managed then "# Managed by programs.mailstronaut\n" else "")
      + lib.generators.toPretty { } (
        {
          schemaVersion = 1;
        }
        // cfg.settings
        // lib.optionalAttrs managed { managed = true; }
      )
    );
in
{
  options.programs.mailstronaut = {
    enable = lib.mkEnableOption "the Mailstronaut email client";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.mailstronaut.packages.\${system}.default";
      description = "The Mailstronaut package to install.";
    };
    settings = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
      example = lib.literalExpression ''
        {
          appearance = "dark";
          accounts = [ {
            id = "personal";
            provider = "icloud";
            address = "person@icloud.com";
            displayName = "iCloud";
            userName = "person";
            auth = { kind = "password"; secretRef = "personal-icloud"; };
          } ];
        }
      '';
      description = ''
        Native Mailstronaut configuration, exactly as config.nix would contain it.
        Passwords and tokens never belong here: `secretRef` names a keyring entry.
      '';
    };
    mutableSettings = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Keep one writable config.nix owned by the application instead of a read-only
        managed link. Declared attributes are reapplied on every activation and
        undeclared values survive; whole lists replace. Removing a declaration stops
        enforcing it without changing the current value.
      '';
    };
    startOnLogin = lib.mkEnableOption ''
      starting Mailstronaut without a window in the native graphical session.
      The native configuration must also enable background operation
    '';
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      { home.packages = [ cfg.package ]; }
      (lib.mkIf (!cfg.mutableSettings) {
        xdg.configFile."mailstronaut/config.nix".source = native true;
      })
      (lib.mkIf cfg.mutableSettings {
        # Reconcile against the previous managed source before HM unlinks it.
        home.activation.mailstronautSettings =
          lib.hm.dag.entryBetween [ "linkGeneration" ] [ "writeBoundary" ] ''
            run ${lib.getExe cfg.package} config reconcile \
              --declared ${native false} \
              --path ${lib.escapeShellArg configPath}
          '';
      })
      (lib.mkIf cfg.startOnLogin {
        systemd.user.services.mailstronaut = {
          Unit = {
            Description = "Mailstronaut background mail and reminders";
            After = [ "graphical-session-pre.target" ];
            PartOf = [ "graphical-session.target" ];
            ConditionEnvironment = "WAYLAND_DISPLAY";
          };
          Service = {
            Type = "exec";
            ExecStart = ''${lib.getExe cfg.package} --background --config "${systemdConfigPath}"'';
            Restart = "on-abnormal";
            RestartSec = 5;
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })
    ]
  );
}
