#!/usr/bin/env bash
# Sourced by the private Wayland harness with synthetic mail and >99 unread rows.
set -euo pipefail
: "${root:?private profile}" "${config:?native source}" "${binary:?application}"
check_seen
capture appearance-light-inbox
wtype -M ctrl -M shift d -m shift -m ctrl -s 400
capture appearance-dark-inbox
wtype -M ctrl , -m ctrl -s 400
capture appearance-dark-settings
wtype -M ctrl -M shift d -m shift -m ctrl -s 400
capture appearance-light-settings
click 310 126
wtype 'local models' -s 400
capture appearance-local-models
if [[ ${MAILSTRONAUT_PREFERENCES_GUI:-0} == 1 ]]; then
  model="$root/synthetic-model"
  mkdir -p "$model"
  click 1130 418; wtype "$model" -s 150
  click 1130 500; wtype 'synthetic-revision' -s 150
  capture appearance-model-staged
  click 275 783
  sleep .7
  "$binary" config export --path "$config" > "$root/model-saved.nix"
  grep -Fq "modelDirectory = \"$model\";" "$root/model-saved.nix"
  grep -Fq 'modelRevision = "synthetic-revision";' "$root/model-saved.nix"
  capture appearance-model-saved
fi
check_seen
wtype -M ctrl n -m ctrl -s 400
capture appearance-compose
# Drive the actual Attach button through the existing private FileChooser portal.
(
  fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt app test executable}")
  artifacts="$root/appearance-portal"
  mkdir -p "$artifacts"
  printf 'Synthetic local attachment.\n' > "$artifacts/report.txt"
  python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).as_uri())' "$artifacts/report.txt" > "$artifacts/chooser-answer"
  MAILSTRONAUT_PORTAL_ROOT="$artifacts" "$fixture" portal_fixture::serve_portals --ignored --exact > "$artifacts/portal.log" 2>&1 & chooser=$!
  trap 'kill "$chooser" 2>/dev/null || true; wait "$chooser" 2>/dev/null || true' EXIT
  for _ in {1..100}; do [[ -f "$artifacts/portal-ready" ]] && break; kill -0 "$chooser"; sleep .1; done
  [[ -f "$artifacts/portal-ready" ]]
  click 568 134
  database="$XDG_DATA_HOME/mailstronaut/mail.sqlite3"
  for _ in {1..75}; do
    count=$(python3 -c 'import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute("SELECT count(*) FROM draft_attachments").fetchone()[0])' "$database")
    [[ $count == 1 ]] && break
    sleep .1
  done
  [[ $count == 1 ]]
  capture appearance-attachment
)
printf 'UI_APPEARANCE_CAPTURE_OK: %s\n' "$root"
