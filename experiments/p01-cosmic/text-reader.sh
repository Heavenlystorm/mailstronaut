#!/usr/bin/env bash
# Run a prebuilt binary in nix develop .#gui-test. Uses only private displays,
# buses, clipboard, theme and XDG directories, never the user's desktop session.
set -euo pipefail
binary=$(realpath "${1:?pass a prebuilt Mailstronaut binary}")
artifact=$(realpath -m "${2:?pass a project-local artifact directory}")
mkdir -p "$artifact"/{runtime,config,cache,data,state}
chmod 700 "$artifact/runtime"
export XDG_RUNTIME_DIR="$artifact/runtime" XDG_CONFIG_HOME="$artifact/config"
export XDG_CACHE_HOME="$artifact/cache" XDG_DATA_HOME="$artifact/data" XDG_STATE_HOME="$artifact/state"
XDG_DATA_DIRS="${MAILSTRONAUT_TEST_THEME_DATA:?use the gui-test Nix shell}:$(dirname "$(dirname "${MAILSTRONAUT_TEST_AT_SPI_BUS:?use the gui-test Nix shell}")")/share"
export XDG_DATA_DIRS
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11
export NO_AT_BRIDGE=0
export ATSPI_DBUS_IMPLEMENTATION=dbus-daemon
export GSETTINGS_BACKEND=memory
unset WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS
if [[ ${MAILSTRONAUT_PRIVATE_BUS:-} != 1 ]]; then
  # D-Bus activated services must inherit the private XDG/settings environment.
  exec dbus-run-session -- env MAILSTRONAUT_PRIVATE_BUS=1 bash "$0" "$@"
fi
mode="$XDG_CONFIG_HOME/cosmic/com.system76.CosmicTheme.Mode/v1"
mkdir -p "$mode"
printf false > "$mode/is_dark"
printf false > "$mode/auto_switch"
app='' xvfb='' bus='' control=''
cleanup() {
  for pid in "$control" "$app" "$bus" "$xvfb"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
Xvfb -displayfd 3 -screen 0 2400x1800x24 -nolisten tcp 3>"$artifact/display" >"$artifact/xvfb.log" 2>&1 &
xvfb=$!
for ((attempt=0; attempt<100; attempt++)); do [[ -s "$artifact/display" ]] && break; sleep .1; done
DISPLAY=":$(cat "$artifact/display")"
export DISPLAY
"${MAILSTRONAUT_TEST_AT_SPI_BUS:?use the gui-test Nix shell}" --launch-immediately >"$artifact/at-spi-bus.log" 2>&1 &
bus=$!
session_bus=$DBUS_SESSION_BUS_ADDRESS

start_app() {
  DBUS_SESSION_BUS_ADDRESS="$1" WINIT_X11_SCALE_FACTOR=$scale "$binary" >"$artifact/$scale-$2-app.log" 2>&1 & app=$!
  window=
  for ((attempt=0; attempt<100; attempt++)); do
    window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true)
    [[ -n "$window" ]] && break
    kill -0 "$app"
    sleep .1
  done
  [[ -n "$window" ]]
  xdotool windowfocus "$window"
  sleep 2
  read -r x y < <(awk -v s="$scale" 'BEGIN {printf "%d %d\n", 55*s, 215*s}')
  xdotool mousemove --window "$window" "$x" "$y" click 1
  sleep .5
}

capture() {
  xdotool mousemove --window "$window" 5 5
  sleep .3
  magick import -window "$window" "$artifact/$scale-$1.png"
}
keys() { xdotool key --clearmodifiers "$@"; sleep .3; }
copy_all() {
  printf 'clipboard probe sentinel' | xclip -selection clipboard -in
  keys ctrl+a ctrl+c
  timeout 5 xclip -selection clipboard -out > "$artifact/$scale-$1.txt"
  python3 - "$artifact/$scale-$1.txt" <<'PY'
import pathlib, sys
expected = ('Synthetic Servo viewport\n\nNo mail\n\n'
            'Café — français, italiano: caffè. Ελληνικά. 日本語.\n\n'
            + 'Synthetic scrolling marker\n\n' * 80 + 'End of synthetic message.')
actual = pathlib.Path(sys.argv[1]).read_bytes()
assert actual == expected.encode(), (len(actual), len(expected.encode()))
PY
}
different() {
  local value
  value=$(magick compare -metric AE "$artifact/$scale-$1.png" "$artifact/$scale-$2.png" null: 2>&1 || true)
  python3 - "$value" <<'PY'
import sys
assert float(sys.argv[1].split()[0]) > 100, sys.argv[1]
PY
}
same() {
  local value
  value=$(magick compare -metric AE "$artifact/$scale-$1.png" "$artifact/$scale-$2.png" null: 2>&1 || true)
  python3 - "$value" <<'PY'
import sys
assert float(sys.argv[1].split()[0]) == 0, sys.argv[1]
PY
}

