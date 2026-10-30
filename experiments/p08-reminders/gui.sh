#!/usr/bin/env bash
# Sourced by the existing private native conversation fixture after startup.
set -euo pipefail
: "${root:?private root}" "${window:?native window}" "${binary:?prebuilt app}"
click 300 245
sleep .6
copy_body reminder-message 'Synthetic unrelated same-subject message.'
click 905 299
sleep .6
capture reminder-editor
if [[ ${MAILSTRONAUT_REMINDERS_INSPECT_ONLY:-0} == 1 ]]; then
  printf 'Reminder editor inspection: %s/conversations-gui/reminder-editor.png\n' "$root"
  stop
  return
fi
click 235 290
sleep .6
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYHOUR'
import json,sqlite3,sys,time
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]));r=c.execute('SELECT account_id,message_id,due_at,time_zone,generation,state FROM reminders').fetchone()
assert r[:2]==('conversation-synthetic',ids['unrelated'])
assert 3580<=r[2]-time.time()<=3601
assert r[3:]==('Europe/Paris',1,'pending')
assert c.execute('SELECT message_id,desired FROM pending_operations').fetchall()==[(ids['unrelated'],1)]
print('Native one-hour preset saved the exact clicked message without changing other read flags.')
PYHOUR
capture reminder-one-hour
click 500 290
sleep .6
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYTOMORROW'
import datetime,sqlite3,sys
from zoneinfo import ZoneInfo
z=ZoneInfo('Europe/Paris');tomorrow=datetime.datetime.now(z).date()+datetime.timedelta(days=1)
r=sqlite3.connect(sys.argv[1]).execute('SELECT due_at,generation FROM reminders').fetchone()
assert datetime.datetime.fromtimestamp(r[0],z)==datetime.datetime.combine(tomorrow,datetime.time(9),z)
assert r[1]==2
print('Native tomorrow preset rescheduled the same reminder using the configured calendar day.')
PYTOMORROW
read -r day_x day_y < <(python3 - <<'PYDAY'
import datetime
from zoneinfo import ZoneInfo
now=datetime.datetime.now(ZoneInfo('Europe/Paris')).date();next_day=now+datetime.timedelta(days=1)
first=now.replace(day=1);cell=(next_day-first).days+first.weekday()
print(217+48*(cell%7),440+48*(cell//7))
PYDAY
)
click "$day_x" "$day_y"
click 235 740
xdotool key --clearmodifiers ctrl+a
xdotool type --clearmodifiers '10:45'
click 400 740
sleep .6
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYCUSTOM'
import datetime,sqlite3,sys
from zoneinfo import ZoneInfo
z=ZoneInfo('Europe/Paris');tomorrow=datetime.datetime.now(z).date()+datetime.timedelta(days=1)
r=sqlite3.connect(sys.argv[1]).execute('SELECT due_at,generation FROM reminders').fetchone()
assert datetime.datetime.fromtimestamp(r[0],z)==datetime.datetime.combine(tomorrow,datetime.time(10,45),z),r
assert r[1]==3
print('Native calendar and HH:MM field saved a custom civil reminder.')
PYCUSTOM
capture reminder-custom
xdotool mousemove --window "$window" 700 600 click --repeat 10 --delay 40 5
sleep .3
capture reminder-list
click 240 730
sleep .6
copy_body reminder-open 'Synthetic unrelated same-subject message.'
click 575 339
sleep .5
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYUNREAD'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert c.execute('SELECT desired FROM pending_operations WHERE message_id=?',(ids['unrelated'],)).fetchone()[0]==0
assert c.execute('SELECT state,generation FROM reminders').fetchone()==('pending',3)
print('Opening the reminder resolved its exact original; marking it unread left the schedule intact.')
PYUNREAD
stop
start reminder-offline
sleep .6
click 55 406
sleep .6
capture reminder-offline
if [[ ${MAILSTRONAUT_REMINDERS_LIST_INSPECT_ONLY:-0} == 1 ]]; then
  printf 'Reminder list inspection: %s/conversations-gui/reminder-offline.png\n' "$root"
  stop
  return
fi
click 240 334
sleep .6
copy_body reminder-offline-open 'Synthetic unrelated same-subject message.'
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" "$root/conversation-ids.json" <<'PYOPEN'
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);ids=json.load(open(sys.argv[2]))
assert c.execute('SELECT desired FROM pending_operations WHERE message_id=?',(ids['unrelated'],)).fetchone()[0]==1
assert c.execute('SELECT state,generation FROM reminders').fetchone()==('pending',3)
print('Explicitly opening the persisted reminder after restart marked only its original message read.')
PYOPEN
click 55 406
sleep .4
click 360 334
sleep .6
python3 - "$XDG_DATA_HOME/mailstronaut/mail.sqlite3" <<'PYCANCEL'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute('SELECT state,generation,withdraw_pending FROM reminders').fetchone()==('cancelled',4,1)
assert c.execute('SELECT count(*) FROM drafts').fetchone()[0]==0
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Native cancellation persisted its generation and pending withdrawal without creating outgoing mail.')
PYCANCEL
capture reminder-cancelled
stop
printf 'Native reminder acceptance artifacts: %s/conversations-gui\n' "$root"

