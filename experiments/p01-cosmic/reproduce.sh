#!/usr/bin/env bash
# Run a prebuilt binary through nix develop .#gui-test --command bash this-file BINARY.
# Only private displays, config directories and stock pinned COSMIC theme assets.
set -euo pipefail
binary=$(realpath "${1:?pass a prebuilt Mailstronaut binary}")
artifact=${2:-$(mktemp -d /tmp/mailstronaut-cosmic-ui.XXXXXX)}
printf 'Artifacts: %s\n' "$artifact"
mkdir -p "$artifact"/{runtime,config,cache,data,state}
chmod 700 "$artifact/runtime"
export XDG_RUNTIME_DIR="$artifact/runtime"
export XDG_CONFIG_HOME="$artifact/config"
export XDG_CACHE_HOME="$artifact/cache"
export XDG_DATA_HOME="$artifact/data"
export XDG_STATE_HOME="$artifact/state"
export XDG_DATA_DIRS="${MAILSTRONAUT_TEST_THEME_DATA:?use the gui-test Nix shell}"
# Deliberately exercise COSMIC's filesystem watcher fallback. A connected D-Bus
# session without cosmic-settings-daemon does not provide working theme watches.
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/no-session-bus"
export LIBGL_ALWAYS_SOFTWARE=1
export WGPU_BACKEND=gl
export LC_ALL=C
unset WAYLAND_DISPLAY
mode="$XDG_CONFIG_HOME/cosmic/com.system76.CosmicTheme.Mode/v1"
mkdir -p "$mode"
printf true > "$mode/is_dark"
printf false > "$mode/auto_switch"
app_pid=
xvfb_pid=
weston_pid=
cleanup() {
  for pid in "$app_pid" "$weston_pid" "$xvfb_pid"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT

# Assert a background pixel well inside the fixed synthetic viewport, rather
# than accepting a process that is alive or a screenshot of an empty window.
check_pixel() {
  local value
  value=$(magick "$1" -format '%[fx:p{700,350}.r+p{700,350}.g+p{700,350}.b]' info:)
  python3 - "$value" "$2" <<'PY'
import sys
value = float(sys.argv[1])
assert {'dark': value < .1, 'light': value > 2.9, 'native': .1 < value < 2.9}[sys.argv[2]], (value, sys.argv[2])
PY
}

# Sample this process, including all its threads, in five-second idle intervals.
# Absolute PSS is not reader-incremental memory or an engine-free baseline.
sample_idle() {
  python3 - "$app_pid" "$1" "$2" <<'PY'
import json, os, pathlib, statistics, sys, time
proc = pathlib.Path('/proc') / sys.argv[1]
ticks_per_second = os.sysconf('SC_CLK_TCK')
def sample():
    fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
    return time.monotonic(), int(fields[11]) + int(fields[12])
def memory():
    return int(next(line.split()[1] for line in (proc / 'smaps_rollup').read_text().splitlines() if line.startswith('Pss:')))
start = previous = sample()
percentages = []
pss = [memory()]
for _ in range(int(sys.argv[3])):
    time.sleep(5)
    current = sample()
    percentages.append((current[1] - previous[1]) / ticks_per_second / (current[0] - previous[0]) * 100)
    pss.append(memory())
    previous = current
pathlib.Path(sys.argv[2]).write_text(json.dumps({
    'seconds': previous[0] - start[0], 'percent_of_one_cpu': percentages,
    'median_cpu_percent': statistics.median(percentages), 'pss_kib': pss,
}, indent=2) + '\n')
PY
}

export XDG_SESSION_TYPE=x11
Xvfb -displayfd 3 -screen 0 1200x900x24 -nolisten tcp 3>"$artifact/display" >"$artifact/xvfb.log" 2>&1 &
xvfb_pid=$!
for ((attempt=0;attempt<100;attempt++)); do
  [[ -s "$artifact/display" ]] && break
  sleep .1
done
DISPLAY=":$(cat "$artifact/display")"
export DISPLAY
"$binary" >"$artifact/x11.log" 2>&1 &
app_pid=$!
for ((attempt=0;attempt<100;attempt++)); do
  window=$(xdotool search --onlyvisible --pid "$app_pid" 2>/dev/null | head -n1 || true)
  [[ -n "$window" ]] && break
  kill -0 "$app_pid"
  sleep .1
done
[[ -n "$window" ]]
sleep 2
magick import -window "$window" "$artifact/x11-dark.png"
check_pixel "$artifact/x11-dark.png" dark
printf false > "$mode/is_dark"
sleep 2
magick import -window "$window" "$artifact/x11-light.png"
check_pixel "$artifact/x11-light.png" light
xdotool mousemove --window "$window" 55 167 click 1
sleep .5
magick import -window "$window" "$artifact/x11-about.png"
check_pixel "$artifact/x11-about.png" native
sample_idle "$artifact/x11-about-idle.json" 3
xdotool mousemove --window "$window" 55 119 click 1
xdotool mousemove --window "$window" 5 5
sleep .5
magick import -window "$window" "$artifact/x11-return.png"
check_pixel "$artifact/x11-return.png" light
xdotool windowsize "$window" 800 620
sleep 1
magick import -window "$window" "$artifact/x11-resize.png"
[[ $(magick identify -format '%wx%h' "$artifact/x11-resize.png") == 800x620 ]]
check_pixel "$artifact/x11-resize.png" light

sample_idle "$artifact/x11-idle.json" 12
kill "$app_pid"
wait "$app_pid" || true
app_pid=

unset DISPLAY
export XDG_SESSION_TYPE=wayland
export WAYLAND_DISPLAY=mailstronaut-test
printf true > "$mode/is_dark"
# The debug capture protocol is exposed only on this private 0700 runtime dir.
weston --backend=headless --renderer=pixman --width=1024 --height=768 --shell=kiosk-shell.so --socket="$WAYLAND_DISPLAY" --idle-time=0 --no-config --debug --log="$artifact/weston.log" &
weston_pid=$!
for ((attempt=0;attempt<100;attempt++)); do
  [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
  kill -0 "$weston_pid"
  sleep .1
done
[[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]
"$binary" >"$artifact/wayland.log" 2>&1 &
app_pid=$!
for color in dark light; do
  if [[ "$color" == light ]]; then printf false > "$mode/is_dark"; fi
  sleep 2
  mkdir "$artifact/wayland-$color"
  (cd "$artifact/wayland-$color" && timeout 10 weston-screenshooter)
  captures=("$artifact/wayland-$color/"*.png)
  [[ ${#captures[@]} == 1 ]]
  check_pixel "${captures[0]}" "$color"
  kill -0 "$app_pid"
done
printf 'Synthetic window checks passed; artifacts: %s\n' "$artifact"
