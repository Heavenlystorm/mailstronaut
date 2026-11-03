#!/usr/bin/env bash
# Called only by credentials.sh: a sandboxed Flatpak process reaches the desktop portal on this private
# bus through the sandbox's D-Bus proxy, and reads the versions of the FileChooser and Notification
# portals that the protocol-accurate fixture serves. This proves reachability only. The sandbox has no
# keyring route, so the configured account cannot activate here, and no chooser or notification flow
# runs.
set -euo pipefail
: "${1:?flatpak marker}"
root=$(realpath "${2:?private fixture root}")
artifacts="$root/flatpak-portal"
home="$artifacts/home"
mkdir -p "$home"
flatpak_user_dir=${MAILSTRONAUT_FLATPAK_USER_DIR:?isolated Flatpak installation}
chooser_fixture=$(realpath "${MAILSTRONAUT_PORTAL_FIXTURE:?prebuilt application test executable}")
# dbus-send from the Platform runtime, inside the sandbox, with network unshared and a private HOME.
send="$artifacts/dbus-send-in-sandbox"
cat > "$send" <<WRAP
#!/usr/bin/env bash
exec env HOME="$home" FLATPAK_USER_DIR="$flatpak_user_dir" flatpak run \
  --command=dbus-send --unshare=network io.github.eduardz1.Mailstronaut "\$@"
WRAP
chmod +x "$send"
portal=''
cleanup() {
  if [[ -n "$portal" ]]; then kill "$portal" 2>/dev/null || true; wait "$portal" 2>/dev/null || true; fi
}
trap cleanup EXIT
# The portal fixture on this private bus. It refuses any bus whose address lacks its root's parent.
MAILSTRONAUT_PORTAL_ROOT=$artifacts "$chooser_fixture" portal_fixture::serve_portals --ignored --exact \
  > "$artifacts/portal.log" 2>&1 & portal=$!
for _ in {1..100}; do [[ -f "$artifacts/portal-ready" ]] && break; kill -0 "$portal"; sleep .1; done
[[ -f "$artifacts/portal-ready" ]]

for pair in "org.freedesktop.portal.FileChooser 4" "org.freedesktop.portal.Notification 1"; do
  read -r interface expected <<< "$pair"
  reply=$("$send" --print-reply --dest=org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop \
    org.freedesktop.DBus.Properties.Get "string:$interface" string:version)
  grep -q "uint32 $expected" <<< "$reply"
  printf 'OK: the sandbox reached %s version %s through the portal proxy\n' "$interface" "$expected"
done
printf 'Flatpak portal reachability acceptance: %s\n' "$artifacts"
