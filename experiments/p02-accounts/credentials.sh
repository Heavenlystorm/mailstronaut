#!/usr/bin/env bash
# Explicit, synthetic-only Secret Service acceptance in private project directories.
# Run: nix develop .#gui-test --command bash experiments/p02-accounts/credentials.sh
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
mkdir -p target/p02-accounts
# Keep Unix-domain socket paths short enough for the private headless compositor.
root=$(realpath "$(mktemp -d "$PWD/target/p02.XXXXXX")")
export MAILSTRONAUT_SECRET_TEST_ROOT="$root"
export XDG_DATA_HOME="$root/data" XDG_CONFIG_HOME="$root/config"
export XDG_CACHE_HOME="$root/cache" XDG_RUNTIME_DIR="$root/runtime"
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
unset DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS DISPLAY WAYLAND_DISPLAY
bus_pid='' keyring_pid=''
cleanup() {
  if [[ -n "$keyring_pid" ]]; then kill "$keyring_pid" 2>/dev/null || true; wait "$keyring_pid" 2>/dev/null || true; fi
  if [[ -n "$bus_pid" ]]; then kill "$bus_pid" 2>/dev/null || true; wait "$bus_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
# The bus path itself is checked by the ignored Rust acceptance test.
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=$root/runtime/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
  </policy>
</busconfig>
CONFIG
# No service directories: probing readiness cannot auto-start another daemon and
# race the explicitly unlocked fixture against a second keyring process.
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 &
bus_pid=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
for _ in {1..100}; do [[ -S "$root/runtime/bus" ]] && break; sleep 0.05; done
# This public synthetic fixture value is not a user credential. The daemon writes
# only under the private XDG data root; the user's bus/keyring is never contacted.
printf '%s' 'mailstronaut-synthetic-fixture-only' | gnome-keyring-daemon --foreground --unlock --components=secrets --control-directory="$root/runtime/control" > "$root/keyring.log" 2>&1 &
keyring_pid=$!
for _ in {1..100}; do
  if dbus-send --session --dest=org.freedesktop.secrets --type=method_call --print-reply /org/freedesktop/secrets org.freedesktop.DBus.Peer.Ping > /dev/null 2>&1; then break; fi
  sleep 0.05
done
if [[ -n ${MAILSTRONAUT_ACCOUNT_GUI_BINARY:-} ]]; then
  bash experiments/p02-accounts/gui.sh "$MAILSTRONAUT_ACCOUNT_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_INBOX_GUI_BINARY:-} ]]; then
  bash experiments/p04-inbox/gui.sh "$MAILSTRONAUT_INBOX_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_THREADS_GUI_BINARY:-} ]]; then
  bash experiments/p06-conversations/gui.sh "$MAILSTRONAUT_THREADS_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_ONBOARDING_AUTH_GUI_BINARY:-} ]]; then
  bash experiments/p05-onboarding/auth-gui.sh "$MAILSTRONAUT_ONBOARDING_AUTH_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_SYNC_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/gui.sh "$MAILSTRONAUT_SYNC_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_IDLE_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/idle.sh "$MAILSTRONAUT_IDLE_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_DELIVERY_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/delivery.sh "$MAILSTRONAUT_DELIVERY_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_SEND_FAILURES_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/failures.sh "$MAILSTRONAUT_SEND_FAILURES_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_SENT_COPY_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/sent-copy.sh "$MAILSTRONAUT_SENT_COPY_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_DISCARD_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/discard.sh "$MAILSTRONAUT_DISCARD_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_ATTACH_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/attachments.sh "$MAILSTRONAUT_ATTACH_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_IDENTITY_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/identity.sh "$MAILSTRONAUT_IDENTITY_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_FORWARD_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/forward.sh "$MAILSTRONAUT_FORWARD_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_FLATPAK_GUI_BINARY:-} ]]; then
  bash flatpak/gui-smoke.sh "$MAILSTRONAUT_FLATPAK_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_FLATPAK_PORTAL_BINARY:-} ]]; then
  bash flatpak/portal-reach.sh "$MAILSTRONAUT_FLATPAK_PORTAL_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_SEARCH_GUI_BINARY:-} ]]; then
  bash experiments/p06-search/gui.sh "$MAILSTRONAUT_SEARCH_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_PRIORITY_GUI_BINARY:-} ]]; then
  bash experiments/p09-priority/gui.sh "$MAILSTRONAUT_PRIORITY_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_LARGE_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/large.sh "$MAILSTRONAUT_LARGE_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_OLDER_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/older.sh "$MAILSTRONAUT_OLDER_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_HISTORY_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/history.sh "$MAILSTRONAUT_HISTORY_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_FAULTS_GUI_BINARY:-} ]]; then
  bash experiments/p04-sync/faults.sh "$MAILSTRONAUT_FAULTS_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_ACTIONS_GUI_BINARY:-} ]]; then
  for variant in full no-move neither; do
    bash experiments/p07-compose/actions.sh "$MAILSTRONAUT_ACTIONS_GUI_BINARY" "$root" "$variant"
  done
