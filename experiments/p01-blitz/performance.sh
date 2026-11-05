#!/usr/bin/env bash
# Synthetic native Wayland measurement of one long HTML original and 100 short ones: keyboard
# switching, explicit-open latency to the painted marker, resident memory and hidden idle CPU.
# No real account, keyring, network service or X server is used. The numbers describe this
# headless software-rendered session; they are not a power or desktop benchmark.
# Usage: performance.sh APP_BINARY WORKER_TEST_EXECUTABLE   (run inside nix develop .#gui-test)
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
fixture=$(realpath "${2:?prebuilt core worker test executable}")
root=$(mktemp -d "$PWD/target/p01-perf.XXXXXX")
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
artifacts="$root/performance"
mkdir -p "$artifacts"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
bus=''
cleanup() {
  session_cleanup
  if [[ -n "$bus" ]]; then kill "$bus" 2>/dev/null || true; wait "$bus" 2>/dev/null || true; fi
}
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
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
config="$root/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  # Closing the window keeps native services alive, so the hidden measurement runs that path.
  background = {enabled = true; autostart = false;};
  sync.automatic = false;
  accounts = map (identity: {
    id = identity.id;
    provider = "manual";
    address = identity.address;
    displayName = identity.id;
    userName = identity.id;
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 587; security = "starttls";};
    auth = {kind = "password"; secretRef = "synthetic-conversation-password";};
  }) [
    {id = "conversation-synthetic"; address = "reader@example.test";}
    {id = "other-synthetic"; address = "other@example.test";}
  ];
}
NIX
export MAILSTRONAUT_THREADS_CONFIG="$config" MAILSTRONAUT_THREADS_GUI_BINARY="$binary"
MAILSTRONAUT_PERFORMANCE_MAILBOX=1 "$fixture" seed_conversation_gui --ignored --exact --nocapture \
  > "$root/seed.log" 2>&1
[[ -s $root/data/mailstronaut/mail.sqlite3 ]]
# The shared helper owns the headless sway session, input and the application lifecycle.
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap cleanup EXIT
start performance
window_open || { printf '%s\n' 'The application did not open a window' >&2; exit 1; }
driver="$artifacts/driver.py"
cat > "$driver" <<'PYDRIVER'
"""Drive the native mailbox with keys, sample painted pixels, and write numbers only."""
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

artifacts = Path(os.environ["ARTIFACTS"])
app_pid = int(os.environ["APP_PID"])
clock_ticks = os.sysconf("SC_CLK_TCK")
switches = 100
timeout = 8.0


def ppm_pixels(data):
    # The header is "P6", width, height and maxval, each followed by whitespace.
    fields, offset = [], 0
    while len(fields) < 4:
        while data[offset:offset + 1].isspace():
            offset += 1
        start = offset
        while not data[offset:offset + 1].isspace():
            offset += 1
        fields.append(data[start:offset].decode())
    offset += 1
    assert fields[0] == "P6" and fields[3] == "255", fields
    return int(fields[1]), int(fields[2]), data[offset:]


def screen():
    return ppm_pixels(subprocess.run(["grim", "-t", "ppm", "-"], check=True, capture_output=True).stdout)


