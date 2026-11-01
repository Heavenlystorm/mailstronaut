#!/usr/bin/env bash
# Sourced by the synthetic mailbox harness on its private Wayland session/bus.
set -euo pipefail
: "${root:?private profile}" "${config:?native source}" "${binary:?current application}"
before=$(sha256sum "$config")
wtype -M ctrl , -m ctrl -s 400 -M ctrl -M shift e -m shift -m ctrl -s 600
capture export-dialog
if [[ ${MAILSTRONAUT_EXPORT_CAPTURE_ONLY:-0} == 1 ]]; then
  printf 'Export inspection: %s\n' "$root"
  return 0
fi
"$binary" config export --path "$config" > "$root/cli-export.nix"
click 827 541
timeout 5 wl-paste --no-newline > "$root/copied-export.nix"
cmp "$root/copied-export.nix" "$root/cli-export.nix"
# Native read-only selection/copy is independent of copying through the footer.
click 620 340
wtype -M ctrl a -m ctrl -M ctrl c -m ctrl -s 200
timeout 5 wl-paste --no-newline > "$root/selected-export.nix"
cmp "$root/selected-export.nix" "$root/cli-export.nix"
wtype 'this must not replace the snapshot' -M ctrl a -m ctrl -M ctrl c -m ctrl -s 200
timeout 5 wl-paste --no-newline > "$root/readonly-export.nix"
cmp "$root/readonly-export.nix" "$root/cli-export.nix"
"$binary" config check --path "$root/readonly-export.nix"
check_seen
[[ $(sha256sum "$config") == "$before" ]]
capture export-readonly
wtype -k Escape -s 300 -M ctrl -M shift d -m shift -m ctrl -s 400 -M ctrl -M shift e -m shift -m ctrl -s 500
capture export-dark
click 880 541
sleep .5
capture export-save-unavailable
check_seen
[[ $(sha256sum "$config") == "$before" ]]
printf 'Export UI acceptance: %s\n' "$root"
