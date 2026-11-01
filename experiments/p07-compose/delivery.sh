#!/usr/bin/env bash
# Called only by credentials.sh: what the user sees when a send does not simply succeed.
# One message goes through an unreachable server, a temporary rejection, a lost
# acknowledgment and a final acceptance. All mail and servers are synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/delivery-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
export MAILSTRONAUT_SMTP_PORT_FILE="$artifacts/port"
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "delivery-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "localhost"; port = $port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "delivery-password"; };
  }];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic delivery fixture' \
  application "$(cat assets/app-id)" account delivery-synthetic reference delivery-password purpose password
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
state() { python3 -c 'import sqlite3,sys; print("%s:%s" % sqlite3.connect(sys.argv[1]).execute("SELECT phase,uncertain FROM outbox").fetchone())' "$database" 2>/dev/null; }
await_state() {
  for _ in {1..150}; do
    [[ $(state || true) == "$2" ]] && { printf 'OK: %s (%s)\n' "$1" "$2"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (want %s, have %s)\n' "$1" "$2" "$(state || true)" >&2
  return 1
}

start compose
wtype -s 200 -M ctrl n -m ctrl -s 700
wtype -s 100 'to@example.test' -s 100 -M ctrl -M shift b -m shift -m ctrl -s 400
wtype -s 100 -k Tab -s 100 'blind@example.test' -k Tab -s 100
wtype 'Synthetic delivery · Unicode subject' -k Tab -s 100 'Synthetic body for every attempt.'
sleep 1
wtype -s 300 -M ctrl -k Return -m ctrl
await_state 'an unreachable server leaves the mail not sent, with no duplicate warning' failed:0
sleep 1
capture unreachable

serve MAILSTRONAUT_SMTP_REJECT_DATA=1
click 250 442
await_attempt
await_state 'a temporary rejection still needs no warning' failed:0
cp "$artifacts/received.eml" "$artifacts/rejected.eml"
sleep 1
capture rejected

serve MAILSTRONAUT_SMTP_DROP_ACK=1
click 250 422
await_attempt
await_state 'a lost acknowledgment becomes delivery unknown' delivery_unknown:1
cp "$artifacts/received.eml" "$artifacts/unknown.eml"
sleep 1
capture unknown

# Nothing resends by itself, including across a restart, with a server ready to accept.
serve
sleep 5
close_window
start restarted
wtype -s 200 -M ctrl 2 -m ctrl -s 700
sleep 5
kill -0 "$server"
[[ $(state) == delivery_unknown:1 ]]
printf '%s\n' 'OK: neither waiting nor a restart resent it'
capture restarted

# The decision is explicit: dismissing the warning sends nothing.
click 277 373
sleep 1
capture decision
wtype -s 300 -k Escape -s 700
kill -0 "$server"
[[ $(state) == delivery_unknown:1 ]]
printf '%s\n' 'OK: dismissing the warning sent nothing'

# An unreachable retry cannot erase that the earlier attempt may have delivered.
stop_server
click 277 373
sleep 1
click 797 203
sleep 2
await_state 'an unreachable retry keeps the warning' delivery_unknown:1
capture unreachable-retry

serve
# The failure banner pushes the row down.
click 277 442
sleep 1
click 797 203
await_attempt
await_state 'confirming sends it again' accepted:1
sleep 1
capture accepted
cmp "$artifacts/rejected.eml" "$artifacts/unknown.eml"
cmp "$artifacts/unknown.eml" "$artifacts/received.eml"
grep -q '^Message-ID: <' "$artifacts/received.eml"
printf '%s\n' 'OK: every attempt carried the same frozen bytes and Message-ID'
close_window
if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native delivery failure acceptance: %s\n' "$artifacts"
