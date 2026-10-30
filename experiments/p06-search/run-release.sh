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
case ${1:-library} in
  library) prefix=release; build=release; runtime=bench ;;
  embedding) prefix=embedding-release; build=embedding-release; runtime=embedding-smoke ;;
  *) printf 'Usage: %s [library|embedding]\n' "$0" >&2; exit 2 ;;
esac
printf 'running\n' > "target/p06-search/$prefix.status"
git rev-parse HEAD > "target/p06-search/$prefix-revision.txt"
result=0
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh "$build" > "target/p06-search/$prefix.log" 2>&1 || result=$?
if [[ $result == 0 ]]; then
  nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh "$runtime" > "target/p06-search/$prefix-runtime.log" 2>&1 || result=$?
fi
printf '%s\n' "$result" > "target/p06-search/$prefix.status.tmp"
mv "target/p06-search/$prefix.status.tmp" "target/p06-search/$prefix.status"
exit "$result"
