#!/usr/bin/env bash
# Called only by credentials.sh: Undo, and moving mail on servers with and without MOVE and
# UIDPLUS, through the real interface against a real server. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
variant=${3:?full, no-move or neither}
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
# Each server gets its own profile, because a changed endpoint is a different mailbox.
export XDG_DATA_HOME="$root/data-actions-$variant"
mkdir -p "$XDG_DATA_HOME"
artifacts="$root/actions-$variant"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
case $variant in
  full) dovecot_extra='' ;;
  no-move) dovecot_extra=$'imap_capability {\n  MOVE = no\n}' ;;
  neither) dovecot_extra=$'imap_capability {\n  MOVE = no\n  UIDPLUS = no\n}' ;;
  *) exit 2 ;;
esac
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
message() {
  seed_message "$1" <<MAIL
From: Correspondent $1 <$1@example.test>
To: reader@example.test
Subject: Synthetic $1
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic body marker-$1.
MAIL
  touch -d "$2 hours ago" "$mail_root/new/$1.synthetic"
}
message delta 1
message charlie 2
message bravo 3
message alpha 4
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 30;
  accounts = [{
    id = "actions-$variant";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $imap_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "dovecot-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic actions fixture' \
  application "$(cat assets/app-id)" account "actions-$variant" reference dovecot-password purpose password
# Another client, for what this application must leave alone and for checking the server.
other_client() {
  python3 - "$imap_port" "$imap_user" "$imap_password" "$ca" "$@" <<'PY'
import imaplib, ssl, sys
port, user, password, ca, action = sys.argv[1:6]
client = imaplib.IMAP4_SSL("localhost", int(port), ssl_context=ssl.create_default_context(cafile=ca))
client.login(user, password)
if action == "capabilities":
    print(client.capability()[1][0].decode())
elif action == "delete-without-expunge":
    client.select("INBOX")
    _, found = client.uid("SEARCH", None, "BODY", sys.argv[6])
    client.uid("STORE", found[0].split()[-1], "+FLAGS", r"(\Deleted)")
elif action == "create-folder":
    assert client.create(sys.argv[6])[0] == "OK"
client.logout()
PY
}
capabilities=$(other_client capabilities)
if [[ $variant == full ]]; then
  # The menu shows the decoded label Project-Été. IMAP and SQLite use this wire name.
  picked_folder='Project-&AMk-t&AOk-'
  other_client create-folder "$picked_folder"
fi
for capability in MOVE UIDPLUS; do
  case "$variant,$capability" in
    full,*) wanted=yes ;;
    no-move,UIDPLUS) wanted=yes ;;
    *) wanted=no ;;
  esac
  if [[ " $capabilities " == *" $capability "* ]]; then have=yes; else have=no; fi
  [[ $have == "$wanted" ]] || { printf 'Server offers %s: %s, expected %s\n' "$capability" "$have" "$wanted" >&2; exit 1; }
done
printf 'OK: %s server: %s\n' "$variant" "$(tr ' ' '\n' <<< "$capabilities" | grep -E '^(MOVE|UIDPLUS)$' | tr '\n' ' ')"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  dovecot_cleanup
}
trap cleanup EXIT

query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..200}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
await_server() {
  local what=$1 folder=$2 marker=$3
  for _ in {1..300}; do
    server_flags "$folder" "$marker" > /dev/null 2>&1 && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out on the server: %s\n' "$what" >&2
  return 1
}
in_inbox="SELECT count(*) FROM effective_memberships m JOIN folders f ON f.account_id=m.account_id AND f.id=m.folder_id WHERE f.is_inbox=1"
open_newest() { click 350 150; sleep 1; }
row_menu() {
  swaymsg seat seat0 cursor set 350 270 > /dev/null
  swaymsg seat seat0 cursor press button3 > /dev/null
  swaymsg seat seat0 cursor release button3 > /dev/null
  sleep .4
}

start initial
await 'the inbox was cached' 4 'SELECT count(*) FROM messages'
await 'SPECIAL-USE gave the Archive role' Archive "SELECT name FROM folders WHERE role='archive'"
sleep 2

if [[ $variant == neither ]]; then
  # With no safe way to move one message, the server's mail must not be touched at all.
  open_newest
  wtype -s 300 -M alt e -m alt -s 600
  await 'the message left the list at once' 3 "$in_inbox"
  capture hidden
  await 'a server that cannot move safely gets the message back' 4 "$in_inbox"
  sleep 1
  capture refused
  server_flags INBOX marker-delta > /dev/null
  if server_flags Archive marker-delta > /dev/null 2>&1; then
    printf '%s\n' 'A message was moved on a server that cannot do so safely' >&2; exit 1
  fi
  printf '%s\n' 'OK: nothing changed on the server, and the message is back'
  [[ $(query 'SELECT count(*) FROM pending_moves') == 0 ]]
  printf '%s\n' 'OK: a move that can never work is not tried again'
  # One refused move must not stop this account's mail from arriving.
  message echo 0
  await 'new mail still arrives after the refusal' 5 'SELECT count(*) FROM messages'
  sleep 2
  capture still-syncing
elif [[ $variant == no-move ]]; then
  # Another client has marked a message for deletion. A plain EXPUNGE would destroy it.
  other_client delete-without-expunge marker-alpha
  open_newest
  wtype -s 300 -M alt e -m alt -s 600
  await_server 'COPY and UID EXPUNGE moved the message' Archive marker-delta
  for _ in {1..100}; do server_flags INBOX marker-delta > /dev/null 2>&1 || break; sleep .2; done
  if server_flags INBOX marker-delta > /dev/null 2>&1; then
    printf '%s\n' 'The moved message is still in the inbox' >&2; exit 1
  fi
  [[ $(server_flags INBOX marker-alpha) == *T* ]]
  printf '%s\n' "OK: another client's message marked for deletion was left alone"
  sleep 1
  capture moved
