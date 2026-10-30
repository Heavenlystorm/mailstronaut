#!/usr/bin/env bash
# Run as a transient user service so restarting the coding UI does not kill Nix.
# Invoke through systemd-run --user; do not start a second full package build.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
mkdir -p target/nix-cache
exec {gate_lock}>target/p01-package-gate.lock
if ! flock --nonblock "$gate_lock"; then
  printf '%s\n' 'Another coordinated package gate is already running.' >&2
  exit 73
fi
export XDG_CACHE_HOME="$root/target/nix-cache"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
printf 'running\n' > target/p01-package-gate.status
git rev-parse HEAD > target/p01-package-gate-revision.txt
build_jobs=$(nproc)
printf 'Build parallelism: %s CPUs\n' "$build_jobs" > target/p01-package-gate-service.log
result=0
bash ci/nix-check.sh \
  >> target/p01-package-gate-service.log 2>&1 || result=$?
printf '%s\n' "$result" > target/p01-package-gate.status.tmp
mv target/p01-package-gate.status.tmp target/p01-package-gate.status
printf 'Package gate exited %s; log: %s/target/p01-package-gate-service.log\n' "$result" "$root"
exit "$result"
