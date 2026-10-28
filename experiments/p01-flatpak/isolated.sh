#!/usr/bin/env bash
# Run a Flatpak probe command without changing the user's Flatpak installation.
# Invoke from the project root inside the optional Nix development shell.
set -euo pipefail

project_root=$(git rev-parse --show-toplevel)
cd "$project_root"
state="$project_root/target/p01-flatpak"
mkdir -p "$state"/{runtime,cache,data,config,system}
chmod 700 "$state/runtime"
export FLATPAK_USER_DIR="$state/installation"
export FLATPAK_SYSTEM_DIR="$state/system"
export XDG_RUNTIME_DIR="$state/runtime"
export XDG_CACHE_HOME="$state/cache"
export XDG_DATA_HOME="$state/data"
export XDG_CONFIG_HOME="$state/config"
export FLATPAK_DOWNLOAD_TMPDIR="$state/cache"
export FLATPAK_FANCY_OUTPUT=0
export FLATPAK_TTY_PROGRESS=0

if (( $# == 0 )); then
  printf '%s\n' 'Pass a probe command, for example: flatpak --user list --runtime' >&2
  exit 2
fi
exec "$@"
