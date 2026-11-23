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
expired_date=$(python3 -c 'import datetime as d; print(d.date.today()-d.timedelta(days=2))')
message expired 'Registrar' 'Expired questionnaire' "Please complete this questionnaire by $expired_date."
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  timeZone = "Europe/Paris";
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
await 'the inbox was cached' 8 'SELECT count(*) FROM messages'
await 'every message was analyzed' 8 'SELECT count(*) FROM analysis'
await 'only actionable, unexpired requests qualified' 'Domanda veloce | Réinscription en école doctorale' "$group"
await 'an expired questionnaire got no fresh lifetime' 1 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject='Expired questionnaire' AND a.expires_at IS NULL AND EXISTS(SELECT 1 FROM json_each(a.data,'\$.reasons') WHERE value='AlreadyExpired')"
[[ $(query 'SELECT count(*) FROM effective_memberships WHERE seen=1') == 0 ]]
printf '%s\n' 'OK: analysis set no Seen flag'
sleep 1
capture needs-attention

# Replace the configuration while the application stays open. New mail still syncs
# with analysis disabled, then becomes eligible when the rules are enabled again.
cp "$config" "$artifacts/enabled.nix"
sed 's/priority.enabled = true/priority.enabled = false/' "$artifacts/enabled.nix" > "$config"
wtype -M ctrl -M shift -k r -m shift -m ctrl -s 1000
message rules-reload 'Office' 'Rules reload sample' 'Here are the meeting notes.'
wtype -M ctrl -k r -m ctrl
await 'new mail was cached with priority off' 1 "SELECT count(*) FROM messages WHERE subject='Rules reload sample'"
reload_result="SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject='Rules reload sample'"
[[ $(query "$reload_result") == 0 ]]
capture rules-disabled
cp "$artifacts/enabled.nix" "$config"
wtype -M ctrl -M shift -k r -m shift -m ctrl
await 'the replacement worker analyzed the untouched mail' 1 "$reload_result"
[[ $(query 'SELECT count(*) FROM effective_memberships WHERE seen=1') == 0 ]]

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
if [[ ${MAILSTRONAUT_DEADLINE_CORRECTION:-0} == 1 ]]; then
  correction="SELECT c.due FROM deadline_corrections c JOIN messages m ON m.id=c.message_id WHERE m.subject='Unclear form'"
  click 656 370
  capture correction-dialog
  wtype -k Escape -s 500
  [[ $(query 'SELECT count(*) FROM deadline_corrections') == 0 ]]
  click 656 370
  click 640 172
  capture correction-calendar
  # GPUI Kit opens April 2030 from the detected date. The 15th falls
  # in the second column of the third week in this Sunday-first calendar.
  click 493 352
  capture correction-selected
  click 790 220
  await 'the selected deadline persisted on the exact original' '2030-04-15' "$correction"
  [[ $(query "SELECT count(*) FROM deadline_corrections WHERE priority_until IS NOT NULL") == 0 ]]
  [[ $(query "SELECT json_extract(data,'\$.zone') FROM deadline_corrections") == 'Europe/Paris' ]]
  [[ $(query "SELECT json_extract(data,'\$.ends_at') FROM deadline_corrections") == "$(date -d '2030-04-15T22:00:00Z' +%s)" ]]
  capture corrected
  close_window
  start correction-restart
  click 595 65
  wtype -s 80 'Unclear form'
  sleep 1.5
  click 350 150
  sleep 1.5
  await 'the correction survived reopening the original' '2030-04-15' "$correction"
  capture corrected-after-restart
fi
# A group longer than its limit says so, and Show all lists every one of them.
for number in $(seq 1 21); do
  message "bulk$number" 'Registrar' "Form $number" 'Please return this form by tomorrow.'
done
# This phase tests grouping, not IDLE. Wake sync after the fixture delivery so
# a watch reconnect during the preceding restart cannot delay the walkthrough.
wtype -M ctrl r -m ctrl
await 'every bulk form was analyzed' 21 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject LIKE 'Form %' AND a.expires_at IS NOT NULL"
# The group belongs to the unfiltered inbox, so the search left from above is cleared first.
click 595 65
wtype -M ctrl a -m ctrl -k BackSpace -s 1500
capture busy-group
click 515 110
sleep 1.5
capture show-all
[[ $(query "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject LIKE 'Form %' AND a.expires_at>strftime('%s','now') AND a.dismissed=0 AND m.read_cleared=0") == 21 ]]
printf '%s\n' 'OK: all 21 forms still need attention, more than the group shows at first'
if [[ ${MAILSTRONAUT_DEADLINE_CORRECTION:-0} == 1 ]]; then
  message timed-correction 'Registrar' 'Timed correction sample' 'The questionnaire is due 15 April 2030 at 10:30.'
  wtype -M ctrl r -m ctrl
  await 'a timed original was analyzed' 1 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject='Timed correction sample' AND a.expires_at IS NOT NULL"
  click 595 65
  wtype -s 80 'Timed correction sample' -s 1200
  click 350 150
  capture timed-original
  click 656 390
  capture timed-dialog
  click 640 172
  capture timed-picker
  # Edit the embedded clock using the same time field as custom reminders.
  click 618 470
  wtype -s 200 '1142' -s 300 -k Escape -s 300
  capture timed-selected
  click 790 220
  await 'the timed deadline persisted with the new time' '2030-04-15 11:42' "SELECT c.due FROM deadline_corrections c JOIN messages m ON m.id=c.message_id WHERE m.subject='Timed correction sample'"
  [[ $(query "SELECT json_extract(c.data,'\$.ends_at') FROM deadline_corrections c JOIN messages m ON m.id=c.message_id WHERE m.subject='Timed correction sample'") == "$(date -d '2030-04-15T09:42:00Z' +%s)" ]]
  capture timed-corrected
fi
if [[ ${MAILSTRONAUT_DEADLINE_CONTEXT:-0} == 1 ]]; then
  message context-quote 'Office' 'Context grammar example' 'The phrase “Please send the form by tomorrow!” is an example in a textbook.'
  message context-old 'Office' 'Context old form' 'The text asking you to renew by tomorrow was copied from the old form.'
  message context-current 'Office' 'Context current form' 'The note to renew before tomorrow was copied from an old form. Please send “proof of address” by tomorrow.'
  wtype -M ctrl r -m ctrl
  await 'all three contextual cases were analyzed' 3 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject LIKE 'Context %' AND a.policy=3"
  await 'the current obligation survived the copied clause' 1 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject='Context current form' AND a.expires_at IS NOT NULL AND json_array_length(a.data,'\$.deadlines')=1"
  await 'quoted and copied obligations gave neither priority nor deadlines' 2 "SELECT count(*) FROM analysis a JOIN messages m ON m.id=a.message_id WHERE m.subject IN ('Context grammar example','Context old form') AND a.expires_at IS NULL AND json_array_length(a.data,'\$.deadlines')=0"
  await 'the contextual checks did not mark any original read' 0 "SELECT count(*) FROM effective_memberships s JOIN messages m ON m.id=s.message_id WHERE m.subject LIKE 'Context %' AND s.seen=1"
  click 595 65
  wtype -M ctrl a -m ctrl -k BackSpace -s 300 'Context current form' -s 1200
  click 350 150
  capture context-current-deadline
fi
close_window

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native priority acceptance: %s\n' "$artifacts"
