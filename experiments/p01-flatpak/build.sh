#!/usr/bin/env bash
# Runs inside the experimental Flatpak module, never on the host.
set -euo pipefail
phase=${1:-metadata}
case "$phase" in metadata|compile) ;; *) printf 'Expected metadata or compile\n' >&2; exit 2 ;; esac
build_jobs=$(nproc)
export CARGO_BUILD_JOBS="$build_jobs"
export CMAKE_BUILD_PARALLEL_LEVEL="$build_jobs"
export MAKEFLAGS="-j$build_jobs"
printf 'Build parallelism: %s CPUs\n' "$build_jobs"
mkdir -p "$CARGO_HOME"
vendor_root=/run/build/mailstronaut/vendor
cp "$vendor_root/.cargo/config.toml" "$CARGO_HOME/config.toml"
rustc -Vv
cargo --version
"${CC:-cc}" --version
cargo fetch --frozen --offline
if [[ "$phase" == metadata ]]; then
  printf 'Metadata phase passed; no compilation attempted.\n'
  exit 0
fi
export AS="${CC:-cc}"
if [[ -z ${LIBCLANG_PATH:-} ]] || ! compgen -G "$LIBCLANG_PATH/libclang.so*" > /dev/null; then
  printf 'Set LIBCLANG_PATH to the SDK directory containing libclang.so before compilation.\n' >&2
  exit 1
fi
cargo build --frozen --offline --release --workspace --all-features --jobs "$build_jobs"
install -Dm755 target/release/mailstronaut "${FLATPAK_DEST:-/app}/bin/mailstronaut"
