#!/usr/bin/env bash
set -euo pipefail
binary=$1
artifact=${2:-$(mktemp -d /tmp/mailstronaut-hidpi.XXXXXX)}
mkdir -p "$artifact"
echo "Artifacts: $artifact"
mkdir -p "$artifact"/{runtime,config,cache,data,state}
chmod 700 "$artifact/runtime"
export XDG_RUNTIME_DIR="$artifact/runtime" XDG_CONFIG_HOME="$artifact/config" XDG_CACHE_HOME="$artifact/cache" XDG_DATA_HOME="$artifact/data" XDG_STATE_HOME="$artifact/state" XDG_DATA_DIRS="$MAILSTRONAUT_TEST_THEME_DATA" DBUS_SESSION_BUS_ADDRESS="unix:path=$artifact/runtime/no-bus" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY
mode="$XDG_CONFIG_HOME/cosmic/com.system76.CosmicTheme.Mode/v1"
mkdir -p "$mode"
printf false > "$mode/is_dark"
printf false > "$mode/auto_switch"
Xvfb -displayfd 3 -screen 0 2400x1800x24 -nolisten tcp 3>"$artifact/display" >"$artifact/xvfb.log" 2>&1 & xvfb=$!
app=
trap '[[ -z "$app" ]] || kill "$app" 2>/dev/null || true; kill "$xvfb" 2>/dev/null || true' EXIT
for ((attempt=0; attempt<100; attempt++)); do [[ -s "$artifact/display" ]] && break; sleep .1; done
DISPLAY=":$(cat "$artifact/display")"
export DISPLAY
for scale in 1 1.5 2; do
 WINIT_X11_SCALE_FACTOR=$scale "$binary" >"$artifact/$scale.log" 2>&1 & app=$!
 window=
 for ((attempt=0; attempt<100; attempt++)); do window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true); [[ -n "$window" ]] && break; kill -0 "$app"; sleep .1; done
 [[ -n "$window" ]]
 sleep 3
 xdotool mousemove --window "$window" 10 10
 magick import -window "$window" "$artifact/$scale-before.png"
 # Native pointer lies within the image at every scale for the default window.
 xdotool mousemove --window "$window" 400 350 click --repeat 5 --delay 100 5
 xdotool mousemove --window "$window" 10 10
 sleep 2
 magick import -window "$window" "$artifact/$scale-after.png"
 # Compare a fixed interior document region, away from the cursor and native UI.
 # The title is present before scrolling and leaves this region afterwards.
 read -r x y w h < <(awk -v s="$scale" 'BEGIN {printf "%d %d %d %d\n", 175*s, 125*s, 250*s, 55*s}')
 for phase in before after; do
  magick "$artifact/$scale-$phase.png" -crop "${w}x${h}+${x}+${y}" +repage "$artifact/$scale-content-$phase.png"
 done
 difference=$(magick compare -metric AE "$artifact/$scale-content-before.png" "$artifact/$scale-content-after.png" null: 2>&1 || true)
 printf '%s\n' "$difference" > "$artifact/$scale-difference.txt"
 if ! awk -v value="$difference" 'BEGIN { exit !(value + 0 > 100) }'; then echo "Document region did not change at $scale" >&2; exit 1; fi
 read -r width height < <(magick identify -format '%w %h\n' "$artifact/$scale-before.png")
 (( width < 2400 && height < 1800 )) || { echo "Window clipped at $scale" >&2; exit 1; }
 kill "$app"; wait "$app" || true; app=
done
