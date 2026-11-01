#!/usr/bin/env bash
# Sourced by native acceptance scripts: a private headless sway session driven by real
# keyboard and pointer input. Callers set `binary`, `config` and `artifacts`, keep a
# private bus in DBUS_SESSION_BUS_ADDRESS, and call `session_cleanup` from their trap.
: "${binary:?prebuilt application}" "${config:?native configuration}" "${artifacts:?artifact directory}"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export VK_DRIVER_FILES="${MAILSTRONAUT_TEST_VULKAN_ICD:?pinned software Vulkan driver}"
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
app='' keyboard='' pointer='' compositor=''
session_cleanup() {
  for pid in "$app" "$keyboard" "$pointer" "$compositor"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
cat > "$artifacts/sway.conf" <<'SWAY'
xwayland disable
output HEADLESS-1 mode 1280x840
seat seat0 fallback true
default_border none
SWAY
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway --unsupported-gpu --config "$artifacts/sway.conf" > "$artifacts/compositor.log" 2>&1 & compositor=$!
for _ in {1..200}; do
  socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'wayland-*' -print -quit)
  sway_socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)
  [[ -n "$socket" && -S $sway_socket ]] && break
  kill -0 "$compositor"; sleep .05
done
[[ -S $socket ]]
export WAYLAND_DISPLAY="$socket" SWAYSOCK="$sway_socket"
WLRCTL_TEST_KEEP_POINTER=1 wlrctl pointer move 0 0 > "$artifacts/pointer.log" 2>&1 & pointer=$!
sleep .5
window_open() {
  swaymsg -t get_tree | python3 -c 'import json,sys; t=json.load(sys.stdin); walk=lambda n: n.get("app_id")=="io.github.eduardz1.Mailstronaut" or any(walk(c) for c in n.get("nodes",[])); sys.exit(0 if walk(t) else 1)'
}
start() {
  # Retain the keyboard capability, and make a live keyed device current before
  # a reconnect. A destroyed one-shot virtual keyboard can leave this synthetic
  # seat without a keymap, causing Modifiers to precede Keymap for a new client.
  if [[ -n "$keyboard" ]]; then kill "$keyboard"; wait "$keyboard" 2>/dev/null || true; fi
  wtype -k Escape -s 300000 > "$artifacts/keyboard.log" 2>&1 & keyboard=$!
  sleep .3
  "$binary" --config "$config" > "$artifacts/$1-app.log" 2>&1 & app=$!
  for _ in {1..150}; do
    window_open && break
    kill -0 "$app"; sleep .1
  done
  sleep 1
  kill -0 "$app"
}
# Ask the window to close, as a desktop does, and require a normal exit.
close_window() {
  swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
  for _ in {1..100}; do
    kill -0 "$app" 2>/dev/null || break
    sleep .1
  done
  if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'Application failed to exit after closing' >&2; return 1; fi
  wait "$app"; app=''
}
capture() { timeout 15 grim "$artifacts/$1.png"; }
click() {
  swaymsg seat seat0 cursor set "$1" "$2" > /dev/null
  swaymsg seat seat0 cursor press button1 > /dev/null
  swaymsg seat seat0 cursor release button1 > /dev/null
  sleep .3
}