else
  await 'the searchable destination was discovered' 1 "SELECT count(*) FROM folders WHERE name='$picked_folder'"
  open_newest
  click 922 244
  wtype -s 300 'Project' -s 350
  capture move-folder-search
  wtype -k Escape -s 300
  [[ $(query 'SELECT count(*) FROM pending_moves') == 0 ]]
  [[ $(query "$in_inbox") == 4 ]]
  # Changing the account declaration while its popup is open retires its choice.
  click 922 244
  wtype -s 300 'Project' -s 300 -M ctrl -M shift r -m shift -m ctrl -s 1000 -k Return -s 300
  [[ $(query 'SELECT count(*) FROM pending_moves') == 0 ]]
  open_newest
  click 922 244
  wtype -s 300 'Project' -s 350 -k Down -k Return -s 350
  await 'the searched label queued its exact IMAP wire name' "$picked_folder" 'SELECT destination FROM pending_moves'
  click 1205 76
  await 'Undo cancelled the searched move' 0 'SELECT count(*) FROM pending_moves'
  await 'Undo restored the original membership' 4 "$in_inbox"
  sleep 9
  server_flags INBOX marker-delta > /dev/null
  if server_flags "$picked_folder" marker-delta > /dev/null 2>&1; then
    printf '%s\n' 'The server received an undone picker move' >&2; exit 1
  fi
  printf '%s\n' 'OK: dismissing, reloading and undoing the picker left the server message in its inbox'
  # Keep delta open while using the menu on the next unread row, charlie.
  open_newest
  seen="SELECT count(*) FROM effective_memberships WHERE seen=1"
  starred="SELECT m.subject FROM effective_memberships e JOIN messages m ON e.message_id=m.id WHERE e.starred=1"
  await 'only the explicitly opened delta is seen' 1 "$seen"
  row_menu
  capture row-context
  wtype -k Escape -s 300
  [[ $(query "$seen") == 1 ]]
  row_menu
  wtype -k Down -k Down -k Down -k Return -s 400
  await 'row-menu Star targets charlie' 'Synthetic charlie' "$starred"
  [[ $(query "$seen") == 1 ]]
  # The menu restored focus to its row. Click Select text in the existing reader.
  click 763 297
  wtype -s 200 -M ctrl a -m ctrl -s 100 -M ctrl c -m ctrl -s 200
  for _ in {1..25}; do
    wl-paste --no-newline > "$artifacts/open-body.txt" 2>/dev/null || true
    grep -q 'Synthetic body marker-delta.' "$artifacts/open-body.txt" && break
    sleep .2
  done
  grep -q 'Synthetic body marker-delta.' "$artifacts/open-body.txt"
  row_menu
  click 388 316
  await 'explicit row-menu Mark read updates charlie' 2 "$seen"
  sleep 1
  row_menu
  capture row-context-read
  click 388 316
  await 'explicit row-menu Mark unread restores charlie' 1 "$seen"
  row_menu
  click 387 371
  await 'the row menu archived charlie, leaving delta in view' 'Synthetic charlie' 'SELECT m.subject FROM pending_moves p JOIN messages m ON p.message_id=m.id'
  sleep .8
  capture row-context-undo
  click 1205 76
  await 'Undo restored the context-menu move' 4 "$in_inbox"
  [[ $(query "$seen") == 1 ]]
  for _ in {1..50}; do [[ $(server_flags INBOX marker-charlie) != *S* ]] && break; sleep .2; done
  [[ $(server_flags INBOX marker-charlie) != *S* ]]
  printf '%s\n' 'OK: opening, dismissing and acting on the row menu kept charlie unread and delta open'
  # Undo within the grace period: back at once, and the server never hears of it.
  open_newest
  wtype -s 300 -M alt e -m alt -s 600
  await 'Archive hid the message at once' 3 "$in_inbox"
  capture undo-offered
  # The Undo button of the notice in the top right, which replaced the earlier shortcut.
  click 1205 76
  sleep .3
  await 'Undo brought it back' 4 "$in_inbox"
  capture undone
  sleep 10
  server_flags INBOX marker-delta > /dev/null
  if server_flags Archive marker-delta > /dev/null 2>&1; then
    printf '%s\n' 'An undone move reached the server' >&2; exit 1
  fi
  printf '%s\n' 'OK: past the grace period, the server never heard of the undone move'
  # Without Undo, the move happens.
  open_newest
  wtype -s 300 -M alt e -m alt -s 600
  await 'Archive hid it again' 3 "$in_inbox"
  await_server 'UID MOVE reached the server' Archive marker-delta
  server_flags INBOX marker-alpha > /dev/null
  server_flags INBOX marker-bravo > /dev/null
  server_flags INBOX marker-charlie > /dev/null
  printf '%s\n' 'OK: the other messages stayed in the inbox'
  # Trash goes to its own folder.
  sleep 2
  open_newest
  wtype -s 300 -M alt d -m alt -s 600
  await 'Trash hid the next message' 2 "$in_inbox"
  await_server 'it reached the Trash folder' Trash marker-charlie
  sleep 1
  capture moved
fi
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native actions acceptance (%s): %s\n' "$variant" "$artifacts"
