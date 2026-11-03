#!/usr/bin/env bash
# Private native Wayland close, background sync, notification-open and explicit quit.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
portal_fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt application test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
artifacts="$root/background-gui"
mkdir -p "$artifacts"
export XDG_DATA_HOME="$root/background-data"
mkdir -p "$XDG_DATA_HOME"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-background-user
imap_password=SYNTHETIC-BACKGROUND-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
: "${imap_port:?synthetic IMAP port}"
message() {
  seed_message "$1" <<MAIL
From: Synthetic sender <$1@example.test>
To: reader@example.test
Subject: Synthetic $1
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic background marker-$1.
MAIL
}
message first
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  background.enabled = true;
  sync.pollSeconds = 3600;
  accounts = [{
    id = "background-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $imap_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "background-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic background fixture' \
  application "$(cat assets/app-id)" account background-synthetic reference background-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
portal=''
cleanup() {
  session_cleanup
  dovecot_cleanup
  if [[ -n $portal ]]; then kill "$portal" 2>/dev/null || true; wait "$portal" 2>/dev/null || true; fi
}
trap cleanup EXIT
MAILSTRONAUT_PORTAL_ROOT=$artifacts "$portal_fixture" portal_fixture::serve_portals --ignored --exact > "$artifacts/portal.log" 2>&1 & portal=$!
for _ in {1..100}; do [[ -f $artifacts/portal-ready ]] && break; kill -0 "$portal"; sleep .1; done
[[ -f $artifacts/portal-ready ]]
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
renderer_threads() {
  python3 - "$app" <<'PY'
from pathlib import Path
import sys
tasks = Path('/proc') / sys.argv[1] / 'task'
count = 0
for path in tasks.glob('*/comm'):
    try:
        count += path.read_text().startswith('mailstronaut-ht')
    except FileNotFoundError:
        pass
print(count)
PY
}
await_query() {
  local expected=$1 sql=$2
  for _ in {1..150}; do
    [[ $(query "$sql" || true) == "$expected" ]] && return 0
    kill -0 "$app"; sleep .1
  done
  printf 'Timed out waiting for synthetic database state: %s\n' "$artifacts" >&2; return 1
}
start initial
await_query 1 'SELECT count(*) FROM messages'
# Schedule against the synchronized original while the production process is
# stopped, then restart it to exercise its real scheduler and profile lock.
first_id=$(query "SELECT id FROM messages WHERE subject='Synthetic first'")
wtype -s 200 -M ctrl q -m ctrl
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then printf 'Initial quit failed: %s\n' "$artifacts" >&2; exit 1; fi
wait "$app"; app=''
MAILSTRONAUT_BACKGROUND_SEED_ROOT="$root" MAILSTRONAUT_BACKGROUND_SEED_MESSAGE="$first_id" \
  "$portal_fixture" lifecycle_tests::seed_background_reminder --ignored --exact > "$artifacts/seed.log" 2>&1
start scheduled
[[ $(query "SELECT count(*) FROM reminders WHERE state='pending'") == 1 ]]
sleep 2
[[ $(renderer_threads) -ge 1 ]]
capture before-close
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
for _ in {1..100}; do ! window_open && break; kill -0 "$app"; sleep .1; done
if window_open; then printf 'Window did not close: %s\n' "$artifacts" >&2; exit 1; fi
kill -0 "$app"
for _ in {1..100}; do [[ $(renderer_threads) == 0 ]] && break; kill -0 "$app"; sleep .1; done
[[ $(renderer_threads) == 0 ]]
sleep 2
message second
await_query 2 'SELECT count(*) FROM messages'
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
[[ $(server_flags INBOX marker-second) != *S* ]]
receipts="$artifacts/notifications.jsonl"
for _ in {1..100}; do [[ -f $receipts ]] && grep -q '"add"' "$receipts" && break; kill -0 "$app"; sleep .1; done
grep -q '"add"' "$receipts"
timeout 10 "$binary" --config "$config" > "$artifacts/forwarded-app.log" 2>&1
for _ in {1..150}; do window_open && break; kill -0 "$app"; sleep .1; done
window_open
kill -0 "$app"
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
capture launcher-reopened
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
for _ in {1..100}; do ! window_open && break; kill -0 "$app"; sleep .1; done
if window_open; then printf 'Reopened window did not close: %s\n' "$artifacts" >&2; exit 1; fi
kill -0 "$app"
sleep 2
message third
await_query 3 'SELECT count(*) FROM messages'
for _ in {1..100}; do [[ $(grep -c '"add"' "$receipts" || true) == 2 ]] && break; kill -0 "$app"; sleep .1; done
[[ $(grep -c '"add"' "$receipts" || true) == 2 ]]
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
[[ $(server_flags INBOX marker-third) != *S* ]]
printf '%s\n' 'new-mail-background-synthetic open-inbox' > "$artifacts/notification-invoke"
for _ in {1..150}; do window_open && break; kill -0 "$app"; sleep .1; done
window_open
await_query 1 "SELECT count(*) FROM memberships m JOIN messages x ON x.id=m.message_id WHERE m.seen=1 AND x.subject='Synthetic third'"
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
capture reopened
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
for _ in {1..100}; do ! window_open && break; kill -0 "$app"; sleep .1; done
if window_open; then printf 'Reminder test window did not close: %s\n' "$artifacts" >&2; exit 1; fi
[[ $(query "SELECT count(*) FROM reminders WHERE state='pending'") == 1 ]]
# Wait long enough for the scheduled deadline; assert that delivery happens
# while headless, then send an obsolete action before the current one.
for _ in {1..1200}; do
  [[ $(query "SELECT count(*) FROM reminders WHERE state='delivered'" || true) == 1 ]] && break
  kill -0 "$app"; sleep .1
done
[[ $(query "SELECT count(*) FROM reminders WHERE state='delivered'") == 1 ]]
if window_open; then printf 'Reminder delivered with a window open: %s\n' "$artifacts" >&2; exit 1; fi
notification=$(query 'SELECT notification_id FROM reminders')
generation=$(query 'SELECT generation FROM reminders')
printf '%s open %s\n' "$notification" "$((generation + 1))" > "$artifacts/notification-invoke"
for _ in {1..100}; do [[ ! -e $artifacts/notification-invoke ]] && break; sleep .1; done
[[ ! -e $artifacts/notification-invoke ]]
sleep 1
if window_open; then printf 'Stale reminder opened a window: %s\n' "$artifacts" >&2; exit 1; fi
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
printf '%s open %s\n' "$notification" "$generation" > "$artifacts/notification-invoke"
for _ in {1..150}; do window_open && break; kill -0 "$app"; sleep .1; done
window_open
await_query 1 "SELECT count(*) FROM memberships m JOIN messages x ON x.id=m.message_id WHERE m.seen=1 AND x.subject='Synthetic first'"
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 2 ]]
[[ $(server_flags INBOX marker-second) != *S* ]]
capture reminder-reopened
wtype -s 200 -M ctrl q -m ctrl
for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .1; done
if kill -0 "$app" 2>/dev/null; then printf 'Explicit quit did not stop the application: %s\n' "$artifacts" >&2; exit 1; fi
wait "$app"; app=''
sed -i 's/background.enabled = true;/background.enabled = false;/' "$config"
start disabled
close_window
if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf 'Synthetic credential appeared in a log: %s\n' "$artifacts" >&2; exit 1
fi
printf 'Native background sync, launcher, new-mail and reminder reopening, stale reminder rejection, explicit quit and disabled close passed: %s\n' "$artifacts"
