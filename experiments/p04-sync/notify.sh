#!/usr/bin/env bash
# Called only by credentials.sh: new-mail notifications, against a real server and a fake
# notification portal on the private bus. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
variant=${3:?private, preview or disabled}
portal_fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt application test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
export XDG_DATA_HOME="$root/data-notify-$variant"
mkdir -p "$XDG_DATA_HOME"
artifacts="$root/notify-$variant"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
message() {
  seed_message "$1" <<MAIL
From: Correspondent $1 <$1@example.test>
To: reader@example.test
Subject: Synthetic subject $1
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic private body marker-$1.
MAIL
}
message existing-one
message existing-two
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
case $variant in
  private) notifications='notifications = { enabled = true; showPreview = false; };' ;;
  preview) notifications='notifications = { enabled = true; showPreview = true; };' ;;
  disabled) notifications='notifications = { enabled = false; showPreview = true; };' ;;
  *) exit 2 ;;
esac
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  $notifications
  accounts = [{
    id = "notify-$variant";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $imap_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "dovecot-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic notification fixture' \
  application "$(cat assets/app-id)" account "notify-$variant" reference dovecot-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
portal=''
cleanup() {
  session_cleanup
  dovecot_cleanup
  if [[ -n "$portal" ]]; then kill "$portal" 2>/dev/null || true; wait "$portal" 2>/dev/null || true; fi
}
trap cleanup EXIT
MAILSTRONAUT_PORTAL_ROOT=$artifacts "$portal_fixture" portal_fixture::serve_portals --ignored --exact > "$artifacts/portal.log" 2>&1 & portal=$!
for _ in {1..100}; do [[ -f "$artifacts/portal-ready" ]] && break; kill -0 "$portal"; sleep .1; done
[[ -f "$artifacts/portal-ready" ]]

query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..100}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
receipts="$artifacts/notifications.jsonl"
shown() { [[ -f $receipts ]] && grep -c '"add"' "$receipts" || echo 0; }
last_shown() { grep '"add"' "$receipts" | tail -1; }
await_shown() {
  local what=$1 expected=$2
  for _ in {1..75}; do
    [[ $(shown) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (%s shown)\n' "$what" "$(shown)" >&2
  return 1
}

start initial
await 'the inbox was cached' 2 'SELECT count(*) FROM messages'
sleep 3
[[ $(shown) == 0 ]]
printf '%s\n' 'OK: mail already there when the application started is not announced'

# With the window in front, the list itself shows new mail.
message while-focused
await 'a delivery arrived with the window in front' 3 'SELECT count(*) FROM messages'
sleep 3
[[ $(shown) == 0 ]]
printf '%s\n' 'OK: no notification while the window is in front'

capture inbox
# Leave the inbox, so that clicking a notification has something to prove. Another
# workspace then takes the window out of the front.
wtype -s 200 -M ctrl 3 -m ctrl -s 700
capture elsewhere
swaymsg workspace 2 > /dev/null
sleep 1
message away-one
await 'a delivery arrived with the window away' 4 'SELECT count(*) FROM messages'
if [[ $variant == disabled ]]; then
  sleep 4
  [[ $(shown) == 0 ]]
  printf '%s\n' 'OK: disabled notifications stay silent'
  [[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
  printf '%s\n' 'OK: nothing was marked read'
else
  await_shown 'the first delivery was announced' 1
  last_shown > "$artifacts/first.json"
  message away-two
  await 'a second delivery arrived' 5 'SELECT count(*) FROM messages'
  await_shown 'the second delivery was announced' 2
  last_shown > "$artifacts/second.json"
  python3 - "$artifacts" "$variant" <<'PY'
import json, sys
root, variant = sys.argv[1:]
first = json.load(open(f"{root}/first.json"))
second = json.load(open(f"{root}/second.json"))
assert first["add"] == second["add"] == f"new-mail-notify-{variant}", (first, second)
assert first["title"] == "New mail · Synthetic reader", first
assert first["action"] == "open-inbox", first
if variant == "private":
    assert first["body"] == "1 new message", first
    assert second["body"] == "2 new messages", second
else:
    assert first["body"] == "Correspondent away-one\nSynthetic subject away-one", first
    assert "2 new messages" in second["body"] and "Synthetic subject away-two" in second["body"], second
for note in (first, second):
    assert "marker-" not in note["body"], "a message body must never appear"
print("OK: one replaced notification per account, saying only what the settings allow")
PY
  # Clicking it brings the inbox forward, and opens the newest message it announced.
  printf '%s\n' "new-mail-notify-$variant open-inbox" > "$artifacts/notification-invoke"
  sleep 1
  swaymsg workspace 1 > /dev/null
  for _ in {1..50}; do grep -q '"remove"' "$receipts" && break; kill -0 "$app"; sleep .2; done
  grep -q '"remove"' "$receipts"
  printf '%s\n' 'OK: returning to the window withdrew the notification'
  sleep 1
  capture returned
  heading() { magick "$artifacts/$1.png" "$artifacts/$2.png" -compose difference -composite -crop 200x40+0+45 -format '%[fx:mean<0.00001]' info:; }
  [[ $(heading inbox elsewhere) == 0 ]]
  [[ $(heading inbox returned) == 1 ]]
  printf '%s\n' 'OK: clicking the notification brought the inbox forward'
  await 'the click opened the newest announced message' 1 \
    "SELECT count(*) FROM memberships m JOIN messages x ON x.id=m.message_id WHERE m.seen=1 AND x.subject='Synthetic subject away-two'"
  sleep 1
  capture opened
  [[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
  printf '%s\n' 'OK: opening by the click marked only that message read'
fi
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native notification acceptance (%s): %s\n' "$variant" "$artifacts"
