#!/usr/bin/env bash
# Real application D-Bus interfaces; private activation-free bus and synthetic data.
set -euo pipefail
fixture=$(realpath "${MAILSTRONAUT_INSTANCE_FIXTURE:?prebuilt application test executable}")
root=$(mktemp -d "$PWD/target/p08-instance.XXXXXX")
mkdir -m 700 "$root/runtime"
touch "$root/synthetic-instance"
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type><listen>unix:path=$root/runtime/bus</listen><auth>EXTERNAL</auth>
  <policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy>
</busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
cleanup() { kill "$bus" 2>/dev/null || true; wait "$bus" 2>/dev/null || true; }
trap cleanup EXIT
for _ in {1..100}; do [[ -S $root/runtime/bus ]] && break; kill -0 "$bus"; sleep .05; done
[[ -S $root/runtime/bus ]]
MAILSTRONAUT_INSTANCE_ROOT="$root" DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus" \
  timeout 20 "$fixture" instance::tests::private_ownership_activation_conflicts_shutdown_and_timeout --ignored --exact --nocapture
printf 'Private single-instance ownership and activation passed: %s\n' "$root"
