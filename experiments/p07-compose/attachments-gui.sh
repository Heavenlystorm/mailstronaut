#!/usr/bin/env bash
# Invoked only by the private FileChooser fixture. No SMTP or keyring is used.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
mkdir -p "$root/gui"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11 WINIT_X11_SCALE_FACTOR=1
unset WAYLAND_DISPLAY
app='' xvfb=''
cleanup() {
  for pid in "$app" "$xvfb"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
cat > "$root/config.nix" <<NIX
{ schemaVersion = 1; managed = true; appearance = "dark";
  accounts = [{id = "attachments-synthetic"; provider = "manual";
    address = "sender@example.test"; displayName = "Synthetic sender"; userName = "synthetic-user";
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 465; security = "tls";};
    auth = {kind = "password"; secretRef = "unused-synthetic-only";};
  }];
}
NIX
Xvfb -displayfd 3 -screen 0 1280x1100x24 -nolisten tcp 3> "$root/gui/display" > "$root/gui/xvfb.log" 2>&1 & xvfb=$!
for _ in {1..100}; do [[ -s "$root/gui/display" ]] && break; sleep .05; done
DISPLAY=":$(cat "$root/gui/display")"; export DISPLAY
start() {
  "$binary" --config "$root/config.nix" > "$root/gui/$1-app.log" 2>&1 & app=$!
  window=''
  for _ in {1..100}; do
    window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true)
    [[ -n "$window" ]] && break
    kill -0 "$app"; sleep .1
  done
  [[ -n "$window" ]]; xdotool windowfocus "$window"; sleep 1
}
click() { xdotool mousemove --window "$window" "$1" "$2" click 1; }
capture() { xdotool mousemove --window "$window" 5 5; sleep .2; magick import -window "$window" "$root/gui/$1.png"; }
start initial
click 55 358; sleep .3; click 250 145; sleep .4
capture composer
if [[ ${MAILSTRONAUT_ATTACHMENT_INSPECT_ONLY:-0} == 1 ]]; then exit 0; fi
click 260 644
wait_count() {
  local expected=$1 count=''
  for _ in {1..100}; do
    count=$(python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYCOUNT'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);print(c.execute('SELECT count(*) FROM draft_attachments').fetchone()[0])
PYCOUNT
)
    [[ $count == "$expected" ]] && return
    sleep .05
  done
  printf 'Expected %s attachments, found %s\n' "$expected" "$count" >&2
  return 1
}
wait_count 1
capture attached
click 390 684
wait_count 0
capture removed
click 260 644
wait_count 1
click 1000 24
wait "$app"; app=''
rm "$root/granted-file.bin"
start recovered
click 55 358; sleep .3; click 300 336; sleep .5
wait_count 1
capture recovered
# Remove only after opening the saved draft, demonstrating the native list is live.
click 390 684
wait_count 0
capture recovered-removed
click 1000 24
wait "$app"; app=''
printf 'Native Attach/Remove and close/restart recovery passed after deleting the selected source. No mail was sent.\n'
