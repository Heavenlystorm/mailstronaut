#!/usr/bin/env bash
# Sourced only by the private synthetic native Wayland mailbox harness.
set -euo pipefail
: "${root:?private profile}" "${config:?native source}" "${binary:?current application}"
trap 'printf "Preferences failed at line %s; profile: %s\n" "$LINENO" "$root" >&2' ERR
filter_preferences() {
  click 310 126
  wtype -M ctrl a -m ctrl -k BackSpace "$1" -s 300
}
edit_morning() {
  filter_preferences morning
  click 1170 258
  wtype '1030' -s 200
}
choose_paris() {
  filter_preferences 'time zone'
  click 1130 258
  wtype Paris -s 300
  capture preferences-zone-picker
  wtype -k Down -k Return -s 300 -k Escape -s 200
  capture preferences-zone-chosen
}
original=$(sha256sum "$config")
wtype -M ctrl , -m ctrl -s 500
capture preferences-source
filter_preferences morning
capture preferences-before-time
edit_morning
choose_paris
[[ $(sha256sum "$config") == "$original" ]]
# Discard must restore both persistent toolkit fields without writing the source.
click 430 783
filter_preferences morning
capture preferences-discarded
[[ $(magick "$root/preferences-before-time.png" -crop 64x20+1157+248 +repage -format '%#' info:) == $(magick "$root/preferences-discarded.png" -crop 64x20+1157+248 +repage -format '%#' info:) ]]
[[ $(sha256sum "$config") == "$original" ]]
edit_morning
choose_paris
filter_preferences evening
click 1170 258
wtype '1915' -s 200
filter_preferences 'Include message previews'
click 1245 258
filter_preferences 'Notify me about new mail and reminders'
click 1245 258
filter_preferences 'Poll interval'
click 1200 258
wtype -M ctrl a -m ctrl '120' -k Return -s 200
filter_preferences 'Keep running after closing'
click 1245 275
capture preferences-background
filter_preferences appearance
click 1225 274
wtype -k Up -k Return -s 300
capture preferences-staged
[[ $(sha256sum "$config") == "$original" ]]
click 275 783
sleep 1
"$binary" config check --path "$config"
"$binary" config export --path "$config" > "$root/preferences-saved.nix"
python3 - "$root/preferences-saved.nix" "$config" <<'PYSAVED'
import sys,stat,re
from pathlib import Path
text=Path(sys.argv[1]).read_text(); source=Path(sys.argv[2]); raw=source.read_text()
for fragment in ['appearance = "dark";', 'pollSeconds = 120;', 'automatic = false;',
                 'timeZone = "Europe/Paris";', 'morning = "10:30";', 'evening = "19:15";',
                 'notifications = { enabled = false; showPreview = true; };']:
    assert fragment in text, (fragment,text)
assert text.count('secretRef = "synthetic-conversation-password";') == 2
assert re.search(r'background\s*=\s*\{[^}]*enabled\s*=\s*true;', text), text
assert 'maxBodyBytes' not in raw and 'initialDays' not in raw, 'untouched defaults materialized'
assert stat.S_IMODE(source.stat().st_mode) == 0o600
PYSAVED
"$binary" config export --path "$root/preferences-saved.nix" > "$root/preferences-reimported.nix"
cmp "$root/preferences-saved.nix" "$root/preferences-reimported.nix"
check_seen
# A current Settings view exports exactly the saved effective source.
wtype -M ctrl -M shift e -m shift -m ctrl -s 400
click 827 541
timeout 5 wl-paste --no-newline > "$root/preferences-copied.nix"
cmp "$root/preferences-saved.nix" "$root/preferences-copied.nix"
wtype -k Escape -s 200
filter_preferences morning
capture preferences-saved-dark
stop
start offline
wtype -M ctrl , -m ctrl -s 400
filter_preferences morning
capture preferences-restarted
check_seen
"$binary" config export --path "$config" > "$root/preferences-after-restart.nix"
cmp "$root/preferences-saved.nix" "$root/preferences-after-restart.nix"
# A newer HM activation wins on disk. A stale GUI save must fail and preserve it.
filter_preferences 'Include message previews'
click 1245 258
printf '{ schemaVersion = 1; appearance = "light"; }\n' > "$root/declared.nix"
"$binary" config reconcile --declared "$root/declared.nix" --path "$config"
reactivated=$(sha256sum "$config")
click 275 783
sleep .6
[[ $(sha256sum "$config") == "$reactivated" ]]
capture preferences-conflict
wtype -M ctrl -M shift r -m shift -m ctrl -s 600
"$binary" config export --path "$config" > "$root/preferences-reactivated.nix"
grep -Fq 'appearance = "light";' "$root/preferences-reactivated.nix"
grep -Fq 'pollSeconds = 120;' "$root/preferences-reactivated.nix"
grep -Fq 'showPreview = true;' "$root/preferences-reactivated.nix"
filter_preferences morning
capture preferences-reactivated
# Invalid reloads retain the last valid runtime and expose a persistent Alert.
cp "$config" "$root/valid.nix"
printf '{ schemaVersion = 99; }\n' > "$config"
wtype -M ctrl -M shift r -m shift -m ctrl -s 600
capture preferences-invalid
wtype -M ctrl -M shift e -m shift -m ctrl -s 400
click 827 541
timeout 5 wl-paste --no-newline > "$root/preferences-last-valid.nix"
cmp "$root/preferences-reactivated.nix" "$root/preferences-last-valid.nix"
wtype -k Escape -s 200
cp "$root/valid.nix" "$config"
# Managed regular sources disable preference editing; keyring pages stay separate.
python3 - "$config" <<'PYMANAGED'
from pathlib import Path
import sys
p=Path(sys.argv[1]); p.write_text(p.read_text().replace('managed = false;', 'managed = true;'))
PYMANAGED
wtype -M ctrl -M shift r -m shift -m ctrl -s 500
managed=$(sha256sum "$config")
filter_preferences morning
click 1170 258
wtype '0800' -s 200
[[ $(sha256sum "$config") == "$managed" ]]
capture preferences-managed
# An unadopted authored expression is also readable but never rewritten by UI.
printf '{ schemaVersion = 1; sync.automatic = false; }\n' > "$config"
wtype -M ctrl -M shift r -m shift -m ctrl -s 500
authored=$(sha256sum "$config")
filter_preferences morning
click 1170 258
wtype '0700' -s 200
[[ $(sha256sum "$config") == "$authored" ]]
capture preferences-authored
check_seen
stop
if grep -Eiq 'panic|fatal|error:' "$root/initial-app.log" "$root/offline-app.log"; then
  printf '%s\n' 'Native preferences reported a runtime failure' >&2; exit 1
fi
printf 'Native preferences acceptance (staging/discard/save/restart/export/HM conflict/last-valid/ownership/Seen): %s\n' "$root"
