#!/usr/bin/env bash
# A prebuilt ignored test on a private bus; no real file grants or mail.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
binary=$(realpath "${1:?prebuilt application unit-test executable}")
mkdir -p target/p07-attachments
root=$(mktemp -d "$PWD/target/p07-attachments/portal.XXXXXXXX")
export MAILSTRONAUT_ATTACHMENT_TEST_ROOT="$root"
export XDG_DATA_HOME="$root/data" XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache" XDG_RUNTIME_DIR="$root/runtime"
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
unset DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:path=$root/runtime/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
cleanup() { kill "$bus" 2>/dev/null || true; wait "$bus" 2>/dev/null || true; }
trap cleanup EXIT
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
for _ in {1..100}; do [[ -S "$root/runtime/bus" ]] && break; sleep .05; done
"$binary" --exact composer::attachments::portal_tests::private_portal_grants_cancel_invalid_uris_and_failed_copies --ignored --nocapture | tee "$root/test.log"
printf 'Private attachment portal artifacts: %s\n' "$root"
