#!/usr/bin/env bash
set -euo pipefail
binary=$1
artifact=$2
mkdir -p "$artifact"/{runtime,config,cache,data,state}
chmod 700 "$artifact/runtime"
export XDG_RUNTIME_DIR="$artifact/runtime" XDG_CONFIG_HOME="$artifact/config" XDG_CACHE_HOME="$artifact/cache" XDG_DATA_HOME="$artifact/data" XDG_STATE_HOME="$artifact/state" XDG_DATA_DIRS="$MAILSTRONAUT_TEST_THEME_DATA" DBUS_SESSION_BUS_ADDRESS="unix:path=$artifact/runtime/no-bus" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY
mode="$XDG_CONFIG_HOME/cosmic/com.system76.CosmicTheme.Mode/v1"
mkdir -p "$mode"
printf false > "$mode/is_dark"
printf false > "$mode/auto_switch"
Xvfb -displayfd 3 -screen 0 1600x1200x24 -nolisten tcp 3>"$artifact/display" >"$artifact/xvfb.log" 2>&1 & xvfb=$!
app=
trap '[[ -z "$app" ]] || kill "$app" 2>/dev/null || true; kill "$xvfb" 2>/dev/null || true' EXIT
for ((attempt=0; attempt<100; attempt++)); do [[ -s "$artifact/display" ]] && break; sleep .1; done
DISPLAY=":$(cat "$artifact/display")"; export DISPLAY
WINIT_X11_SCALE_FACTOR=1 "$binary" >"$artifact/app.log" 2>&1 & app=$!
window=
for ((attempt=0; attempt<100; attempt++)); do window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true); [[ -n "$window" ]] && break; kill -0 "$app"; sleep .1; done
[[ -n "$window" ]]
sleep 3
xdotool mousemove --window "$window" 10 10
magick import -window "$window" "$artifact/preview-before.png"
xdotool mousemove --window "$window" 55 167 click 1
sleep 2
xdotool mousemove --window "$window" 10 10
magick import -window "$window" "$artifact/about.png"
# A theme change while no WebView exists must reach the newly created view.
printf true > "$mode/is_dark"
sleep 1
xdotool mousemove --window "$window" 55 119 click 1
sleep 3
xdotool mousemove --window "$window" 10 10
magick import -window "$window" "$artifact/preview-after.png"
dark=$(magick "$artifact/preview-after.png" -format '%[fx:p{700,350}.r+p{700,350}.g+p{700,350}.b]' info:)
awk -v value="$dark" 'BEGIN { exit !(value < .1) }'
printf false > "$mode/is_dark"
sleep 1
kill -0 "$app"

# Exercise repeated disposal/reopening in the private server, retaining captures.
for ((cycle=1; cycle<=10; cycle++)); do
    xdotool mousemove --window "$window" 55 167 click 1
    sleep .3
    xdotool mousemove --window "$window" 55 119 click 1
    sleep .5
    kill -0 "$app"
done
xdotool mousemove --window "$window" 10 10
magick import -window "$window" "$artifact/preview-cycle-10.png"
light=$(magick "$artifact/preview-cycle-10.png" -format '%[fx:p{700,350}.r+p{700,350}.g+p{700,350}.b]' info:)
awk -v value="$light" 'BEGIN { exit !(value > 2.9) }'
