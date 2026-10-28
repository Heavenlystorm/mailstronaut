#!/usr/bin/env bash
# Stage the Nix package inputs for the experimental Flatpak SDK probe.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"
state="$root/target/p01-flatpak"
destination="$state/source"
if (( $# != 0 && $# != 2 )); then
  printf 'Usage: %s [PACKAGE_SRC CARGO_DEPS]\n' "$0" >&2
  exit 2
fi
if [[ -e "$destination" ]] && [[ -n $(ls -A "$destination") ]]; then
  printf 'Refusing to overwrite nonempty staging directory: %s\n' "$destination" >&2
  exit 1
fi
if (( $# == 2 )); then
  src=$1
  deps=$2
else
  mkdir -p "$state/nix-cache"
  export XDG_CACHE_HOME="$state/nix-cache"
  src=$(nix eval --raw .#packages.x86_64-linux.mailstronaut.src)
  deps=$(nix eval --raw .#packages.x86_64-linux.mailstronaut.cargoDeps)
fi
[[ -d "$src" && -d "$deps" && -f "$src/Cargo.lock" && -f "$deps/.cargo/config.toml" ]]
cmp "$src/Cargo.lock" "$deps/Cargo.lock"
cmp Cargo.lock "$src/Cargo.lock"
if [[ -e "$src/.cargo" ]]; then
  printf 'Package source unexpectedly includes development .cargo configuration\n' >&2
  exit 1
fi
mkdir -p "$destination/app"
cp -R --reflink=auto --no-preserve=mode,ownership "$src/." "$destination/app/"
cp -R --reflink=auto --no-preserve=mode,ownership "$deps" "$destination/vendor"
chmod -R u+w "$destination"
config="$destination/vendor/.cargo/config.toml"
if ! grep -q '@vendor@' "$config"; then
  printf 'Vendor source configuration lacks the expected placeholder\n' >&2
  exit 1
fi
sed -i 's|@vendor@|/run/build/mailstronaut/vendor|g' "$config"
for name in mozjs_locale_glue mozjs_properties_glue mozjs_collator_glue mozjs_normalizer_glue mozjs_unicode_bidi_ffi; do
  matches=("$destination/vendor/source-registry-0/$name"-*)
  if (( ${#matches[@]} != 1 )) || [[ ! -f "${matches[0]}/Cargo.lock" ]]; then
    printf 'Expected exactly one vendored lock for %s\n' "$name" >&2
    exit 1
  fi
  # fetchCargoVendor supplies empty per-file checksum maps for these crates.
  # The registry package checksum identifies the original archive, not this
  # intentionally substituted standalone lock. Do not generalize to cargo vendor.
  python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["files"] == {}' "${matches[0]}/.cargo-checksum.json"
  cp "$destination/app/Cargo.lock" "${matches[0]}/Cargo.lock"
done
printf 'Staged source: %s\nPackage source: %s\nCargo dependencies: %s\n' "$destination" "$src" "$deps"