fi
if [[ -n ${MAILSTRONAUT_NOTIFY_GUI_BINARY:-} ]]; then
  for variant in private preview disabled; do
    bash experiments/p04-sync/notify.sh "$MAILSTRONAUT_NOTIFY_GUI_BINARY" "$root" "$variant"
  done
fi
if [[ -n ${MAILSTRONAUT_COMPOSE_GUI_BINARY:-} ]]; then
  bash experiments/p07-compose/gui.sh "$MAILSTRONAUT_COMPOSE_GUI_BINARY" "$root"
fi
if [[ -n ${MAILSTRONAUT_ACCOUNT_ACTION_FIXTURE:-} ]]; then
  "$(realpath "$MAILSTRONAUT_ACCOUNT_ACTION_FIXTURE")" account_actions::tests::private_cancellation_and_validation_never_unlock --exact --ignored --nocapture | tee "$root/account-actions.log"
fi
cargo_flags=(--locked --offline)
if [[ ${MAILSTRONAUT_CREDENTIAL_TEST_PROFILE:-debug} == release ]]; then
  cargo_flags+=(--release)
fi
# GUI shells add native inputs that invalidate Cargo build-script fingerprints.
# Build in the normal shell first, then execute that fixture here without Cargo.
if [[ -n ${MAILSTRONAUT_CREDENTIAL_FIXTURE:-} ]]; then
  credential_tests=("$(realpath "$MAILSTRONAUT_CREDENTIAL_FIXTURE")")
else
  credential_tests=(cargo test -p mailstronaut-mail --test credentials "${cargo_flags[@]}" --)
fi
# Each fixture deliberately locks its private collection. Run one lifecycle per
# fresh daemon/bus rather than depending on a desktop unlock prompter in tests.
if [[ -n ${MAILSTRONAUT_QUIET_ACTION_FIXTURE:-} ]]; then
  "$(realpath "$MAILSTRONAUT_QUIET_ACTION_FIXTURE")" account_actions::tests::private_automatic_access_never_unlocks --exact --ignored --nocapture | tee "$root/quiet-actions.log"
elif [[ -n ${MAILSTRONAUT_OAUTH_FIXTURE:-} ]]; then
  "$(realpath "$MAILSTRONAUT_OAUTH_FIXTURE")" private_secret_service_oauth_rotation_is_serialized_and_revocation_stops_retries --exact --ignored --nocapture | tee "$root/oauth.log"
else
  "${credential_tests[@]}" private_secret_service_round_trip_isolation_rotation_and_lock --exact --ignored --nocapture | tee "$root/test.log"
fi
kill "$keyring_pid"
wait "$keyring_pid" 2>/dev/null || true
keyring_pid=''
find "$root/data" -type f -exec sha256sum '{}' + | sort > "$root/before-unavailable.sha256"
"${credential_tests[@]}" private_missing_service_fails_closed --exact --ignored --nocapture | tee "$root/unavailable.log"
find "$root/data" -type f -exec sha256sum '{}' + | sort > "$root/after-unavailable.sha256"
cmp "$root/before-unavailable.sha256" "$root/after-unavailable.sha256"
python3 - "$root/data" <<'PY'
import pathlib, sys
files = list(pathlib.Path(sys.argv[1]).rglob('*.keyring'))
assert files, 'fixture did not persist an encrypted keyring'
for path in files:
    data = path.read_bytes()
    for value in (b'SYNTHETIC-ONBOARDING-PASSWORD-NOT-REAL', b'synthetic-rotated', b'synthetic-other-account', b'synthetic-refresh', b'synthetic-duplicate', b'SYNTHETIC-OAUTH-INITIAL', b'SYNTHETIC-OAUTH-REFRESH-ONE', b'SYNTHETIC-OAUTH-REFRESH-TWO', b'SYNTHETIC-OAUTH-ROTATED', b'SYNTHETIC-OAUTH-REFRESH-THREE', b'SYNTHETIC-OAUTH-RECONNECTED', b'SYNTHETIC-PORTAL-REFRESH-ONE', b'SYNTHETIC-PORTAL-REFRESH-TWO', b'SYNTHETIC-OAUTH-ACCESS-NOT-REAL'):
        assert value not in data, 'synthetic credential appeared in persisted keyring bytes'
print('Persisted fixture keyring contains none of the synthetic credential markers in plaintext.')
PY
printf 'Private keyring artifacts: %s\n' "$root"
