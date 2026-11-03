#!/usr/bin/env bash
# Two synthetic accounts synchronize simultaneously in a private Wayland session.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/multi-progress"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-progress-user
imap_password=SYNTHETIC-PROGRESS-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
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
cat > "$config" <<NIX
{
  schemaVersion = 1; managed = true; appearance = "light"; sync.pollSeconds = 3600;
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
cleanup() { session_cleanup; dovecot_cleanup; }
trap cleanup EXIT
start sync
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
for attempt in {1..40}; do
  capture "progress-$attempt"
  [[ $(query "SELECT count(*) FROM backfill WHERE reviewed>0" || true) -ge 2 ]] && break
  kill -0 "$app"; sleep .2
done
[[ $(query 'SELECT count(*) FROM backfill WHERE reviewed>0') -ge 2 ]]
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
close_window
printf 'Two-account synthetic sync progress captured: %s\n' "$artifacts"
