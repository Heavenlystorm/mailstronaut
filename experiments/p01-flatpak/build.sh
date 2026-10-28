#!/usr/bin/env bash
# Runs inside the experimental Flatpak module, never on the host.
set -euo pipefail
phase=${1:-metadata}
case "$phase" in metadata|compile) ;; *) printf 'Expected metadata or compile\n' >&2; exit 2 ;; esac
mkdir -p "$CARGO_HOME"
vendor_root=/run/build/mailstronaut/vendor
cp "$vendor_root/.cargo/config.toml" "$CARGO_HOME/config.toml"
rustc -Vv
cargo --version
"${CC:-cc}" --version
cargo fetch --frozen --offline
for name in mozjs_locale_glue mozjs_properties_glue mozjs_collator_glue mozjs_normalizer_glue mozjs_unicode_bidi_ffi; do
  matches=("$vendor_root/source-registry-0/$name"-*)
  if (( ${#matches[@]} != 1 )) || [[ ! -f "${matches[0]}/Cargo.toml" ]]; then
    printf 'Expected exactly one vendored manifest for %s\n' "$name" >&2
    exit 1
  fi
  printf 'Offline nested metadata: %s\n' "$name"
  (cd "${matches[0]}" && cargo metadata --offline --all-features --format-version 1 > /dev/null)
done
if [[ "$phase" == metadata ]]; then
  printf 'Metadata phase passed; no compilation attempted.\n'
  exit 0
fi
export AS="${CC:-cc}"
if [[ -z ${LIBCLANG_PATH:-} ]] || ! compgen -G "$LIBCLANG_PATH/libclang.so*" > /dev/null; then
  printf 'Set LIBCLANG_PATH to the SDK directory containing libclang.so before compilation.\n' >&2
  exit 1
fi
cargo build --frozen --offline --release --workspace --all-features -j 2
install -Dm755 target/release/mailstronaut "${FLATPAK_DEST:-/app}/bin/mailstronaut"
