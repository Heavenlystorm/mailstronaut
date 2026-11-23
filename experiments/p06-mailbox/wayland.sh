#!/usr/bin/env bash
# Synthetic native Wayland mailbox behavior; no real account, keyring or X server.
set -euo pipefail
binary=$(realpath "${1:-target/debug/mailstronaut}")
fixture=$(realpath "${MAILSTRONAUT_MAILBOX_FIXTURE:?prebuilt core worker test executable}")
root=$(mktemp -d "$PWD/target/p06.XXXXXX")
export HOME="$root/home"
mkdir -p "$HOME"
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export VK_DRIVER_FILES="${MAILSTRONAUT_TEST_VULKAN_ICD:?pinned software Vulkan driver}"
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
app='' keyboard='' pointer='' compositor='' bus='' blocker='' export_portal=''
cleanup() {
  for pid in "$app" "$keyboard" "$pointer" "$compositor" "$export_portal" "$bus" "$blocker"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
# A private bus without activation directories cannot launch host desktop services.
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=$root/runtime/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/><allow own="*"/>
    <allow send_destination="*"/><allow receive_sender="*"/>
  </policy>
</busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
for _ in {1..100}; do [[ -S $root/runtime/bus ]] && break; sleep .05; done

config="$root/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
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
if [[ ${MAILSTRONAUT_REMINDERS_GUI:-0} == 1 ]]; then
  sed -i '/schemaVersion = 1;/a\  timeZone = "Europe/Paris";\n  notifications.enabled = false;' "$config"
fi
if [[ ${MAILSTRONAUT_PREFERENCES_GUI:-0} == 1 || ${MAILSTRONAUT_LOGIN_GUI:-0} == 1 || ${MAILSTRONAUT_HISTORY_SETTING_GUI:-0} == 1 ]]; then
  python3 - "$config" <<'PYMUTABLE'
from pathlib import Path
import sys
p=Path(sys.argv[1])
p.write_text('# Mailstronaut editable configuration\n' + p.read_text().replace('managed = true;', 'managed = false;'))
PYMUTABLE
fi
if [[ ${MAILSTRONAUT_LOGIN_GUI:-0} == 1 ]]; then
  sed -i 's/displayName = identity.id;/displayName = "Synthetic Reader";/' "$config"
fi
export MAILSTRONAUT_THREADS_CONFIG="$config" MAILSTRONAUT_THREADS_GUI_BINARY="$binary"
navigation_seed=${MAILSTRONAUT_MAILBOX_NAVIGATION:-0}
if [[ ${MAILSTRONAUT_CONTINUOUS_GUI:-0} == 1 ]]; then navigation_seed=1; fi
MAILSTRONAUT_MAILBOX_NAVIGATION=$navigation_seed "$fixture" seed_conversation_gui \
  --ignored --exact --nocapture > "$root/seed.log" 2>&1
if [[ ${MAILSTRONAUT_LONG_THREAD_GUI:-0} == 1 ]]; then
  "${MAILSTRONAUT_LONG_THREAD_FIXTURE:?prebuilt long-thread fixture}" --ignored --exact seed_long_thread > "$root/long-seed.log" 2>&1
fi
if [[ ${MAILSTRONAUT_INDEX_PROGRESS_GUI:-0} == 1 ]]; then
  # A backlog of canonical deletion records puts a real fuzzy match in the final
  # batch. The app observes the same queue when rebuilding a previously purged index.
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYQUEUE'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.executemany("INSERT INTO search_pending(message_id) VALUES (?)",
                   ((number,) for number in range(100000, 132768)))
PYQUEUE
fi
if [[ ${MAILSTRONAUT_ONBOARDING_GUI:-0} == 1 ]]; then
  mv "$config" "$root/seeded-config.nix"
fi
cat > "$root/sway.conf" <<'SWAY'
xwayland disable
output HEADLESS-1 mode 1280x840
seat seat0 fallback true
default_border none
SWAY
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway --unsupported-gpu --config "$root/sway.conf" > "$root/compositor.log" 2>&1 & compositor=$!
for _ in {1..200}; do
  socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'wayland-*' -print -quit)
  sway_socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)
  [[ -n "$socket" && -S $sway_socket ]] && break
  kill -0 "$compositor"; sleep .05
