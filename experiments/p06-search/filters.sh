#!/usr/bin/env bash
# Explicit search filters over synthetic mail in a private Wayland session.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/search-filters"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-filter-user
imap_password=SYNTHETIC-FILTER-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
message() {
  seed_message "$1" <<MAIL
From: $2 <$1@example.test>
To: reader@example.test
Subject: $3
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic budget review.
MAIL
}
message old 'Finance Office' 'Old budget review'
message current 'Finance Office' 'Current budget review'
message other 'Library' 'Other budget review'
touch -d '45 days ago' "$mail_root/new/old.synthetic"
touch -d '1 hour ago' "$mail_root/new/current.synthetic"
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1; managed = true; appearance = "light"; timeZone = "UTC";
  sync.pollSeconds = 3600;
  accounts = [{ id = "filter-synthetic"; provider = "manual";
    address = "reader@example.test"; displayName = "Synthetic reader"; userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "localhost"; port = $imap_port; security = "tls"; };
    tls.caFiles = [ "$ca" ]; auth = { kind = "password"; secretRef = "filter-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic filter fixture' \
  application "$(cat assets/app-id)" account filter-synthetic reference filter-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() { session_cleanup; dovecot_cleanup; }
trap cleanup EXIT
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
start initial
for _ in {1..150}; do
  [[ $(query 'SELECT count(*) FROM messages' || true) == 3 && $(query 'SELECT count(*) FROM search_pending' || true) == 0 ]] && break
  kill -0 "$app"; sleep .2
done
[[ $(query 'SELECT count(*) FROM messages') == 3 ]]
sleep 1
wtype -M ctrl k -m ctrl 'budegt' -s 1500
click 1075 65
capture filters-open
click 1050 185
wtype 'Finance'
click 1050 265
capture dates-open
click 965 347
capture dates-selected
click 950 360
wtype -k Escape -s 800
click 700 600
capture filtered
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
# Metadata-only search must work without consuming the text index's hit budget.
wtype -M ctrl k -m ctrl -M ctrl a -m ctrl -k BackSpace -s 800
capture filters-only
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
# Keep the filters across folder navigation, without opening a row.
click 100 350
capture filtered-sent
click 100 318
sleep 1
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
# A From restriction must ignore To, and still find addresses hidden by display names.
click 1050 65
click 1050 185
wtype -M ctrl a -m ctrl 'reader@example.test'
click 950 360
wtype -k Escape -s 800
click 700 600
click 350 150
capture recipient-not-sender
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
click 1050 65
click 1050 185
wtype -M ctrl a -m ctrl 'current@example.test'
click 950 360
wtype -k Escape -s 800
click 700 600
wtype -M ctrl k -m ctrl 'budegt' -s 800
click 350 150
for _ in {1..100}; do
  [[ $(query "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Current budget review'" || true) == 1 ]] && break
  kill -0 "$app"; sleep .1
done
[[ $(query "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Current budget review'") == 1 ]]
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
capture filtered-opened
click 1050 65
click 1060 360
wtype -k Escape -s 800
click 700 600
capture cleared
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 1 ]]
click 350 150
for _ in {1..100}; do
  [[ $(query "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Other budget review'" || true) == 1 ]] && break
  kill -0 "$app"; sleep .1
done
[[ $(query "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Other budget review'") == 1 ]]
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 2 ]]
close_window
printf 'Native sender/date filters, folder changes, clear and explicit-open-only Seen passed: %s\n' "$artifacts"
