# Evaluate the real module with stubbed Home Manager plumbing, then run its output
# through the installed binary in a disposable HOME. The user's dotfiles are never used.
{ self, pkgs }:
let
  inherit (pkgs) lib;
  package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
  hmLib = lib.extend (
    _: _: {
      hm.dag.entryBetween = before: after: data: { inherit before after data; };
    }
  );
  evaluate =
    settings: mutableSettings: startOnLogin:
    (hmLib.evalModules {
      specialArgs = { inherit pkgs; };
      modules = [
        (import ./home-manager.nix self)
        {
          options = {
            home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; };
            home.activation = lib.mkOption { type = lib.types.attrsOf lib.types.anything; };
            xdg.configHome = lib.mkOption { type = lib.types.str; };
            xdg.configFile = lib.mkOption { type = lib.types.attrsOf lib.types.anything; };
            systemd.user.services = lib.mkOption {
              type = lib.types.attrsOf lib.types.anything;
              default = { };
            };
          };
          config = {
            xdg.configHome = "/mailstronaut-test-home/.config";
            home.activation = { };
            xdg.configFile = { };
            programs.mailstronaut = {
              enable = true;
              inherit
                package
                settings
                mutableSettings
                startOnLogin
                ;
            };
          };
        }
      ];
    }).config;
  account = id: {
    inherit id;
    provider = "manual";
    address = "${id}@example.test";
    displayName = id;
    userName = id;
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
      secretRef = "${id}-password";
    };
  };
  declared = {
    appearance = "dark";
    accounts = [ (account "declared") ];
  };
  managed = evaluate declared false false;
  first = evaluate declared true false;
  login = evaluate (declared // { background.enabled = true; }) false true;
  second = evaluate {
    appearance = "light";
    accounts = [ ];
  } true false;
  # Substitute only the fixture home argument; production paths are shell-escaped.
  activate =
    hm:
    lib.replaceStrings
      [ (lib.escapeShellArg "/mailstronaut-test-home/.config/mailstronaut/config.nix") ]
      [ ''"$HOME/.config/mailstronaut/config.nix"'' ]
      hm.home.activation.mailstronautSettings.data;
in
assert !(managed.systemd.user.services ? mailstronaut);
assert login.systemd.user.services.mailstronaut.Install.WantedBy == [ "graphical-session.target" ];
pkgs.runCommand "mailstronaut-home-manager-module" { } ''
  export HOME="$PWD/home"
  mkdir -p "$HOME"
  run() { "$@"; }
  bin=${lib.getExe package}

  # Managed: a read-only native file the application accepts as it is.
  file=${managed.xdg.configFile."mailstronaut/config.nix".source}
  "$bin" config check --path "$file"
  grep -q 'managed = true' "$file"

  # Mutable: the first activation creates a private regular file the app owns.
  config="$HOME/.config/mailstronaut/config.nix"
  ${activate first}
  [ -f "$config" ] && [ ! -L "$config" ]
  [ "$(stat -c %a "$config")" = 600 ]
  "$bin" config export --path "$config" > exported-first.nix
  grep -q 'appearance = "dark"' exported-first.nix
  grep -q 'declared@example.test' exported-first.nix

  # A value changed in the app survives; declared values are reapplied.
  cat > "$config" <<'NIX'
  # Mailstronaut editable configuration
  { schemaVersion = 1; appearance = "system"; sync.pollSeconds = 120; }
  NIX
  "$bin" config check --path "$config"
  ${activate first}
  "$bin" config export --path "$config" > exported-second.nix
  grep -q 'appearance = "dark"' exported-second.nix
  grep -q 'pollSeconds = 120' exported-second.nix

  # Whole lists replace: an explicit empty list disables the declared account.
  ${activate second}
  "$bin" config export --path "$config" > exported-third.nix
  grep -q 'appearance = "light"' exported-third.nix
  grep -q 'accounts = \[ \]' exported-third.nix
  grep -q 'pollSeconds = 120' exported-third.nix

  # A broken declaration leaves the last valid file untouched.
  before="$(sha256sum "$config")"
  printf '{ schemaVersion = 99; }' > bad.nix
  if "$bin" config reconcile --declared bad.nix --path "$config"; then exit 1; fi
  [ "$(sha256sum "$config")" = "$before" ]

  # Export round-trips: the snapshot is itself a valid native configuration.
  "$bin" config check --path exported-third.nix

  # Only the module's known managed store link may become a mutable regular file.
  before="$(sha256sum "$file")"
  ln -s "$file" managed-link.nix
  "$bin" config reconcile --declared ${pkgs.writeText "declared.nix" "{ schemaVersion = 1; }"} --path managed-link.nix
  [ -f managed-link.nix ] && [ ! -L managed-link.nix ]
  [ "$(sha256sum "$file")" = "$before" ]
  grep -q '^# Mailstronaut editable configuration' managed-link.nix

  # Another link may be the user's own dotfile, and is refused rather than replaced.
  printf '{ schemaVersion = 1; }' > dotfile.nix
  ln -s "$PWD/dotfile.nix" link.nix
  if "$bin" config reconcile --declared ${pkgs.writeText "declared.nix" "{ schemaVersion = 1; }"} --path link.nix; then exit 1; fi
  [ "$(cat dotfile.nix)" = '{ schemaVersion = 1; }' ]
  touch "$out"
''
