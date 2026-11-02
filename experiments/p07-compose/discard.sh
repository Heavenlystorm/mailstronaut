#!/usr/bin/env bash
# Called only by credentials.sh: discarding a draft asks first, a cancelled discard keeps the
# draft, and a confirmed discard stays discarded across a restart. All mail is synthetic and no
# server is contacted.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/discard-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "discard-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
    auth = { kind = "password"; secretRef = "discard-password"; };
  }];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic discard fixture' \
  application "$(cat assets/app-id)" account discard-synthetic reference discard-password purpose password
# Composer positions measured from captures: the header's discard button, and the dialog's
# Cancel and confirm buttons.
discard_x=616
discard_y=134
cancel_x=731
cancel_y=180
confirm_x=810
confirm_y=180
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap session_cleanup EXIT
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..150}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
open_count="SELECT count(*) FROM drafts WHERE closed=0"
closed_count="SELECT count(*) FROM drafts WHERE closed=1"
compose() {
  wtype -s 200 -M ctrl n -m ctrl -s 700
  wtype -s 100 'to@example.test' -s 100 -k Tab -s 100
  wtype 'Synthetic discard · Unicode subject' -k Tab -s 100 'Synthetic body that will be discarded.'
  sleep 2
}

start discard
compose
await 'the draft is saved before any discard' 1 "$open_count"
click "$discard_x" "$discard_y"
sleep 1
capture discard-asked
click "$cancel_x" "$cancel_y"
sleep 1
await 'cancelling the discard keeps the draft open' 1 "$open_count"
click "$discard_x" "$discard_y"
sleep 1
click "$confirm_x" "$confirm_y"
sleep 2
await 'confirming the discard closes the draft' 0 "$open_count"
await 'the discarded draft remains as a closed tombstone' 1 "$closed_count"
# A late autosave must not recreate the discarded text.
sleep 3
[[ $(query "$open_count") == 0 ]]
printf '%s\n' 'OK: no late autosave recreated the discarded draft'
capture discarded
close_window

start discard-restarted
sleep 3
[[ $(query "$open_count") == 0 ]]
printf '%s\n' 'OK: a discarded draft does not come back after a restart'
capture discard-restarted
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Discard acceptance: %s\n' "$artifacts"
