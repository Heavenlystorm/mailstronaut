#!/usr/bin/env bash
# Use through isolated.sh after the SDK compile installs into the probe app dir.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
state="$root/target/p01-flatpak"
if [[ ${FLATPAK_USER_DIR:-} != "$state/installation" ]]; then
  printf '%s\n' 'Run through experiments/p01-flatpak/isolated.sh' >&2
  exit 2
fi
[[ -x "$state/inventory/files/bin/mailstronaut" ]]
# Platform runtime, not SDK; no inherited LLVM-extension library path or tools.
flatpak build --runtime --unshare=network --unset-env=LD_LIBRARY_PATH \
  --env=PATH=/nonexistent "$state/inventory" /app/bin/mailstronaut --version
flatpak build --runtime --unshare=network --unset-env=LD_LIBRARY_PATH \
  --env=PATH=/nonexistent \
  --bind-mount="/run/probe-examples=$state/source/app/examples" \
  "$state/inventory" /app/bin/mailstronaut config check \
  --path /run/probe-examples/config.nix
printf '%s\n' 'Platform-runtime CLI/configuration smoke passed.'
