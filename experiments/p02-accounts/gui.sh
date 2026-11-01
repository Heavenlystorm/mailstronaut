#!/usr/bin/env bash
# Called only by credentials.sh, against its private bus/keyring and synthetic TLS server.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
fixture=$(realpath "${MAILSTRONAUT_IMAP_FIXTURE:?prebuilt imap_tls fixture}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" && $XDG_CONFIG_HOME == "$root/config" ]]
mkdir -p "$root/gui"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export VK_DRIVER_FILES="${MAILSTRONAUT_TEST_VULKAN_ICD:?pinned software Vulkan driver}"
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
export MAILSTRONAUT_GUI_PORT_FILE="$root/gui/port"
app='' keyboard='' pointer='' compositor='' server=''
cleanup() {
  for pid in "$app" "$keyboard" "$pointer" "$compositor" "$server"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
"$fixture" --ignored --exact gui_loopback_server --nocapture > "$root/gui/server.log" 2>&1 & server=$!
for _ in {1..100}; do [[ -s $MAILSTRONAUT_GUI_PORT_FILE ]] && break; kill -0 "$server"; sleep .05; done
port=$(cat "$MAILSTRONAUT_GUI_PORT_FILE")
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$root/gui/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "inbox-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic inbox";
    userName = "synthetic-user";
    imap = { host = "localhost"; port = $port; security = "tls"; };
    smtp = { host = "smtp.example.invalid"; port = 587; security = "starttls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "inbox-password"; };
  }];
}
NIX
cp "$config" "$root/gui/original-config.nix"
cat > "$root/gui/sway.conf" <<'SWAY'
xwayland disable
output HEADLESS-1 mode 1280x840
seat seat0 fallback true
default_border none
SWAY
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway --unsupported-gpu --config "$root/gui/sway.conf" > "$root/gui/compositor.log" 2>&1 & compositor=$!
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
WLRCTL_TEST_KEEP_POINTER=1 wlrctl pointer move 0 0 > "$root/gui/pointer.log" 2>&1 & pointer=$!
sleep .5
start() {
  # Retain the keyboard capability, and make a live keyed device current before
  # a reconnect. A destroyed one-shot virtual keyboard can leave this synthetic
  # seat without a keymap, causing Modifiers to precede Keymap for a new client.
  if [[ -n "$keyboard" ]]; then kill "$keyboard"; wait "$keyboard" 2>/dev/null || true; fi
  wtype -k Escape -s 300000 > "$root/gui/keyboard.log" 2>&1 & keyboard=$!
  sleep .3
  if [[ $1 == offline ]]; then
    WAYLAND_DEBUG=client RUST_BACKTRACE=1 "$binary" --config "$config" > "$root/gui/$1-app.log" 2>&1 & app=$!
  else
    "$binary" --config "$config" > "$root/gui/$1-app.log" 2>&1 & app=$!
  fi
  for _ in {1..150}; do
    if swaymsg -t get_tree | python3 -c 'import json,sys; t=json.load(sys.stdin); walk=lambda n: n.get("app_id")=="io.github.eduardz1.Mailstronaut" or any(walk(c) for c in n.get("nodes",[])); sys.exit(0 if walk(t) else 1)'; then break; fi
    kill -0 "$app"; sleep .1
  done
  sleep 1
  kill -0 "$app"
}
stop() { kill "$app"; wait "$app" 2>/dev/null || true; app=''; }
capture() { timeout 15 grim "$root/gui/$1.png"; }
click() {
  swaymsg seat seat0 cursor set "$1" "$2" > /dev/null
  swaymsg seat seat0 cursor press button1 > /dev/null
  swaymsg seat seat0 cursor release button1 > /dev/null
  sleep .3
}
start initial
wtype -s 300 -M ctrl , -m ctrl -s 500
capture configuration
if [[ ${MAILSTRONAUT_GUI_CAPTURE_ONLY:-0} == 1 ]]; then
  printf 'Inspection only: %s/gui\n' "$root"
  exit 0
fi
# Coordinates come from actual native captures; no test-only app actions exist.
printf 'source copy observer control' | wl-copy
click 1245 317
[[ $(timeout 5 wl-paste --no-newline) == "$config" ]]
click 300 204
capture account
click 700 460
wtype -s 200 SYNTHETIC-AUTH-SECRET-NOT-REAL -s 200
capture masked
printf 'masked clipboard observer control' | wl-copy
wtype -s 200 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 300
[[ $(timeout 5 wl-paste --no-newline) == 'masked clipboard observer control' ]]
click 1190 460
application=$(cat assets/app-id)
for _ in {1..50}; do
  if timeout 5 secret-tool lookup application "$application" account inbox-synthetic reference inbox-password purpose password |
    python3 -c 'import sys; sys.exit(0 if sys.stdin.read().removesuffix("\n")=="SYNTHETIC-AUTH-SECRET-NOT-REAL" else 1)'; then break; fi
  kill -0 "$app"; sleep .1
