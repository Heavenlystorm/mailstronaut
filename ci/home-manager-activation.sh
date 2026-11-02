#!/usr/bin/env bash
# Activate the optional Home Manager module for real, in a disposable home directory, and check
# what activation produced: an immutable link into the store, then a writable copy whose undeclared
# UI edits survive re-activation while declared values are reapplied. Run from the repository root.
set -euo pipefail
root=$(pwd)
work=$(mktemp -d "$root/target/hm-activation.XXXXXX")
# Keep the throwaway home when a check fails, so its contents can be inspected.
trap 'status=$?; [[ $status == 0 ]] && rm -rf "$work"; exit "$status"' EXIT
home="$work/home"
mkdir -p "$home"
config="$home/.config/mailstronaut/config.nix"
build() {
  local appearance=$1 mutable=$2
  MAILSTRONAUT_HM_HOME="$home" nix build --impure --no-link --print-out-paths \
    --extra-experimental-features 'nix-command flakes' --expr "
      let
        flake = builtins.getFlake \"path:$root\";
        pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
      in import $root/nix/home-manager-activation.nix {
        self = flake;
        inherit pkgs;
        appearance = \"$appearance\";
        mutable = $mutable;
      }"
}

# Immutable: the configuration is a read-only link into the store.
activation=$(build dark false)
HOME="$home" "$activation/activate"
[[ -L $config ]]
grep -q 'appearance = "dark"' "$(readlink -f "$config")"
grep -q 'managed = true' "$(readlink -f "$config")"
printf '%s\n' 'OK: immutable activation links the managed configuration into the store'
rm -rf "$home" && mkdir -p "$home"

# Mutable, first activation: a writable file owned by the application.
activation=$(build dark true)
HOME="$home" "$activation/activate"
[[ -f $config && ! -L $config ]]
grep -q 'appearance = "dark"' "$config"
printf '%s\n' 'OK: mutable activation creates a writable configuration on first activation'

# A UI edit to a value the configuration does not declare: an attribute added right after the
# top-level opening brace, whether the serializer wrote one line or many.
sed -i '0,/{/s//{ sync.pollSeconds = 900;/' "$config"

# Mutable, repeated activation with a changed declaration: the declared value wins, and the UI edit
# survives.
printf '%s\n' '--- configuration after the UI edit:'
cat "$config"
activation=$(build light true)
HOME="$home" "$activation/activate"
grep -q 'appearance = "light"' "$config"
grep -q 'pollSeconds = 900' "$config"
printf '%s\n' 'OK: repeated mutable activation reapplies the declared value and keeps the UI edit'

printf '%s\n' 'Home Manager activation: passed'
