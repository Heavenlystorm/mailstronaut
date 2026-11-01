#!/usr/bin/env bash
# Sourced by the private native mailbox harness; every message is synthetic.
set -euo pipefail
: "${root:?private fixture root}" "${config:?native configuration}" "${app:?primary application}" "${binary:?prebuilt application}"
trap 'printf "Single-instance acceptance failed at line %s: %s\n" "$LINENO" "$root" >&2' ERR
original_config=$(sha256sum "$config")
owner() {
  dbus-send --session --print-reply --dest=org.freedesktop.DBus \
    /org/freedesktop/DBus org.freedesktop.DBus.GetConnectionUnixProcessID \
    string:io.github.eduardz1.Mailstronaut | python3 -c 'import re,sys; print(re.search(r"uint32 (\d+)",sys.stdin.read())[1])'
}
window_count() {
  swaymsg -t get_tree | python3 -c '
import json,sys
def windows(n):
    return int(n.get("app_id")=="io.github.eduardz1.Mailstronaut") + sum(windows(c) for c in n.get("nodes",[])+n.get("floating_nodes",[]))
assert windows(json.load(sys.stdin))==1
'
}
[[ $(owner) == "$app" ]]
window_count
# Selecting a row is deliberately passive. Raising its window must not open it.
wtype -M ctrl l -m ctrl -k Down -s 300
check_seen
for attempt in {1..8}; do
  # A forwarding process does not need a display or create another GPU/platform.
  env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET timeout 15 "$binary" --config "$config" \
    > "$root/secondary-$attempt.log" 2>&1
done
[[ $(owner) == "$app" ]]
window_count
check_seen
printf '%s\n' 'Repeated display-independent launches returned to the same owner and one native window; Seen stayed unchanged.'

# HM-managed source links can change generations while the first process runs.
# The declared location remains the same source; activation must not follow the
# link target to manufacture a different instance identity. Contents stay equal.
cp "$config" "$root/generation-a.nix"
cp "$config" "$root/generation-b.nix"
ln -sfn "$root/generation-a.nix" "$config"
timeout 15 "$binary" --config "$config" > "$root/generation-a.log" 2>&1
ln -sfn "$root/generation-b.nix" "$config"
timeout 15 "$binary" --config "$config" > "$root/generation-b.log" 2>&1
[[ $(owner) == "$app" ]]
window_count
check_seen
printf '%s\n' 'Replacing the declared configuration symlink retained the same application identity.'

cp "$config" "$root/other-config.nix"
if timeout 15 "$binary" --config "$root/other-config.nix" > "$root/conflict.log" 2>&1; then
  printf '%s\n' 'A conflicting configuration was silently accepted.' >&2; exit 1
fi
grep -q 'configuration differs' "$root/conflict.log"
if timeout 15 "$binary" --demo > "$root/demo-conflict.log" 2>&1; then
  printf '%s\n' 'A conflicting demo instance was started.' >&2; exit 1
fi
if DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/missing" timeout 15 "$binary" --config "$config" > "$root/no-bus.log" 2>&1; then
  printf '%s\n' 'An application started without session ownership.' >&2; exit 1
fi
grep -q 'session bus is unavailable' "$root/no-bus.log"
[[ $(owner) == "$app" ]]
window_count
check_seen

# Hide the existing window on another workspace, then exercise the public GPUI
# Wayland activation request. Focus remains subject to compositor policy.
click 640 60
swaymsg workspace number 2 > /dev/null
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET timeout 15 "$binary" --config "$config" > "$root/raise.log" 2>&1
sleep .5
swaymsg -t get_workspaces > "$root/activation-workspaces.json"
python3 - "$root/activation-workspaces.json" <<'PYFOCUS'
import json,sys
focused=next(w['num'] for w in json.load(open(sys.argv[1])) if w['focused'])
assert focused in [1,2],focused
print(f'Existing-window activation requested; private Sway focused workspace {focused} (1=app, 2=other). Fresh caller-token focus acceptance remains open.',flush=True)
PYFOCUS
# Positive control for the focus observer. This is an explicit test-compositor
# action, not an application workaround or proof that the request took focus.
swaymsg workspace number 1 > /dev/null
swaymsg -t get_workspaces | python3 -c 'import json,sys; assert next(w["num"] for w in json.load(sys.stdin) if w["focused"])==1'
capture instance-owned
check_seen

swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'The last-window-close did not exit.' >&2; exit 1; fi
wait "$app"; app=''
start offline
[[ $(owner) == "$app" ]]
window_count
check_seen
capture instance-restarted
# A lost session bus must stop this owner and its window-owned work. It cannot
# quietly survive outside the ownership namespace used by subsequent launches.
kill "$bus"; wait "$bus" 2>/dev/null || true; bus=''
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'The application survived loss of session ownership.' >&2; exit 1; fi
wait "$app"; app=''
grep -q 'session bus closed' "$root/offline-app.log"
[[ $(sha256sum "$config") == "$original_config" ]]
python3 - "$root" <<'PYLOGS'
from pathlib import Path
import sys
for name in ['initial-app.log','offline-app.log']:
    text=(Path(sys.argv[1])/name).read_text().lower()
    assert 'panicked at' not in text and 'fatal error' not in text,name
PYLOGS
printf 'Native ownership, configuration/demo conflicts, missing bus, passive activation routing, normal close/restart and bus-loss shutdown passed: %s\n' "$root"
