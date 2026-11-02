#!/usr/bin/env bash
# Sourced by the private mailbox harness after seeding then hiding its declaration.
set -euo pipefail
: "${root:?private profile}" "${config:?native source}" "${binary:?application}"
trap 'printf "Onboarding failed at line %s; profile: %s\n" "$LINENO" "$root" >&2' ERR
[[ ! -e $config ]]
capture onboarding-start
wtype -M ctrl n -m ctrl -s 300
check_seen
click 850 605
sleep 1
"$binary" config check --path "$config"
python3 - "$config" <<'PYINIT'
from pathlib import Path
import stat,sys
source=Path(sys.argv[1])
assert source.read_text().startswith('# Mailstronaut editable configuration\n')
assert stat.S_IMODE(source.stat().st_mode)==0o600
PYINIT
# This synthetic profile never connects to a real account or prompts a keyring.
printf '{ schemaVersion = 1; appearance = "light"; sync.automatic = false; }\n' > "$root/declared.nix"
"$binary" config reconcile --declared "$root/declared.nix" --path "$config"
wtype -M ctrl -M shift r -m shift -m ctrl -s 700
click 300 203
original=$(sha256sum "$config")
capture onboarding-form
click 600 334; wtype 'synthetic-manual'
click 1010 334; wtype 'Synthetic Reader'
click 700 420; wtype 'reader@example.test'
click 650 486; wtype 'imap.example.invalid'
click 650 620; wtype -s 200 'smtp.example.invalid' -s 400
[[ $(sha256sum "$config") == "$original" ]]
swaymsg seat seat0 cursor set 1200 700 > /dev/null
for _ in {1..5}; do wlrctl pointer scroll 100 0; sleep .2; done
sleep .5
capture onboarding-manual-bottom
click 850 724
swaymsg seat seat0 cursor set 1274 650 > /dev/null
for _ in {1..5}; do wlrctl pointer scroll 150 0; sleep .2; done
sleep .5
capture onboarding-advanced
click 600 354; wtype -s 100 'synthetic-incoming' -s 100
click 1000 354; wtype -s 100 'synthetic-outgoing' -s 100
click 453 393
click 600 465; wtype -s 100 'alias@example.test' -s 100
click 600 655; wtype -s 100 'Sent items' -s 100
click 1000 655; wtype -s 100 'Archive items' -s 100
click 600 722; wtype -s 100 'Deleted items' -s 100
[[ $(sha256sum "$config") == "$original" ]]
capture onboarding-staged
click 850 772
sleep 1
"$binary" config check --path "$config"
timeout 4 "$binary" --internal-evaluate-config < "$config" > "$root/values.json"
python3 - "$root/values.json" <<'PYMANUAL'
import json,sys
c=json.load(open(sys.argv[1]))['Success']; assert not c['sync']['automatic']
a,=c['accounts']; assert a['id']=='synthetic-manual'
assert a['userName']=='synthetic-incoming' and a['smtpUserName']=='synthetic-outgoing'
assert a['imap']==dict(host='imap.example.invalid',port=993,security='tls')
assert a['smtp']==dict(host='smtp.example.invalid',port=587,security='starttls')
assert a['aliases']==['alias@example.test']
assert a['folders']['sent']=='Sent items' and a['folders']['archive']=='Archive items'
assert a['folders']['trash']=='Deleted items'
assert a['auth']['secretRef']!=a['smtpAuth']['secretRef']
PYMANUAL
check_seen
capture onboarding-manual-saved
click 300 239
capture onboarding-reset
click 850 267
capture onboarding-provider-menu
click 850 308
click 600 334; wtype -s 100 'synthetic-google' -s 100
click 1010 334; wtype -s 100 'Synthetic Google' -s 100
click 700 420; wtype -s 100 'reader@gmail.com' -s 100
# Custom registrations are an explicit advanced override, not ordinary sign-in.
wtype -k Tab -k Return -s 300 -k Tab -s 100 'synthetic-registered-public-client' -s 100
capture onboarding-google-staged
# Return to Advanced, collapse it, then activate Add through native Tab/Enter.
wtype -M shift -k Tab -m shift -k Return -s 200 -k Tab -k Return -s 1000
timeout 4 "$binary" --internal-evaluate-config < "$config" > "$root/google-values.json"
python3 - "$root/google-values.json" <<'PYGOOGLE'
import json,sys
a=json.load(open(sys.argv[1]))['Success']['accounts']; assert len(a)==2
g=a[1]; assert g['id']=='synthetic-google' and g['provider']=='google'
assert g['auth']['kind']=='oauth2' and g['auth']['provider']=='google'
assert g['auth']['clientId']=='synthetic-registered-public-client'
PYGOOGLE
capture onboarding-google-saved
click 300 275
click 850 267
click 850 338
click 600 334; wtype -s 100 'synthetic-icloud' -s 100
click 1010 334; wtype -s 100 'Synthetic iCloud' -s 100
click 700 420; wtype -s 100 'reader@icloud.com' -s 100
capture onboarding-icloud-staged
wtype -k Tab -k Tab -k Return -s 1000
timeout 4 "$binary" --internal-evaluate-config < "$config" > "$root/all-values.json"
python3 - "$root/all-values.json" <<'PYCLOUD'
import json,sys
a=json.load(open(sys.argv[1]))['Success']['accounts']; assert len(a)==3
c=a[2]; assert c['id']=='synthetic-icloud' and c['provider']=='icloud'
assert c['auth']['kind']=='password' and c['userName']=='reader@icloud.com'
refs=[x['auth']['secretRef'] for x in a]+[a[0]['smtpAuth']['secretRef']]
assert len(set(refs))==4 and all(len(ref)==36 for ref in refs)
PYCLOUD
capture onboarding-icloud-saved
check_seen
"$binary" config export --path "$config" > "$root/exported.nix"
"$binary" config export --path "$root/exported.nix" > "$root/reimported.nix"
cmp "$root/exported.nix" "$root/reimported.nix"
cp "$config" "$root/saved-config.nix"
original=$(sha256sum "$config")
# Duplicate declarations are rejected visibly without clearing the current accounts.
click 300 311
click 850 267
click 850 338
click 600 334; wtype -s 100 'synthetic-icloud' -s 100
click 1010 334; wtype -s 100 'Duplicate' -s 100
click 700 420; wtype -s 100 'duplicate@icloud.com' -s 100
wtype -k Tab -k Tab -k Return -s 800
[[ $(sha256sum "$config") == "$original" ]]
capture onboarding-duplicate
stop
start offline
wtype -M ctrl , -m ctrl -s 500
capture onboarding-restarted
cmp "$config" "$root/saved-config.nix"
check_seen
printf 'Native first-run, manual/Google/iCloud declarations, keyboard Save, auth routing, duplicate refusal, native export and restart passed: %s\n' "$root"
