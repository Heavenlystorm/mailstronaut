#!/usr/bin/env bash
# Sourced by the synthetic mailbox harness on its private Wayland session/bus.
set -euo pipefail
: "${root:?private profile}" "${config:?native source}" "${binary:?current application}"
portal=${MAILSTRONAUT_EXPORT_PORTAL_FIXTURE:-}
if [[ -n $portal ]]; then
  portal_root="$root/export-portal"
  mkdir -p "$portal_root"
  MAILSTRONAUT_PORTAL_ROOT="$portal_root" "$(realpath "$portal")" \
    portal_fixture::serve_portals --ignored --exact > "$portal_root/server.log" 2>&1 & export_portal=$!
  for _ in {1..100}; do
    [[ -f $portal_root/portal-ready ]] && break
    kill -0 "$export_portal"; sleep .05
  done
  [[ -f $portal_root/portal-ready ]]
fi
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
if [[ -n $portal ]]; then
  saved="$portal_root/export · été.nix"
  for response in cancel deny "$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).as_uri())' "$saved")"; do
    printf '%s\n' "$response" > "$portal_root/chooser-answer"
    click 880 541
    sleep .6
    if [[ $response == file:* ]]; then
      cmp "$saved" "$root/cli-export.nix"
      [[ $(stat -c '%a' "$saved") == 600 ]]
      "$binary" config check --path "$saved"
      cp "$saved" "$root/portal-export.nix"
    else
      [[ ! -e $saved ]]
    fi
  done
  printf '%s\n' '{ sentinel = true; }' > "$saved"
  click 880 541
  sleep .6
  capture export-existing-preserved
  [[ $(cat "$saved") == '{ sentinel = true; }' ]]
  python3 - "$portal_root/chooser-requests.jsonl" <<'PYREQUESTS'
import json,sys
with open(sys.argv[1]) as receipts:
    requests=[json.loads(line) for line in receipts]
assert len(requests)==4 and all(request['method']=='SaveFile' for request in requests),requests
print('FileChooser save, cancellation, refusal and no-clobber passed.')
PYREQUESTS
else
  click 880 541
  sleep .5
  capture export-save-unavailable
fi
check_seen
[[ $(sha256sum "$config") == "$before" ]]
printf 'Export UI acceptance: %s\n' "$root"