done
[[ -S $socket ]]
export WAYLAND_DISPLAY="$socket"
export SWAYSOCK="$sway_socket"
[[ -S $SWAYSOCK ]]
WLRCTL_TEST_KEEP_POINTER=1 wlrctl pointer move 0 0 > "$root/pointer.log" 2>&1 & pointer=$!
sleep .5
start() {
  # Retain the keyboard capability, and make a live keyed device current before
  # a reconnect. A destroyed one-shot virtual keyboard can leave this synthetic
  # seat without a keymap, causing Modifiers to precede Keymap for a new client.
  if [[ -n "$keyboard" ]]; then kill "$keyboard"; wait "$keyboard" 2>/dev/null || true; fi
  wtype -k Escape -s 300000 > "$root/keyboard.log" 2>&1 & keyboard=$!
  sleep .3
  if [[ $1 == offline ]]; then
    WAYLAND_DEBUG=client RUST_BACKTRACE=1 "$binary" --config "$config" > "$root/$1-app.log" 2>&1 & app=$!
  else
    "$binary" --config "$config" > "$root/$1-app.log" 2>&1 & app=$!
  fi
  for _ in {1..150}; do
    if swaymsg -t get_tree | python3 -c 'import json,sys; t=json.load(sys.stdin); walk=lambda n: n.get("app_id")=="io.github.eduardz1.Mailstronaut" or any(walk(c) for c in n.get("nodes",[])); sys.exit(0 if walk(t) else 1)'; then break; fi
    kill -0 "$app"; sleep .1
  done
  sleep 1
  kill -0 "$app"
}
stop() { kill "$app"; wait "$app" 2>/dev/null || true; app=''; }
capture() { timeout 15 grim "$root/$1.png"; }
click() {
  swaymsg seat seat0 cursor set "$1" "$2" > /dev/null
  swaymsg seat seat0 cursor press button1 > /dev/null
  swaymsg seat seat0 cursor release button1 > /dev/null
  sleep .3
}

check_seen() {
  python3 - "$root" "$@" <<'PYSEEN'
import json, sqlite3, sys
from pathlib import Path
root=Path(sys.argv[1]); ids=json.loads((root/'conversation-ids.json').read_text())
c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
expected=set(sys.argv[2:])
for key,mid in ids.items():
    seen=c.execute('SELECT min(seen) FROM effective_memberships WHERE message_id=?',(mid,)).fetchone()[0]
    assert seen==int(key=='parent' or key in expected), (key,seen,expected)
assert c.execute('SELECT count(*) FROM drafts').fetchone()[0]==0
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Effective Seen matches explicit opens:',sorted(expected),flush=True)
PYSEEN
}
copy_body() {
  wtype -s 200 -M ctrl -M shift t -m shift -m ctrl -s 200 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 200
  # The copy reaches the compositor a moment later, and a slow debug build can lag.
  for _ in {1..25}; do
    timeout 5 wl-paste --no-newline > "$root/$1-body.txt" 2> /dev/null || true
    grep -q "$2" "$root/$1-body.txt" && return 0
    sleep .2
  done
  return 1
}
start initial
sleep 1
capture initial
if [[ ${MAILSTRONAUT_MAILBOX_CAPTURE_ONLY:-0} == 1 ]]; then printf 'Inspection: %s\n' "$root"; exit 0; fi
check_seen
if [[ ${MAILSTRONAUT_INDEX_PROGRESS_GUI:-0} == 1 ]]; then
  database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
  remaining() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute("SELECT COUNT(*) FROM search_pending").fetchone()[0])' "$database"; }
  wtype -M ctrl k -m ctrl -s 120 'unrelatd' -s 400
  [[ $(remaining) -gt 64 ]]
  capture indexing-active
  for _ in {1..1200}; do [[ $(remaining) == 0 ]] && break; kill -0 "$app"; sleep .1; done
  [[ $(remaining) == 0 ]]
  sleep 1
  capture indexing-complete
  check_seen
  click 350 150
  sleep .5
  copy_body indexed 'Synthetic unrelated same-subject message.'
  check_seen unrelated
  printf 'Active search refreshed without typing again: %s\n' "$root"
  exit 0
fi
if [[ ${MAILSTRONAUT_LONG_THREAD_GUI:-0} == 1 ]]; then
  click 280 120
  sleep 1
  capture long-thread-open
  click 920 292
  sleep .5
  capture long-thread-expanded
  keys=()
  for _ in {1..104}; do keys+=(-k Down -s 30); done
  wtype "${keys[@]}" -s 800
  python3 - "$root" <<'PYTHREAD'
