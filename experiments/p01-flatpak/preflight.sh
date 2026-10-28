#!/usr/bin/env bash
# Use through isolated.sh in the optional Nix shell, after provisioning SDK refs.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
state="$root/target/p01-flatpak"
if [[ ${FLATPAK_USER_DIR:-} != "$state/installation" ]]; then
  printf '%s\n' 'Run this probe through experiments/p01-flatpak/isolated.sh' >&2
  exit 2
fi
if (( $# > 1 )); then
  printf 'Usage: %s [SDK_LOCK_JSON]\n' "$0" >&2
  exit 2
fi
lock=${1:-experiments/p01-flatpak/sdk-lock.json}
pin_rows=$(jq -er '
  if .architecture == "x86_64" and .branch == "26.08"
     and (.refs | type) == "object"
     and (.refs | keys) == [
       "org.freedesktop.Platform",
       "org.freedesktop.Sdk",
       "org.freedesktop.Sdk.Extension.llvm22",
       "org.freedesktop.Sdk.Extension.rust-stable"
     ]
     and all(.refs[]; type == "string" and test("^[0-9a-f]{64}$"))
  then .refs | to_entries[] | [.key, .value] | @tsv
  else error("Invalid SDK lock: require x86_64/26.08 and exactly four commit pins")
  end' "$lock")
arch=$(jq -r .architecture "$lock")
branch=$(jq -r .branch "$lock")
flatpak --version
flatpak-builder --version
unshare -Ur true
bwrap --unshare-user --ro-bind / / --proc /proc --dev /dev "$(command -v true)"
while IFS=$'\t' read -r ref expected; do
  actual=$(flatpak info --user --show-commit "$ref/$arch/$branch")
  printf '%s/%s/%s %s\n' "$ref" "$arch" "$branch" "$actual"
  if [[ "$actual" != "$expected" ]]; then
    printf 'Installed commit differs from sdk-lock.json for %s\n' "$ref" >&2
    exit 1
  fi
done <<< "$pin_rows"
init_args=()
if [[ -e "$state/inventory/metadata" ]]; then init_args+=(--update); fi
flatpak build-init "${init_args[@]}" --arch="$arch" \
  --sdk-extension=org.freedesktop.Sdk.Extension.rust-stable \
  --sdk-extension=org.freedesktop.Sdk.Extension.llvm22 \
  "$state/inventory" io.github.eduardz1.Mailstronaut \
  org.freedesktop.Sdk org.freedesktop.Platform "$branch"
flatpak build --unshare=network \
  --bind-mount="/run/build/mailstronaut-probe=$root/experiments/p01-flatpak" \
  "$state/inventory" bash /run/build/mailstronaut-probe/inventory.sh
