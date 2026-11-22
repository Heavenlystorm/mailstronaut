#!/usr/bin/env bash
# Native browser handoff and encrypted refresh against an existing localhost IdP fixture.
set -euo pipefail
binary=$(realpath "${1:?prebuilt application}")
root=$(realpath "${2:?private profile}")
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$root/runtime/bus" ]]
[[ $XDG_DATA_HOME == "$root/data" ]]
fixture=$(realpath "${MAILSTRONAUT_GUI_OAUTH_FIXTURE:?prebuilt OAuth test executable}")
artifacts="$root/onboarding-oauth"
mkdir -p "$artifacts"
port_file="$artifacts/port"
MAILSTRONAUT_GUI_OAUTH_PORT_FILE="$port_file" "$fixture" gui_loopback_server --ignored --exact --nocapture > "$artifacts/idp.log" 2>&1 &
idp=$!
cleanup() {
  if declare -F session_cleanup > /dev/null; then session_cleanup; fi
  if [[ -n $idp ]]; then
    kill "$idp" 2>/dev/null || true
    wait "$idp" 2>/dev/null || true
  fi
}
trap cleanup EXIT
for _ in {1..100}; do
  [[ -s $port_file ]] && break
  kill -0 "$idp"; sleep .05
done
port=$(< "$port_file")
[[ $port =~ ^[0-9]+$ ]]
unused=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
ca=$(realpath crates/mailstronaut-mail/tests/certs/ca.pem)
config="$artifacts/config.nix"
cat > "$config" <<NIX
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [{
    id = "oauth-synthetic";
    provider = "manual";
    address = "reader@example.test";
    displayName = "Synthetic OAuth";
    userName = "reader@example.test";
    imap = { host = "localhost"; port = $unused; security = "tls"; };
    smtp = { host = "localhost"; port = $unused; security = "tls"; };
    tls.caFiles = [ "$ca" ];
    auth = {
      kind = "oauth2";
      provider = "custom";
      clientId = "synthetic-public-client";
      secretRef = "synthetic-oauth";
      authorizationUrl = "https://localhost:$port/authorize";
      tokenUrl = "https://localhost:$port/token";
      scopes = [ "mail.read" "mail.send" ];
      sasl = "xoauth2";
    };
  }];
}
NIX
source_hash=$(sha256sum "$config")
key=(application "$(cat assets/app-id)" account oauth-synthetic reference synthetic-oauth purpose oauth-refresh)
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
trap 'printf "Native OAuth failed at line %s: %s\n" "$LINENO" "$artifacts" >&2' ERR
has_grant() {
  secret-tool lookup "${key[@]}" 2>/dev/null | grep -qF -- "$1"
}
if secret-tool lookup "${key[@]}" > /dev/null 2>&1; then
  printf '%s\n' 'The synthetic account already has a stored grant.' >&2
  exit 1
fi
start signed-out
wtype -M ctrl , -m ctrl -s 500
click 300 203
capture signed-out
click 510 712
for _ in {1..150}; do
  has_grant SYNTHETIC-PORTAL-REFRESH-ONE && break
  kill -0 "$app" "$idp"; sleep .1
done
has_grant SYNTHETIC-PORTAL-REFRESH-ONE
capture signed-in
close_window
start after-restart
wtype -M ctrl r -m ctrl -s 700
for _ in {1..150}; do
  has_grant SYNTHETIC-PORTAL-REFRESH-TWO && break
  kill -0 "$app"; sleep .1
done
has_grant SYNTHETIC-PORTAL-REFRESH-TWO
wait "$idp"
idp=''
[[ $(sha256sum "$config") == "$source_hash" ]]
if grep -Eq 'SYNTHETIC-PORTAL-(CODE|REFRESH)|SYNTHETIC-OAUTH-ACCESS' "$artifacts"/*-app.log; then
  printf '%s\n' 'Synthetic OAuth credentials appeared in an application log' >&2
  exit 1
fi
capture refreshed-after-restart
close_window
printf 'Native OAuth handoff, encrypted grant and explicit refresh after restart passed: %s\n' "$artifacts"
