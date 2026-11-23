#!/usr/bin/env bash
# Called only by credentials.sh: an attachment added to a draft through the file chooser can be
# removed from the composer, and the removal survives a late autosave and a restart. No server
# is contacted, and all mail and files are synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/attachments-gui"
mkdir -p "$artifacts"
chooser_fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt application test executable}")
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "attach-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
    auth = { kind = "password"; secretRef = "attach-password"; };
  }];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic attachment fixture' \
  application "$(cat assets/app-id)" account attach-synthetic reference attach-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
chooser=''
cleanup() {
  session_cleanup
  if [[ -n "$chooser" ]]; then kill -CONT "$chooser" 2>/dev/null || true; kill "$chooser" 2>/dev/null || true; wait "$chooser" 2>/dev/null || true; fi
}
trap cleanup EXIT
# A protocol-accurate file chooser on this private bus, answering from a file.
MAILSTRONAUT_PORTAL_ROOT=$artifacts "$chooser_fixture" portal_fixture::serve_portals --ignored --exact > "$artifacts/chooser.log" 2>&1 & chooser=$!
for _ in {1..100}; do [[ -f "$artifacts/portal-ready" ]] && break; kill -0 "$chooser"; sleep .1; done
[[ -f "$artifacts/portal-ready" ]]
# Composer positions measured from captures: the header's attach button, the card's body (which
# reveals the remove control on hover), and the 20-pixel remove disc riding its upper corner.
attach_x=568
attach_y=134
hover_x=620
hover_y=340
remove_x=744
remove_y=311
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
attachments="SELECT count(*) FROM draft_attachments"
open_drafts="SELECT count(*) FROM drafts WHERE closed=0"

attachment="$artifacts/synthetic receipt · été.txt"
printf 'Synthetic receipt contents · été.\n' > "$attachment"
python3 -c 'import sys,urllib.parse; print("file://"+urllib.parse.quote(sys.argv[1]))' "$attachment" > "$artifacts/chooser-answer"

start attach
wtype -s 200 -M ctrl n -m ctrl -s 700
wtype -s 100 'to@example.test' -s 100 -k Tab -s 100
wtype 'Synthetic attachment · Unicode subject' -k Tab -s 100 'Synthetic body with a file.'
sleep 2
await 'the draft is saved before attaching' 1 "$open_drafts"
# Stop the private chooser before it can answer. The pending keystroke lands
# within the autosave delay, and must reach the database while the dialog is open.
printf '%s\n' cancel > "$artifacts/chooser-answer"
kill -STOP "$chooser"
wtype ' Pending before chooser.'
click "$attach_x" "$attach_y"
pending="SELECT count(*) FROM drafts WHERE closed=0 AND instr(json_extract(content,'$.body'),'Pending before chooser.')>0"
await 'the last edit reached SQLite while the file chooser was held' 1 "$pending"
kill -CONT "$chooser"
for _ in {1..50}; do [[ -s $artifacts/chooser-requests.jsonl ]] && break; sleep .1; done
[[ -s $artifacts/chooser-requests.jsonl ]]
sleep .5
[[ $(query "$attachments") == 0 ]]
capture chooser-cancelled
# Losing access before import leaves the same editable draft and its saved text.
printf 'file://%s/missing-file.txt\n' "$artifacts" > "$artifacts/chooser-answer"
click "$attach_x" "$attach_y"
sleep .7
[[ $(query "$pending") == 1 && $(query "$attachments") == 0 ]]
capture unreadable-file
python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).as_uri())' "$attachment" > "$artifacts/chooser-answer"
click "$attach_x" "$attach_y"
await 'the file chooser attached one file' 1 "$attachments"
# Attaching kept a private copy, so the original can go away before the removal.
rm "$attachment"
sleep 1
capture attached
# The remove control is visible only while its card is hovered.
swaymsg seat seat0 cursor set "$hover_x" "$hover_y" > /dev/null
sleep .5
click "$remove_x" "$remove_y"
sleep 2
await 'removing the attachment drops its reference' 0 "$attachments"
await 'the draft stays open after removing its attachment' 1 "$open_drafts"
# A late autosave must not restore the removed attachment.
sleep 3
[[ $(query "$attachments") == 0 ]]
printf '%s\n' 'OK: no late autosave restored the removed attachment'
capture removed
close_window

start attach-restarted
sleep 3
[[ $(query "$attachments") == 0 ]]
printf '%s\n' 'OK: a removed attachment stays removed after a restart'
capture attach-restarted
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Attachment removal acceptance: %s\n' "$artifacts"
