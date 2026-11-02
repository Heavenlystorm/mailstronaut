#!/usr/bin/env bash
# Called only by credentials.sh: forwarding an opened message starts a local draft with the
# original as text, attaches nothing, sends nothing, and carries no reply headers. The source is a
# synthetic message on a private Dovecot server.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/forward-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
# The one synthetic message in the inbox that the walkthrough forwards.
seed_message forward-source <<'MAIL'
From: Correspondent <correspondent@example.test>
To: reader@example.test
Subject: Synthetic forward source
Message-ID: <forward-source@example.test>
Date: Mon, 02 Nov 2026 10:00:00 +0000
Content-Type: text/plain; charset=utf-8

Synthetic forwarded body marker-forward.
MAIL
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 3600;
  accounts = [{
    id = "forward-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "forward-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic forward fixture' \
  application "$(cat assets/app-id)" account forward-synthetic reference forward-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap 'session_cleanup; dovecot_cleanup' EXIT
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..900}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
forward_draft="SELECT json_extract(content,'\$.subject') FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1"
outbox_rows="SELECT count(*) FROM outbox"

dovecot_start
start forward
await 'the synthetic message reached the cache' 1 "SELECT count(*) FROM messages WHERE subject='Synthetic forward source'"
# Focus the list, select the message, and open it explicitly.
wtype -s 200 -M ctrl l -m ctrl -s 300 -k Down -s 300 -k Return -s 900
capture forward-opened
wtype -s 200 -M alt f -m alt -s 1200
await 'forwarding opens one local draft' 1 "SELECT count(*) FROM drafts WHERE closed=0"
[[ $(query "$forward_draft") == 'Fwd: Synthetic forward source' ]]
printf '%s\n' 'OK: the forward draft carries a Fwd: subject'
[[ $(query "SELECT instr(content,'Forwarded as text') > 0 FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1") == 1 ]]
[[ $(query "SELECT instr(content,'Synthetic forwarded body marker-forward.') > 0 FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1") == 1 ]]
printf '%s\n' 'OK: the forward draft quotes the original as text'
[[ $(query "SELECT json_extract(content,'\$.in_reply_to') IS NULL FROM drafts WHERE closed=0 ORDER BY rowid DESC LIMIT 1") == 1 ]]
printf '%s\n' 'OK: forwarding carries no reply headers'
[[ $(query "$outbox_rows") == 0 ]]
printf '%s\n' 'OK: forwarding submitted nothing'
sleep 1
capture forward-draft
close_window

if grep -l 'SYNTHETIC-SMTP-SECRET-NOT-REAL' "$artifacts"/*.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in a log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Forward acceptance: %s\n' "$artifacts"
