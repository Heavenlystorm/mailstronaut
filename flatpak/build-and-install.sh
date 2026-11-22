#!/usr/bin/env bash
# Build the production Flatpak manifest offline under the shared full-gate lock, then install it
# into the isolated probe installation. Run inside the optional probe shell from the repository
# root: nix develop .#flatpak --command bash flatpak/build-and-install.sh
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
builder_options=()
if [[ ${MAILSTRONAUT_FLATPAK_DISABLE_ROFILES_FUSE:-0} == 1 ]]; then
  builder_options+=(--disable-rofiles-fuse)
fi
# Share the coordinated full-gate lock so this build never overlaps a Nix gate. Refuse to wait
# behind a gate. Override MAILSTRONAUT_GATE_LOCK only on a host without the shared lock.
gate_lock_path=${MAILSTRONAUT_GATE_LOCK:-/builds/.mailstronaut-nix-gate.lock}
exec {gate_lock}>"$gate_lock_path"
if ! flock --nonblock "$gate_lock"; then
  printf '%s\n' 'Another coordinated package build is already running.' >&2
  exit 73
fi
mkdir -p target/p10-flatpak
printf 'running\n' > target/p10-flatpak-build.status
result=0
if { bash flatpak/prepare.sh &&
  bash experiments/p01-flatpak/isolated.sh bash experiments/p01-flatpak/preflight.sh;
} > target/p10-flatpak-build.log 2>&1; then
  {
    bash experiments/p01-flatpak/isolated.sh flatpak-builder --force-clean --disable-download "${builder_options[@]}" \
      --state-dir=target/p10-flatpak/state --repo=target/p10-flatpak/repo \
      target/p10-flatpak/build flatpak/io.github.eduardz1.Mailstronaut.json &&
    # Preflight verified the installed runtime. Deploy only the app from the local repo.
    bash experiments/p01-flatpak/isolated.sh flatpak install --user --or-update --no-deps --no-related \
      --noninteractive --app "$root/target/p10-flatpak/repo" io.github.eduardz1.Mailstronaut
  } >> target/p10-flatpak-build.log 2>&1 || result=$?
else
  result=$?
fi
printf '%s\n' "$result" > target/p10-flatpak-build.status.tmp
mv target/p10-flatpak-build.status.tmp target/p10-flatpak-build.status
printf 'Flatpak build exited %s; log: %s/target/p10-flatpak-build.log\n' "$result" "$root"
exit "$result"
