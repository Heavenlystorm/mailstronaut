#!/usr/bin/env bash
# Sourced by the private Wayland mailbox harness. Uses synthetic, offline mail.
set -euo pipefail
: "${root:?private fixture root}"
: "${config:?synthetic native declaration}"
trap 'printf "Reminder acceptance failed at line %s: %s\n" "$LINENO" "$root" >&2' ERR
original_config=$(sha256sum "$config")
wtype -s 200 -M ctrl l -m ctrl -k Down -k Return -s 700
copy_body reminder-original 'Synthetic unrelated same-subject message.'
check_seen unrelated
click 1039 244
wtype -s 200 -k Down -k Down -k Down -k Down -k Return -s 500
sleep .7
capture reminder-custom
# Cancel must not persist anything or change the explicit-open state.
click 690 220
python3 - "$root" <<'PYCANCEL'
import sqlite3,sys
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1])/'data/mailstronaut/mail.sqlite3')
assert c.execute('SELECT count(*) FROM reminders').fetchone()[0]==0
PYCANCEL
check_seen unrelated
click 1039 244
wtype -s 200 -k Down -k Down -k Down -k Down -k Return -s 400
sleep .7
click 640 172
# Calendar geometry comes from Kit's standard medium 32px cells. Choose a future
# day in the displayed month and edit its embedded time field, using native input.
read -r day_x day_y hour_y reopen < <(python3 - "$root" <<'PYCALENDAR'
import calendar,datetime,json,sys
from pathlib import Path
from zoneinfo import ZoneInfo
zone=ZoneInfo('Europe/Paris')
initial=datetime.datetime.now(zone).date()+datetime.timedelta(days=1)
chosen=initial+datetime.timedelta(days=1)
if chosen.month!=initial.month: chosen=initial
weekday,count=calendar.monthrange(initial.year,initial.month)
offset=(weekday+1)%7
cell=offset+chosen.day-1
rows=(offset+count+6)//7
at=datetime.datetime.combine(chosen,datetime.time(10,37),zone)
(Path(sys.argv[1])/'reminder-expected.json').write_text(json.dumps({'due':int(at.timestamp())}))
print(461+32*(cell%7),288+32*(cell//7),288+32*(rows-1)+57,int(chosen==initial))
PYCALENDAR
)
click "$day_x" "$day_y"
if [[ $reopen == 1 ]]; then click 640 172; fi
capture reminder-calendar
click 618 "$hour_y"
wtype -s 200 '1037' -s 300 -k Escape -s 300
capture reminder-selected
click 790 220
sleep .6
wtype -M ctrl 3 -m ctrl -s 500
capture reminder-details
python3 - "$root" <<'PYSAVED'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); ids=json.loads((root/'conversation-ids.json').read_text())
expected=json.loads((root/'reminder-expected.json').read_text())
c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
row=c.execute('SELECT message_id,due_at,time_zone,state FROM reminders').fetchone()
assert row==(ids['unrelated'],expected['due'],'Europe/Paris','pending'),(row,expected)
identity=c.execute('SELECT id,generation,notification_id FROM reminders').fetchone()
(root/'reminder-identity.json').write_text(json.dumps(identity))
print('Native calendar/time selection persisted the exact original, UTC instant and timezone.',flush=True)
PYSAVED
check_seen unrelated
# Editing the listed original keeps its identity and changes only the generation
# and selected instant. The same Kit picker handles rescheduling.
click 343 300
sleep .7
capture reminder-reschedule
click 640 172
click 618 "$hour_y"
wtype -s 200 '1142' -s 300 -k Escape -s 200
click 790 220
sleep .6
python3 - "$root" <<'PYRESCHEDULE'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
before=json.loads((root/'reminder-identity.json').read_text())
expected=json.loads((root/'reminder-expected.json').read_text())
identity=c.execute('SELECT id,generation,notification_id FROM reminders').fetchone()
assert identity==(before[0],before[1]+1,before[2]),(before,identity)
assert c.execute('SELECT due_at FROM reminders').fetchone()[0]==expected['due']+65*60
print('Reschedule retained the original and notification identity, with a new generation.',flush=True)
PYRESCHEDULE
capture reminder-rescheduled
swaymsg output HEADLESS-1 mode 960x720 > /dev/null
sleep .3
capture reminder-narrow
swaymsg output HEADLESS-1 mode 1280x840 > /dev/null
wtype -M ctrl -M shift d -m shift -m ctrl -s 400
capture reminder-dark
wtype -M ctrl -M shift d -m shift -m ctrl -s 300
# A different original starts with tomorrow morning, not the prior 11:42 value.
click 80 155
click 595 65
wtype -M ctrl a -m ctrl -k BackSpace -s 300 'other-account' -s 1200
click 350 150
sleep .6
copy_body reminder-other 'Synthetic other-account reply.'
check_seen unrelated other
click 1039 244
wtype -s 200 -k Down -k Down -k Down -k Down -k Return -s 400
sleep .7
capture reminder-other-default
click 790 220
sleep .6
python3 - "$root" <<'PYOTHER'
import datetime,json,sqlite3,sys
from pathlib import Path
from zoneinfo import ZoneInfo
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
ids=json.loads((root/'conversation-ids.json').read_text()); zone=ZoneInfo('Europe/Paris')
tomorrow=datetime.datetime.now(zone).date()+datetime.timedelta(days=1)
expected=int(datetime.datetime.combine(tomorrow,datetime.time(9),zone).timestamp())
assert c.execute('SELECT due_at,time_zone,state FROM reminders WHERE message_id=?',(ids['other'],)).fetchone()==(expected,'Europe/Paris','pending')
assert c.execute('SELECT count(*) FROM reminders').fetchone()[0]==2
(root/'reminder-before-restart.json').write_text(json.dumps(c.execute('SELECT id,message_id,due_at,time_zone,generation,state FROM reminders ORDER BY id').fetchall()))
print('A different original received its own default date/time, without sharing the prior selection.',flush=True)
PYOTHER
# Normal last-window-close must exit when background operation is disabled.
swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
for _ in {1..100}; do
  kill -0 "$app" 2>/dev/null || break
  sleep .1
done
if kill -0 "$app" 2>/dev/null; then
  printf '%s\n' 'The application did not exit after its last window closed.' >&2
  exit 1
fi
wait "$app"; app=''
start offline
check_seen unrelated other
wtype -M ctrl 3 -m ctrl -s 500
capture reminder-restart
python3 - "$root" <<'PYRESTART'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
before=json.loads((root/'reminder-before-restart.json').read_text())
after=c.execute('SELECT id,message_id,due_at,time_zone,generation,state FROM reminders ORDER BY id').fetchall()
assert after==[tuple(row) for row in before],(before,after)
print('Both original-specific reminders survived normal close and offline restart unchanged.',flush=True)
PYRESTART
# The private bus has no notification service. A retained withdrawal failure is
# shown above the list after rescheduling; it must not hide or discard reminders.
click 429 328
sleep .5
click 429 328
sleep .5
capture reminder-cancelled
python3 - "$root" <<'PYDONE'
import sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
assert c.execute('SELECT state FROM reminders').fetchall()==[('cancelled',),('cancelled',)]
assert c.execute('SELECT count(*) FROM drafts').fetchone()[0]==0
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
for name in ['initial-app.log','offline-app.log']:
    text=(root/name).read_text().lower()
    assert 'panicked at' not in text and 'fatal error' not in text,name
print('Cancellation preserved both originals, their Seen state, and the empty draft/outbox state.',flush=True)
PYDONE
check_seen unrelated other
if [[ ${MAILSTRONAUT_REMINDER_PRESETS_GUI:-0} == 1 ]]; then
  cp "$config" "$root/pre-timezone.nix"
  click 80 155
  click 350 150
  sleep .6
  copy_body reminder-presets 'Synthetic unrelated same-subject message.'
  click 1039 244
  wtype -s 200 -k Down -k Down -k Return -s 900
  python3 - "$root" <<'PYEVE'
import datetime as dt, json, sqlite3, sys
from pathlib import Path
from zoneinfo import ZoneInfo
root=Path(sys.argv[1]); mid=json.loads((root/'conversation-ids.json').read_text())['unrelated']
zone=ZoneInfo('Europe/Paris')
now=dt.datetime.now(zone)
evening=dt.datetime.combine(now.date(),dt.time(18),zone)
db=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
due, name, state=db.execute('SELECT due_at,time_zone,state FROM reminders WHERE message_id=?',(mid,)).fetchone()
if now < evening:
    assert (due,name,state)==(int(evening.timestamp()),zone.key,'pending')
else:
    assert state=='cancelled'
print('This evening uses the original civil day and refuses a past time.', flush=True)
PYEVE
  click 1039 244
  wtype -s 200 -k Down -k Down -k Down -k Return -s 900
  python3 - "$root" <<'PYTOM'
import datetime as dt, json, sqlite3, sys
from pathlib import Path
from zoneinfo import ZoneInfo
root=Path(sys.argv[1]); mid=json.loads((root/'conversation-ids.json').read_text())['unrelated']
zone=ZoneInfo('Europe/Paris')
day=dt.datetime.now(zone).date()+dt.timedelta(days=1)
expected=(int(dt.datetime.combine(day,dt.time(9),zone).timestamp()),zone.key,'pending')
db=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
assert db.execute('SELECT due_at,time_zone,state FROM reminders WHERE message_id=?',(mid,)).fetchone()==expected
(root/'paris-reminder.json').write_text(json.dumps(expected))
print('Tomorrow morning uses the next civil day and configured time.',flush=True)
PYTOM
  sed -i 's/timeZone = "Europe\/Paris";/timeZone = "UTC";/' "$config"
  wtype -M ctrl -M shift r -m shift -m ctrl -s 1200
  click 350 274
  sleep .6
  copy_body reminder-utc 'Synthetic other-account reply.'
  click 1039 244
  wtype -s 200 -k Down -k Down -k Down -k Return -s 900
  python3 - "$root" <<'PYZONE'
import datetime as dt, json, sqlite3, sys
from pathlib import Path
root=Path(sys.argv[1]); ids=json.loads((root/'conversation-ids.json').read_text())
db=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
def reminder(mid):
    return db.execute('SELECT due_at,time_zone,state FROM reminders WHERE message_id=?',(mid,)).fetchone()
day=dt.datetime.now(dt.timezone.utc).date()+dt.timedelta(days=1)
assert reminder(ids['other'])==(int(dt.datetime.combine(day,dt.time(9),dt.timezone.utc).timestamp()),'UTC','pending')
assert list(reminder(ids['unrelated']))==json.loads((root/'paris-reminder.json').read_text())
print('Timezone reload changes new presets and preserves an existing reminder instant and zone.',flush=True)
PYZONE
  capture reminder-timezone-reload
  cp "$root/pre-timezone.nix" "$config"
  check_seen unrelated other
fi
[[ $(sha256sum "$config") == "$original_config" ]]
printf 'Native reminder acceptance: %s\n' "$root"
