#!/usr/bin/env bash
# Invoked by the same-home ownership fixture under a private session bus.
set -euo pipefail
binary=${1:?HM-installed binary} config=${2:?HM config} artifacts=${3:?private artifacts}
trap 'session_cleanup' EXIT
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../wayland-session.sh"
filter() {
  click 310 126
  wtype -M ctrl a -m ctrl -k BackSpace -s 150 "$1" -s 350
}
start mutable
window_open
wtype -M ctrl , -m ctrl -s 500
filter 'Poll interval'
click 1200 258
wtype -M ctrl a -m ctrl '120' -k Return -s 200
filter 'Include message previews'
click 1245 258
click 275 783
sleep 1
"$binary" config export --path "$config" > "$artifacts/saved.nix"
grep -Fq 'pollSeconds = 120;' "$artifacts/saved.nix"
grep -Fq 'showPreview = true;' "$artifacts/saved.nix"
capture ownership-saved
close_window
start restarted
window_open
wtype -M ctrl , -m ctrl -s 400 -M ctrl -M shift e -m shift -m ctrl -s 500
click 827 541
timeout 5 wl-paste --no-newline > "$artifacts/copied-export.nix"
cmp "$artifacts/saved.nix" "$artifacts/copied-export.nix"
capture ownership-restarted-export
close_window
