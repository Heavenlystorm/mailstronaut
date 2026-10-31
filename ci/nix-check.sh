#!/usr/bin/env bash
# CI entry point. Retry only immutable source fetching, never compiler/test failures.
set -euo pipefail

df -h "$PWD" /nix/store
for attempt in 1 2 3; do
  printf 'Fetching pinned dependency sources (attempt %s/3)\n' "$attempt"
  if vendor_path=$(nix build --no-link --print-out-paths --no-update-lock-file .#mailstronaut.cargoDeps); then
    break
  else
    result=$?
  fi
  if [[ $attempt == 3 ]]; then
    exit "$result"
  fi
  sleep "$((attempt * 10))"
done

# Fixed-output staging contains Cargo.lock itself. A cached output can hide an
# outdated cargoHash even when only workspace dependency references changed.
workspace_lock_hash=$(nix hash file --type sha256 Cargo.lock)
vendor_lock_hash=$(nix hash file --type sha256 "$vendor_path/Cargo.lock")
if [[ $workspace_lock_hash != "$vendor_lock_hash" ]]; then
  printf '%s\n' 'Vendored Cargo.lock is stale; refresh cargoHash from a fresh staging build.' >&2
  exit 1
fi

nix flake check --no-update-lock-file --print-build-logs --cores "$(nproc)" --max-jobs 1
df -h "$PWD" /nix/store
