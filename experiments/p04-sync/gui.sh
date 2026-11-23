#!/usr/bin/env bash
# Called only by credentials.sh: the application against a real IMAP server and the
# loopback SMTP fixture, with a private bus and keyring. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
smtp_fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/sync-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# This walkthrough doubles as the proof of the polling fallback: without IDLE, a new
# delivery can only be found by the next poll.
dovecot_extra=$'imap_capability {\n  IDLE = no\n}'
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
for key in alpha bravo charlie; do
  seed_message "$key" <<MAIL
From: Correspondent $key <$key@example.test>
To: reader@example.test
Subject: Synthetic $key
Message-ID: <$key@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic body marker-$key.
MAIL
done
seed_message echo <<MAIL
From: Correspondent echo <echo@example.test>
To: reader@example.test
Subject: Synthetic echo with a file
Message-ID: <echo@example.test>
Date: $(date -R)
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary=synthetic

--synthetic
Content-Type: text/plain; charset=utf-8

Synthetic body marker-echo.
--synthetic
Content-Type: application/pdf
Content-Disposition: attachment; filename="synthetic report.pdf"
Content-Transfer-Encoding: base64

U3ludGhldGljIGZpbGU=
--synthetic--
MAIL
touch -d '4 hours ago' "$mail_root/new/echo.synthetic"
# Maildir INTERNALDATE is the file's mtime: fix the list order, newest first.
touch -d '3 hours ago' "$mail_root/new/alpha.synthetic"
touch -d '2 hours ago' "$mail_root/new/bravo.synthetic"
touch -d '1 hour ago' "$mail_root/new/charlie.synthetic"
dovecot_start
export MAILSTRONAUT_SMTP_PORT_FILE="$artifacts/smtp-port"
smtp_server=''
"$smtp_fixture" --ignored --exact gui_loopback_server --nocapture > "$artifacts/smtp.log" 2>&1 & smtp_server=$!
for _ in {1..100}; do [[ -s $MAILSTRONAUT_SMTP_PORT_FILE ]] && break; kill -0 "$smtp_server"; sleep .05; done
smtp_port=$(cat "$MAILSTRONAUT_SMTP_PORT_FILE")
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 30;
  accounts = [{
    id = "dovecot-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $smtp_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "dovecot-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic sync fixture' \
  application "$(cat assets/app-id)" account dovecot-synthetic reference dovecot-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  dovecot_cleanup
  if [[ -n "$smtp_server" ]]; then kill "$smtp_server" 2>/dev/null || true; wait "$smtp_server" 2>/dev/null || true; fi
}
trap cleanup EXIT

query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..300}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
await_server() {
  local what=$1 folder=$2 marker=$3 flag=$4
  for _ in {1..300}; do
    if [[ $(server_flags "$folder" "$marker" 2>/dev/null || true) == *"$flag"* ]]; then printf 'OK: %s\n' "$what"; return 0; fi
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out on the server: %s\n' "$what" >&2
  return 1
}

python3 - "$imap_port" "$imap_user" "$imap_password" "$ca" <<'PY'
import imaplib, ssl, sys
port, user, password, ca = sys.argv[1:]
client = imaplib.IMAP4_SSL("localhost", int(port), ssl_context=ssl.create_default_context(cafile=ca))
client.login(user, password)
_, capabilities = client.capability()
assert b"IDLE" not in capabilities[0].split(), "this server must not offer IDLE"
client.logout()
PY
printf '%s\n' 'OK: this server does not offer IDLE'

start initial
# Nothing is pressed: starting the application fetches the inbox by itself.
await 'automatic sync cached the inbox' 4 'SELECT count(*) FROM messages'
await 'SPECIAL-USE gave the Sent role' Sent "SELECT name FROM folders WHERE role='sent'"
await 'SPECIAL-USE gave the Trash role' Trash "SELECT name FROM folders WHERE role='trash'"
await 'SPECIAL-USE gave the Archive role' Archive "SELECT name FROM folders WHERE role='archive'"
await 'fetching left the mail unread' 0 'SELECT count(*) FROM memberships WHERE seen=1'
sleep 1
capture synced
[[ $(server_flags INBOX marker-charlie) != *S* ]]
printf '%s\n' 'OK: PEEK left the server copy unread'

if [[ ${MAILSTRONAUT_MERGE_SELECTION_GUI:-0} == 1 ]]; then
  await 'initial groups projected' 4 'SELECT count(*) FROM conversation_members'
  members='FROM conversation_members c JOIN messages m ON m.id=c.message_id'
  loser=$(query "SELECT max(c.conversation_id) $members WHERE m.subject IN ('Synthetic bravo','Synthetic alpha')")
  subject=$(query "SELECT m.subject $members WHERE c.conversation_id=$loser")
  keys=(-k Down -k Down)
  case "$subject" in
    'Synthetic alpha') keys+=(-k Down);;
    'Synthetic bravo') ;;
    *) exit 1;;
  esac
  wtype -M ctrl l -m ctrl "${keys[@]}" -s 500
  capture merge-selected
  seed_message bridge <<MAIL
