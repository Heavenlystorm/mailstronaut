#!/usr/bin/env bash
# CI entry point. Retry only immutable source fetching, never compiler/test failures.
set -euo pipefail

df -h "$PWD" /nix/store
for attempt in 1 2 3; do
  printf 'Fetching pinned dependency sources (attempt %s/3)\n' "$attempt"
  if nix build --no-link --no-update-lock-file .#mailstronaut.cargoDeps; then
    break
  else
    result=$?
  fi
  if [[ $attempt == 3 ]]; then
    exit "$result"
  fi
  sleep "$((attempt * 10))"
done

nix flake check --no-update-lock-file --print-build-logs --cores "$(nproc)" --max-jobs 1
df -h "$PWD" /nix/store
