#!/usr/bin/env bash
# Sourced by wayland.sh after starting its private synthetic profile.
set -euo pipefail
: "${root:?private fixture root}"
: "${config:?synthetic native configuration}"
wtype -s 200 -M ctrl l -m ctrl -k Down -k Down -k Down -k Return -s 700
copy_body navigation-second 'Synthetic second reply.'
check_seen second
capture navigation-thread
click 650 240
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
click 900 485
check_seen first second
capture navigation-details
click 900 485
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
click 80 332
sleep .4
check_seen first second page-098
wtype -M ctrl l -m ctrl -k Down -k Return -s 500
copy_body navigation-sent 'Synthetic sent parent.'
capture navigation-sent
check_seen first second page-098
click 95 370
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
