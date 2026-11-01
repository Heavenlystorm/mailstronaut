#!/usr/bin/env bash
# Run a command against this working tree on the CI server instead of the laptop:
#   bash ci/remote.sh nix develop --command bash ci/check.sh
#   bash ci/remote.sh --fetch 'target/p06.*/*.png' nix develop .#gui-test --command ...
# Each checkout should set its own MAILSTRONAUT_REMOTE_DIR, because the synced tree
# (and its Cargo target) mirrors exactly one working tree. New files need `git add -N`
# before a flake can see them, exactly as in a local nix develop. A linked worktree's
# .git only points at a path on this machine, so there the server keeps its own
# throwaway repository, which exists to tell the flake which files belong to the tree.
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
# Share authentication across mkdir, rsync and execution, including jump hosts.
# The private socket belongs only to this invocation and is inside this project.
control=$(mktemp -d "$root/target/remote/ssh.XXXXXX")
trap '
  ssh -S "$control/c" -O exit "$host" > /dev/null 2>&1 || true
  rmdir "$control" 2> /dev/null || true
' EXIT
remote_ssh=(ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=2 -o LogLevel=ERROR
  -o ControlMaster=auto -o ControlPersist=60 -o "ControlPath=$control/c")
printf -v transport '%q ' "${remote_ssh[@]}"
"${remote_ssh[@]}" "$host" "mkdir -p '$dir'"
excludes=(--exclude /target --exclude /result --exclude '/result-*')
index=''
if [[ -f $root/.git ]]; then
  excludes+=(--exclude /.git)
  # target may be a symlink to a larger disk, which the /target/ ignore rule misses.
  index="{ [ -d .git ] || { git init -q . && echo /target >> .git/info/exclude; }; } && git add -A . && "
fi
rsync -az --delete "${excludes[@]}" \
  -e "$transport" "$root/" "$host:$dir/"
# The server's nix.conf predates flakes; this enables them for one invocation only.
printf -v command '%q ' "$@"
status=0
"${remote_ssh[@]}" "$host" \
  "cd '$dir' && ${index}NIX_CONFIG='experimental-features = nix-command flakes
max-jobs = auto' ${command}" || status=$?
if [[ -n $fetch ]]; then
  rsync -az -e "$transport" \
    "$host:$dir/$fetch" "$root/target/remote/" || status=$((status == 0 ? 1 : status))
fi
exit "$status"
