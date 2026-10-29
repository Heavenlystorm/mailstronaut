#!/usr/bin/env bash
# Called by the private Secret Service fixture, never against a user's bus/mail.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_IMAP_FIXTURE:?prebuilt imap_tls test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus"* ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
mkdir -p "$root/inbox-gui"
if [[ ${MAILSTRONAUT_FLAGS_GUI:-0} == 1 ]]; then export MAILSTRONAUT_GUI_FLAGS=1; fi
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
stop() {
  if [[ ${MAILSTRONAUT_REPLY_GUI:-0} == 1 || ${MAILSTRONAUT_FLAGS_GUI:-0} == 1 ]]; then
    xdotool mousemove --window "$window" 1000 24 click 1
    for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
    if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'Close failed to finish draft persistence' >&2; exit 1; fi
    wait "$app"
  else
    kill "$app"; wait "$app" 2>/dev/null || true
  fi
  app=''
}
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
reply_drafts() {
  # The only source message and its recipients are synthetic. This flow never
  # clicks Send, and SMTP is deliberately configured to an invalid test host.
  xdotool mousemove --window "$window" 555 300 click 1
  sleep .8
  capture reply
  xdotool mousemove --window "$window" 300 420 click 1
  xdotool key --clearmodifiers ctrl+Home
  xdotool type --clearmodifiers --delay 1 'Unsaved reply edit. '
  # Switch immediately: reply-all must save the current editor before replacing it.
  xdotool mousemove --window "$window" 55 310 click 1
  xdotool mousemove --window "$window" 635 300 click 1
  sleep .8
  capture reply-all
  xdotool mousemove --window "$window" 55 310 click 1
  xdotool mousemove --window "$window" 755 300 click 1
  sleep .8
  capture text-forward
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYREPLY'
import json,sqlite3,sys
db=sqlite3.connect(sys.argv[1])
rows=db.execute('SELECT account_id,content,closed FROM drafts ORDER BY rowid').fetchall()
assert len(rows)==3 and all(account=='inbox-synthetic' and closed==0 for account,_,closed in rows)
reply,all_reply,forward=[json.loads(content) for _,content,_ in rows]
assert reply['from']=='Synthetic inbox <reader@example.test>'
assert reply['to']=='Synthetic Sender <sender@example.test>'
assert reply['body'].startswith('Unsaved reply edit. ')
for draft in (reply,all_reply):
    assert draft['subject']=='Re: Offline fixture'
    assert draft['in_reply_to']=='fixture@example.test'
    assert draft['references']==['fixture@example.test']
    assert draft['bcc']=='' and '> Synthetic body for offline reading.' in draft['body']
assert forward['subject']=='Fwd: Offline fixture' and forward['to']==''
assert forward['in_reply_to'] is None and forward['references']==[]
assert 'attachments not included' in forward['body'] and forward['bcc']==''
assert db.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Native reply/reply-all/text forward created three durable drafts, saved an intervening edit, and never queued mail.')
PYREPLY
}
queue_flags() {
  xdotool mousemove --window "$window" 575 340 click 1
  sleep .5
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYBEFOREUNDO'
import sqlite3,sys
assert sqlite3.connect(sys.argv[1]).execute("SELECT count(*) FROM pending_operations WHERE kind='seen' AND desired=1 AND state='pending'").fetchone()[0]==2
PYBEFOREUNDO
  xdotool mousemove --window "$window" 800 340 click 1
  sleep .5
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYUNDO'
import sqlite3,sys
assert sqlite3.connect(sys.argv[1]).execute('SELECT count(*) FROM pending_operations WHERE kind IS NOT NULL').fetchone()[0]==0
print('Native unsent flag undo returned to the observed server state.')
PYUNDO
  xdotool mousemove --window "$window" 575 340 click 1
  sleep .5
  xdotool mousemove --window "$window" 665 340 click 1
  sleep .5
  capture queued-flags
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYQUEUED'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("SELECT kind,desired,state,count(*) FROM pending_operations WHERE kind IS NOT NULL GROUP BY kind,desired,state ORDER BY kind").fetchall()==[('seen',1,'pending',2),('starred',0,'pending',2)]
assert c.execute('SELECT uid,seen,starred FROM memberships ORDER BY uid').fetchall()==[(1001,0,0),(1003,1,1)]
print('Native read/unstar changes committed locally for both inbox copies without a network write.')
PYQUEUED
}
apply_flags() {
  capture offline-pending-flags
  # Foreground unlock of the same private encrypted keyring after app restart.
  xdotool mousemove --window "$window" 55 263 click 1
  xdotool mousemove --window "$window" 300 242 click 1
  xdotool mousemove --window "$window" 240 362 click 1
  sleep .5
  xdotool mousemove --window "$window" 55 310 click 1
  xdotool mousemove --window "$window" 590 145 click 1
  for _ in {1..100}; do
    if python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYREADY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);sys.exit(0 if c.execute('SELECT uid,seen,starred FROM memberships ORDER BY uid').fetchall()==[(1001,1,0),(1003,1,0)] and c.execute('SELECT count(*) FROM pending_operations WHERE kind IS NOT NULL').fetchone()[0]==0 else 1)
