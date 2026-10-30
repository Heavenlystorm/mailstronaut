#!/usr/bin/env bash
# A single persistent development job; never competes with another release gate.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
mkdir -p target/p06-search target/nix-cache
exec {gate_lock}>target/p01-package-gate.lock
if ! flock --nonblock "$gate_lock"; then
  printf '%s\n' 'Another coordinated package/probe gate is running.' >&2
  exit 73
fi
export XDG_CACHE_HOME="$root/target/nix-cache"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
printf 'running\n' > target/p06-search/release.status
git rev-parse HEAD > target/p06-search/release-revision.txt
result=0
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh release > target/p06-search/release.log 2>&1 || result=$?
if [[ $result == 0 ]]; then
  nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh bench > target/p06-search/benchmark.log 2>&1 || result=$?
fi
printf '%s\n' "$result" > target/p06-search/release.status.tmp
mv target/p06-search/release.status.tmp target/p06-search/release.status
exit "$result"
