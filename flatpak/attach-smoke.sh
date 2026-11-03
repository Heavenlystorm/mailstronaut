#!/usr/bin/env bash
# Called only by credentials.sh: inside the installed Flatpak, an account whose password sits in the
# encrypted keyring is active through the Secret portal, and the composer attaches a file through the
# FileChooser portal to a draft. The Secret and FileChooser portals are the protocol-accurate fixture on
# this private bus. The keyring, password and attachment are synthetic, and no server is contacted.
set -euo pipefail
: "${1:?flatpak marker}"
root=$(realpath "${2:?private fixture root}")
artifacts="$root/flatpak-attach"
inputs="$artifacts/inputs"
home="$artifacts/home"
mkdir -p "$home" "$inputs"
flatpak_user_dir=${MAILSTRONAUT_FLATPAK_USER_DIR:?isolated Flatpak installation}
chooser_fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt application test executable}")
seed_fixture=$(realpath "${MAILSTRONAUT_SEED_FIXTURE:?prebuilt keyring helper test executable}")
# The sandboxed application keeps its data under its private HOME, and its keyring beside it.
data="$home/.var/app/io.github.eduardz1.Mailstronaut/data"
database="$data/mailstronaut/mail.sqlite3"
keyring="$data/keyrings/default.keyring"
# The sandbox sees only the read-only inputs directory, so the declaration lives there at the same path.
config="$inputs/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = true;
  sync.pollSeconds = 3600;
  accounts = [{
    id = "flatpak-attach";
    provider = "manual";
    address = "sender@example.test";
    displayName = "Synthetic sender";
    userName = "synthetic-smtp-user";
    imap = { host = "imap.example.invalid"; port = 993; security = "tls"; };
    smtp = { host = "imap.example.invalid"; port = 465; security = "tls"; };
    auth = { kind = "password"; secretRef = "flatpak-attach-password"; };
  }];
}
NIX
# The Secret portal's synthetic application secret, which the fixture serves on the private bus.
printf '%s' 'SYNTHETIC-PORTAL-SECRET-NOT-REAL-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ' \
  > "$artifacts/portal-secret"
# Seed the synthetic password into the keyring the sandbox will open with that secret.
MAILSTRONAUT_SEED_KEYRING="$keyring" MAILSTRONAUT_SEED_SECRET="$artifacts/portal-secret" \
  MAILSTRONAUT_SEED_PASSWORD='SYNTHETIC-PASSWORD-NOT-REAL' \
  "$seed_fixture" --ignored --exact seed_portal_keyring --nocapture > "$artifacts/seed.log" 2>&1
[[ -s $keyring ]]
printf '%s\n' 'OK: the portal-backed keyring holds the synthetic password'
# The installed application in its sandbox: a private HOME, network unshared, and read-only access to
# the synthetic inputs only.
wrapper="$artifacts/mailstronaut-flatpak"
cat > "$wrapper" <<WRAP
#!/usr/bin/env bash
exec env HOME="$home" FLATPAK_USER_DIR="$flatpak_user_dir" flatpak run \
  --command=mailstronaut --unshare=network --filesystem="$inputs:ro" \
  io.github.eduardz1.Mailstronaut "\$@"
WRAP
chmod +x "$wrapper"
binary=$wrapper
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../experiments/wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../experiments/wayland-session.sh"
portal=''
cleanup() {
  session_cleanup
  if [[ -n "$portal" ]]; then kill "$portal" 2>/dev/null || true; wait "$portal" 2>/dev/null || true; fi
}
trap cleanup EXIT
# The portal fixture on this private bus. Its root is this check's directory, which the bus must contain.
MAILSTRONAUT_PORTAL_ROOT=$artifacts "$chooser_fixture" portal_fixture::serve_portals --ignored --exact \
  > "$artifacts/portal.log" 2>&1 & portal=$!
for _ in {1..100}; do [[ -f "$artifacts/portal-ready" ]] && break; kill -0 "$portal"; sleep .1; done
[[ -f "$artifacts/portal-ready" ]]
attachment="$inputs/synthetic receipt · été.txt"
printf 'Synthetic receipt contents · été.\n' > "$attachment"
python3 -c 'import sys,urllib.parse; print("file://"+urllib.parse.quote(sys.argv[1]))' "$attachment" \
  > "$artifacts/chooser-answer"
query() { python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])' "$database" "$1" 2>/dev/null; }
await() {
  local what=$1 expected=$2 sql=$3
  for _ in {1..150}; do
    [[ $(query "$sql" || true) == "$expected" ]] && { printf 'OK: %s\n' "$what"; return 0; }
    kill -0 "$app"; sleep .2
  done
  printf 'Timed out waiting for: %s (last value %s)\n' "$what" "$(query "$sql" || true)" >&2
  return 1
}
# Composer positions match the desktop walkthroughs: the header's attach button.
attach_x=568
attach_y=134

start attach
if window_open; then printf '%s\n' 'OK: the sandboxed window opened'; else printf '%s\n' 'The sandboxed window did not open' >&2; fi
capture attach-started
wtype -s 200 -M ctrl n -m ctrl -s 700
wtype -s 100 'to@example.test' -s 100 -k Tab -s 100
wtype 'Synthetic flatpak attach · Unicode subject' -k Tab -s 100 'Synthetic body with a file.'
sleep 2
await 'the sandboxed composer saved the draft' 1 "SELECT count(*) FROM drafts WHERE closed=0"
click "$attach_x" "$attach_y"
await 'the file chooser portal attached one file inside the sandbox' 1 "SELECT count(*) FROM draft_attachments"
sleep 1
capture flatpak-attached
close_window

if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Flatpak attach acceptance: %s\n' "$artifacts"