PYREADY
    then break; fi
    kill -0 "$app"; sleep .1
  done
  wait "$server"; server=''
  capture confirmed-flags
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
if [[ ${MAILSTRONAUT_REPLY_GUI:-0} == 1 ]]; then reply_drafts; fi
if [[ ${MAILSTRONAUT_FLAGS_GUI:-0} == 1 ]]; then queue_flags; fi
stop
if [[ ${MAILSTRONAUT_FLAGS_GUI:-0} != 1 ]]; then wait "$server"; server=''; fi
sed -i 's/appearance = "dark"/appearance = "light"/' "$root/inbox-config.nix"
start offline
xdotool mousemove --window "$window" 55 310 click 1
sleep .5
copy_body offline
capture offline
if [[ ${MAILSTRONAUT_FLAGS_GUI:-0} == 1 ]]; then apply_flags; fi
if [[ ${MAILSTRONAUT_REPLY_GUI:-0} == 1 ]]; then
  xdotool mousemove --window "$window" 55 358 click 1
  sleep .5
  capture offline-reply-drafts
  xdotool mousemove --window "$window" 300 420 click 1
  sleep .5
  capture offline-reply-editor
  xdotool mousemove --window "$window" 300 420 click 1
  xdotool key --clearmodifiers ctrl+a ctrl+c
  timeout 5 xclip -selection clipboard -out > "$root/inbox-gui/recovered-reply.txt"
  python3 - "$root/inbox-gui/recovered-reply.txt" <<'PYCOPY'
import pathlib,sys
text=pathlib.Path(sys.argv[1]).read_text()
assert text.startswith('Unsaved reply edit. ') and '> Synthetic body for offline reading.' in text
print('Native editor reopened the saved reply offline with its last edit and quote intact.')
PYCOPY
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYRESTART'
import json,sqlite3,sys
db=sqlite3.connect(sys.argv[1]); rows=db.execute('SELECT content FROM drafts ORDER BY rowid').fetchall()
assert len(rows)==3 and json.loads(rows[0][0])['body'].startswith('Unsaved reply edit. ')
assert db.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Offline restart preserved all three reply/forward drafts and did not queue or submit mail.')
PYRESTART
fi
stop
if [[ ${MAILSTRONAUT_FLAGS_GUI:-0} == 1 ]]; then
  start confirmed-offline
  xdotool mousemove --window "$window" 55 310 click 1
  sleep .5
  copy_body confirmed-offline
  capture confirmed-offline
  stop
fi
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "${MAILSTRONAUT_FLAGS_GUI:-0}" <<'PY'
import sqlite3,sys
db=sqlite3.connect(sys.argv[1]); expected=[(1001,1),(1003,1)] if sys.argv[2]=='1' else [(1001,0),(1003,1)]; assert db.execute('SELECT uid,seen FROM memberships ORDER BY uid').fetchall()==expected
assert db.execute('SELECT count(*) FROM messages').fetchone()[0]==1
print('GUI read-only PEEK and explicit flag changes matched the requested fixture mode across offline restart.')
PY
if rg -F 'SYNTHETIC-AUTH-SECRET-NOT-REAL' "$root/inbox-gui/"*-app.log; then
  printf '%s\n' 'Application log exposed the synthetic credential marker' >&2
  exit 1
fi
printf 'Native inbox GUI artifacts: %s/inbox-gui\n' "$root"
