#!/usr/bin/env bash
# Run through nix develop .#gui-test, against a prebuilt current application.
set -euo pipefail
binary=$(realpath "${1:-target/debug/mailstronaut}")
root=$(mktemp -d "$PWD/target/gpui-wayland.XXXXXX")
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET
if DISPLAY=:9876 "$binary" --demo > "$root/no-wayland.log" 2>&1; then
  printf '%s\n' 'Application incorrectly accepted an X11-only session' >&2
  exit 1
fi
grep -q 'requires a native Wayland session' "$root/no-wayland.log"
export WAYLAND_DISPLAY=mailstronaut-test
export VK_DRIVER_FILES="${MAILSTRONAUT_TEST_VULKAN_ICD:?pinned software Vulkan driver}"
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
compositor='' app='' bus=''
cleanup() {
  for pid in "$app" "$compositor" "$bus"; do
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
# A standard headless compositor supplies the seat GPUI requires. XWayland is disabled.
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
  [[ -n "$socket" ]] && break
  kill -0 "$compositor"; sleep .05
done
[[ -S $socket ]]
export WAYLAND_DISPLAY="$socket"
WAYLAND_DEBUG=client "$binary" --demo > "$root/client.log" 2>&1 & app=$!
for _ in {1..300}; do
  grep -q 'wl_surface.*commit' "$root/client.log" && break
  kill -0 "$app"; sleep .1
done
grep -q 'xdg_toplevel.*set_app_id.*io.github.eduardz1.Mailstronaut' "$root/client.log"
grep -q 'wl_surface.*commit' "$root/client.log"
# Allow the off-thread HTML fixture and subsequent frame to finish.
sleep 3
kill -0 "$app"
timeout 15 grim "$root/window.png" > "$root/capture.log" 2>&1
[[ -s $root/window.png ]]
# Keep the virtual keyboard alive while GPUI binds its seat and input objects.
wtype -s 400 -M ctrl k -m ctrl -s 200 synthetic-query -s 300
timeout 15 grim "$root/search.png"
wtype -s 400 -M ctrl -M shift d -m shift -m ctrl -s 2000
timeout 15 grim "$root/dark.png"
[[ -s $root/search.png && -s $root/dark.png ]]
# Verify actual appearance changes, rather than treating input-command exit as proof.
[[ $(magick "$root/window.png" -format '%[fx:p{10,80}.r>0.5]' info:) == 1 ]]
[[ $(magick "$root/dark.png" -format '%[fx:p{10,80}.r<0.5]' info:) == 1 ]]
[[ $(magick "$root/dark.png" -format '%[fx:p{700,550}.r<0.5]' info:) == 1 ]]
# Exercise the standard native text reader and the Wayland clipboard. This
# profile contains only synthetic mail; never capture a real user's message.
printf 'clipboard observer control' | wl-copy
[[ $(timeout 5 wl-paste --no-newline) == 'clipboard observer control' ]]
wtype -s 400 -M ctrl -M shift t -m shift -m ctrl -s 300 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 300
timeout 5 wl-paste --no-newline > "$root/reader-all.txt"
grep -q 'Synthetic reader line 001' "$root/reader-all.txt"
grep -q 'Synthetic reader line 120' "$root/reader-all.txt"
grep -q 'Bonjour · Ciao · 世界' "$root/reader-all.txt"
# With all text selected, typing, deletion and paste must leave the body intact.
wtype -s 400 MUTATION -k BackSpace -M ctrl v -m ctrl -s 200
printf 'edit copy observer control' | wl-copy
wtype -s 400 -M ctrl a -m ctrl -M ctrl c -m ctrl -s 300
timeout 5 wl-paste --no-newline > "$root/reader-after-edit.txt"
cmp "$root/reader-all.txt" "$root/reader-after-edit.txt"
# Document-end navigation must scroll to, select and copy the final Unicode line.
wtype -s 400 -M ctrl -k End -m ctrl -M shift -k Home -m shift -M ctrl c -m ctrl -s 300
timeout 5 wl-paste --no-newline > "$root/reader-end.txt"
[[ $(cat "$root/reader-end.txt") == 'End of synthetic message · fin · 世界.' ]]
timeout 15 grim "$root/reader-end.png"
# Theme changes retain reader selection and never reopen a message.
wtype -s 400 -M ctrl -M shift d -m shift -m ctrl -s 1000
printf 'theme copy observer control' | wl-copy
wtype -s 400 -M ctrl c -m ctrl -s 300
timeout 5 wl-paste --no-newline > "$root/reader-after-theme.txt"
cmp "$root/reader-end.txt" "$root/reader-after-theme.txt"
if grep -Eiq 'panic|error:|fatal' "$root/client.log"; then
  printf '%s\n' 'Application reported a runtime failure; inspect client.log' >&2
  exit 1
fi
printf 'Native Wayland window, Blitz preview and read-only Unicode reader passed: %s\n' "$root"
