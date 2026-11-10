#!/usr/bin/env bash
# Reproduce production Flatpak archives from the package's standard Nix inputs.
set -euo pipefail
unset TAR_OPTIONS XZ_DEFAULTS XZ_OPT
update=false
case "${1:-}" in
  '') [[ $# == 0 ]] || exit 2 ;;
  --update-pins) [[ $# == 1 ]] || exit 2; update=true ;;
  *) printf 'Usage: %s [--update-pins]\n' "$0" >&2; exit 2 ;;
esac
cd "$(git rev-parse --show-toplevel)"
state="$PWD/target/p10-flatpak"
mkdir -p "$state"
exec {prepare_lock}>"$state/.prepare.lock"
flock "$prepare_lock"
src=$(nix eval --no-update-lock-file --raw .#mailstronaut.src)
deps=$(nix build --no-link --print-out-paths --no-update-lock-file .#mailstronaut.cargoDeps)
cmp Cargo.lock "$src/Cargo.lock"
cmp Cargo.lock "$deps/Cargo.lock"
if [[ -e "$src/.cargo" ]]; then
  printf '%s\n' 'Package source includes development Cargo settings; refusing SDK inputs.' >&2
  exit 1
fi
staging=$(mktemp -d "$state/stage.XXXXXX")
# Remove only this invocation's temporary copy, including on failure.
trap 'chmod -R u+w "$staging" || true; rm -rf -- "$staging"' EXIT
cp -R --reflink=auto --no-preserve=ownership "$src" "$staging/app"
cp -R --reflink=auto --no-preserve=ownership "$deps" "$staging/vendor"
chmod -R u+w "$staging"
config="$staging/vendor/.cargo/config.toml"
if ! grep -q '@vendor@' "$config"; then
  printf '%s\n' 'Nix vendor configuration lacks the expected path placeholder.' >&2
  exit 1
fi
sed -i 's|@vendor@|/run/build/mailstronaut/vendor|g' "$config"
# Normalize metadata and fix XZ block boundaries. Thread count may vary without
# changing output; --no-adjust forbids falling back to a different encoding.
for pair in 'app app-source' 'vendor vendor'; do
  read -r directory archive <<< "$pair"
  tar --sort=name --format=gnu --mtime=@1 --owner=0 --group=0 --numeric-owner \
    --mode='u+rwX,go+rX,go-w' -C "$staging" -cf - "$directory" \
    | xz -6 --format=xz --check=crc64 --threads=0 --block-size=24MiB --no-adjust \
      > "$staging/$archive.tar.xz"
done
python3 - "$staging" "$update" <<'PY'
import hashlib
import json
import pathlib
import sys

staging = pathlib.Path(sys.argv[1])
manifest = pathlib.Path('flatpak/io.github.eduardz1.Mailstronaut.json')
data = json.loads(manifest.read_text())
module = next(module for module in data['modules'] if module['name'] == 'mailstronaut')
changed = False
for name in ('app-source.tar.xz', 'vendor.tar.xz'):
    source = next(source for source in module['sources']
                  if source.get('path') == '../target/p10-flatpak/' + name)
    with (staging / name).open('rb') as archive:
        digest = hashlib.file_digest(archive, 'sha256').hexdigest()
    print(f'{name}: {digest}')
    changed |= source['sha256'] != digest
    source['sha256'] = digest
if changed:
    if sys.argv[2] != 'true':
        sys.exit('Archives differ from manifest pins. Review the source changes, then run '
                 'bash flatpak/prepare.sh --update-pins as a maintainer.')
    manifest.write_text(json.dumps(data, indent=2) + '\n')
    print('Updated manifest source pins.')
PY
mv "$staging/app-source.tar.xz" "$state/app-source.tar.xz"
mv "$staging/vendor.tar.xz" "$state/vendor.tar.xz"
printf 'Prepared pinned Flatpak archives in %s\n' "$state"
