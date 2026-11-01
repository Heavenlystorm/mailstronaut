#!/usr/bin/env bash
# Called only by credentials.sh: the rule-based priority group in the real list, against
# mail fetched from a real server. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/priority-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
# Deadlines must stay in the future whenever this runs, in each language's month names.
french_date=$(python3 -c 'import datetime as d; m=["janvier","février","mars","avril","mai","juin","juillet","août","septembre","octobre","novembre","décembre"]; x=d.date.today()+d.timedelta(days=10); print(x.day, m[x.month-1])')
message() {
  seed_message "$1" <<MAIL
From: $2 <$1@example.test>
To: ${5:-reader@example.test}
Subject: $3
Message-ID: <$1@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

$4
MAIL
}
message french 'École doctorale' 'Réinscription en école doctorale' "Votre dossier de réinscription doit être déposé avant le $french_date."
message italian 'Maria' 'Domanda veloce' 'Puoi dirmi quale aula abbiamo prenotato per il seminario?'
message newsletter 'Department News' 'Weekly newsletter' 'New hires this week. Please reply by tomorrow if you want to join. Unsubscribe here.'
message flight 'Airline' 'Flight confirmation' 'Your flight leaves tomorrow. Have a good trip.'
message colleague 'Tom' 'Meeting notes' 'Here are the notes from yesterday.'
message elsewhere 'Someone' 'For a list' 'Could you look at this?' 'list@example.test'
message ambiguous 'Office' 'Unclear form' 'The form is due 03/04/2030.'
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  priority.enabled = true;
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
printf '%s' "$imap_password" | secret-tool store --label='Synthetic priority fixture' \
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
group='SELECT COALESCE(group_concat(subject," | "),"") FROM (SELECT m.subject FROM analysis a JOIN messages m ON m.id=a.message_id WHERE a.expires_at IS NOT NULL ORDER BY m.subject)'
live="SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE a.expires_at>strftime('%s','now') AND a.dismissed=0 AND m.read_cleared=0"

start initial
await 'the inbox was cached' 7 'SELECT count(*) FROM messages'
await 'every message was analyzed' 7 'SELECT count(*) FROM analysis'
await 'only the two that ask something of the reader qualified' 'Domanda veloce | Réinscription en école doctorale' "$group"
[[ $(query 'SELECT count(*) FROM effective_memberships WHERE seen=1') == 0 ]]
printf '%s\n' 'OK: analysis set no Seen flag'
sleep 1
capture needs-attention

# Dismissing does not open or read the message, and it stays gone.
click 533 274
await 'dismissing took one message out of the group' 1 "$live"
[[ $(query 'SELECT count(*) FROM effective_memberships WHERE seen=1') == 0 ]]
printf '%s\n' 'OK: dismissing did not read the message'
sleep 1
capture dismissed

# Opening the remaining one ends its priority, and marking it unread does not restore it.
click 350 180
await 'opening took the last message out of the group' 0 "$live"
sleep 1
capture opened
wtype -s 300 -M alt u -m alt -s 800
await 'it is unread again' 0 'SELECT count(*) FROM effective_memberships WHERE seen=1'
[[ $(query "$live") == 0 ]]
printf '%s\n' 'OK: marking it unread did not bring it back'
sleep 1
capture unread-again

# Mail arriving while the application runs is analyzed without touching anything.
message arriving 'Finance Office' 'Expense report' 'Could you send me your expense report by tomorrow?'
await 'a new delivery qualified by itself' 1 "$live"
sleep 2
capture arrived
close_window

# Results persist, so nothing is analyzed twice and dismissals survive a restart.
before=$(query 'SELECT count(*) FROM analysis')
start restarted
await 'the delivery is still in the group after a restart' 1 "$live"
[[ $(query 'SELECT count(*) FROM analysis') == "$before" ]]
printf '%s\n' 'OK: results and dismissals survived a restart'
sleep 2
capture restarted

# Opening a message says why it was flagged, quoting the words a deadline came from.
click 350 170
sleep 1.5
capture explained
# A date that cannot be read with certainty is shown for the reader, and decided nothing.
click 595 65
sleep .3
wtype -s 80 'Unclear form'
sleep 1.5
click 350 150
sleep 1.5
capture uncertain
[[ $(query "SELECT a.expires_at IS NULL FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject='Unclear form'") == 1 ]]
printf '%s\n' 'OK: an uncertain date gave the message no priority'
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native priority acceptance: %s\n' "$artifacts"