import json, sqlite3, sys
from pathlib import Path
root = Path(sys.argv[1])
ids = json.loads((root / 'long-thread-ids.json').read_text())
c = sqlite3.connect(root / 'data/mailstronaut/mail.sqlite3')
seen = [n for n, mid in enumerate(ids, 1) if c.execute('SELECT min(seen) FROM effective_memberships WHERE message_id=?', (mid,)).fetchone()[0]]
assert seen == [105], seen
PYTHREAD
  capture long-thread-passive
  wtype -k Return
  # Waiting for clipboard output alone can miss the focus request while
  # the asynchronous reader is still opening the selected original.
  number=$(python3 - "$root" <<'PYTHREAD'
import json, sqlite3, sys, time
from pathlib import Path
root = Path(sys.argv[1])
ids = json.loads((root / 'long-thread-ids.json').read_text())
db = sqlite3.connect(root / 'data/mailstronaut/mail.sqlite3')
for _ in range(100):
    seen = [n for n, mid in enumerate(ids, 1)
            if db.execute('SELECT min(seen) FROM effective_memberships WHERE message_id=?', (mid,)).fetchone()[0]]
    if seen != [105]:
        assert len(seen) == 2 and seen[-1] == 105, seen
        print(f'{seen[0]:03}')
        break
    time.sleep(0.1)
else:
    raise SystemExit('the confirmed thread original never became Seen')
PYTHREAD
)
  capture long-thread-just-opened
  copy_body long-thread-old "Synthetic thread original $number."
  capture long-thread-confirmed
  printf 'LONG_THREAD_GUI_OK: %s\n' "$root"
  exit 0
fi
if [[ ${MAILSTRONAUT_HISTORY_SETTING_GUI:-0} == 1 ]]; then
  wtype -M ctrl , -m ctrl -s 500
  click 310 126
  wtype 'History limit' -s 300
  capture history-setting-before
  click 1200 275
  wtype -M ctrl a -m ctrl '30' -k Return -s 200
  click 275 783
  sleep 1
  "$binary" config export --path "$config" > "$root/history-setting-saved.nix"
  grep -Fq 'initialDays = 30;' "$root/history-setting-saved.nix"
  capture history-setting-saved
  click 310 126
  wtype -M ctrl a -m ctrl 'History limit' -s 300
  click 1200 275
  wtype -M ctrl a -m ctrl '0' -k Return -s 200
  click 275 783
  sleep 1
  "$binary" --internal-evaluate-config < "$config" > "$root/history-setting-values.json"
  python3 - "$root/history-setting-values.json" <<'PYHISTORY'
import json, sys
with open(sys.argv[1], encoding='utf-8') as source:
    assert json.load(source)['Success']['sync'].get('initialDays') is None
PYHISTORY
  check_seen
  printf 'HISTORY_SETTING_GUI_OK: %s\n' "$root"
  exit 0
fi
if [[ ${MAILSTRONAUT_LOGIN_GUI:-0} == 1 ]]; then
  wtype -M ctrl , -m ctrl -s 500
  click 300 205
  capture account-logins
  click 650 544
  wtype -M ctrl a -m ctrl 'short-login' -s 200
  click 650 624
  wtype 'smtp-login' -s 200
  click 850 669
  sleep 1
  "$binary" config export --path "$config" > "$root/login-saved.nix"
  grep -Fq 'userName = "short-login";' "$root/login-saved.nix"
  grep -Fq 'smtpUserName = "smtp-login";' "$root/login-saved.nix"
  capture account-logins-saved
  check_seen
  printf 'LOGIN_GUI_OK: %s\n' "$root"
  exit 0
