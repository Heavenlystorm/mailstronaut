#!/usr/bin/env bash
# Called by the private Secret Service fixture, never against a user's bus/mail.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_IMAP_FIXTURE:?prebuilt imap_tls test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus"* ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
mkdir -p "$root/inbox-gui"
export MAILSTRONAUT_GUI_PORT_FILE="$root/inbox-gui/port"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11 WINIT_X11_SCALE_FACTOR=1
unset WAYLAND_DISPLAY
app='' xvfb='' server=''
cleanup() {
  for pid in "$app" "$xvfb" "$server"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
"$fixture" --ignored --exact gui_loopback_server --nocapture > "$root/inbox-gui/server.log" 2>&1 & server=$!
for _ in {1..100}; do [[ -s $MAILSTRONAUT_GUI_PORT_FILE ]] && break; kill -0 "$server"; sleep .05; done
port=$(cat "$MAILSTRONAUT_GUI_PORT_FILE")
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
cat > "$root/inbox-config.nix" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "dark";
  accounts = [{
    id = "inbox-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic inbox";
    userName = "synthetic-user";
    imap = {host = "localhost"; port = $port; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 587; security = "starttls";};
    tls.caFiles = [ "$ca" ];
    auth = {kind = "password"; secretRef = "inbox-password";};
  }];
}
NIX
printf 'SYNTHETIC-AUTH-SECRET-NOT-REAL' | secret-tool store --label='Synthetic inbox fixture' \
  application "$(cat assets/app-id)" account inbox-synthetic reference inbox-password purpose password
Xvfb -displayfd 3 -screen 0 1280x900x24 -nolisten tcp 3> "$root/inbox-gui/display" > "$root/inbox-gui/xvfb.log" 2>&1 & xvfb=$!
for _ in {1..100}; do [[ -s "$root/inbox-gui/display" ]] && break; sleep .05; done
DISPLAY=":$(cat "$root/inbox-gui/display")"
export DISPLAY
start() {
  "$binary" --config "$root/inbox-config.nix" > "$root/inbox-gui/$1-app.log" 2>&1 & app=$!
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
stop() { kill "$app"; wait "$app" 2>/dev/null || true; app=''; }
capture() { xdotool mousemove --window "$window" 5 5; sleep .2; magick import -window "$window" "$root/inbox-gui/$1.png"; }
copy_body() {
  xdotool mousemove --window "$window" 300 280 click 1
  sleep .5
  xdotool key --clearmodifiers ctrl+a ctrl+c
  timeout 5 xclip -selection clipboard -out > "$root/inbox-gui/$1-body.txt"
  python3 - "$root/inbox-gui/$1-body.txt" <<'PY'
import pathlib,sys
assert 'Synthetic body for offline reading.' in pathlib.Path(sys.argv[1]).read_text()
PY
}
start online
xdotool mousemove --window "$window" 300 242 click 1
xdotool mousemove --window "$window" 240 362 click 1
sleep .5
xdotool mousemove --window "$window" 55 310 click 1
sleep .5
capture empty
xdotool mousemove --window "$window" 230 145 click 1
for _ in {1..100}; do
  if python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3,sys
try:
    db=sqlite3.connect(sys.argv[1]); ready=db.execute('SELECT count(*) FROM memberships').fetchone()[0]==2
except sqlite3.OperationalError:
    ready=False
sys.exit(0 if ready else 1)
PY
  then break; fi
  kill -0 "$app"; sleep .1
done
capture inbox
copy_body online
capture reader
stop
wait "$server"; server=''
sed -i 's/appearance = "dark"/appearance = "light"/' "$root/inbox-config.nix"
start offline
xdotool mousemove --window "$window" 55 310 click 1
sleep .5
copy_body offline
capture offline
stop
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3,sys
db=sqlite3.connect(sys.argv[1]); assert db.execute('SELECT uid,seen FROM memberships ORDER BY uid').fetchall()==[(1001,0),(1003,1)]
assert db.execute('SELECT count(*) FROM messages').fetchone()[0]==1
print('GUI PEEK preserved server flags and persisted canonical offline content.')
PY
if rg -F 'SYNTHETIC-AUTH-SECRET-NOT-REAL' "$root/inbox-gui/"*-app.log; then
  printf '%s\n' 'Application log exposed the synthetic credential marker' >&2
  exit 1
fi
printf 'Native inbox GUI artifacts: %s/inbox-gui\n' "$root"
