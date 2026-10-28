#!/usr/bin/env bash
# Run through nix develop after the normal flake gate. Builds only one package at a time.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
mkdir -p target/nix-cache
exec {cache_lock}>target/p01-package-gate.lock
if ! flock --nonblock "$cache_lock"; then
  printf '%s\n' 'Another coordinated package build is running.' >&2
  exit 73
fi
export XDG_CACHE_HOME="$root/target/nix-cache" CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
work=$(mktemp -d "$root/target/p01-cache-reuse.XXXXXXXX")
printf 'Cache acceptance artifacts: %s\n' "$work"
printf 'running\n' > "$work/status"
trap 'printf "%s\n" "$?" > "$work/status"' EXIT
system=$(nix eval --impure --raw --expr builtins.currentSystem)
jobs=$(nproc)
base_ref="git+file://$root"
fixture="$work/fixture"
python3 - "$root" "$fixture" <<'PY'
from pathlib import Path
import shutil
import subprocess
import sys
root, fixture = map(Path, sys.argv[1:])
fixture.mkdir()
for raw in subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).split(b'\0'):
    if not raw:
        continue
    relative = Path(raw.decode())
    source = root / relative
    if source.is_file() or source.is_symlink():
        destination = fixture / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination, follow_symlinks=False)
main = fixture / 'crates/mailstronaut/src/main.rs'
main.write_bytes(main.read_bytes() + b'\n// Source-only dependency-cache acceptance fixture: ' + fixture.parent.name.encode() + b'\n')
for relative in [Path('Cargo.lock'), Path('Cargo.toml'), *root.glob('crates/*/Cargo.toml')]:
    relative = relative.relative_to(root) if relative.is_absolute() else relative
    assert (root / relative).read_bytes() == (fixture / relative).read_bytes(), relative
PY
identities='p: { package = p.drvPath; packageOutput = p.outPath; source = p.src.outPath or p.src; artifacts = p.cargoArtifacts.drvPath; artifactOutput = p.cargoArtifacts.outPath; vendor = p.cargoDeps.drvPath; vendorOutput = p.cargoDeps.outPath; }'
nix eval --no-update-lock-file --json "$base_ref#packages.$system.default" \
  --apply "$identities" > "$work/baseline.json"
nix eval --no-update-lock-file --json "path:$fixture#packages.$system.default" \
  --apply "$identities" > "$work/variant.json"
python3 - "$work" <<'PY'
import json
from pathlib import Path
import sys
work = Path(sys.argv[1])
base = json.loads((work / 'baseline.json').read_text())
variant = json.loads((work / 'variant.json').read_text())
for field in ('artifacts', 'artifactOutput', 'vendor', 'vendorOutput'):
    assert base[field] == variant[field], f'{field} changed across a source-only edit'
assert base['source'] != variant['source'], 'fixture did not change package source'
assert base['package'] != variant['package'], 'fixture did not change package derivation'
print('Exact dependency/vendor identities are stable; package and source differ.')
PY
start=$(date +%s)
nix build --no-update-lock-file --print-build-logs --cores "$jobs" --max-jobs 1 \
  --out-link "$work/baseline" "$base_ref#default" > "$work/baseline.log" 2>&1
printf '%s\n' "$(( $(date +%s) - start ))" > "$work/baseline-seconds"
start=$(date +%s)
nix build --no-update-lock-file --print-build-logs --cores "$jobs" --max-jobs 1 \
  --out-link "$work/variant" "path:$fixture#default" > "$work/variant.log" 2>&1
printf '%s\n' "$(( $(date +%s) - start ))" > "$work/variant-seconds"
python3 - "$work" <<'PY'
from pathlib import Path
import re
import sys
work = Path(sys.argv[1])
log = (work / 'variant.log').read_text()
built = re.findall(r'\b(?:Compiling|Checking) ([\w-]+) v', log)
workspace = {'mailstronaut', 'mailstronaut-config', 'mailstronaut-core', 'mailstronaut-render'}
assert set(built) <= workspace, f'third-party crates recompiled: {set(built) - workspace}'
assert 'mailstronaut' in built, 'source-only application build was not observed'
assert 'test result: ok.' in log, 'actual tests were not observed'
print('Source-only build compiled workspace crates only; third-party artifacts were reused.')
PY
for kind in baseline variant; do
  nix-store --query --requisites "$(readlink -f "$work/$kind")" > "$work/$kind-closure.txt"
  bash experiments/p01-nix/installed-smoke.sh "$(readlink -f "$work/$kind")" \
    > "$work/$kind-smoke.log" 2>&1
done
python3 - "$work" <<'PY_CLOSURE'
import json
from pathlib import Path
import sys
work = Path(sys.argv[1])
artifact = json.loads((work / 'baseline.json').read_text())['artifactOutput']
for kind in ('baseline', 'variant'):
    closure = (work / f'{kind}-closure.txt').read_text().splitlines()
    assert artifact not in closure, 'build artifacts leaked into the runtime closure'
print('Dependency artifact is absent from both installed runtime closures.')
PY_CLOSURE
printf 'Cache source-only acceptance passed. Artifacts: %s\n' "$work"
