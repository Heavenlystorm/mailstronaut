#!/usr/bin/env bash
# Only the private Secret Service fixture calls this synthetic SMTP/GUI acceptance.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_SMTP_FIXTURE:?prebuilt smtp test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus"* ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
mkdir -p "$root/compose-gui"
export MAILSTRONAUT_SMTP_PORT_FILE="$root/compose-gui/port"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11 WINIT_X11_SCALE_FACTOR=1
unset WAYLAND_DISPLAY
app='' xvfb='' server=''
cleanup() {
  for pid in "$app" "$xvfb" "$server"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
"$fixture" --ignored --exact gui_loopback_server --nocapture > "$root/compose-gui/server.log" 2>&1 & server=$!
for _ in {1..100}; do [[ -s $MAILSTRONAUT_SMTP_PORT_FILE ]] && break; kill -0 "$server"; sleep .05; done
port=$(cat "$MAILSTRONAUT_SMTP_PORT_FILE")
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
cat > "$root/compose-config.nix" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "dark";
  accounts = [{
    id = "compose-synthetic";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    aliases = [ "alias@example.test" ];
    userName = "synthetic-smtp-user";
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "localhost"; port = $port; security = "tls";};
    tls.caFiles = [ "$ca" ];
    auth = {kind = "password"; secretRef = "compose-password";};
  }];
}
NIX
printf 'SYNTHETIC-SMTP-SECRET-NOT-REAL' | secret-tool store --label='Synthetic compose fixture' \
  application "$(cat assets/app-id)" account compose-synthetic reference compose-password purpose password
Xvfb -displayfd 3 -screen 0 1280x1000x24 -nolisten tcp 3> "$root/compose-gui/display" > "$root/compose-gui/xvfb.log" 2>&1 & xvfb=$!
for _ in {1..100}; do [[ -s "$root/compose-gui/display" ]] && break; sleep .05; done
DISPLAY=":$(cat "$root/compose-gui/display")"
export DISPLAY
start() {
  "$binary" --config "$root/compose-config.nix" > "$root/compose-gui/$1-app.log" 2>&1 & app=$!
  window=''
  for _ in {1..100}; do
    window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true)
    [[ -n "$window" ]] && break
    kill -0 "$app"; sleep .1
  done
  [[ -n "$window" ]]
  xdotool windowfocus "$window"
  sleep 1
}
capture() { xdotool mousemove --window "$window" 5 5; sleep .2; magick import -window "$window" "$root/compose-gui/$1.png"; }
click() { xdotool mousemove --window "$window" "$1" "$2" click 1; }
type_text() { xdotool type --clearmodifiers --delay 1 "$1"; }
start editing
# Select configured incoming credential and unlock the shared encrypted keyring.
click 300 242
click 240 362
sleep .5
click 55 358
sleep .5
click 250 145
sleep .5
capture initial
# Remaining actions use positions established by inspection of the native widgets.
if [[ ${MAILSTRONAUT_COMPOSE_INSPECT_ONLY:-0} == 1 ]]; then
  printf 'Compose inspection artifacts: %s/compose-gui\n' "$root"
  exit 0
fi
click 300 232; type_text 'to@example.test'
click 300 278; type_text 'copy@example.test'
click 300 324; type_text 'blind@example.test'
click 300 370; type_text 'Synthetic desktop draft'
click 300 420; type_text 'Synthetic body saved before closing.'
sleep 1
capture saved
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import json,sqlite3,sys
row=sqlite3.connect(sys.argv[1]).execute('SELECT content,closed FROM drafts').fetchone()
c=json.loads(row[0]); assert row[1]==0
assert c['to']=='to@example.test' and c['cc']=='copy@example.test' and c['bcc']=='blind@example.test'
assert c['subject']=='Synthetic desktop draft' and c['body']=='Synthetic body saved before closing.'
print('Native editable fields and local autosave verified.')
PY
# Close immediately after the final keystroke, before the 500ms debounce expires.
type_text ' Final edit at close.'
click 1000 24
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'Application failed to close after saving' >&2; exit 1; fi
wait "$app"; app=''
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import json,sqlite3,sys
c=json.loads(sqlite3.connect(sys.argv[1]).execute('SELECT content FROM drafts').fetchone()[0])
assert c['body']=='Synthetic body saved before closing. Final edit at close.'
print('Close flushed the last edit and exited normally.')
PY
start recovered
click 300 242; click 240 362
sleep .5
click 55 358
sleep .5
capture recovered-list
click 300 306
sleep .5
capture recovered-editor
# Explicit foreground action against the local synthetic SMTP endpoint only.
click 312 644
for _ in {1..100}; do
  if python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "${MAILSTRONAUT_SMTP_DROP_ACK:-0}" <<'PY'
import sqlite3,sys
rows=sqlite3.connect(sys.argv[1]).execute('SELECT phase FROM outbox').fetchall()
sys.exit(0 if rows==[('delivery_unknown' if sys.argv[2]=='1' else 'accepted',)] else 1)
PY
  then break; fi
  kill -0 "$app"; sleep .1
done
wait "$server"; server=''
capture submitted
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/compose-gui/attempt" <<'PYTOKEN'
import pathlib,sqlite3,sys
pathlib.Path(sys.argv[2]).write_text(sqlite3.connect(sys.argv[1]).execute('SELECT attempt_id FROM outbox').fetchone()[0])
PYTOKEN
click 1000 24
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then
  printf '%s\n' 'Application failed to exit after closing' >&2
  exit 1
fi
wait "$app"; app=''
# Reopening must not transmit queued/accepted/uncertain messages automatically.
sed -i 's/appearance = "dark"/appearance = "light"/' "$root/compose-config.nix"
start offline
click 55 358
sleep 1
capture offline-outbox
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/compose-gui/received.eml" "${MAILSTRONAUT_SMTP_DROP_ACK:-0}" <<'PY'
import json,pathlib,sqlite3,sys
from email import policy
from email.parser import BytesParser
db=sqlite3.connect(sys.argv[1]); phase,mid,hash,uncertain=db.execute('SELECT phase,message_id,blob_hash,uncertain FROM outbox').fetchone()
assert phase==('delivery_unknown' if sys.argv[3]=='1' else 'accepted')
assert bool(uncertain)==(sys.argv[3]=='1')
assert db.execute('SELECT attempt_id FROM outbox').fetchone()[0]==(pathlib.Path(sys.argv[2]).parent/'attempt').read_text()
raw=pathlib.Path(sys.argv[2]).read_bytes(); frozen=pathlib.Path(sys.argv[1]).parent/'blobs'/hash
assert raw==frozen.read_bytes()
mail=BytesParser(policy=policy.default).parsebytes(raw)
assert mail['Bcc'] is None and mail['Message-ID']==f'<{mid}>'
assert mail.get_content().strip()=='Synthetic body saved before closing. Final edit at close.'
assert db.execute('SELECT closed FROM drafts').fetchall()==[(1,)]
print('Restart preserved terminal submission state, fixed Message-ID and exact recipient DATA bytes.')
PY
click 1000 24
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then
  printf '%s\n' 'Application failed to exit after closing' >&2
  exit 1
fi
wait "$app"; app=''
if rg -F 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$root/compose-gui/"*-app.log; then
  printf '%s\n' 'Application log exposed the synthetic credential marker' >&2
  exit 1
fi
printf 'Native compose GUI artifacts: %s/compose-gui\n' "$root"
