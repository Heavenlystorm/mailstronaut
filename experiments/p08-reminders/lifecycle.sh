#!/usr/bin/env bash
# A prebuilt test-only GPUI lifecycle probe in an isolated native Wayland session.
set -euo pipefail
fixture=$(realpath "${MAILSTRONAUT_LIFECYCLE_FIXTURE:?prebuilt application test executable}")
root=$(mktemp -d "$PWD/target/p08-lifecycle.XXXXXX")
export HOME="$root/home" XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
touch "$root/synthetic-lifecycle"
unset DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type><listen>unix:path=$root/runtime/bus</listen><auth>EXTERNAL</auth>
  <policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy>
</busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
export MAILSTRONAUT_LIFECYCLE_ROOT="$root" MAILSTRONAUT_LIFECYCLE_FIXTURE="$fixture"
for _ in {1..100}; do [[ -S $root/runtime/bus ]] && break; kill -0 "$bus"; sleep .05; done
binary="$root/fixture-runner"
cat > "$binary" <<'RUNNER'
#!/usr/bin/env bash
exec "$MAILSTRONAUT_LIFECYCLE_FIXTURE" lifecycle_tests::native_headless_attach_close_detach_reopen --exact --ignored --nocapture
RUNNER
chmod 700 "$binary"
config="$root/unused.nix" artifacts="$root"
cleanup() { session_cleanup; kill "$bus" 2>/dev/null || true; wait "$bus" 2>/dev/null || true; }
trap cleanup EXIT
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
wait_for() {
  for _ in {1..200}; do [[ -f $root/$1 ]] && return 0; kill -0 "$app"; sleep .05; done
  printf 'Lifecycle did not reach %s: %s\n' "$1" "$root" >&2; return 1
}
start first
wait_for attached-1
capture first-window
swaymsg -t get_tree > "$root/first-tree.json"
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
wait_for headless-1
kill -0 "$app"
if window_open; then printf '%s\n' 'A hidden window survived detachment.' >&2; exit 1; fi
wait_for heartbeat
before=$(cat "$root/heartbeat")
sleep .2
after=$(cat "$root/heartbeat")
(( after > before ))
touch "$root/reopen"
wait_for attached-2
for _ in {1..100}; do window_open && break; kill -0 "$app"; sleep .05; done
window_open
capture second-window
swaymsg -t get_tree > "$root/second-tree.json"
python3 - "$root" "$app" <<'PYWINDOWS'
import json,sys
from pathlib import Path
root=Path(sys.argv[1]); pid=int(sys.argv[2])
def windows(n):
    found=[n] if n.get('app_id')=='io.github.eduardz1.Mailstronaut' else []
    return found+sum((windows(c) for c in n.get('nodes',[])+n.get('floating_nodes',[])),[])
first=windows(json.loads((root/'first-tree.json').read_text()))
second=windows(json.loads((root/'second-tree.json').read_text()))
assert len(first)==len(second)==1
assert first[0]['pid']==second[0]['pid']==pid
assert first[0]['id']!=second[0]['id']
print('Two distinct compositor windows belong to the same retained process.',flush=True)
PYWINDOWS
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
wait_for passed
wait "$app"; app=''
printf 'Native headless attach/detach feasibility passed: %s\n' "$root"
