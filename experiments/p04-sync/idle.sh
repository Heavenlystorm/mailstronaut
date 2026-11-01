#!/usr/bin/env bash
# Called only by credentials.sh: IDLE against a real server. The poll interval is an
# hour, so a change seen within seconds can only have come through IDLE. All synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/idle-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
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
}
for key in alpha bravo charlie; do message "$key"; done
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  accounts = [{
    id = "dovecot-synthetic";
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
printf '%s' "$imap_password" | secret-tool store --label='Synthetic IDLE fixture' \
  application "$(cat assets/app-id)" account dovecot-synthetic reference dovecot-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  dovecot_cleanup
}
trap cleanup EXIT

inbox_count='SELECT count(*) FROM effective_memberships m JOIN folders f ON f.account_id=m.account_id AND f.id=m.folder_id WHERE f.is_inbox=1'
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
# Bounded in seconds: the next poll is an hour away.
await() {
  local what=$1 limit=$2 expected=$3 sql=$4 began=$SECONDS
  for _ in $(seq 1 $((limit * 5))); do
    if [[ $(query "$sql" || true) == "$expected" ]]; then
      printf 'OK: %s (within %ss)\n' "$what" "$((SECONDS - began))"
      return 0
    fi
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out after %ss: %s (last value %s)\n' "$limit" "$what" "$(query "$sql" || true)" >&2
  return 1
}
# Another mail client, on its own connection.
other_client() {
  python3 - "$imap_port" "$imap_user" "$imap_password" "$ca" "$@" <<'PY'
import imaplib, ssl, sys
port, user, password, ca, action, marker = sys.argv[1:7]
client = imaplib.IMAP4_SSL("localhost", int(port), ssl_context=ssl.create_default_context(cafile=ca))
client.login(user, password)
client.select("INBOX")
_, found = client.uid("SEARCH", None, "BODY", marker)
uid = found[0].split()[-1]
if action == "seen":
    client.uid("STORE", uid, "+FLAGS", r"(\Seen)")
elif action == "star":
    client.uid("STORE", uid, "+FLAGS", r"(\Flagged)")
elif action == "expunge":
    client.uid("STORE", uid, "+FLAGS", r"(\Deleted)")
    client.expunge()
client.logout()
PY
}

# Folders whose wire names are modified UTF-7, one holding unread mail of its own.
python3 - "$imap_port" "$imap_user" "$imap_password" "$ca" <<'PY'
import imaplib, ssl, sys
port, user, password, ca = sys.argv[1:]
client = imaplib.IMAP4_SSL("localhost", int(port), ssl_context=ssl.create_default_context(cafile=ca))
client.login(user, password)
client.create('"&AMk-tudes"')
client.create('"Projets/&ZeVnLA-"')
for key in ("one", "two"):
    client.append('"&AMk-tudes"', None, None, (
        f"From: Correspondent {key} <{key}@example.test>\r\nTo: reader@example.test\r\n"
        f"Subject: Synthetic studies {key}\r\nMessage-ID: <studies-{key}@example.test>\r\n"
        f"Content-Type: text/plain; charset=utf-8\r\n\r\nSynthetic body marker-studies-{key}.\r\n"
    ).encode())
client.logout()
PY

start initial
await 'first cycle cached the inbox' 60 3 'SELECT count(*) FROM messages'
await 'fetching left the mail unread' 5 0 'SELECT count(*) FROM memberships WHERE seen=1'
# The watcher connects after the first good cycle; give it time to reach IDLE.
sleep 2

message delta
await 'IDLE brought a new delivery' 15 4 'SELECT count(*) FROM messages'
[[ $(server_flags INBOX marker-delta) != *S* ]]
printf '%s\n' 'OK: the IDLE-triggered fetch left the server copy unread'
# The row is drawn when the cycle finishes, just after the database has it.
sleep 2
capture delivered

# Opening a folder by its readable name fetches it by its exact wire name. Roles come
# first in the sidebar, so these two follow Trash.
click 85 532
await 'the folder shown as Études was fetched by its wire name' 15 2 \
  "SELECT count(*) FROM effective_memberships m JOIN folders f ON f.account_id=m.account_id AND f.id=m.folder_id WHERE f.name='&AMk-tudes'"
sleep 2
capture folders
click 85 210
sleep 1

other_client seen marker-alpha
await 'IDLE brought a Seen flag set by another client' 15 1 'SELECT count(*) FROM memberships WHERE seen=1'
other_client expunge marker-bravo
await 'IDLE brought an expunge by another client' 15 3 "$inbox_count"

# One connection is watching and nothing is happening: this must cost almost nothing.
ticks() { awk '{print $14 + $15}' "/proc/$app/stat"; }
before=$(ticks); sleep 20; after=$(ticks)
printf 'Idle application CPU over 20s: %s ticks of %s/s\n' "$((after - before))" "$(getconf CLK_TCK)"

# The server goes away while a message arrives, and comes back. Nothing but the
# reconnect can notice this: the poll is an hour away and the old connection is gone.
dovecot_stop
message echo
sleep 3
dovecot_start
await 'reconnecting found mail delivered during the outage' 120 4 "$inbox_count"
# And the new connection is itself watching.
sleep 2
message foxtrot
await 'IDLE resumed after the reconnect' 15 5 "$inbox_count"
sleep 2
capture reconnected
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native IDLE acceptance: %s\n' "$artifacts"
