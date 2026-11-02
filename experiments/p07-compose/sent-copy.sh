#!/usr/bin/env bash
# Called only by credentials.sh: a message the server accepted whose Sent copy cannot be
# saved yet. A sync while IMAP is down keeps the copy pending without submitting the message
# again, and the next sync saves exactly one copy once IMAP is back. All mail and servers are
# synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/sent-copy-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
export MAILSTRONAUT_SMTP_PORT_FILE="$artifacts/port"
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  accounts = [{
    id = "sent-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "sent-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic sent-copy fixture' \
  application "$(cat assets/app-id)" account sent-synthetic reference sent-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
server=''
stop_server() {
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; server=''; fi
}
cleanup() {
  session_cleanup
  stop_server
  dovecot_cleanup
}
trap cleanup EXIT
# The next SMTP behavior, on the port the configuration already names.
serve() {
  stop_server
  rm -f "$MAILSTRONAUT_SMTP_PORT_FILE"
  env "$@" MAILSTRONAUT_SMTP_BIND_PORT="$port" "$fixture" --ignored --exact gui_loopback_server --nocapture \
    >> "$artifacts/server.log" 2>&1 & server=$!
  for _ in {1..100}; do [[ -s $MAILSTRONAUT_SMTP_PORT_FILE ]] && return 0; kill -0 "$server"; sleep .05; done
  return 1
}
# The fixture exits once it has seen one DATA, and keeps what it received.
await_attempt() {
  for _ in {1..150}; do kill -0 "$server" 2>/dev/null || break; kill -0 "$app"; sleep .1; done
  wait "$server"; server=''
}
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..900}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
# The one outbox row this walkthrough creates.
latest_phase="SELECT phase || ':' || uncertain FROM outbox ORDER BY rowid DESC LIMIT 1"
latest_copy="SELECT sent_copy FROM outbox ORDER BY rowid DESC LIMIT 1"
latest_message="SELECT message_id FROM outbox ORDER BY rowid DESC LIMIT 1"

dovecot_start
start sent
# The first sync learns the server's Sent folder.
await 'the first sync learned the Sent folder' 1 "SELECT count(*) FROM folders WHERE name='Sent'"

# The server goes down before the message is sent, so the Sent copy cannot be saved.
dovecot_stop
serve
wtype -s 200 -M ctrl n -m ctrl -s 700
wtype -s 100 'to@example.test' -s 100 -M ctrl -M shift b -m shift -m ctrl -s 400
wtype -s 100 -k Tab -s 100 'blind@example.test' -k Tab -s 100
wtype 'Synthetic sent copy · Unicode subject' -k Tab -s 100 'Synthetic body for the Sent copy.'
sleep 1
wtype -s 300 -M ctrl -k Return -m ctrl
await_attempt
await 'the server accepted the message' 'accepted:0' "$latest_phase"
await 'the Sent copy waits while IMAP is down' pending "$latest_copy"
sleep 1
capture sent-copy-pending

# A sync while IMAP is still down keeps the copy pending. A resubmission would reach this
# fixture as another DATA, so the fixture must still be waiting.
serve
wtype -s 300 -M ctrl r -m ctrl -s 3000
kill -0 "$server"
[[ $(query "$latest_phase") == accepted:0 ]]
[[ $(query "$latest_copy") == pending ]]
printf '%s\n' 'OK: a sync while IMAP was down neither saved the copy nor submitted the message again'
stop_server

# IMAP comes back, and the next sync saves the copy.
dovecot_start
wtype -s 300 -M ctrl r -m ctrl -s 300
await 'the next sync saved the Sent copy' done "$latest_copy"
[[ $(query "$latest_phase") == accepted:0 ]]

# The Sent folder holds exactly one copy, byte-identical to the mail the server accepted.
message_id=$(query "$latest_message")
# Dovecot keeps maildir files with LF line endings and records the CRLF size in the file name
# (W=), so the saved copy is compared with the submitted CRLF message after normalizing line
# endings, and its W= size must equal the submitted message's size.
python3 - "$mail_root" "$message_id" "$artifacts/sent-copy.eml" "$artifacts/received.eml" <<'PY'
import glob, os, re, shutil, sys
root, message_id, target, received_path = sys.argv[1:]
base = os.path.join(root, '.Sent')
found = sorted(
    path
    for path in glob.glob(os.path.join(base, 'cur', '*')) + glob.glob(os.path.join(base, 'new', '*'))
    if f'Message-ID: <{message_id}>' in open(path, errors='replace').read()
)
assert len(found) == 1, found
shutil.copyfile(found[0], target)
received = open(received_path, 'rb').read()
assert int(re.search(r',W=(\d+)', os.path.basename(found[0])).group(1)) == len(received)
assert open(target, 'rb').read().replace(b'\r\n', b'\n') == received.replace(b'\r\n', b'\n')
PY
printf '%s\n' 'OK: the Sent folder holds one copy, identical to the submitted message'
sleep 1
capture sent-copy-saved
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Sent copy acceptance: %s\n' "$artifacts"