fi
if [[ ${MAILSTRONAUT_CONTINUOUS_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=continuous.sh
  source "$(dirname "${BASH_SOURCE[0]}")/continuous.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_APPEARANCE_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=appearance.sh
  source "$(dirname "${BASH_SOURCE[0]}")/appearance.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_INSTANCE_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=../p08-reminders/instance-gui.sh
  source "$(dirname "${BASH_SOURCE[0]}")/../p08-reminders/instance-gui.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_REMINDERS_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=../p08-reminders/gui.sh
  source "$(dirname "${BASH_SOURCE[0]}")/../p08-reminders/gui.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_ONBOARDING_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=../p05-onboarding/gui.sh
  source "$(dirname "${BASH_SOURCE[0]}")/../p05-onboarding/gui.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_PREFERENCES_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=../p02-config/preferences-gui.sh
  source "$(dirname "${BASH_SOURCE[0]}")/../p02-config/preferences-gui.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_EXPORT_GUI:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=../p02-config/export-gui.sh
  source "$(dirname "${BASH_SOURCE[0]}")/../p02-config/export-gui.sh"
  exit 0
fi
if [[ ${MAILSTRONAUT_MAILBOX_NAVIGATION:-0} == 1 ]]; then
  # shellcheck source-path=SCRIPTDIR
  # shellcheck source=navigation.sh
  source "$(dirname "${BASH_SOURCE[0]}")/navigation.sh"
  exit 0
fi
# Arrow selection, refresh and page return are passive, never a content open.
wtype -s 200 -M ctrl l -m ctrl -k Down -s 200 -M ctrl r -m ctrl -s 600
check_seen
capture passive-selection
wtype -s 200 -M ctrl , -m ctrl -s 200 -M ctrl 1 -m ctrl -s 600
check_seen
# Keep a write transaction pending while opening a different message and refreshing.
# This makes the earlier flag callback overlap the newer queued body load.
python3 - "$root" <<'PYLOCK' > "$root/contention.log" 2>&1 &
import sqlite3,sys,time
from pathlib import Path
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
c.execute('BEGIN IMMEDIATE'); (root/'locked').touch()
for _ in range(600):
    if (root/'release').exists(): break
    time.sleep(.05)
else: raise AssertionError('transaction release timed out')
c.rollback()
PYLOCK
blocker=$!
for _ in {1..100}; do [[ -f $root/locked ]] && break; kill -0 "$blocker"; sleep .05; done
[[ -f $root/locked ]]
# Select the thread with two unread originals (third row) and explicitly open it.
wtype -s 200 -M ctrl l -m ctrl -k Down -k Down -k Return -s 400
copy_body blocked 'Synthetic second reply.'
# Before the previous write finishes, explicitly open the other account and refresh.
wtype -s 200 -M ctrl l -m ctrl -k Up -k Return -M ctrl r -m ctrl
printf release > "$root/release"
wait "$blocker"; blocker=''
sleep 1
copy_body newest 'Synthetic other-account reply.'
check_seen second other
capture canonical-unread
# Positive control: the original thread badge had two unread messages. Its
# pixels must change after one open while a badge remains for the unread sibling.
[[ $(magick "$root/initial.png" "$root/canonical-unread.png" -compose difference -composite -crop 16x16+532+368 -format '%[fx:mean>0.001]' info:) == 1 ]]
[[ $(magick "$root/canonical-unread.png" -crop 16x16+532+368 -colorspace gray -threshold 30% -format '%[fx:mean<0.8]' info:) == 1 ]]
# The other account has no unread sibling, so its dark badge must disappear.
[[ $(magick "$root/canonical-unread.png" -crop 16x16+532+244 -colorspace gray -threshold 30% -format '%[fx:mean>0.98]' info:) == 1 ]]
# Confirmed selection must survive the automatic unread-row repaint.
wtype -s 200 -M ctrl l -m ctrl -k Return -s 400
copy_body retained 'Synthetic other-account reply.'
check_seen second other
# Search for the older original. Selection is passive; confirmation opens that original.
wtype -s 200 -M ctrl k -m ctrl 'first reply' -k Return -s 500 -M ctrl l -m ctrl -k Down -s 200
check_seen second other
capture matching-original-selected
wtype -s 200 -k Return -s 700
copy_body match 'Synthetic first reply.'
check_seen first second other
capture thread-read
# Clear query and explicitly reopen the thread before switching appearance.
wtype -s 200 -M ctrl k -m ctrl -M ctrl a -m ctrl -k BackSpace -k Return -s 500 -M ctrl -M shift d -m shift -m ctrl -s 500
wtype -s 200 -M ctrl l -m ctrl -k Down -k Down -k Down -k Return -s 400
copy_body dark 'Synthetic second reply.'
capture dark
check_seen first second other
stop
start offline
check_seen first second other
capture offline
# A mouse open must transfer focus out of the query field to the native list.
# Then Down is passive selection and Enter opens that next row without Ctrl+L.
wtype -s 200 -M ctrl k -m ctrl
click 350 150
sleep .5
wtype -s 200 -k Down -k Return -s 500
copy_body mouse-then-keyboard 'Synthetic other-account reply.'
check_seen first second other unrelated
capture mouse-then-keyboard
stop
if grep -Eiq 'panic|fatal|error:' "$root/initial-app.log" "$root/offline-app.log"; then
  printf '%s\n' 'Native mailbox reported a runtime failure' >&2; exit 1
fi
printf 'Native Wayland Kit List, passive selection, explicit original open, concurrent refresh, canonical unread and offline restart passed: %s\n' "$root"
