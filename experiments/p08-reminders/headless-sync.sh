#!/usr/bin/env bash
# Called with the private Secret Service fixture: sync synthetic mail without a window.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application test executable}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
artifacts="$root/headless-sync"
mkdir -p "$artifacts"
imap_user=synthetic-headless-user
imap_password=SYNTHETIC-HEADLESS-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
app=''
cleanup() {
  if [[ -n $app ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi
  dovecot_cleanup
}
trap cleanup EXIT
message() {
  seed_message "$1" <<MAIL
From: Synthetic sender <$1@example.test>
To: reader@example.test
Subject: Synthetic $1
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic headless marker-$1.
MAIL
}
message first
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
printf '%s' "$imap_password" | secret-tool store --label='Synthetic headless fixture' \
  application "$(cat assets/app-id)" account headless-synthetic reference headless-password purpose password
python3 - "$artifacts/config.json" "$imap_port" "$imap_user" "$ca" <<'PY'
import json, sys
path, port, user, ca = sys.argv[1:]
config = {
    'schemaVersion': 1, 'notifications': {'enabled': False}, 'sync': {'pollSeconds': 3600},
    'accounts': [{'id': 'headless-synthetic', 'provider': 'manual', 'address': 'reader@example.test',
                  'displayName': 'Synthetic reader', 'userName': user,
                  'imap': {'host': 'localhost', 'port': int(port), 'security': 'tls'},
                  'smtp': {'host': 'localhost', 'port': int(port), 'security': 'tls'},
                  'tls': {'caFiles': [ca]}, 'auth': {'kind': 'password', 'secretRef': 'headless-password'}}],
}
with open(path, 'w', encoding='utf-8') as output:
    json.dump(config, output)
PY
MAILSTRONAUT_HEADLESS_SYNC_ROOT="$artifacts" "$binary" --exact lifecycle_tests::headless_sync_with_private_dovecot --ignored --nocapture > "$artifacts/test.log" 2>&1 & app=$!
database="$artifacts/profile/mail.sqlite3"
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await_count() {
  local expected=$1
  for _ in {1..150}; do
    [[ $(query 'SELECT count(*) FROM messages' || true) == "$expected" ]] && return 0
    kill -0 "$app"
    sleep .1
  done
  printf 'Timed out waiting for %s synthetic messages: %s\n' "$expected" "$artifacts" >&2
  return 1
}
await_count 1
sleep 2 # Allow the first successful cycle to establish IDLE.
message second
await_count 2
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
[[ $(server_flags INBOX marker-second) != *S* ]]
touch "$artifacts/stop"
wait "$app"; app=''
printf 'Headless sync fetched both synthetic messages without marking them read: %s\n' "$artifacts"
