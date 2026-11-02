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
cp -R --reflink=auto --no-preserve=ownership "$src/." "$destination/app/"
cp -R --reflink=auto --no-preserve=ownership "$deps" "$destination/vendor"
chmod -R u+w "$destination"
config="$destination/vendor/.cargo/config.toml"
if ! grep -q '@vendor@' "$config"; then
  printf 'Vendor source configuration lacks the expected placeholder\n' >&2
  exit 1
fi
sed -i 's|@vendor@|/run/build/mailstronaut/vendor|g' "$config"
printf 'Staged source: %s\nPackage source: %s\nCargo dependencies: %s\n' "$destination" "$src" "$deps"
