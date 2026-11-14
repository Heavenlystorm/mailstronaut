#!/usr/bin/env bash
# Called only by credentials.sh: multilingual meaning search in the real native field,
# against mail fetched from a real server. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/meaning-gui"
model=$(realpath "${MAILSTRONAUT_TEST_MODEL_DIR:?explicit pinned local model}")
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
# Mailstronaut editable configuration
{
  schemaVersion = 1;
  managed = false;
  search = { semantic = true; modelDirectory = "$model"; };
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
  for _ in {1..300}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  capture timeout
  printf 'Timed out: %s\n' "$what" >&2; return 1
}
type_query() {
  wtype -M ctrl k -m ctrl -M ctrl a -m ctrl -k BackSpace -s 150
  wtype "$1" -s "${2:-2000}"
}
seen() { query 'SELECT count(*) FROM memberships WHERE seen=1'; }
start initial
await 'all synthetic mail fetched' 4 'SELECT count(*) FROM messages'
await 'word index ready' 0 'SELECT count(*) FROM search_pending'
await 'meaning queue committed' 0 'SELECT count(*) FROM semantic_pending'
[[ $(seen) == 0 ]]
type_query 'reminder about doctoral school re-enrollment'
click 350 150
[[ $(seen) == 0 ]]
capture word-results
# Related suggestions are an explicit mode, not silently asserted word matches.
click 975 66
# Retire several queries while model work can be queued; only the last query may
# populate the list, and none of this work is an explicit message open.
type_query 'conference trip' 400
type_query 'grocery delivery' 400
type_query 'reminder about doctoral school re-enrollment' 15000
capture meaning-results
# The former second-row distractor must be absent after the similarity floor.
click 350 350
[[ $(seen) == 0 ]]
# Passive keyboard selection and model work must never mark Seen.
wtype -k Down -k Up -s 500
[[ $(seen) == 0 ]]
click 350 210
await 'English meaning query opens French notice first' 1 "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Réinscription en école doctorale'"
[[ $(seen) == 1 ]]
capture meaning-opened
# Explicit quotes retain word/phrase intent, even with the model enabled.
type_query '"reminder about doctoral school re-enrollment"'
click 350 150
[[ $(seen) == 1 ]]
capture exact-words-only
# Normal typo-tolerant search continues to use the existing library.
type_query 'budegt'
click 975 66
sleep 2
click 350 150
await 'word matcher still finds budget' 1 "SELECT count(*) FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Quarterly budget review'"
[[ $(seen) == 2 ]]
# Reuse standard Settings fields and native Nix ownership, not a preference overlay.
wtype -M ctrl , -m ctrl -s 500
click 310 126
wtype 'meaning' -s 500
capture meaning-settings
click 1250 286
click 275 783
for _ in {1..150}; do
  grep -q 'semantic = false' "$config" && break
  kill -0 "$app"; sleep .2
done
grep -q 'semantic = false' "$config"
capture meaning-disabled-settings
wtype -M ctrl 1 -m ctrl -s 400
type_query 'reminder about doctoral school re-enrollment'
click 350 150
[[ $(seen) == 2 ]]
capture meaning-disabled
# Restart with a missing model. Cached semantic vectors must not fake availability;
# lexical search and normal mail remain usable.
close_window
python3 - "$config" "$model" <<'PYTHON'
import pathlib,sys
path=pathlib.Path(sys.argv[1])
text=path.read_text().replace('semantic = false','semantic = true').replace(sys.argv[2],sys.argv[2]+'-missing')
path.write_text(text)
PYTHON
start unavailable
type_query 'budegt'
click 350 150
[[ $(seen) == 2 ]]
capture model-unavailable-lexical
wtype -M ctrl , -m ctrl -s 500
click 310 126
wtype meaning -s 500
capture model-unavailable-settings
close_window
printf 'Native multilingual meaning retrieval, explicit-only Seen, exact intent, word search, Settings save and missing-model fallback passed: %s\n' "$artifacts"
