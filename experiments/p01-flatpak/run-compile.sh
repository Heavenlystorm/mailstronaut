#!/usr/bin/env bash
# Use a transient user service for the long, explicit SDK compile phase.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
mkdir -p target/nix-cache
# Share the coordinated full-gate lock so this compile never overlaps a Nix gate. Override
# MAILSTRONAUT_GATE_LOCK only on a host that has no shared gate lock.
gate_lock_path=${MAILSTRONAUT_GATE_LOCK:-/builds/.mailstronaut-nix-gate.lock}
exec {gate_lock}>"$gate_lock_path"
if ! flock --nonblock "$gate_lock"; then
  printf '%s\n' 'Another coordinated package build is already running.' >&2
  exit 73
fi
export XDG_CACHE_HOME="$root/target/nix-cache"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
printf 'running\n' > target/p01-flatpak-compile.status
result=0
nix develop --impure --expr 'import ./experiments/p01-flatpak/shell.nix' \
  --command bash experiments/p01-flatpak/isolated.sh \
  bash experiments/p01-flatpak/run.sh compile \
  > target/p01-flatpak-compile.log 2>&1 || result=$?
printf '%s\n' "$result" > target/p01-flatpak-compile.status.tmp
mv target/p01-flatpak-compile.status.tmp target/p01-flatpak-compile.status
printf 'SDK compile exited %s; log: %s/target/p01-flatpak-compile.log\n' "$result" "$root"
exit "$result"