for scale in 1 1.5 2; do
  # COSMIC's filesystem theme watcher is its fallback when no settings service
  # is available. The separate accessibility pass below uses the private bus.
  start_app "unix:path=$artifact/runtime/no-theme-bus" theme
  capture top
  copy_all full-copy
  keys Home
  keys shift+Right shift+Right shift+Right shift+Right shift+Right shift+Right shift+Right shift+Right shift+Right ctrl+c
  timeout 5 xclip -selection clipboard -out > "$artifact/$scale-selection.txt"
  [[ $(cat "$artifact/$scale-selection.txt") == Synthetic ]]
  keys ctrl+Home
  capture before-page
  keys Next
  capture page-down
  different before-page page-down
  keys Prior
  capture page-up
  same before-page page-up
  keys ctrl+End
  capture bottom
  different before-page bottom
  keys ctrl+Home
  capture returned-top
  same before-page returned-top
  # Theme updates preserve the body and native copy; resizing retains usable focus.
  printf true > "$mode/is_dark"
  sleep 1
  capture dark
  different returned-top dark
  copy_all dark-copy
  xdotool windowsize "$window" 800 620
  sleep .5
  keys ctrl+End
  capture resized-bottom
  copy_all resized-copy
  keys ctrl+Home
  # Tab away and Shift+Tab back must restore keyboard copy, without a mouse click.
  keys Tab shift+Tab
  copy_all tab-copy
  # Re-enter Text from its keyboard-focused navigation button, then cycle focus.
  keys shift+Tab Return
  copy_all keyboard-open-copy
  keys Tab shift+Tab
  copy_all keyboard-tab-copy
  kill "$app"; wait "$app" || true; app=
  start_app "$session_bus" accessibility
  copy_all reopened-copy
  # Inspect the actual AT-SPI tree on a private session bus. This is not an Orca
  # walkthrough and does not establish text-range or selection accessibility.
  GTK_A11Y=atspi zenity --info --no-wrap --text='Native reader accessibility observer control' \
    --title='Mailstronaut observer control' >"$artifact/$scale-observer-control.log" 2>&1 & control=$!
  "${MAILSTRONAUT_TEST_AT_SPI_PYTHON:?use the gui-test Nix shell}" "$(dirname "$0")/text-reader-atspi.py" "$artifact/$scale-atspi.json" "$window"
  kill "$control"; wait "$control" || true; control=
  xdotool windowfocus "$window"
  # Native mouse selection uses the known fixture layout. Accessibility results
  # are reported separately; this must not pretend an absent text tree passed.
  read -r x y < <(awk -v s="$scale" 'BEGIN {printf "%d %d\n", 184*s, 114*s}')
  printf 'mouse selection sentinel' | xclip -selection clipboard -in
  xdotool mousemove "$x" "$y" click --repeat 2 --delay 100 1
  keys ctrl+c
  timeout 5 xclip -selection clipboard -out > "$artifact/$scale-mouse-selection.txt"
  [[ $(cat "$artifact/$scale-mouse-selection.txt") == Synthetic ]]
  capture mouse-selection
  # Reopening HTML creates a fresh viewport and forwards the current dark theme.
  read -r x y < <(awk -v s="$scale" 'BEGIN {printf "%d %d\n", 55*s, 119*s}')
  xdotool mousemove --window "$window" "$x" "$y" click 1
  read -r x y < <(awk -v s="$scale" 'BEGIN {printf "%d %d\n", 700*s, 350*s}')
  rendered=false
  for ((attempt=0; attempt<30; attempt++)); do
    capture html-return
    value=$(magick "$artifact/$scale-html-return.png" -format "%[fx:p{$x,$y}.r+p{$x,$y}.g+p{$x,$y}.b]" info:)
    if awk -v value="$value" 'BEGIN {exit !(value + 0 < .1)}'; then rendered=true; break; fi
  done
  [[ "$rendered" == true ]]
  kill "$app"; wait "$app" || true; app=
  printf false > "$mode/is_dark"
done
printf 'Native selection/copy/scroll checks passed; accessibility results: %s/*-atspi.json\n' "$artifact"
