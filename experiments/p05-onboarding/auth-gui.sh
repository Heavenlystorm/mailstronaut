#!/usr/bin/env bash
# Called by credentials.sh on its private bus/keyring; only loopback synthetic mail.
set -euo pipefail
if [[ -n ${MAILSTRONAUT_GUI_OAUTH_FIXTURE:-} ]]; then
  exec bash "$(dirname "${BASH_SOURCE[0]}")/oauth-gui.sh" "$@"
fi
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private profile}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/onboarding-auth"
mkdir -p "$artifacts"
imap_user=synthetic-onboarding-user
imap_password=SYNTHETIC-ONBOARDING-PASSWORD-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../p04-sync/dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/../p04-sync/dovecot.sh"
cleanup() {
  if declare -F session_cleanup > /dev/null; then session_cleanup; fi
  dovecot_cleanup
}
trap cleanup EXIT
seed_message welcome <<MAIL
From: Synthetic Correspondent <sender@example.test>
To: reader@example.test
Subject: Synthetic onboarding welcome
Message-ID: <onboarding-welcome@example.test>
Date: $(date -R)
Content-Type: text/plain; charset=utf-8

Synthetic onboarding message; it must remain unread.
MAIL
dovecot_start
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.pollSeconds = 300;
  accounts = [{
    id = "onboarding-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic reader";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $imap_port; security = "tls"; };
    smtp = { host = "smtp.example.invalid"; port = 587; security = "starttls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "onboarding-fresh-reference"; };
  }];
}
NIX
original=$(sha256sum "$config")
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap 'printf "Onboarding auth failed at line %s: %s\n" "$LINENO" "$artifacts" >&2' ERR
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
assert_empty() {
  python3 - "$database" <<'PYEMPTY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute('SELECT count(*) FROM messages').fetchone()[0]==0
PYEMPTY
}
start missing
sleep 1
assert_empty
# No credential exists: automatic sync must stop. Saving one is the only wake.
if secret-tool lookup application "$(cat assets/app-id)" account onboarding-synthetic \
  reference onboarding-fresh-reference purpose password > /dev/null; then
  printf '%s\n' 'The private profile unexpectedly has a saved password.' >&2
  exit 1
fi
wtype -M ctrl , -m ctrl -s 400
click 300 203
capture missing
click 700 712
wtype -s 200 "$imap_password" -s 200
click 1190 712
for _ in {1..100}; do
  if python3 - "$database" <<'PYSYNC'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
sys.exit(0 if c.execute('SELECT count(*) FROM messages').fetchone()[0]==1 else 1)
PYSYNC
  then break; fi
  kill -0 "$app"; sleep .1
done
python3 - "$database" <<'PYFINAL'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute('SELECT count(*) FROM messages').fetchone()[0]==1
assert c.execute('SELECT count(*) FROM memberships WHERE seen=1').fetchone()[0]==0
assert c.execute('SELECT count(*) FROM outbox').fetchone()[0]==0
PYFINAL
flags=$(server_flags INBOX onboarding)
[[ $flags != *S* ]]
[[ $(sha256sum "$config") == "$original" ]]
capture saved-and-synced
close_window
printf 'Saving encrypted credentials woke the stopped account; automatic PEEK sync left mail unread: %s\n' "$artifacts"
