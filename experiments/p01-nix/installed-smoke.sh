#!/usr/bin/env bash
# Run through nix develop, passing the completed Nix package output.
# Each installed invocation gets an empty environment and no executable PATH.
set -euo pipefail
if (( $# != 1 )); then
  printf 'Usage: %s PACKAGE_OUTPUT\n' "$0" >&2
  exit 2
fi
root=$(git rev-parse --show-toplevel)
binary="$1/bin/mailstronaut"
if [[ "$binary" != /* ]] || [[ ! -x "$binary" ]]; then
  printf '%s\n' 'Pass an absolute package output containing bin/mailstronaut.' >&2
  exit 2
fi
mkdir -p "$root/target"
artifacts=$(mktemp -d "$root/target/p01-installed-smoke.XXXXXX")
printf '%s\n' "$1" > "$artifacts/package-output.txt"
invoke() {
  env -i PATH=/nonexistent LC_ALL=C.UTF-8 "$binary" "$@"
}
invoke --version > "$artifacts/version.log" 2>&1
invoke config check --path "$root/examples/config.nix" > "$artifacts/example.log" 2>&1
config="$artifacts/generated/config.nix"
invoke config init --path "$config" > "$artifacts/init.log" 2>&1
cp "$config" "$artifacts/original.nix"
invoke config check --path "$config" > "$artifacts/generated.log" 2>&1
if invoke config init --path "$config" > "$artifacts/repeat-init.log" 2>&1; then
  printf '%s\n' 'Repeated initialization unexpectedly succeeded.' >&2
  exit 1
fi
cmp "$config" "$artifacts/original.nix"
[[ $(stat -c '%a' "$config") == 600 ]]
[[ $(stat -c '%a' "$artifacts/generated") == 700 ]]
printf '%s\n' '{ schemaVersion = 999; accounts = [ ]; }' > "$artifacts/invalid.nix"
if invoke config check --path "$artifacts/invalid.nix" > "$artifacts/invalid.log" 2>&1; then
  printf '%s\n' 'Unsupported configuration schema unexpectedly succeeded.' >&2
  exit 1
fi
printf 'Installed CLI/configuration smoke passed. Artifacts: %s\n' "$artifacts"
