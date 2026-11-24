#!/usr/bin/env bash
# Called on the focused synthetic reader by accessibility.sh in the a11y-test shell.
set -euo pipefail
artifacts=${ARTIFACTS:?private accessibility fixture}
a11y=${A11Y_ADDRESS:?private accessibility bus}
: "${MAILSTRONAUT_SPEECHD:?pinned Speech Dispatcher}" "${MAILSTRONAUT_ORCA:?pinned Orca}"
[[ $DBUS_SESSION_BUS_ADDRESS == "unix:path=$XDG_RUNTIME_DIR/bus" ]]
orca_pid=''
dispatch_pid=''
stop_orca() {
  if [[ -n $orca_pid ]]; then
    kill "$orca_pid" 2>/dev/null || true
    wait "$orca_pid" 2>/dev/null || true
  fi
  if [[ -n $dispatch_pid ]]; then
    kill "$dispatch_pid" 2>/dev/null || true
    wait "$dispatch_pid" 2>/dev/null || true
  fi
}
trap stop_orca EXIT
speech_log="$artifacts/orca-debug.log"
mkdir -p "$artifacts/spd/modules" "$artifacts/voice-home"
printf 'default_driver=alsa\n' > "$artifacts/voice-home/.libao"
cp "$MAILSTRONAUT_SPEECHD/etc/speech-dispatcher/modules/espeak-ng.conf" "$artifacts/spd/modules/"
cat > "$artifacts/spd/speechd.conf" <<'CONF'
AudioOutputMethod "libao"
AddModule "espeak-ng" "sd_espeak-ng" "espeak-ng.conf"
DefaultModule "espeak-ng"
CONF
cat > "$artifacts/asound.conf" <<CONF
pcm.!default {
  type file
  slave.pcm { type null }
  file "$artifacts/orca.raw"
  format "raw"
}
CONF
export SPEECHD_ADDRESS="unix_socket:$XDG_RUNTIME_DIR/spd"
HOME="$artifacts/voice-home" ALSA_CONFIG_PATH="$artifacts/asound.conf" "$MAILSTRONAUT_SPEECHD/bin/speech-dispatcher" -s -t 0 -C "$artifacts/spd" \
  -c unix_socket -S "$XDG_RUNTIME_DIR/spd" -P "$artifacts/spd.pid" -L "$artifacts/spd" \
  > "$artifacts/spd.log" 2>&1 & dispatch_pid=$!
for _ in {1..100}; do [[ -S $XDG_RUNTIME_DIR/spd ]] && break; kill -0 "$dispatch_pid"; sleep .05; done
[[ -S $XDG_RUNTIME_DIR/spd ]]
GSETTINGS_BACKEND=memory AT_SPI_BUS_ADDRESS="$a11y" \
  PULSE_SERVER="unix:$XDG_RUNTIME_DIR/pulse/native" \
  "$MAILSTRONAUT_ORCA" --debug-file="$speech_log" > "$artifacts/orca.log" 2>&1 & orca_pid=$!
# The debug stream may buffer its final records. A real session-bus service
# marks startup and keeps the checks independent of that buffering.
gdbus wait --session --timeout 30 org.gnome.Orca.Service
sleep 1
wtype -M ctrl -k k -m ctrl -s 1200 -M ctrl -M shift -k t -m shift -m ctrl -s 1000 \
  -M ctrl -k Home -m ctrl -s 500
gdbus call --session --dest org.gnome.Orca.Service \
  --object-path /org/gnome/Orca/Service/SayAllPresenter \
  --method org.gnome.Orca.Module.ExecuteCommand SayAll true
sleep 7
kill "$orca_pid"
wait "$orca_pid" || true
orca_pid=''
python3 - "$speech_log" <<'PY'
import json
import re
import sys
from html import unescape
from pathlib import Path

log = Path(sys.argv[1]).read_text(encoding="utf-8")
records = re.split(r"^\d\d:\d\d:\d\d\.\d+ - ", log, flags=re.M)
utterances = [record for record in records
              if record.startswith(("SPEECH OUTPUT: '", "SPEECH DISPATCHER: Speaking '"))]
utterances = [unescape(re.sub(r"<[^>]+>", "", record)) for record in utterances]
expected_text = ["Search cached mail", "Message text", "Synthetic performance original 001.",
                 "Réinscription au doctorat", "Scadenza di domani"]
for expected in expected_text:
    if not any(expected in utterance for utterance in utterances):
        raise SystemExit(f"Orca did not announce {expected!r}")
pcm = Path(sys.argv[1]).with_name("orca.raw").read_bytes()
if len(pcm) < 2000 or not any(pcm):
    raise SystemExit("Missing synthesized speech samples")
report = {"announced": expected_text, "recorded_pcm_bytes": len(pcm)}
Path(sys.argv[1]).with_name("screen-reader.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
PY
