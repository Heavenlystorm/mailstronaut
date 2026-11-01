#!/usr/bin/env bash
# Called only by credentials.sh: asking a real server for mail from before the automatic
# sync window, from the mailbox itself. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/older-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
# A message with a chosen arrival date; Maildir takes INTERNALDATE from the file's mtime.
message() {
  local key=$1 age=$2 directory=${3:-$mail_root}
  mkdir -p "$directory/new" "$directory/cur" "$directory/tmp"
  cat > "$directory/new/$key.synthetic" <<MAIL
From: Correspondent $key <$key@example.test>
To: reader@example.test
Subject: Synthetic $key
Message-ID: <$key@example.test>
Date: $(date -R -d "$age")
Content-Type: text/plain; charset=utf-8

Synthetic body marker-$key.
MAIL
  touch -d "$age" "$directory/new/$key.synthetic"
}
# The old message delivered last has the highest UID, as moved or imported mail often does.
message old-first '90 days ago'
message recent-one '2 days ago'
message old-second '60 days ago'
message recent-two '1 day ago'
message old-archived '200 days ago' "$mail_root/.Archive"
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  sync.initialDays = 30;
  accounts = [{
    id = "dovecot-synthetic";
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
printf '%s' "$imap_password" | secret-tool store --label='Synthetic older mail fixture' \
  application "$(cat assets/app-id)" account dovecot-synthetic reference dovecot-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  dovecot_cleanup
}
trap cleanup EXIT

query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..150}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
subjects='SELECT group_concat(subject," | ") FROM (SELECT subject FROM messages ORDER BY subject)'

start initial
await 'automatic sync took only the recent mail' 'Synthetic recent-one | Synthetic recent-two' "$subjects"
sleep 2
capture before

# The button sits at the right end of the bar below the list.
click 480 779
await 'asking fetched the older mail, whatever its UIDs' 'Synthetic old-first | Synthetic old-second | Synthetic recent-one | Synthetic recent-two' "$subjects"
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
printf '%s\n' 'OK: fetching left it unread'
sleep 2
capture fetched

# This small folder is now fetched back to its beginning, so the button has gone.
click 480 779
sleep 2
capture exhausted
[[ $(query 'SELECT count(*) FROM messages') == 4 ]]
printf '%s\n' 'OK: with the beginning reached, there is nothing more to fetch'

# A folder asks for its own older mail. Archive follows Sent and Drafts in the sidebar.
click 93 468
sleep 1
capture archive-empty
click 480 779
await 'the folder fetched its own older mail' 1 "SELECT count(*) FROM messages WHERE subject='Synthetic old-archived'"
sleep 2
capture archive-fetched

# Restarting forgets how far back it got, and nothing is duplicated by asking again.
close_window
start restarted
sleep 3
click 480 779
sleep 3
capture restarted
[[ $(query 'SELECT count(*) FROM messages') == 5 ]]
printf '%s\n' 'OK: fetching again after a restart duplicated nothing'
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native older mail acceptance: %s\n' "$artifacts"
