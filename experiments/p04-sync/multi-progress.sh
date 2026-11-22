#!/usr/bin/env bash
# Two synthetic accounts synchronize simultaneously in a private Wayland session.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
# Dovecot appends /dovecot/run/token-login/imap-urlauth to this path.
artifacts="$root/progress"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-progress-user
imap_password=SYNTHETIC-PROGRESS-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
cleanup() {
  if declare -F session_cleanup > /dev/null; then session_cleanup; fi
  dovecot_cleanup
}
trap cleanup EXIT
python3 - "$mail_root/new" <<'PY'
import datetime, pathlib, sys
root = pathlib.Path(sys.argv[1])
date = datetime.datetime.now(datetime.timezone.utc).strftime('%a, %d %b %Y %H:%M:%S +0000')
for number in range(1200):
    (root / f'{number}.synthetic').write_text(
        f'From: Synthetic <sender@example.test>\nTo: reader@example.test\n'
        f'Subject: Synthetic progress {number}\nMessage-ID: <progress-{number}@example.test>\n'
        f'Date: {date}\nContent-Type: text/plain; charset=utf-8\n\nSynthetic message.\n')
PY
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
automatic=true
if [[ ${MAILSTRONAUT_TEST_MANUAL_SYNC:-0} == 1 ]]; then automatic=false; fi
cat > "$config" <<NIX
{
  schemaVersion = 1; managed = true; appearance = "light"; sync.pollSeconds = 3600;
  sync.automatic = $automatic;
  accounts = map (id: {
    inherit id; provider = "manual"; address = "reader@example.test";
    displayName = id; userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $imap_port; security = "tls"; };
    tls.caFiles = [ "$ca" ]; auth = { kind = "password"; secretRef = id; };
  }) [ "progress-a" "progress-b" ];
}
NIX
for account in progress-a progress-b; do
  printf '%s' "$imap_password" | secret-tool store --label='Synthetic progress fixture' \
    application "$(cat assets/app-id)" account "$account" reference "$account" purpose password
done
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
start sync
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
if [[ $automatic == false ]]; then
  await_count() {
    local expected=$1 sql=$2 observed=''
    for _ in {1..400}; do
      observed=$(query "$sql" || true)
      [[ $observed == "$expected" ]] && return 0
      kill -0 "$app"; sleep .2
    done
    capture manual-timeout
    printf 'Expected %s; observed %s\n' "$expected" "$observed" >&2
    return 1
  }
  await_count 1 "SELECT count(*) FROM sqlite_master WHERE name='messages'"
  sleep 3
  [[ $(query 'SELECT count(*) FROM messages') == 0 ]]
  capture manual-before
  wtype -M ctrl r -m ctrl -s 700
  capture manual-fetch
  await_count 2400 'SELECT count(*) FROM messages'
  await_count 2 "SELECT count(*) FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX' AND b.finished_at IS NOT NULL"
  [[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
  printf '%s\n' 'OK: an explicit refresh with automatic sync off fetched both accounts'
  seed_message after-manual <<MAIL
From: Manual fixture <manual@example.test>
To: reader@example.test
Subject: Mail after explicit sync
Message-ID: <after-manual@example.test>
Date: $(date -R)

Synthetic second arrival.
MAIL
  # Also exceed the 30-second continuation interval. A completed manual pass
  # must leave no history timer or IDLE session fetching the arrival.
  sleep 35
  [[ $(query 'SELECT count(*) FROM messages') == 2400 ]]
  wtype -M ctrl r -m ctrl -s 700
  await_count 2 "SELECT count(*) FROM messages WHERE subject='Mail after explicit sync'"
  [[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
  sleep 1
  capture manual-finished
  close_window
  printf 'Native explicit-only multi-account sync passed: %s\n' "$artifacts"
  exit 0
fi
for attempt in {1..40}; do
  capture "progress-$attempt"
  [[ $(query "SELECT count(*) FROM backfill WHERE reviewed>0" || true) -ge 2 ]] && break
  kill -0 "$app"; sleep .2
done
[[ $(query 'SELECT count(*) FROM backfill WHERE reviewed>0') -ge 2 ]]
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
close_window
printf 'Two-account synthetic sync progress captured: %s\n' "$artifacts"
