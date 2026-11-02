#!/usr/bin/env bash
# Sourced by wayland.sh after starting its private synthetic profile.
set -euo pipefail
: "${root:?private fixture root}"
: "${config:?synthetic native configuration}"
wtype -s 200 -M ctrl l -m ctrl -k Down -k Down -k Down -k Return -s 700
copy_body navigation-second 'Synthetic second reply.'
check_seen second
capture navigation-thread
click 650 284
sleep .5
check_seen second
capture navigation-expanded
# Expanding/focusing the metadata list and moving to a sibling remain passive.
wtype -s 200 -k Down -s 200
check_seen second
wtype -k Return -s 600
copy_body navigation-first 'Synthetic first reply.'
check_seen first second
capture navigation-first
click 912 528
check_seen first second
capture navigation-details
click 912 528
wtype -M ctrl -M shift d -m shift -m ctrl -s 400
capture navigation-dark
swaymsg output HEADLESS-1 mode 960x720 > /dev/null
sleep .5
capture navigation-narrow
swaymsg output HEADLESS-1 mode 1280x840 > /dev/null
wtype -M ctrl -M shift d -m shift -m ctrl -s 500
# Paging clears the original, never reuses it as authority, and stays bounded.
click 523 780
sleep .5
check_seen first second
capture navigation-page-two
[[ $(magick "$root/initial.png" "$root/navigation-page-two.png" -compose difference -composite -crop 700x680+570+110 -format '%[fx:mean<0.00001]' info:) == 1 ]]
wtype -M ctrl l -m ctrl -k Down -k Return -s 600
copy_body navigation-older 'Synthetic cached letter 098.'
check_seen first second page-098
wtype -M ctrl r -m ctrl -s 500 -M ctrl l -m ctrl -k Return -s 500
copy_body navigation-page-refresh 'Synthetic cached letter 098.'
check_seen first second page-098
click 239 780
sleep .5
capture navigation-page-one
[[ $(magick "$root/initial.png" "$root/navigation-page-one.png" -compose difference -composite -crop 700x680+570+110 -format '%[fx:mean<0.00001]' info:) == 1 ]]
# Folder and account routes are offline, explicit and independent of Message-ID.
click 80 350
sleep .4
check_seen first second page-098
wtype -M ctrl l -m ctrl -k Down -k Return -s 500
copy_body navigation-sent 'Synthetic sent parent.'
capture navigation-sent
check_seen first second page-098
click 95 388
sleep .4
wtype -M ctrl k -m ctrl 'cached letter' -k Return -s 400 -M ctrl l -m ctrl -k Down -k Return -s 400
check_seen first second page-098
capture navigation-scoped-empty
wtype -M ctrl k -m ctrl -M ctrl a -m ctrl 'other-account' -k Return -s 400 -M ctrl l -m ctrl -k Down -k Return -s 600
copy_body navigation-other 'Synthetic other-account reply.'
check_seen first second other page-098
capture navigation-other
# Disable the displayed account declaratively. Its cache survives, but its scope
# and original must retire rather than falling back to another account's UIDs.
sed -i '/{id = "other-synthetic"; address = "other@example.test";}/d' "$config"
wtype -M ctrl -M shift r -m shift -m ctrl -s 800
check_seen first second other page-098
capture navigation-disabled
[[ $(magick "$root/initial.png" "$root/navigation-disabled.png" -compose difference -composite -crop 700x680+570+110 -format '%[fx:mean<0.00001]' info:) == 1 ]]
wtype -M ctrl k -m ctrl -M ctrl a -m ctrl -k BackSpace -k Return -s 400
stop
start offline
check_seen first second other page-098
capture navigation-offline
# Explicit message actions: star and unread stay as chosen, and a reply is a local draft.
wtype -s 200 -M ctrl l -m ctrl -k Down -k Return -s 900
copy_body navigation-actions 'Synthetic unrelated same-subject message.'
wtype -s 200 -M alt s -m alt -s 500 -M alt u -m alt -s 900
capture navigation-actions
python3 - "$root" <<'PY'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); ids=json.loads((root/'conversation-ids.json').read_text())
c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
seen,starred=c.execute('SELECT min(seen),max(starred) FROM effective_memberships WHERE message_id=?',(ids['unrelated'],)).fetchone()
assert (seen,starred)==(0,1),(seen,starred)
print('Explicit unread and star recorded for the open original.',flush=True)
PY
wtype -s 200 -M alt r -m alt -s 1200
capture navigation-reply
python3 - "$root" <<'PY'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
content,closed=c.execute('SELECT content,closed FROM drafts').fetchone()
d=json.loads(content)
assert closed==0 and d['subject'].startswith('Re: ') and d['in_reply_to'] and 'alice@example.test' in d['to'], d
assert 'wrote:' in d['body'] and '> Synthetic unrelated' in d['body'], d
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
print('Reply opened an unsent local draft with ancestry and a quotation.',flush=True)
PY
# Remind me stays separate from flags and folders, and appears in its own list.
wtype -s 200 -M ctrl 1 -m ctrl -s 700 -M ctrl l -m ctrl -k Down -k Return -s 900
capture navigation-before-reminder
click 1039 244
sleep .4
capture navigation-reminder-menu
wtype -s 200 -k Down -k Return -s 900
wtype -s 200 -M ctrl 3 -m ctrl -s 700
capture navigation-reminders
python3 - "$root" <<'PY'
import sqlite3,sys,time
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1])/'data/mailstronaut/mail.sqlite3')
state,due=c.execute('SELECT state,due_at FROM reminders').fetchone()
assert state=='pending' and 3500<due-time.time()<3700,(state,due-time.time())
print('A one-hour reminder is durable and pending.',flush=True)
PY
python3 - "$root" <<'PY'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); ids=json.loads((root/'conversation-ids.json').read_text())
c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3')
assert len(ids)==110, len(ids)
assert c.execute('SELECT count(*) FROM messages WHERE account_id=?',('other-synthetic',)).fetchone()[0]==1
for log in ['initial-app.log','offline-app.log']:
    text=(root/log).read_text()
    assert not any(marker in text.lower() for marker in ['panicked at','fatal error']), log
print('Native account/folder, thread, page, scope-retirement and offline checks passed.',flush=True)
PY
printf 'Navigation acceptance: %s\n' "$root"
