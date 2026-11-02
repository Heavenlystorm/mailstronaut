#!/usr/bin/env bash
# Called only by credentials.sh: the complete history is fetched by default, an explicit age
# cap still limits it, and a pass killed mid-way resumes from its durable cursor without
# losing or repeating mail. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/history-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
# A Maildir arrival with a chosen date. Flags are part of the file name, and Dovecot moves
# new mail into cur/ on first access, so the server's flags are read from its contents.
arrived() {
  local key=$1 age=$2 path=$3
  mkdir -p "$(dirname "$path")"
  cat > "$path" <<MAIL
From: Correspondent $key <$key@example.test>
To: reader@example.test
Subject: Synthetic $key
Message-ID: <$key@example.test>
Date: $(date -R -d "$age")
Content-Type: text/plain; charset=utf-8

Synthetic body marker-$key.
MAIL
  touch -d "$age" "$path"
}
# Mail older than any 30-day cap, one of it already read, then recent mail.
arrived ancient '400 days ago' "$mail_root/new/ancient.synthetic"
arrived old-read '200 days ago' "$mail_root/cur/old-read.synthetic:2,S"
arrived old-unread '90 days ago' "$mail_root/new/old-unread.synthetic"
arrived recent-one '2 days ago' "$mail_root/new/recent-one.synthetic"
# A busy folder of 2300 messages, all within the cap, so that the history spans far more
# windows than one cycle examines.
python3 - "$mail_root/new" <<'PY'
import os, sys, time
directory = sys.argv[1]
os.makedirs(directory, exist_ok=True)
stamp = time.time() - 3 * 86400
date = time.strftime('%a, %d %b %Y %H:%M:%S +0000', time.gmtime(stamp))
for number in range(1, 2301):
    path = os.path.join(directory, f'bulk-{number:04d}.synthetic')
    with open(path, 'w', encoding='utf-8') as file:
        file.write(
            f'From: Batch <batch@example.test>\nTo: reader@example.test\n'
            f'Subject: Synthetic batch {number}\nMessage-ID: <bulk-{number}@example.test>\n'
            f'Date: {date}\nContent-Type: text/plain; charset=utf-8\n\n'
            f'Synthetic batch body {number}.\n'
        )
    os.utime(path, (stamp, stamp))
PY
bulk=2300
total=$((bulk + 4))
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
# The optional line declares a history cap; without it the complete history is fetched.
write_config() {
  cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  $1
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
}
write_config ''
printf '%s' "$imap_password" | secret-tool store --label='Synthetic history fixture' \
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
  for _ in {1..900}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
inbox_pass="SELECT b.reviewed FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX'"
inbox_finished="SELECT count(*) FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX' AND b.finished_at IS NOT NULL"
old_subjects="SELECT count(*) FROM messages WHERE subject IN ('Synthetic ancient','Synthetic old-read','Synthetic old-unread')"
inbox_validity="SELECT b.uidvalidity FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX'"

# Without a declared limit the complete history is fetched, however old.
# The application is killed at three points of the same pass. Each time the process dies with
# history half-fetched, and the cursor in the database is all that survives it. What it left
# behind must be consistent: the database is intact, and every cached message has its original on
# disk. The next start resumes the pass from that cursor.
for reviewed in 256 1024 1792; do
  start "crash-$reviewed"
  await "the pass reviewed $reviewed messages and saved its cursor" 1 \
    "SELECT count(*) FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX' AND b.reviewed>=$reviewed AND b.finished_at IS NULL"
  capture "syncing-$reviewed"
  kill -KILL "$app"
  wait "$app" 2>/dev/null || true
  app=''
  printf 'OK: the application was killed after %s messages were reviewed\n' "$reviewed"
  [[ $(query 'PRAGMA integrity_check') == ok ]]
  missing=$(python3 - "$XDG_DATA_HOME/mailstronaut" <<'PY'
import os, sqlite3, sys
root = sys.argv[1]
connection = sqlite3.connect(os.path.join(root, 'mail.sqlite3'))
hashes = {row[0] for row in connection.execute('SELECT blob_hash FROM messages')}
blobs = os.path.join(root, 'blobs')
print(sum(1 for value in hashes if not os.path.isfile(os.path.join(blobs, value))))
PY
  )
  [[ $missing == 0 ]]
  printf '%s\n' 'OK: after the kill the database is intact and every cached message has its original'
done
start resumed
await 'the interrupted pass finished' 1 "$inbox_finished"
await 'the complete history is cached, mail from over a year ago included' "$total" \
  'SELECT count(*) FROM messages'
[[ $(query "$old_subjects") == 3 ]]
printf '%s\n' 'OK: mail older than 30 days was fetched by default'
[[ $(query "$inbox_pass") == "$total" ]]
printf '%s\n' 'OK: resuming reviewed each message once, neither repeating nor skipping any'
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
[[ $(server_flags INBOX 'marker-old-read') == *S* ]]
[[ $(server_flags INBOX 'marker-old-unread') != *S* ]]
printf '%s\n' 'OK: fetching changed no Seen flag, here or on the server'
sleep 2
capture complete
close_window

# A declared 30-day cap on a fresh profile: only what it allows is fetched and recorded.
rm -rf "$XDG_DATA_HOME/mailstronaut"
write_config '  sync.initialDays = 30;'
start capped
await 'the cap fetched only the recent mail' "$((bulk + 1))" 'SELECT count(*) FROM messages'
await 'the capped pass finished' 1 "$inbox_finished"
[[ $(query "$old_subjects") == 0 ]]
[[ $(query "SELECT b.cap_days FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX'") == 30 ]]
printf '%s\n' 'OK: an explicit age cap limits fetching and is recorded with the pass'
close_window

# A restart keeps the cap: nothing older is fetched, and nothing is duplicated.
start capped-again
await 'the capped pass stays finished after a restart' 1 "$inbox_finished"
[[ $(query 'SELECT count(*) FROM messages') == $((bulk + 1)) ]]
[[ $(query "$old_subjects") == 0 ]]
printf '%s\n' 'OK: a restart under the same cap changed nothing'
sleep 2
capture capped
close_window

# The server rebuilds the mailbox's UID namespace. The old namespace's history is not
# duplicated, and the pass starts again under the new one.
old_validity=$(query "$inbox_validity")
dovecot_stop
rm -f "$mail_root"/dovecot-uidlist "$mail_root"/dovecot.index*
dovecot_start
start reset
await 'the pass restarted under the new UIDVALIDITY and finished' 1 \
  "SELECT count(*) FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX' AND b.uidvalidity<>$old_validity AND b.finished_at IS NOT NULL"
[[ $(query 'SELECT count(*) FROM messages') == $((bulk + 1)) ]]
[[ $(query "SELECT count(*) FROM effective_memberships m JOIN folders f ON f.id=m.folder_id WHERE f.name='INBOX'") == $((bulk + 1)) ]]
printf '%s\n' 'OK: a UIDVALIDITY reset re-attached every message once, none duplicated'
sleep 2
capture reset
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native history acceptance: %s\n' "$artifacts"
