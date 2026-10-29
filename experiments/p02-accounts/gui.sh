#!/usr/bin/env bash
# Run only as the optional GUI portion of credentials.sh's private-bus fixture.
set -euo pipefail
binary=$(realpath "${1:?prebuilt binary}")
root=$(realpath "${2:?private keyring fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus"* ]]
[[ $XDG_CONFIG_HOME == "$root/config" ]]
mkdir -p "$root/gui"
export GSETTINGS_BACKEND=memory LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl
export XDG_SESSION_TYPE=x11 WINIT_X11_SCALE_FACTOR=1
unset WAYLAND_DISPLAY
app='' xvfb=''
cleanup() {
  for pid in "$app" "$xvfb"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
Xvfb -displayfd 3 -screen 0 1280x900x24 -nolisten tcp 3>"$root/gui/display" >"$root/gui/xvfb.log" 2>&1 &
xvfb=$!
for _ in {1..100}; do [[ -s "$root/gui/display" ]] && break; sleep .05; done
DISPLAY=":$(cat "$root/gui/display")"
export DISPLAY
mode="$XDG_CONFIG_HOME/cosmic/com.system76.CosmicTheme.Mode/v1"
mkdir -p "$mode"
printf false > "$mode/is_dark"
printf false > "$mode/auto_switch"
start() {
  "$binary" --config "$root/gui-config.nix" > "$root/gui/$1-app.log" 2>&1 & app=$!
  window=''
  for _ in {1..100}; do
    window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | head -n1 || true)
    [[ -n "$window" ]] && break
    kill -0 "$app"
    sleep .1
  done
  [[ -n "$window" ]]
  xdotool windowfocus "$window"
  sleep 1
}
stop() { kill "$app"; wait "$app" 2>/dev/null || true; app=''; }
capture() { xdotool mousemove --window "$window" 5 5; sleep .2; magick import -window "$window" "$root/gui/$1.png"; }
start missing
capture welcome
stop
printf '{ schemaVersion = 999; }\n' > "$root/gui-config.nix"
start invalid-startup
capture invalid-startup
stop
cat > "$root/gui-config.nix" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  accounts = [{
    id = "gui-synthetic";
    provider = "icloud";
    address = "gui@example.test";
    displayName = "GUI synthetic mailbox";
    userName = "gui";
    auth = { kind = "password"; secretRef = "gui-password"; };
  }];
}
NIX
cp "$root/gui-config.nix" "$root/gui/original-config.nix"
start configured
capture configured
xdotool mousemove --window "$window" 320 242 click 1
sleep .2
capture selected
xdotool mousemove --window "$window" 240 362 click 1
sleep .5
capture unlocked
xdotool mousemove --window "$window" 400 408 click 1
xdotool type --clearmodifiers 'synthetic-gui-password'
capture masked
printf 'private clipboard sentinel' | xclip -selection clipboard -in
xdotool key --clearmodifiers ctrl+a ctrl+c
timeout 5 xclip -selection clipboard -out | python3 -c 'import sys; assert sys.stdin.read() == "private clipboard sentinel", "secure field copied its contents"'
xdotool mousemove --window "$window" 240 454 click 1
sleep .5
capture saved
application=$(cat assets/app-id)
timeout 5 secret-tool lookup application "$application" account gui-synthetic reference gui-password purpose password |
  python3 -c 'import sys; assert sys.stdin.read().removesuffix("\n") == "synthetic-gui-password", "GUI credential did not round-trip"'
cmp "$root/gui-config.nix" "$root/gui/original-config.nix"
printf '{ schemaVersion = 999; }\n' > "$root/gui-config.nix"
xdotool mousemove --window "$window" 240 286 click 1
sleep .5
capture invalid-reload
cp "$root/gui/original-config.nix" "$root/gui-config.nix"
xdotool mousemove --window "$window" 240 318 click 1
sleep .5
capture repaired-reload
if rg -F 'synthetic-gui-password' "$root/gui/"*-app.log; then
  printf '%s\n' 'GUI log exposed a synthetic password marker' >&2
  exit 1
fi
printf '%s\n' 'GUI password save/readback, secure clipboard, unchanged managed source and reload walkthrough passed.'
printf 'Account GUI window: %s\nArtifacts: %s/gui\n' "$window" "$root"
stop
cmp "$root/gui-config.nix" "$root/gui/original-config.nix"