def pixel(x, y):
    data = subprocess.run(
        ["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], check=True, capture_output=True
    ).stdout
    _, _, raw = ppm_pixels(data)
    return tuple(raw[0:3])


def timed_pixel(x=0, y=0):
    # The cost of one sample is the floor under every latency reported below.
    started = time.perf_counter()
    pixel(x, y)
    return (time.perf_counter() - started) * 1000


def key(name):
    # wtype sends a keysym only with -k; a bare word would be typed as text.
    subprocess.run(["wtype", "-k", name], check=True)


def marker(index):
    # The fixture's colour for original 'index'; the long original is index 0.
    return (16 + (index % 16) * 12, 16 + ((index // 16) % 16) * 12, 16 + (index // 256) * 100)


def rss_kib():
    for line in Path(f"/proc/{app_pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    raise RuntimeError("no VmRSS")


def cpu_seconds():
    fields = Path(f"/proc/{app_pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / clock_ticks


def locate(color):
    width, height, raw = screen()
    xs, ys, hits = [], [], 0
    for index in range(0, len(raw) - 2, 3):
        if raw[index] == color[0] and raw[index + 1] == color[1] and raw[index + 2] == color[2]:
            pixel_index = index // 3
            xs.append(pixel_index % width)
            ys.append(pixel_index // width)
            hits += 1
    if hits < 100:
        return None
    return min(xs), min(ys), max(xs), max(ys), hits


def wait_for(color, point):
    start = time.perf_counter()
    while time.perf_counter() - start < timeout:
        if pixel(*point) == color:
            return (time.perf_counter() - start) * 1000
        time.sleep(0.01)
    return None


# Calibrate once: focus the message list (Ctrl+L), open the long original, and find its block.
subprocess.run(["wtype", "-M", "ctrl", "-k", "l", "-m", "ctrl"], check=True)
time.sleep(0.5)
key("Down")
time.sleep(0.2)
key("Return")
time.sleep(1.5)
box = locate(marker(0))
if box is None:
    raise SystemExit("calibration failed: the long original's block is not visible in the reader")
x0, y0, x1, y1, hits = box
point = ((x0 + x1) // 2, y0 + 12)
(artifacts / "calibration.json").write_text(json.dumps({"box": box, "point": point}))

# Calibration already painted the long original. Pixel sampling costs set the latency floor.
latencies = []
wrong = []
rss = [{"after": "open-long", "kib": rss_kib()}]
overhead = sorted(timed_pixel(point[0], point[1]) for _ in range(30))
switch_sequence = [index for index in range(1, switches + 1)]
for step, index in enumerate(switch_sequence, start=1):
    key("Down")
    time.sleep(0.05)
    started = time.perf_counter()
    key("Return")
    ms = wait_for(marker(index), point)
    if ms is None:
        wrong.append({"index": index, "painted": list(pixel(*point))})
    else:
        latencies.append(ms)
    if step % 10 == 0:
        rss.append({"after": f"{step}-switches", "kib": rss_kib()})
        time.sleep(0.2)

# Opening the long original from a short neighbour: from index 100 back to the long original,
# then five rounds of short -> long, each measured from the explicit open to the long marker.
long_latencies = []
key_sequence = [argument for _ in range(100) for argument in ("-k", "Up")]
subprocess.run(["wtype", *key_sequence], check=True)
time.sleep(0.3)
key("Return")
if wait_for(marker(0), point) is None:
    raise SystemExit("the long original did not repaint after returning to it")
for _ in range(5):
    key("Down")
    time.sleep(0.05)
    key("Return")
    if wait_for(marker(1), point) is None:
        wrong.append({"round": "short", "painted": list(pixel(*point))})
    key("Up")
    time.sleep(0.05)
    key("Return")
    ms = wait_for(marker(0), point)
    if ms is None:
        wrong.append({"round": "long", "painted": list(pixel(*point))})
    else:
        long_latencies.append(ms)
long_latencies.sort()

latencies.sort()
def percentile(values, fraction):
    return values[min(len(values) - 1, int(fraction * len(values)))]
summary = {
    "switches": len(switch_sequence),
    "wrong_paints": wrong,
    "latency_ms": {
        "count": len(latencies),
        "median": statistics.median(latencies),
        "p95": percentile(latencies, 0.95),
        "max": latencies[-1],
    },
    "long_original_open_ms": {
        "count": len(long_latencies),
        "median": statistics.median(long_latencies),
        "max": long_latencies[-1],
    },
    "pixel_sample_overhead_ms": {
        "median": statistics.median(overhead),
        "p95": percentile(overhead, 0.95),
    },
    "resident_kib": rss,
}

# rss[0] is after the first open; rss[1] is after ten switches.
summary["resident_growth_kib_from_10_to_100_switches"] = rss[-1]["kib"] - rss[1]["kib"]

# Hidden lifetime: closing the window must leave no painting work behind the process.
subprocess.run(["swaymsg", '[app_id="io.github.eduardz1.Mailstronaut"] kill'], check=True, capture_output=True)
time.sleep(2)
still_running = Path(f"/proc/{app_pid}").exists()
summary["hidden"] = {"process_running_after_close": still_running}
if still_running:
    before = cpu_seconds()
    time.sleep(30)
    after = cpu_seconds()
    summary["hidden"]["idle_cpu_seconds_over_30s"] = round(after - before, 3)
    summary["hidden"]["resident_kib_after_close"] = rss_kib()
(artifacts / "performance.json").write_text(json.dumps(summary, indent=2))
print(json.dumps(summary, indent=2))
PYDRIVER
ARTIFACTS="$artifacts" APP_PID="$app" python3 "$driver" > "$artifacts/driver.log" 2>&1 || {
  cat "$artifacts/driver.log" >&2
  printf 'Performance driver failed: %s\n' "$root" >&2
  exit 1
}
cat "$artifacts/driver.log"
printf 'Native performance measurement: %s\n' "$artifacts"
