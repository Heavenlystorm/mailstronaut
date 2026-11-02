#!/usr/bin/env bash
# Called only by credentials.sh: starts the installed Flatpak application inside the private
# headless Wayland session, with its network unshared, and checks that its window appears. The
# configuration holds no accounts, so no mail or server is used.
set -euo pipefail
root=$(realpath "${2:?private fixture root}")
artifacts="$root/flatpak-gui"
mkdir -p "$artifacts/home"
flatpak_user_dir=${MAILSTRONAUT_FLATPAK_USER_DIR:?isolated Flatpak installation}
config="$artifacts/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  sync.automatic = false;
  accounts = [ ];
}
NIX
# The installed application in its sandbox. Only this check's configuration directory is readable,
# and its HOME is private, so the sandbox never touches the CI user's home.
binary="$artifacts/mailstronaut-flatpak"
cat > "$binary" <<WRAP
#!/usr/bin/env bash
exec env HOME="$artifacts/home" FLATPAK_USER_DIR="$flatpak_user_dir" flatpak run \
  --command=mailstronaut --unshare=network --filesystem="$artifacts:ro" \
  io.github.eduardz1.Mailstronaut "\$@"
WRAP
chmod +x "$binary"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../experiments/wayland-session.sh
source "$(dirname "${BASH_SOURCE[0]}")/../experiments/wayland-session.sh"
trap session_cleanup EXIT
start flatpak
window_open || { printf '%s\n' 'The Flatpak application did not open a window' >&2; exit 1; }
printf '%s\n' 'OK: the installed Flatpak opened its window in the sandbox with network unshared'
sleep 2
capture flatpak-window
close_window
if grep -Eil 'panicked at|fatal error' "$artifacts"/*-app.log > /dev/null; then
  printf '%s\n' 'Application reported a runtime failure' >&2; exit 1
fi
printf 'Flatpak GUI smoke acceptance: %s\n' "$artifacts"
