#!/usr/bin/env bash
# Run a command against this working tree on the CI server instead of the laptop:
#   bash ci/remote.sh nix develop --command bash ci/check.sh
#   bash ci/remote.sh --fetch 'target/p06.*/*.png' nix develop .#gui-test --command ...
# Each checkout should set its own MAILSTRONAUT_REMOTE_DIR, because the synced tree
# (and its Cargo target) mirrors exactly one working tree. New files need `git add -N`
# before a flake can see them, exactly as in a local nix develop.
set -euo pipefail
host=${MAILSTRONAUT_REMOTE:-ci-py}
dir=${MAILSTRONAUT_REMOTE_DIR:-mailstronaut-dev}
fetch=''
if [[ ${1:-} == --fetch ]]; then
  fetch=$2
  shift 2
fi
root=$(git rev-parse --show-toplevel)
mkdir -p "$root/target/remote"
ssh -o BatchMode=yes "$host" "mkdir -p '$dir'" 2> /dev/null
rsync -az --delete --exclude /target --exclude /result --exclude '/result-*' \
  -e 'ssh -o BatchMode=yes -o LogLevel=ERROR' "$root/" "$host:$dir/"
# The server's nix.conf predates flakes; this enables them for one invocation only.
printf -v command '%q ' "$@"
status=0
ssh -o BatchMode=yes -o LogLevel=ERROR "$host" \
  "cd '$dir' && NIX_CONFIG='experimental-features = nix-command flakes
max-jobs = auto' ${command}" || status=$?
if [[ -n $fetch ]]; then
  rsync -az -e 'ssh -o BatchMode=yes -o LogLevel=ERROR' \
    "$host:$dir/$fetch" "$root/target/remote/" || status=$((status == 0 ? 1 : status))
fi
exit "$status"
