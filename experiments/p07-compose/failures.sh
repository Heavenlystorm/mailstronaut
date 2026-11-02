#!/usr/bin/env bash
# Called only by credentials.sh: a send stopped before anything is submitted, a refused
# sign-in that the outbox retries once accepted, and a send cancelled while in flight.
# Each case uses a fresh profile database. All mail, credentials and servers are synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/failures-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
export MAILSTRONAUT_SMTP_PORT_FILE="$artifacts/port"
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
# The argument lists the trusted CA files. An empty list leaves the server's certificate
# unverifiable, which is the untrusted-certificate case.
write_config() {
  cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "failure-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "localhost"; port = $port; security = "tls"; };
    tls.caFiles = [ $1 ];
    auth = { kind = "password"; secretRef = "failure-password"; };
  }];
}
NIX
}
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic failure fixture' \
  application "$(cat assets/app-id)" account failure-synthetic reference failure-password purpose password
# Outbox positions measured from captures of this layout: a failed row's Retry button, and the
# composer's Cancel while a send is in flight.
retry_x=249
retry_y=442
cancel_x=581
cancel_y=134
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
# Each case creates one outbox row, so the newest row is the one under test.
state() { python3 -c 'import sqlite3,sys; print("%s:%s" % sqlite3.connect(sys.argv[1]).execute("SELECT phase,uncertain FROM outbox ORDER BY rowid DESC LIMIT 1").fetchone())' "$database" 2>/dev/null; }
await_state() {
  for _ in {1..150}; do
    [[ $(state || true) == "$2" ]] && { printf 'OK: %s (%s)\n' "$1" "$2"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (want %s, have %s)\n' "$1" "$2" "$(state || true)" >&2
  return 1
}
compose() {
  wtype -s 200 -M ctrl n -m ctrl -s 700
  wtype -s 100 'to@example.test' -s 100 -M ctrl -M shift b -m shift -m ctrl -s 400
  wtype -s 100 -k Tab -s 100 'blind@example.test' -k Tab -s 100
  wtype 'Synthetic failure · Unicode subject' -k Tab -s 100 'Synthetic body, never submitted.'
  sleep 1
  wtype -s 300 -M ctrl -k Return -m ctrl
}

# A server whose certificate cannot be verified: the send stops before any SMTP data.
write_config ''
rm -rf "$XDG_DATA_HOME/mailstronaut"
serve
start untrusted
compose
await_state 'an unverifiable certificate stops the send before any data' failed:0
kill -0 "$server"
printf '%s\n' 'OK: the unverifiable certificate sent no data'
sleep 1
capture untrusted
stop_server
close_window

# A refused sign-in stops the send before any data. Once the server accepts the sign-in,
# Retry in the outbox sends the same message, and the refusal is not a duplicate warning.
write_config "\"$ca\""
rm -rf "$XDG_DATA_HOME/mailstronaut"
serve MAILSTRONAUT_SMTP_REJECT_AUTH=1
start signin
compose
await_state 'a refused sign-in stops the send before any data' failed:0
kill -0 "$server"
printf '%s\n' 'OK: the refused sign-in sent no data'
sleep 1
capture signin-refused
stop_server
serve
click "$retry_x" "$retry_y"
await_attempt
await_state 'retrying after the sign-in is accepted' accepted:0
sleep 1
capture signin-retried
close_window

# A send cancelled while in flight submits nothing and leaves the draft restored.
write_config "\"$ca\""
rm -rf "$XDG_DATA_HOME/mailstronaut"
serve MAILSTRONAUT_SMTP_STALL=1
start cancel
compose
sleep 3
capture cancel-sending
click "$cancel_x" "$cancel_y"
sleep 2
await_state 'cancelling an in-flight send leaves it cancelled' cancelled:0
kill -0 "$server"
printf '%s\n' 'OK: the cancelled send submitted nothing'
sleep 1
capture cancelled
stop_server
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Send failure acceptance: %s\n' "$artifacts"