From: Correspondent bridge <bridge@example.test>
To: reader@example.test
Subject: Joined conversation
Message-ID: <bridge@example.test>
References: <alpha@example.test> <bravo@example.test>
Date: $(date -R)

Synthetic body marker-bridge.
MAIL
  wtype -M ctrl r -m ctrl
  await 'late IMAP arrival merged the selected group' 1 "SELECT count(*) FROM conversations WHERE id=$loser AND merged_into IS NOT NULL"
  sleep 1
  capture merge-refreshed
  await 'passive merge left all five messages unread' 0 'SELECT count(*) FROM effective_memberships WHERE seen'
  wtype -k Return -s 500
  await_server 'Enter opened the new representative' INBOX marker-bridge S
  await 'only the explicitly opened original became read' 1 'SELECT count(*) FROM effective_memberships WHERE seen'
  capture merge-opened
  printf 'MERGE_SELECTION_GUI_OK: %s\n' "$artifacts"
  exit 0
fi

# The oldest row carries a file: it is listed, and saving would go through the portal.
click 350 500
sleep .9
capture attachment
# Open the newest row: Seen reaches the real server without any manual sync.
wtype -s 300 -M ctrl l -m ctrl -k Down -k Return -s 900
capture opened
await_server 'explicit open set Seen on the server' INBOX marker-charlie S
wtype -s 300 -M alt s -m alt -s 300
await_server 'Star set Flagged on the server' INBOX marker-charlie F
capture starred

# A new delivery appears without a manual refresh once the poll comes round.
seed_message delta <<MAIL
From: Correspondent delta <delta@example.test>
To: reader@example.test
Subject: Synthetic delta
Message-ID: <delta@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic body marker-delta.
MAIL
await 'polling found a new delivery' 5 'SELECT count(*) FROM messages'

# Reply from the open message, with a blind recipient for the SMTP fixture's check.
wtype -s 300 -M alt r -m alt -s 1200
wtype -s 200 -M ctrl -M shift b -m shift -m ctrl -s 400 -k Tab -s 200 'blind@example.test' -s 300
capture reply
wtype -s 300 -k Tab -k Tab -s 300 -M ctrl -k Return -m ctrl
wait "$smtp_server"; smtp_server=''
await 'SMTP accepted the reply' accepted 'SELECT phase FROM outbox'
await 'the Sent copy was stored on the server' 'done' 'SELECT sent_copy FROM outbox'
server_flags Sent 'marker-charlie' > /dev/null
printf '%s\n' 'OK: the quoted reply is in the Sent folder'
sleep 1
capture sent

# Archive moves the real message and it stays gone from the inbox.
click 350 150
sleep .9
capture before-archive
wtype -s 300 -M alt e -m alt -s 600
capture archived
for _ in {1..300}; do
  server_flags Archive marker-delta > /dev/null 2>&1 && break
  kill -0 "$app"; sleep .2
done
server_flags Archive marker-delta > /dev/null
if server_flags INBOX marker-delta > /dev/null 2>&1; then
  printf '%s\n' 'Archived message is still in the server inbox' >&2; exit 1
fi
printf '%s\n' 'OK: Archive moved the message on the server'
# Another message in the inbox was not touched by the move.
server_flags INBOX marker-alpha > /dev/null
server_flags INBOX marker-bravo > /dev/null

# Another client flags the archived copy: polling carries that back from a folder
# that is not the inbox, and IDLE is not involved at all on this server.
python3 - "$imap_port" "$imap_user" "$imap_password" "$ca" <<'PY'
import imaplib, ssl, sys
port, user, password, ca = sys.argv[1:]
client = imaplib.IMAP4_SSL("localhost", int(port), ssl_context=ssl.create_default_context(cafile=ca))
client.login(user, password)
client.select("Archive")
_, found = client.uid("SEARCH", None, "BODY", "marker-delta")
client.uid("STORE", found[0].split()[-1], "+FLAGS", r"(\Flagged)")
client.logout()
PY
await 'a flag set by another client came back from Archive' 1 \
  "SELECT count(*) FROM effective_memberships m JOIN folders f ON f.account_id=m.account_id AND f.id=m.folder_id WHERE f.name='Archive' AND m.starred=1"

# The server goes away: cached mail stays readable and the failure is reported.
dovecot_stop
wtype -s 300 -M ctrl r -m ctrl -s 2500
capture offline
wtype -s 300 -M ctrl l -m ctrl -k Down -k Return -s 900
capture offline-read
close_window

dovecot_start
start reconnected
await 'nothing was lost across restart' 'done' 'SELECT sent_copy FROM outbox'
sleep 2
capture reconnected
close_window
if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native real-server acceptance: %s\n' "$artifacts"
