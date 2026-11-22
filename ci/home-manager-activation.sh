#!/usr/bin/env bash
# Activate the optional Home Manager module for real, in throwaway home directories, and check what
# activation produced. Covered: an immutable link into the store; a writable copy whose undeclared UI
# edits survive re-activation while declared values are reapplied; removing a declaration keeps the
# current value; an explicit empty account list is honored; an invalid declaration fails without
# changing the file; and a user-owned file is never replaced silently. Run from the repository root.
# Pass a Git flake reference to verify an already-built revision instead of the working tree.
set -euo pipefail
root=$(pwd)
flake_url=${1:-$(python3 -c 'from pathlib import Path; print("git+" + Path.cwd().as_uri())')}
mkdir -p "$root/target"
work=$(mktemp -d "$root/target/hm-activation.XXXXXX")
# Keep the throwaway homes when a check fails, so their contents can be inspected.
trap 'status=$?; [[ $status == 0 ]] && rm -rf "$work"; exit "$status"' EXIT
fixture_home="$work/home"
mkdir -p "$fixture_home"
config="$fixture_home/.config/mailstronaut/config.nix"

# build APPEARANCE_EXPR MUTABLE DECLARE_APPEARANCE ACCOUNTS_EXPR [EXPORTED_FILE]
build() {
  MAILSTRONAUT_HM_HOME="$fixture_home" MAILSTRONAUT_HM_FLAKE="$flake_url" MAILSTRONAUT_HM_EXPORT="${5:-}" \
    nix build --impure --no-link --print-out-paths \
    --extra-experimental-features 'nix-command flakes' --expr "
      let
        flake = builtins.getFlake (builtins.getEnv \"MAILSTRONAUT_HM_FLAKE\");
        pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
        exportPath = builtins.getEnv \"MAILSTRONAUT_HM_EXPORT\";
      in import (flake.outPath + \"/nix/home-manager-activation.nix\") {
        self = flake;
        inherit pkgs;
        appearance = $1;
        mutable = $2;
        declareAppearance = $3;
        accounts = $4;
        settings = if exportPath == \"\" then null else import (builtins.toPath exportPath);
      }"
}

activate() {
  mkdir -p "$fixture_home/.local/state/nix/profiles" "$fixture_home/runtime"
  chmod 700 "$fixture_home/runtime"
  # An unset runtime path makes Home Manager find the host systemd user manager.
  env -u WAYLAND_DISPLAY -u DISPLAY -u NIX_PROFILE \
    XDG_RUNTIME_DIR="$fixture_home/runtime" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=$fixture_home/runtime/no-session-bus" \
    NIX_CONFIG="${NIX_CONFIG:-}"$'\nuse-xdg-base-directories = true\nbuilders =' \
    HOME="$fixture_home" XDG_CONFIG_HOME="$fixture_home/.config" XDG_DATA_HOME="$fixture_home/.local/share" \
    XDG_STATE_HOME="$fixture_home/.local/state" XDG_CACHE_HOME="$fixture_home/.cache" \
    "$1/activate"
}
# value EXPR: the JSON value of an expression evaluated against the configuration file.
value() {
  nix eval --json --impure --expr "let c = import \"$config\"; in $1"
}
checksum() {
  sha256sum "$config" | cut -d' ' -f1
}
# A failing activation must exit non-zero; anything else is a silent acceptance.
expect_refusal() {
  local what=$1 activation=$2 status=0
  activate "$activation" > "$work/refusal.log" 2>&1 || status=$?
  if (( status == 0 )); then
    printf 'Expected refusal, but activation succeeded: %s\n' "$what" >&2
    exit 1
  fi
}

# Immutable: the configuration is a read-only link into the store.
activation=$(build '"dark"' false true null)
activate "$activation"
[[ -L $config ]]
grep -q 'managed = true' "$(readlink -f "$config")"
[[ $(value 'c.appearance') == '"dark"' ]]
printf '%s\n' 'OK: immutable activation links the managed configuration into the store'
rm -rf "$fixture_home" && mkdir -p "$fixture_home"

# Mutable, first activation: a writable file owned by the application.
activation=$(build '"dark"' true true null)
activate "$activation"
[[ -f $config && ! -L $config ]]
[[ $(value 'c.appearance') == '"dark"' ]]
printf '%s\n' 'OK: mutable activation creates a writable configuration on first activation'

# A UI edit to a value the configuration does not declare: an attribute added right after the
# top-level opening brace, whether the serializer wrote one line or many.
sed -i '0,/{/s//{ sync = { pollSeconds = 900; initialDays = 30; };/' "$config"
[[ $(value 'c.sync.pollSeconds') == '900' ]]
[[ $(value 'c.sync.initialDays') == '30' ]]

# Mutable, repeated activation with a changed declaration: the declared value wins, and the UI edit
# survives.
activation=$(build '"light"' true true null)
before=$(checksum)
(
  flock -n 9
  expect_refusal 'a competing config writer' "$activation"
  grep -Fq 'configuration changed or is being edited' "$work/refusal.log"
  [[ $(checksum) == "$before" ]]
) 9<>"$(dirname "$config")/.config.nix.lock"
printf '%s\n' 'OK: activation refused a competing writer without changing the source'
activate "$activation"
[[ $(value 'c.appearance') == '"light"' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
[[ $(value 'c.sync.initialDays') == '30' ]]
printf '%s\n' 'OK: repeated mutable activation reapplies the declared value and keeps the UI edit'

# Removing a declaration stops enforcing it but keeps the current value.
activation=$(build '"light"' true false null)
activate "$activation"
[[ $(value 'c.appearance') == '"light"' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
[[ $(value 'c.sync.initialDays') == '30' ]]
printf '%s\n' 'OK: removing a declaration keeps the current value and the UI edit'

# An explicit empty account list is honored: whole lists replace, so the account is removed.
activation=$(build '"light"' true true '[ ]')
activate "$activation"
[[ $(value 'builtins.length c.accounts') == '0' ]]
[[ $(value 'c.appearance') == '"light"' ]]
printf '%s\n' 'OK: an explicit empty account list replaces the declared accounts'

# An invalid declaration fails the activation and leaves the file byte-identical.
before=$(checksum)
activation=$(build '42' true true null)
expect_refusal 'an invalid appearance' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: an invalid declaration is refused and the configuration is unchanged'

# Import exactly what the installed client exports into a fresh HM generation.
activation=$(build '"light"' true true null)
activate "$activation"
"$activation/home-path/bin/mailstronaut" config export --path "$config" > "$work/exported.nix"
rm -rf "$fixture_home" && mkdir -p "$fixture_home"
activation=$(build '"dark"' true false '[ ]' "$work/exported.nix")
activate "$activation"
[[ $(value 'c.appearance') == '"light"' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
[[ $(value 'c.sync.initialDays') == '30' ]]
[[ $(value 'map (a: a.id) c.accounts') == '["declared"]' ]]
[[ $(value 'c.managed') == 'false' ]]
printf '%s\n' 'OK: the installed client export became the next native Home Manager declaration'

# An explicitly declared null restores complete history, preserving the poll interval.
printf '%s\n' '{ sync.initialDays = null; }' > "$work/complete-history.nix"
activation=$(build '"dark"' true false null "$work/complete-history.nix")
activate "$activation"
[[ $(value 'c.sync.initialDays') == 'null' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
"$activation/home-path/bin/mailstronaut" config check --path "$config"
printf '%s\n' 'OK: Home Manager can keep an app-set cap and explicitly restore complete history'

# Ownership, immutable: Home Manager does not replace a user-owned regular file.
rm -rf "$fixture_home" && mkdir -p "$(dirname "$config")"
printf '%s\n' '{ appearance = "user-owned"; }' > "$config"
before=$(checksum)
activation=$(build '"dark"' false true null)
expect_refusal 'an immutable link over a user-owned file' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: an immutable activation does not replace a user-owned file'

# Ownership, mutable: a file the application did not create is not silently overwritten either.
rm -rf "$fixture_home" && mkdir -p "$(dirname "$config")"
printf '%s\n' '{ appearance = "user-owned"; }' > "$config"
before=$(checksum)
activation=$(build '"dark"' true true null)
expect_refusal 'a mutable configuration over a user-owned file' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: a mutable activation does not silently overwrite a user-owned file'

printf '%s\n' 'Home Manager activation: passed'
