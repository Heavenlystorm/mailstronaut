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
app='' xvfb='' prompt=''
cleanup() {
  for pid in "$app" "$xvfb" "$prompt"; do if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi; done
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
if [[ ${MAILSTRONAUT_THREADS_INSPECT_ONLY:-0} == 1 || ${MAILSTRONAUT_THREADS_BROWSE_GUI:-0} == 1 ]]; then
  click 300 415
  sleep .5
  capture thread
fi
if [[ ${MAILSTRONAUT_THREADS_INSPECT_ONLY:-0} == 1 ]]; then
  click 700 260
  sleep .2
  capture member-menu
  xdotool key Escape
  click 350 100
  capture scope-menu
  printf 'Conversation inspection artifacts: %s/conversations-gui\n' "$root"
  exit 0
fi
copy_body() {
  xdotool key --clearmodifiers ctrl+a ctrl+c
  timeout 5 xclip -selection clipboard -out > "$root/conversations-gui/$1-body.txt"
  python3 - "$root/conversations-gui/$1-body.txt" "$2" <<'PYCOPY'
import pathlib,sys
assert sys.argv[2] in pathlib.Path(sys.argv[1]).read_text(), f"Wrong synthetic body: {sys.argv[1]}"
PYCOPY
}
if [[ ${MAILSTRONAUT_THREADS_BROWSE_GUI:-0} == 1 ]]; then
  search() { click 650 85; xdotool key --clearmodifiers ctrl+a BackSpace; xdotool type --clearmodifiers --delay 0 "$1"; xdotool key Return; sleep .5; }
  scope() { click 350 85; click 350 "$((126+36*$1))"; sleep .5; }
  search first
  click 300 245
  sleep .4
  copy_body search-first 'Synthetic first reply.'
  capture search-first
  search 'sent parent'
  capture inbox-excludes-sent
  scope 1
  click 300 245
  sleep .4
  copy_body account-parent 'Synthetic sent parent.'
  capture account-search
  scope 3
  click 300 245
  sleep .4
  copy_body folder-parent 'Synthetic sent parent.'
  capture sent-folder
  search first
  capture sent-excludes-inbox
  search other-account
  scope 4
  click 300 245
  sleep .4
  copy_body searched-other-account 'Synthetic other-account reply.'
  capture other-account-search
  search 'reply OR questionnaire'
  capture literal-operator-empty
  search ''
  scope 0
  click 300 415
  sleep .4
  copy_body unified-again 'Synthetic second reply.'
  stop
  sed -i 's/appearance = "dark"/appearance = "light"/' "$MAILSTRONAUT_THREADS_CONFIG"
  start offline-search
  sleep .5
  click 55 310
  sleep .4
  search first
  click 300 245
  sleep .4
  copy_body offline-search-first 'Synthetic first reply.'
  capture offline-search
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYBROWSE'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
for table in ['drafts','outbox']:
 assert c.execute(f'SELECT count(*) FROM {table}').fetchone()[0]==0
allowed={ids['first'],ids['second'],ids['other']}
assert all(mid in allowed and kind=='seen' and desired==1 for mid,kind,desired in c.execute('SELECT message_id,kind,desired FROM pending_operations'))
assert c.execute('SELECT count(*) FROM messages').fetchone()[0]==5
print('Native local search opened matching older members, scoped Sent/account data, kept accounts separate, and survived offline restart without outgoing changes; only opened Inbox members were eligible for automatic read.')
PYBROWSE
  stop
  printf 'Native local browsing artifacts: %s/conversations-gui\n' "$root"
  exit 0
fi
# Opening then immediately leaving Inbox must never mark an unseen body read.
click 300 415
click 55 262
sleep 1.2
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYHIDDEN'
import sqlite3,sys
assert sqlite3.connect(sys.argv[1]).execute('SELECT count(*) FROM pending_operations').fetchone()[0]==0
print('Leaving Inbox before its dwell did not mark hidden content read.')
PYHIDDEN
capture hidden-reader
# Prepare the focus target before opening another message, avoiding dialog-startup
# latency being mistaken for time spent in an unfocused reader.
GDK_BACKEND=x11 zenity --info --title='Synthetic read focus fixture' --text='Synthetic focus fixture only' > "$root/conversations-gui/focus.log" 2>&1 & prompt=$!
other_window=''
for _ in {1..100}; do other_window=$(xdotool search --onlyvisible --pid "$prompt" 2>/dev/null | head -n1 || true); [[ -n "$other_window" ]] && break; sleep .05; done
[[ -n "$other_window" ]]
xdotool windowfocus "$window"
click 55 310
sleep .1
click 300 245
xdotool windowfocus "$other_window"
sleep 1.2
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYFOCUS'
import sqlite3,sys
assert sqlite3.connect(sys.argv[1]).execute('SELECT count(*) FROM pending_operations').fetchone()[0]==0
print('An unfocused reader did not mark its selected message read.')
PYFOCUS
capture unfocused-reader
kill "$prompt"; wait "$prompt" 2>/dev/null || true; prompt=''
xdotool windowfocus "$window"
sleep 1.2
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYREAD'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert c.execute('SELECT message_id,kind,desired FROM pending_operations').fetchall()==[(ids['unrelated'],'seen',1)]
print('Returning focus completed a fresh dwell and marked only the selected message read.')
PYREAD
capture focused-read
click 300 447
sleep 1.2
capture before-second-copy
copy_body second 'Synthetic second reply.'
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYTHREADREAD'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert set(c.execute('SELECT message_id,desired FROM pending_operations'))=={(ids['unrelated'],1),(ids['second'],1)}
assert c.execute('SELECT count(*) FROM pending_operations WHERE message_id=?',(ids['first'],)).fetchone()[0]==0
print('Visible reading cleared only its selected conversation member, not its sibling.')
PYTHREADREAD
capture automatic-member-read
click 575 380
sleep 1.2
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYUNREAD'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert c.execute('SELECT desired FROM pending_operations WHERE message_id=?',(ids['second'],)).fetchone()[0]==0
assert c.execute('SELECT read_cleared FROM messages WHERE id=?',(ids['second'],)).fetchone()[0]==1
print('Explicit Mark unread remained unread beyond another dwell; read-cleared history stayed monotonic.')
PYUNREAD
capture explicit-unread
click 700 300
click 700 341
sleep 1.2
copy_body parent 'Synthetic sent parent.'
capture sent-parent
click 565 341
sleep .8
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYDRAFT'
import json,sqlite3,sys
from email.utils import parseaddr
c=sqlite3.connect(sys.argv[1]);account,content=c.execute('SELECT account_id,content FROM drafts').fetchone();draft=json.loads(content)
assert account=='conversation-synthetic' and parseaddr(draft['from'])[1]=='reader@example.test'
assert draft['to']=='alice@example.test' and draft['in_reply_to']=='parent@example.test' and 'Synthetic sent parent.' in draft['body']
assert c.execute('SELECT count(*) FROM pending_operations').fetchone()[0]==2
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Sent-only ancestor reading remained read-only and reply used its own message/account.')
PYDRAFT
capture reply-to-parent
stop
sed -i 's/appearance = "dark"/appearance = "light"/' "$MAILSTRONAUT_THREADS_CONFIG"
start offline
sleep .5
click 55 310
sleep .3
capture offline-inbox
# Rapid message changes retire the second member's new dwell before reading it.
click 300 415
click 300 330
sleep 1.2
copy_body other-account 'Synthetic other-account reply.'
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYRESTART'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert set(c.execute('SELECT message_id,desired FROM pending_operations'))=={(ids['unrelated'],1),(ids['second'],0),(ids['other'],1)}
print('Offline restart preserved explicit unread; a rapid selection change did not let its retired dwell mark it read.')
PYRESTART
capture other-account
click 565 300
sleep .8
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYOTHER'
import json,sqlite3,sys
from email.utils import parseaddr
c=sqlite3.connect(sys.argv[1]);rows=c.execute('SELECT account_id,content FROM drafts ORDER BY rowid').fetchall()
assert len(rows)==2
account,content=rows[-1];draft=json.loads(content)
assert account=='other-synthetic' and parseaddr(draft['from'])[1]=='other@example.test'
assert draft['in_reply_to']=='first@example.test' and 'Synthetic other-account reply.' in draft['body']
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Other-account reply identity and quoted body remained correct after focused read handling.')
PYOTHER
capture other-account-reply
stop
printf 'Native focused-read GUI artifacts: %s/conversations-gui\n' "$root"
