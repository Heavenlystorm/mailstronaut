#!/usr/bin/env bash
# Called only by credentials.sh: choosing another account in a draft's From selector moves the
# draft to that account with its subject and text intact, and the move survives a restart.
# No server is contacted, and all mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/identity-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [
    {
      id = "identity-alpha";
      provider = "manual";
      address = "alpha@example.test";
      displayName = "Synthetic alpha";
      userName = "synthetic-smtp-user";
      imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
      smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
      auth = { kind = "password"; secretRef = "identity-password"; };
    }
    {
      id = "identity-beta";
      provider = "manual";
      address = "beta@example.test";
      displayName = "Synthetic beta";
      userName = "synthetic-smtp-user";
      imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
      smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
      auth = { kind = "password"; secretRef = "identity-password"; };
    }
  ];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic identity fixture' \
  application "$(cat assets/app-id)" account identity-alpha reference identity-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap session_cleanup EXIT
# Composer positions measured from captures: the From selector, and the option for the second
# account in its list.
from_x=909
from_y=182
option_x=780
option_y=255
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
open_drafts="SELECT count(*) FROM drafts WHERE closed=0"
open_account="SELECT account_id FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1"

start identity
wtype -s 200 -M ctrl n -m ctrl -s 700
wtype -s 100 'to@example.test' -s 100 -k Tab -s 100
wtype 'Synthetic identity · Unicode subject' -k Tab -s 100 'Synthetic body that keeps its text.'
sleep 2
await 'the draft starts under the first account' 1 "$open_drafts"
await 'the draft is held by the first account' identity-alpha "$open_account"
click "$from_x" "$from_y"
sleep 1
capture identity-open
click "$option_x" "$option_y"
sleep 3
await 'choosing the second account leaves one open draft' 1 "$open_drafts"
await 'the open draft moved to the second account' identity-beta "$open_account"
[[ $(query "SELECT json_extract(content,'\$.subject') FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1") == 'Synthetic identity · Unicode subject' ]]
printf '%s\n' 'OK: the moved draft keeps its subject'
[[ $(query "SELECT instr(content,'Synthetic body that keeps its text.') > 0 FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1") == 1 ]]
printf '%s\n' 'OK: the moved draft keeps its text'
sleep 1
capture identity-moved
close_window

start identity-restarted
sleep 3
await 'the moved draft stays under the second account after a restart' identity-beta "$open_account"
await 'restarting keeps exactly one open draft' 1 "$open_drafts"
capture identity-restarted
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Identity switch acceptance: %s\n' "$artifacts"
