#!/usr/bin/env bash
# Called only by credentials.sh, against its private bus/keyring and a synthetic SMTP server.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/compose-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
export MAILSTRONAUT_SMTP_PORT_FILE="$artifacts/port"
server=''
"$fixture" --ignored --exact gui_loopback_server --nocapture > "$artifacts/server.log" 2>&1 & server=$!
for _ in {1..100}; do [[ -s $MAILSTRONAUT_SMTP_PORT_FILE ]] && break; kill -0 "$server"; sleep .05; done
port=$(cat "$MAILSTRONAUT_SMTP_PORT_FILE")
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  accounts = [{
    id = "compose-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    aliases = [ "alias@example.test" ];
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "localhost"; port = $port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "compose-password"; };
  }];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic compose fixture' \
  application "$(cat assets/app-id)" account compose-synthetic reference compose-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi
}
trap cleanup EXIT

# A cached synthetic message to reply to would need an inbox; the reply preparation
# itself has its own tests. This run covers editing, durability and submission.
start editing
wtype -s 200 -M ctrl n -m ctrl -s 700
capture empty-draft
if [[ ${MAILSTRONAUT_COMPOSE_CAPTURE_ONLY:-0} == 1 ]]; then
  printf 'Inspection only: %s\n' "$artifacts"
  exit 0
fi
wtype -s 100 'to@example.test' -s 100 -M ctrl -M shift b -m shift -m ctrl -s 400
wtype -s 100 'copy@example.test' -k Tab -s 100 'blind@example.test' -k Tab -s 100
wtype 'Synthetic draft · Unicode subject' -k Tab -s 100 'Synthetic body saved before closing.'
# Software rendering in a debug build can lag well behind the virtual keyboard.
saved=0
for _ in {1..150}; do
  if python3 - "$database" <<'PY'
import json, sqlite3, sys
row = sqlite3.connect(sys.argv[1]).execute('SELECT content,closed FROM drafts').fetchone()
c = json.loads(row[0])
sys.exit(0 if row[1] == 0 and c['body'] == 'Synthetic body saved before closing.' else 1)
PY
  then saved=1; break; fi
  kill -0 "$app"; sleep .2
done
[[ $saved == 1 ]]
capture saved
python3 - "$database" <<'PY'
import json, sqlite3, sys
c = json.loads(sqlite3.connect(sys.argv[1]).execute('SELECT content FROM drafts').fetchone()[0])
assert c['to'] == 'to@example.test' and c['cc'] == 'copy@example.test', c
assert c['bcc'] == 'blind@example.test', c
assert c['subject'] == 'Synthetic draft · Unicode subject', c
print('Native editable fields and local autosave verified.')
PY
# Close right after the last keystroke, before the autosave delay can expire.
wtype ' Final edit at close.'
close_window
python3 - "$database" <<'PY'
import json, sqlite3, sys
c = json.loads(sqlite3.connect(sys.argv[1]).execute('SELECT content FROM drafts').fetchone()[0])
assert c['body'] == 'Synthetic body saved before closing. Final edit at close.', c
print('Closing the window kept the last edit and exited normally.')
PY
start recovered
wtype -s 200 -M ctrl 2 -m ctrl -s 700
capture recovered-list
# The saved draft is the first row beneath the Drafts heading.
click 350 205
sleep .7
capture recovered-editor
python3 - "$database" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0] == 0
print('Reopening a draft submitted nothing.')
PY
# Ctrl+Enter from the focused message body is the only send in this run.
wtype -s 300 -M ctrl -k Return -m ctrl
for _ in {1..150}; do
  kill -0 "$server" 2>/dev/null || break
  kill -0 "$app"; sleep .1
done
wait "$server"; server=''
sleep 1
capture sent
python3 - "$database" "$artifacts/received.eml" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
assert c.execute('SELECT phase,uncertain FROM outbox').fetchall() == [('accepted', 0)]
assert c.execute('SELECT count(*) FROM drafts WHERE closed=0').fetchone()[0] == 0
mail = open(sys.argv[2], 'rb').read().decode()
assert 'Final edit at close.' in mail and 'to@example.test' in mail and 'copy@example.test' in mail
assert 'blind@example.test' not in mail and 'Bcc:' not in mail
assert 'SYNTHETIC-SMTP-SECRET-NOT-REAL' not in mail
print('One accepted submission; the blind recipient stayed in the envelope only.')
PY
close_window
start restarted
wtype -s 200 -M ctrl 2 -m ctrl -s 700
capture restarted-outbox
close_window
if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native compose acceptance: %s\n' "$artifacts"
