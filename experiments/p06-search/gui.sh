#!/usr/bin/env bash
# Called only by credentials.sh: typing misspelled queries into the real search field,
# against mail fetched from a real server. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/search-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
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

$4
MAIL
}
message french 'École doctorale' 'Réinscription en école doctorale' 'Votre dossier de réinscription doit être déposé avant le 15 novembre.'
message italian 'Dipartimento' 'Riunione del dipartimento' 'La riunione è confermata per giovedì a Torino.'
message english 'Finance Office' 'Quarterly budget review' 'Please review the attached budget before Friday.'
message other 'Library' 'Book return' 'Your books are due back next week.'
# Oldest first in the list order, so that the French notice is never first by accident.
touch -d '4 hours ago' "$mail_root/new/french.synthetic"
touch -d '3 hours ago' "$mail_root/new/italian.synthetic"
touch -d '2 hours ago' "$mail_root/new/english.synthetic"
touch -d '1 hour ago' "$mail_root/new/other.synthetic"
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
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
printf '%s' "$imap_password" | secret-tool store --label='Synthetic search fixture' \
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
seen_subjects() {
  query "SELECT COALESCE(group_concat(m.subject,' | '),'') FROM messages m JOIN effective_memberships s ON s.message_id=m.id WHERE s.seen=1"
}
# Replace what is in the search field with this text, as a person would.
type_query() {
  click 595 65
  sleep .3
  wtype -s 100 -M ctrl a -m ctrl -k BackSpace -s 200
  wtype -s 80 "$1"
  sleep 1.2
}
# Opening the first row is what proves which row came first: it sets Seen on that message.
open_first_row() {
  click 350 150
  sleep 1
}
expect_opened() {
  [[ $(seen_subjects) == *"$1"* ]] || { printf 'Expected the first row to be "%s"; seen: %s\n' "$1" "$(seen_subjects)" >&2; return 1; }
  printf 'OK: %s\n' "$2"
}

start initial
await 'the inbox was cached' 4 'SELECT count(*) FROM messages'
await 'everything was indexed' 0 'SELECT count(*) FROM search_pending'
sleep 1

type_query 'reinscripton'
capture typo
[[ $(query 'SELECT count(*) FROM memberships WHERE seen=1') == 0 ]]
printf '%s\n' 'OK: searching and listing results set no Seen flag'
open_first_row
capture typo-opened
expect_opened 'Réinscription' 'a misspelled, accent-free word found the French notice'

type_query 'riunone giovdi'
open_first_row
expect_opened 'Riunione' 'two misspelled Italian words found the Italian message'

type_query 'from:finance subject:budg'
capture fields
open_first_row
expect_opened 'Quarterly budget review' 'field names and a word beginning found the English message'

type_query 'zzzzzzzzzz'
capture nothing

# Mail arriving now becomes searchable without touching anything.
message delivered 'Latecomer' 'Unmistakable xylophone notice' 'Delivered while the application is running.'
await 'the new delivery was cached' 5 'SELECT count(*) FROM messages'
await 'the new delivery was indexed' 0 'SELECT count(*) FROM search_pending'
type_query 'xylofone'
capture delivered
open_first_row
expect_opened 'xylophone' 'a delivery that arrived while running is searchable'
close_window

# The index is only derived data: with it gone, it fills itself again from the store.
rm -rf "$XDG_DATA_HOME/mailstronaut/search"
start rebuilt
await 'the index rebuilt itself' 0 'SELECT count(*) FROM search_pending'
sleep 1
type_query 'bugdet'
capture rebuilt
open_first_row
expect_opened 'Quarterly budget review' 'search works from the rebuilt index'
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native search acceptance: %s\n' "$artifacts"
