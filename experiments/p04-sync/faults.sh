#!/usr/bin/env bash
# Called only by credentials.sh: connections reset in the middle of a sync are retried without
# losing or duplicating mail, and an idle application uses almost no processor time. All mail
# is synthetic.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private fixture root}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
artifacts="$root/faults-gui"
mkdir -p "$artifacts"
database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
imap_user=synthetic-smtp-user
imap_password=SYNTHETIC-SMTP-SECRET-NOT-REAL
# shellcheck source-path=SCRIPTDIR
# shellcheck source=dovecot.sh
source "$(dirname "${BASH_SOURCE[0]}")/dovecot.sh"
# Six hundred recent messages: several windows, so that a reset lands in the middle of one.
bulk=600
python3 - "$mail_root/new" <<'PY'
import os, sys, time
directory = sys.argv[1]
os.makedirs(directory, exist_ok=True)
stamp = time.time() - 3 * 86400
date = time.strftime('%a, %d %b %Y %H:%M:%S +0000', time.gmtime(stamp))
for number in range(1, 601):
    path = os.path.join(directory, f'bulk-{number:04d}.synthetic')
    with open(path, 'w', encoding='utf-8') as file:
        file.write(
            f'From: Batch <batch@example.test>\nTo: reader@example.test\n'
            f'Subject: Synthetic batch {number}\nMessage-ID: <bulk-{number}@example.test>\n'
            f'Date: {date}\nContent-Type: text/plain; charset=utf-8\n\n'
            f'Synthetic batch body {number}.\n'
        )
    os.utime(path, (stamp, stamp))
PY
dovecot_start
# The application talks to a loopback proxy that resets the first six connections once the
# server has sent each of them 20 KB, which is past the TLS handshake and inside a window.
proxy_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -I "$(dirname "${BASH_SOURCE[0]}")/drop-proxy.py" "$proxy_port" "$imap_port" "$artifacts/resets.log" 6 20000 &
proxy=$!
for _ in {1..100}; do grep -qx ready "$artifacts/resets.log" 2> /dev/null && break; sleep .1; done
# A third account with a wrong password, reached through a proxy that counts its connections
# and never resets them.
wrong_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
python3 -I "$(dirname "${BASH_SOURCE[0]}")/drop-proxy.py" "$wrong_port" "$imap_port" "$artifacts/attempts.log" 0 0 &
attempts_proxy=$!
for _ in {1..100}; do grep -qx ready "$artifacts/attempts.log" 2> /dev/null && break; sleep .1; done
# A second account whose server is not listening: its failures must not hold up the first.
unreachable_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
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
    imap = { host = "localhost"; port = $proxy_port; security = "tls"; };
    smtp = { host = "localhost"; port = $proxy_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "dovecot-password"; };
  } {
    id = "unreachable-synthetic";
    provider = "manual";
    address = "unreachable@example.test";
    displayName = "Unreachable server";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $unreachable_port; security = "tls"; };
    smtp = { host = "localhost"; port = $unreachable_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "unreachable-password"; };
  } {
    id = "wrong-synthetic";
    provider = "manual";
    address = "wrong@example.test";
    displayName = "Wrong password";
    userName = "$imap_user";
    imap = { host = "localhost"; port = $wrong_port; security = "tls"; };
    smtp = { host = "localhost"; port = $wrong_port; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = { kind = "password"; secretRef = "wrong-password"; };
  }];
}
NIX
printf '%s' "$imap_password" | secret-tool store --label='Synthetic fault fixture' \
  application "$(cat assets/app-id)" account dovecot-synthetic reference dovecot-password purpose password
printf '%s' "$imap_password" | secret-tool store --label='Synthetic unreachable fixture' \
  application "$(cat assets/app-id)" account unreachable-synthetic reference unreachable-password purpose password
printf '%s' 'SYNTHETIC-WRONG-NOT-REAL' | secret-tool store --label='Synthetic wrong password fixture' \
  application "$(cat assets/app-id)" account wrong-synthetic reference wrong-password purpose password
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
cleanup() {
  session_cleanup
  kill "$proxy" "$attempts_proxy" 2> /dev/null || true
  dovecot_cleanup
}
trap cleanup EXIT

query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3 tries=${4:-900}
  for _ in $(seq 1 "$tries"); do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
inbox_finished="SELECT count(*) FROM backfill b JOIN folders f ON f.id=b.folder_id WHERE f.name='INBOX' AND b.finished_at IS NOT NULL"

# Processor time of the running application, measured over ten seconds from /proc.
idle_cpu() {
  python3 - "$app" <<'PY'
import os, sys, time
pid = sys.argv[1]
def ticks():
    fields = open(f'/proc/{pid}/stat').read().rsplit(')', 1)[1].split()
    return int(fields[11]) + int(fields[12])
hertz = os.sysconf('SC_CLK_TCK')
start, began = ticks(), time.monotonic()
time.sleep(10)
elapsed = time.monotonic() - began
print(f'{100 * (ticks() - start) / hertz / elapsed:.2f}')
PY
}

start initial
await 'the history reached its beginning despite the resets' 1 "$inbox_finished" 3000
[[ $(query 'SELECT count(*) FROM messages') == "$bulk" ]]
[[ $(query 'SELECT count(*) FROM memberships') == "$bulk" ]]
resets=$(grep -c '^reset' "$artifacts/resets.log" || true)
[[ $resets -ge 1 ]]
printf 'OK: %s connections were reset mid-sync, and each message was kept exactly once\n' "$resets"
[[ $(query "SELECT count(*) FROM messages WHERE account_id='dovecot-synthetic'") == "$bulk" ]]
kill -0 "$app"
printf '%s\n' 'OK: an unreachable account did not hold back the other one'
# A wrong password is classed as a stop: the account is tried once or twice, then left alone.
attempts=$(grep -c '^accept' "$artifacts/attempts.log" || true)
[[ $attempts -ge 1 && $attempts -le 3 ]]
[[ $(query "SELECT count(*) FROM messages WHERE account_id='wrong-synthetic'") == 0 ]]
printf 'OK: a wrong password was tried %s time(s) and then left alone\n' "$attempts"
sleep 2
capture resets
idle=$(idle_cpu)
awk -v value="$idle" 'BEGIN { exit !(value < 5) }'
printf 'OK: the idle application used %s%% of a processor over ten seconds\n' "$idle"
close_window

# Closing while the first sync is still running: the window closes and the sync stops at once.
rm -rf "$XDG_DATA_HOME/mailstronaut"
start closing
await 'the first sync began a history pass' 1 'SELECT count(*) FROM backfill'
pending=$(query "SELECT count(*) FROM backfill WHERE finished_at IS NULL" || echo 0)
started=$(date +%s%N)
close_window
elapsed=$(( ($(date +%s%N) - started) / 1000000 ))
[[ $elapsed -lt 5000 ]]
printf 'OK: closing took %s ms with %s unfinished history passes\n' "$elapsed" "$pending"

if grep -l "$imap_password" "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Synthetic credential appeared in an application log' >&2; exit 1
fi
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Native fault acceptance: %s\n' "$artifacts"
