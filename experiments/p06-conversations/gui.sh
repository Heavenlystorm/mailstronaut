#!/usr/bin/env bash
# Offline-only native conversations in the existing private XDG/bus fixture.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus"* ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
mkdir -p "$root/conversations-gui"
export MAILSTRONAUT_THREADS_CONFIG="$root/conversations-config.nix"
cat > "$MAILSTRONAUT_THREADS_CONFIG" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "dark";
  accounts = map (identity: {
    id = identity.id;
    provider = "manual";
    address = identity.address;
    displayName = identity.id;
    userName = identity.id;
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 587; security = "starttls";};
    auth = {kind = "password"; secretRef = "synthetic-conversation-password";};
  }) [
    {id = "conversation-synthetic"; address = "reader@example.test";}
    {id = "other-synthetic"; address = "other@example.test";}
  ];
}
NIX
"${MAILSTRONAUT_THREADS_FIXTURE:?prebuilt core worker test executable}" --ignored --exact seed_conversation_gui --nocapture > "$root/conversations-gui/seed.log" 2>&1
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11 WINIT_X11_SCALE_FACTOR=1
unset WAYLAND_DISPLAY
app='' xvfb=''
cleanup() {
  for pid in "$app" "$xvfb"; do if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi; done
}
trap cleanup EXIT
Xvfb -displayfd 3 -screen 0 1280x1000x24 -nolisten tcp 3> "$root/conversations-gui/display" > "$root/conversations-gui/xvfb.log" 2>&1 & xvfb=$!
for _ in {1..100}; do [[ -s "$root/conversations-gui/display" ]] && break; sleep .05; done
DISPLAY=":$(cat "$root/conversations-gui/display")"; export DISPLAY
start() {
"$binary" --config "$MAILSTRONAUT_THREADS_CONFIG" > "$root/conversations-gui/$1-app.log" 2>&1 & app=$!
window=''
for _ in {1..100}; do window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true); [[ -n "$window" ]] && break; kill -0 "$app"; sleep .1; done
[[ -n "$window" ]]; xdotool windowfocus "$window"
}
stop() {
  click 1000 24
  for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
  if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'Conversation app failed normal close' >&2; exit 1; fi
  wait "$app"; app=''
}
start initial
for _ in {1..100}; do
  if python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYREADY'
import sqlite3,sys
try:
 c=sqlite3.connect(sys.argv[1]); ready=c.execute('SELECT count(*) FROM accounts WHERE projected_thread_revision=thread_revision').fetchone()[0]==2
except sqlite3.OperationalError: ready=False
sys.exit(0 if ready else 1)
PYREADY
  then break; fi
  kill -0 "$app"; sleep .1
done
click() { xdotool mousemove --window "$window" "$1" "$2" click 1; }
capture() { xdotool mousemove --window "$window" 5 5; sleep .2; magick import -window "$window" "$root/conversations-gui/$1.png"; }
click 55 310
sleep .5
capture inbox
click 300 415
sleep .5
capture thread
if [[ ${MAILSTRONAUT_THREADS_INSPECT_ONLY:-0} == 1 ]]; then
  click 700 260
  sleep .2
  capture member-menu
  printf 'Conversation inspection artifacts: %s/conversations-gui\n' "$root"
  exit 0
fi
copy_body() {
  xdotool key --clearmodifiers ctrl+a ctrl+c
  timeout 5 xclip -selection clipboard -out > "$root/conversations-gui/$1-body.txt"
  python3 - "$root/conversations-gui/$1-body.txt" "$2" <<'PYCOPY'
import pathlib,sys
assert sys.argv[2] in pathlib.Path(sys.argv[1]).read_text()
PYCOPY
}
copy_body second 'Synthetic second reply.'
click 575 348
sleep .7
capture one-member-read
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYFLAG'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]); ids=json.load(open(sys.argv[2]))
assert c.execute("SELECT message_id,kind,desired FROM pending_operations WHERE kind IS NOT NULL").fetchall()==[(ids['second'],'seen',1)]
assert c.execute("SELECT count(DISTINCT conversation_id) FROM conversation_members WHERE account_id='conversation-synthetic'").fetchone()[0]==2
assert c.execute("SELECT count(*) FROM conversation_members WHERE account_id='other-synthetic'").fetchone()[0]==1
print('Native conversation read marked only its selected inbox message; equal subjects and other accounts remained separate.')
PYFLAG
click 700 300
click 700 377
sleep .5
copy_body first 'Synthetic first reply.'
capture first-member
click 700 300
click 700 341
sleep .5
copy_body parent 'Synthetic sent parent.'
capture sent-parent
# The ancestor has no Inbox membership; its read/star controls are disabled.
click 575 380
sleep .3
click 565 341
sleep .8
capture reply-to-parent
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYDRAFT'
import json,sqlite3,sys
from email.utils import parseaddr
c=sqlite3.connect(sys.argv[1]); account,content=c.execute('SELECT account_id,content FROM drafts').fetchone(); draft=json.loads(content)
assert account=='conversation-synthetic' and parseaddr(draft['from'])[1]=='reader@example.test'
assert draft['to']=='alice@example.test' and draft['in_reply_to']=='parent@example.test'
assert draft['references']==['parent@example.test'] and 'Synthetic sent parent.' in draft['body']
assert c.execute('SELECT count(*) FROM pending_operations WHERE kind IS NOT NULL').fetchone()[0]==1
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Native Sent-ancestor selection quoted the right message/account and disabled ancestor-only flag edits; no mail queued.')
PYDRAFT
stop
sed -i 's/appearance = "dark"/appearance = "light"/' "$MAILSTRONAUT_THREADS_CONFIG"
start offline
sleep .5
click 55 310
sleep .5
capture offline-inbox
click 300 415
sleep .5
copy_body recovered-second 'Synthetic second reply.'
capture offline-thread
# Identical RFC identifiers in another account still open that account's body.
click 300 330
sleep .5
copy_body other-account 'Synthetic other-account reply.'
capture other-account
click 565 268
sleep .8
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYOTHER'
import json,sqlite3,sys
from email.utils import parseaddr
c=sqlite3.connect(sys.argv[1]); rows=c.execute('SELECT account_id,content FROM drafts ORDER BY rowid').fetchall()
assert len(rows)==2
account,content=rows[-1]; draft=json.loads(content)
assert account=='other-synthetic' and parseaddr(draft['from'])[1]=='other@example.test'
assert draft['in_reply_to']=='first@example.test' and 'Synthetic other-account reply.' in draft['body']
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Offline restart kept grouping and per-message state; the other account opened and replied with its own identity.')
PYOTHER
capture other-account-reply
stop
printf 'Native conversation GUI artifacts: %s/conversations-gui\n' "$root"