done
timeout 5 secret-tool lookup application "$application" account inbox-synthetic reference inbox-password purpose password |
  python3 -c 'import sys; assert sys.stdin.read().removesuffix("\n")=="SYNTHETIC-AUTH-SECRET-NOT-REAL", "UI password save failed"'
cmp "$config" "$root/gui/original-config.nix"
capture saved
click 500 504
for _ in {1..100}; do
  if python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3, sys
c=sqlite3.connect(sys.argv[1]); sys.exit(0 if c.execute('SELECT count(*) FROM memberships').fetchone()[0]==2 else 1)
PY
  then break; fi
  kill -0 "$app"; sleep .1
done
wait "$server"; server=''
capture synced
assert_unopened() {
  python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute('SELECT uid,seen FROM memberships ORDER BY uid').fetchall()==[(1001,0),(1003,1)]
assert c.execute('SELECT count(*) FROM messages').fetchone()[0]==1
assert c.execute('SELECT count(*) FROM pending_operations WHERE kind IS NOT NULL').fetchone()[0]==0
PY
}
assert_unopened
# Global mail search must leave Settings and focus the visible query field.
printf 'search focus observer control' | wl-copy
wtype -s 200 -M ctrl k -m ctrl -s 400 Offline -k Return -s 800 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 300
[[ $(timeout 5 wl-paste --no-newline) == 'Offline' ]]
capture search-from-settings
assert_unopened
wtype -s 200 -M ctrl a -m ctrl -k BackSpace -k Return -s 800
wtype -s 200 -M ctrl 1 -m ctrl -s 500
capture inbox
assert_unopened
# Invalid reload keeps the last valid account/cache and does not authorize Seen.
printf '{ schemaVersion = 999; }\n' > "$config"
wtype -s 200 -M ctrl -M shift r -m shift -m ctrl -s 800
capture invalid-reload
assert_unopened
copy_body() {
  click 350 150
  wtype -s 400 -M ctrl -M shift t -m shift -m ctrl -s 300 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 300
  timeout 5 wl-paste --no-newline > "$root/gui/$1-body.txt"
  grep -q 'Synthetic body for offline reading.' "$root/gui/$1-body.txt"
}
copy_body online
capture reader
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("SELECT count(*) FROM pending_operations WHERE kind='seen' AND desired=1 AND state='pending'").fetchone()[0]==2
print('Explicit open queued Seen only after loading the original; passive sync/list/reload did not.')
PY
stop
sed 's/appearance = "light"/appearance = "dark"/' "$root/gui/original-config.nix" > "$config"
cp "$config" "$root/gui/dark-config.nix"
start offline
copy_body offline
cmp "$root/gui/online-body.txt" "$root/gui/offline-body.txt"
capture offline
cmp "$config" "$root/gui/dark-config.nix"
# Removing an account disables it without purging the cached original.
printf '{ schemaVersion = 1; managed = true; appearance = "dark"; accounts = [ ]; }\n' > "$config"
wtype -s 200 -M ctrl -M shift r -m shift -m ctrl -s 800
capture disabled
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]); assert c.execute('SELECT count(*) FROM messages').fetchone()[0]==1
print('Disabling the configured account retained its cached original across reload.')
PY
stop
if grep -F 'SYNTHETIC-AUTH-SECRET-NOT-REAL' "$root/gui/"*-app.log; then
  printf '%s\n' 'Application log exposed the synthetic password' >&2
  exit 1
fi
python3 - "$XDG_DATA_HOME" <<'PY'
from pathlib import Path
import sys
files=list(Path(sys.argv[1]).rglob('*.keyring'))
assert files, 'GUI password did not persist an encrypted keyring'
assert all(b'SYNTHETIC-AUTH-SECRET-NOT-REAL' not in p.read_bytes() for p in files)
print('The GUI credential marker is absent from persisted keyring bytes.')
PY
printf 'Native Wayland Settings, masked keyring save, TLS PEEK sync, explicit-open Seen, invalid reload, offline restart and disable-without-purge passed: %s/gui\n' "$root"
