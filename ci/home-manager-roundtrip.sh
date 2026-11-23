#!/usr/bin/env bash
# Sourced by home-manager-activation.sh. Requires a disposable Unix account
# and the native tools in the gui-test shell.
: "${root:?repository}" "${work:?private workspace}" "${config:?private config}" "${fixture_home:?private home}"
global_state=${NIX_STATE_DIR:-/nix/var/nix}
for legacy in "$global_state/profiles/per-user/$USER/home-manager" "$global_state/gcroots/per-user/$USER/current-home"; do
  if [[ -e $legacy || -L $legacy ]]; then
    printf 'Use a disposable Unix account without legacy Home Manager roots.\n' >&2
    exit 1
  fi
done
unset HOME_MANAGER_BACKUP_EXT HOME_MANAGER_BACKUP_COMMAND HOME_MANAGER_BACKUP_OVERWRITE
declared="$work/declared.nix"
cat > "$declared" <<'DECL'
{
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "ownership";
    provider = "icloud";
    address = "ownership@example.invalid";
    userName = "ownership";
    displayName = "Été \"quoted\" \\ fixture";
    auth = { kind = "password"; secretRef = "fixture-reference"; };
  }];
}
DECL
generation() {
  build '"light"' "$1" false null "$2"
}
current="$fixture_home/.local/state/home-manager/gcroots/current-home"
immutable=$(generation false "$declared")
activate "$immutable"
[[ $(readlink -f "$current") == "$immutable" && -L $config && $(value 'c.managed') == true ]]
original_target=$(readlink -f "$config")
original_hash=$(checksum)

printf '%s\n' 'builtins.removeAttrs (import ./declared.nix) [ "accounts" ]' > "$work/mutable.nix"
mutable=$(generation true "$work/mutable.nix")
activate "$mutable"
[[ $(readlink -f "$current") == "$mutable" && -f $config && ! -L $config && $(stat -c %a "$config") == 600 ]]
[[ $(value 'c.managed') == false && $(sha256sum "$original_target" | cut -d' ' -f1) == "$original_hash" ]]
[[ $(value 'map (a: a.id) (c.accounts or [])') == '["ownership"]' ]] || {
  printf '%s\n' 'Changing ownership dropped the now-undeclared account from the original configuration.' >&2
  exit 1
}
printf '%s\n' 'OK: managed link became private writable source in the same Home Manager profile'

mkdir -p "$work/wayland"
cat > "$work/bus.conf" <<'BUS'
<busconfig>
  <type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
  <policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy>
</busconfig>
BUS
env HOME="$fixture_home" XDG_CONFIG_HOME="$fixture_home/.config" XDG_STATE_HOME="$fixture_home/.local/state" \
    XDG_DATA_HOME="$fixture_home/.local/share" XDG_CACHE_HOME="$fixture_home/.cache" XDG_RUNTIME_DIR="$fixture_home/runtime" \
    dbus-run-session --config-file="$work/bus.conf" -- bash "$root/experiments/p02-config/ownership-gui.sh" \
    "$mutable/home-path/bin/mailstronaut" "$config" "$work/wayland"
[[ $(value 'c.sync.pollSeconds') == 120 && $(value 'c.notifications.showPreview') == true ]]
[[ $(value 'c.sync ? initialDays') == false && $(value 'c.sync ? maxBodyBytes') == false ]]
printf '%s\n' 'OK: native Settings saved, restarted and copied the exact effective export with omitted source defaults'

printf '%s\n' '{ sync.pollSeconds = 240; }' > "$work/override.nix"
override=$(generation true "$work/override.nix")
activate "$override"
[[ $(value 'c.sync.pollSeconds') == 240 && $(value 'c.notifications.showPreview') == true ]]
[[ $(value 'builtins.length c.accounts') == 1 && $(value 'c.sync.automatic') == false ]]
printf '%s\n' 'OK: redeclared interval won while the undeclared account and edited preview survived'

restored=$(generation false "$work/wayland/copied-export.nix")
before=$(checksum)
expect_refusal 'replacing the edited mutable source' "$restored"
[[ $(checksum) == "$before" && $(readlink -f "$current") == "$override" ]]
HOME_MANAGER_BACKUP_EXT=before-managed activate "$restored"
[[ $(readlink -f "$current") == "$restored" && -L $config && $(value 'c.managed') == true ]]
[[ $(sha256sum "$config.before-managed" | cut -d' ' -f1) == "$before" ]]
"$restored/home-path/bin/mailstronaut" config export --path "$config" > "$work/reimported.nix"
cmp "$work/wayland/copied-export.nix" "$work/reimported.nix"
printf 'OK: imported GUI export equals the managed generation, with a byte-identical backup of the previous source\nHome Manager GUI round trip: %s\n' "$work"
