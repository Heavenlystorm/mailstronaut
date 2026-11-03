#!/usr/bin/env bash
# Activate the optional Home Manager module for real, in throwaway home directories, and check what
# activation produced. Covered: an immutable link into the store; a writable copy whose undeclared UI
# edits survive re-activation while declared values are reapplied; removing a declaration keeps the
# current value; an explicit empty account list is honored; an invalid declaration fails without
# changing the file; and a user-owned file is never replaced silently. Run from the repository root.
set -euo pipefail
root=$(pwd)
work=$(mktemp -d "$root/target/hm-activation.XXXXXX")
# Keep the throwaway homes when a check fails, so their contents can be inspected.
trap 'status=$?; [[ $status == 0 ]] && rm -rf "$work"; exit "$status"' EXIT
home="$work/home"
mkdir -p "$home"
config="$home/.config/mailstronaut/config.nix"

# build APPEARANCE_EXPR MUTABLE DECLARE_APPEARANCE ACCOUNTS_EXPR: the Nix arguments are expressions.
build() {
  MAILSTRONAUT_HM_HOME="$home" nix build --impure --no-link --print-out-paths \
    --extra-experimental-features 'nix-command flakes' --expr "
      let
        flake = builtins.getFlake \"path:$root\";
        pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
      in import $root/nix/home-manager-activation.nix {
        self = flake;
        inherit pkgs;
        appearance = $1;
        mutable = $2;
        declareAppearance = $3;
        accounts = $4;
      }"
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
  HOME="$home" "$activation/activate" > "$work/refusal.log" 2>&1 || status=$?
  if (( status == 0 )); then
    printf 'Expected refusal, but activation succeeded: %s\n' "$what" >&2
    exit 1
  fi
}

# Immutable: the configuration is a read-only link into the store.
activation=$(build '"dark"' false true null)
HOME="$home" "$activation/activate"
[[ -L $config ]]
grep -q 'managed = true' "$(readlink -f "$config")"
[[ $(value 'c.appearance') == '"dark"' ]]
printf '%s\n' 'OK: immutable activation links the managed configuration into the store'
rm -rf "$home" && mkdir -p "$home"

# Mutable, first activation: a writable file owned by the application.
activation=$(build '"dark"' true true null)
HOME="$home" "$activation/activate"
[[ -f $config && ! -L $config ]]
[[ $(value 'c.appearance') == '"dark"' ]]
printf '%s\n' 'OK: mutable activation creates a writable configuration on first activation'

# A UI edit to a value the configuration does not declare: an attribute added right after the
# top-level opening brace, whether the serializer wrote one line or many.
sed -i '0,/{/s//{ sync.pollSeconds = 900;/' "$config"
[[ $(value 'c.sync.pollSeconds') == '900' ]]

# Mutable, repeated activation with a changed declaration: the declared value wins, and the UI edit
# survives.
activation=$(build '"light"' true true null)
HOME="$home" "$activation/activate"
[[ $(value 'c.appearance') == '"light"' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
printf '%s\n' 'OK: repeated mutable activation reapplies the declared value and keeps the UI edit'

# Removing a declaration stops enforcing it but keeps the current value.
activation=$(build '"light"' true false null)
HOME="$home" "$activation/activate"
[[ $(value 'c.appearance') == '"light"' ]]
[[ $(value 'c.sync.pollSeconds') == '900' ]]
printf '%s\n' 'OK: removing a declaration keeps the current value and the UI edit'

# An explicit empty account list is honored: whole lists replace, so the account is removed.
activation=$(build '"light"' true true '[ ]')
HOME="$home" "$activation/activate"
[[ $(value 'builtins.length c.accounts') == '0' ]]
[[ $(value 'c.appearance') == '"light"' ]]
printf '%s\n' 'OK: an explicit empty account list replaces the declared accounts'

# An invalid declaration fails the activation and leaves the file byte-identical.
before=$(checksum)
activation=$(build '42' true true null)
expect_refusal 'an invalid appearance' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: an invalid declaration is refused and the configuration is unchanged'

# Ownership, immutable: Home Manager does not replace a user-owned regular file.
rm -rf "$home" && mkdir -p "$(dirname "$config")"
printf '%s\n' '{ appearance = "user-owned"; }' > "$config"
before=$(checksum)
activation=$(build '"dark"' false true null)
expect_refusal 'an immutable link over a user-owned file' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: an immutable activation does not replace a user-owned file'

# Ownership, mutable: a file the application did not create is not silently overwritten either.
rm -rf "$home" && mkdir -p "$(dirname "$config")"
printf '%s\n' '{ appearance = "user-owned"; }' > "$config"
before=$(checksum)
activation=$(build '"dark"' true true null)
expect_refusal 'a mutable configuration over a user-owned file' "$activation"
[[ $(checksum) == "$before" ]]
printf '%s\n' 'OK: a mutable activation does not silently overwrite a user-owned file'

printf '%s\n' 'Home Manager activation: passed'
