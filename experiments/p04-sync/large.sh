#!/usr/bin/env bash
# Called only by credentials.sh: mail with large attachments, against a real server. A 5 MiB
# message is kept whole, and a 30 MiB one keeps its text with its attachment left on the
# server. All mail is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
part_fixture=$(realpath "${MAILSTRONAUT_PART_FIXTURE:?prebuilt part download test executable}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/large-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
# A real multipart message with a base64 attachment of a known size and digest.
generate() {
  python3 - "$mail_root/new/$1.synthetic" "$1" "$2" "$3" <<'PY'
import base64, hashlib, random, sys
path, key, megabytes, subject = sys.argv[1:]
data = random.Random(key).randbytes(int(megabytes) * 1024 * 1024)
print(hashlib.sha256(data).hexdigest())
text = base64.b64encode(data).decode()
encoded = "\r\n".join(text[start:start + 76] for start in range(0, len(text), 76))
with open(path, "w", newline="") as out:
    out.write(
        f"From: Correspondent {key} <{key}@example.test>\r\nTo: reader@example.test\r\n"
        f"Subject: {subject}\r\nMessage-ID: <{key}@example.test>\r\n"
        "Date: Sun, 1 Nov 2026 09:00:00 +0100\r\nMIME-Version: 1.0\r\n"
        'Content-Type: multipart/mixed; boundary="synthetic"\r\n\r\n'
        "--synthetic\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
        f"Synthetic body marker-{key}. Bonjour, le rapport est joint.\r\n"
        "--synthetic\r\nContent-Type: application/pdf\r\n"
        f'Content-Disposition: attachment; filename="rapport-{key}.pdf"\r\n'
        "Content-Transfer-Encoding: base64\r\n\r\n"
        f"{encoded}\r\n--synthetic--\r\n"
    )
PY
}
small_digest=$(generate medium 5 'Rapport moyen')
large_digest=$(generate huge 30 'Rapport énorme')
seed_message plain <<MAIL
From: Correspondent plain <plain@example.test>
To: reader@example.test
Subject: An ordinary message
Message-ID: <plain@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic body marker-plain.
MAIL
touch -d '3 hours ago' "$mail_root/new/huge.synthetic"
touch -d '2 hours ago' "$mail_root/new/medium.synthetic"
touch -d '1 hour ago' "$mail_root/new/plain.synthetic"
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
printf '%s' "$imap_password" | secret-tool store --label='Synthetic large mail fixture' \
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
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}

start initial
await 'all three messages appeared, whatever their size' 3 'SELECT count(*) FROM messages'
await 'the 5 MiB message is kept whole' 0 "SELECT count(*) FROM messages WHERE subject='Rapport moyen' AND omitted IS NOT NULL"
await 'the 30 MiB message records what it left behind' 1 "SELECT count(*) FROM messages WHERE subject='Rapport énorme' AND omitted IS NOT NULL"
await 'neither was marked read by fetching' 0 'SELECT count(*) FROM memberships WHERE seen=1'
blobs=$(du -sm "$XDG_DATA_HOME/mailstronaut" | cut -f1)
(( blobs < 20 )) || { printf 'The store holds %s MiB, so the large attachment was kept\n' "$blobs" >&2; exit 1; }
printf 'OK: the store holds only %s MiB after caching 35 MiB of mail\n' "$blobs"
await 'the text of the large message is searchable' 1 "SELECT count(*) FROM message_search WHERE message_search MATCH 'huge' AND rowid IN (SELECT id FROM messages WHERE subject='Rapport énorme')"
sleep 2
capture listed

# The oldest row is the large one. Opening it shows its text and its attachment.
click 350 380
sleep 1.5
capture large-opened
await 'opening it marked it read' 1 "SELECT count(*) FROM effective_memberships s JOIN messages m ON m.id=s.message_id WHERE s.seen=1 AND m.subject='Rapport énorme'"
# The middle row keeps its attachment on this device, listed the ordinary way.
click 350 260
sleep 1.5
capture medium-opened
close_window

# Downloading the part against the same server. The application's chooser needs a desktop
# portal, so this drives the same calls that its Save action makes.
message_uid=$(python3 - "$database" <<'PY'
import sqlite3, sys
print(sqlite3.connect(sys.argv[1]).execute("SELECT s.uid FROM memberships s JOIN messages m ON m.id=s.message_id WHERE m.subject='Rapport énorme'").fetchone()[0])
PY
)
MAILSTRONAUT_PART_PORT=$imap_port MAILSTRONAUT_PART_UID=$message_uid MAILSTRONAUT_PART_DIGEST=$large_digest \
  MAILSTRONAUT_PART_USER=$imap_user MAILSTRONAUT_PART_PASSWORD=$imap_password \
  "$part_fixture" --ignored --exact a_large_part_downloads_intact_from_a_real_server --nocapture 2>&1 | grep -E '^OK|FAILED|panicked' || exit 1
: "$small_digest"

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native large mail acceptance: %s\n' "$artifacts"
