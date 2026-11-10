#!/usr/bin/env bash
# Run on CI with: nix develop .#gui-test --command bash flatpak/check.sh
# Requires the current app to have been built by build-and-install.sh first.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
mkdir -p target/p10-flatpak
export FLATPAK_SYSTEM_DIR="$PWD/target/p01-flatpak/system"
export MAILSTRONAUT_FLATPAK_USER_DIR="$PWD/target/p01-flatpak/installation"
bash experiments/p01-flatpak/isolated.sh flatpak run --command=mailstronaut \
  --unshare=network io.github.eduardz1.Mailstronaut --version
bash experiments/p01-flatpak/isolated.sh flatpak run --command=mailstronaut \
  --unshare=network --filesystem="$PWD/examples:ro" io.github.eduardz1.Mailstronaut \
  config check --path "$PWD/examples/config.nix"
# Keep the workspace's unified feature graph; do not recompile individual targets.
cargo test --workspace --all-features --locked --offline --no-run --message-format=json \
  > target/p10-flatpak/test-artifacts.json
fixture_paths=$(python3 - <<'PY'
import json
import pathlib

artifacts = [json.loads(line) for line in
             pathlib.Path('target/p10-flatpak/test-artifacts.json').read_text().splitlines()]
for name in ('mailstronaut', 'flatpak_keyring'):
    paths = {item['executable'] for item in artifacts
             if item.get('reason') == 'compiler-artifact'
             and item['target']['name'] == name
             and item['profile']['test'] and item.get('executable')}
    if len(paths) != 1:
        raise SystemExit(f'Expected one test executable for {name}; got {len(paths)}')
    print(paths.pop())
PY
)
mapfile -t fixtures <<< "$fixture_paths"
export MAILSTRONAUT_PORTAL_FIXTURE="${fixtures[0]}"
export MAILSTRONAUT_SEED_FIXTURE="${fixtures[1]}"
MAILSTRONAUT_FLATPAK_GUI_BINARY=installed \
MAILSTRONAUT_FLATPAK_PORTAL_BINARY=installed \
MAILSTRONAUT_FLATPAK_ATTACH_BINARY=installed \
  bash experiments/p02-accounts/credentials.sh
printf 'flatpak-acceptance-exit=0\n'
