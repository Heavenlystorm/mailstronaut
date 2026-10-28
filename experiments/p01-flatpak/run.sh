#!/usr/bin/env bash
# Reuse the staged source directly in the SDK, without another source-tree copy.
# Coordinate the explicit compile phase with other full builds.
set -euo pipefail
phase=${1:-metadata}
case "$phase" in metadata|compile|builder-metadata) ;; *) printf 'Expected metadata, builder-metadata or compile\n' >&2; exit 2 ;; esac
root=$(git rev-parse --show-toplevel)
cd "$root"
bash experiments/p01-flatpak/preflight.sh
if [[ "$phase" == builder-metadata ]]; then
  exec flatpak-builder --user --arch=x86_64 --disable-rofiles-fuse \
    --disable-download --build-only \
    --state-dir="$root/target/p01-flatpak/builder-state" \
    "$root/target/p01-flatpak/metadata-build" \
    experiments/p01-flatpak/manifest.json
fi
flatpak build --unshare=network \
  --env=PATH=/usr/lib/sdk/rust-stable/bin:/usr/lib/sdk/llvm22/bin:/usr/bin \
  --env=LD_LIBRARY_PATH=/usr/lib/sdk/llvm22/lib \
  --env=LIBCLANG_PATH=/usr/lib/sdk/llvm22/lib \
  --env=CC=clang --env=CXX=clang++ \
  --env=CARGO_HOME=/run/build/mailstronaut/cargo-home \
  --env=CARGO_NET_OFFLINE=true --env=CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never \
  --env=CMAKE_BUILD_PARALLEL_LEVEL=2 --env=MAKEFLAGS=-j2 \
  --bind-mount="/run/build/mailstronaut=$root/target/p01-flatpak/source" \
  --bind-mount="/run/build/mailstronaut-probe=$root/experiments/p01-flatpak" \
  --build-dir=/run/build/mailstronaut/app \
  "$root/target/p01-flatpak/inventory" \
  bash /run/build/mailstronaut-probe/build.sh "$phase"
