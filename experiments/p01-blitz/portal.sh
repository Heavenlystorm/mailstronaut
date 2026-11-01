#!/usr/bin/env bash
# Real ASHPD calls on an activation-free private bus; no browser or network.
set -euo pipefail
fixture=$(realpath "${MAILSTRONAUT_LINK_PORTAL_FIXTURE:?prebuilt application test executable}")
root=$(mktemp -d)
mkdir -m 700 "$root/runtime"
touch "$root/synthetic-link-portal"
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
MAILSTRONAUT_LINK_PORTAL_ROOT="$root" DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus" \
  timeout 20 "$fixture" link_portal::tests::handoff_is_explicit_bounded_cancellable_and_redacted --ignored --exact --nocapture
printf 'Private web-link portal transport passed: %s\n' "$root"
