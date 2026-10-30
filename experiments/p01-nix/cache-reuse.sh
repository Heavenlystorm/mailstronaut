#!/usr/bin/env bash
# Run through nix develop, with one package build at a time.
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
sandbox=${CACHE_REUSE_SANDBOX:-true}
case "$sandbox" in
  true|false) ;;
  *) printf '%s\n' 'CACHE_REUSE_SANDBOX must be true or false.' >&2; exit 2 ;;
esac
# Restricted daemon settings can be ignored with only a warning. Reject that
# before compiling when an unsandboxed, CI-equivalent probe is requested.
nix store info --option sandbox "$sandbox" > "$work/store-info.log" 2>&1
if [[ $sandbox == false ]] && rg -q "ignoring the client-specified setting 'sandbox'" "$work/store-info.log"; then
  cat "$work/store-info.log" >&2
  printf '%s\n' 'Unsandboxed acceptance requires a store that permits disabling sandboxing.' >&2
  exit 1
fi
printf 'Requested sandbox setting: %s\n' "$sandbox"
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
cli = fixture / 'crates/mailstronaut/src/config_cli.rs'
source = cli.read_text()
needle = '"mailstronaut {}"'
assert source.count(needle) == 1, 'expected one public version format'
marker = f'cache probe {fixture.parent.name}'
cli.write_text(source.replace(needle, f'"mailstronaut {{}} ({marker})"'))
for relative in [Path('Cargo.lock'), Path('Cargo.toml'), *root.glob('crates/*/Cargo.toml')]:
    relative = relative.relative_to(root) if relative.is_absolute() else relative
    assert (root / relative).read_bytes() == (fixture / relative).read_bytes(), relative
PY
cargo fmt --manifest-path "$fixture/Cargo.toml" --all
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
nix build --option sandbox "$sandbox" --no-update-lock-file --print-build-logs --cores "$jobs" --max-jobs 1 \
  --out-link "$work/baseline" "$base_ref#default" > "$work/baseline.log" 2>&1
printf '%s\n' "$(( $(date +%s) - start ))" > "$work/baseline-seconds"
start=$(date +%s)
nix build --option sandbox "$sandbox" --no-update-lock-file --print-build-logs --cores "$jobs" --max-jobs 1 \
  --out-link "$work/variant" "path:$fixture#default" > "$work/variant.log" 2>&1
printf '%s\n' "$(( $(date +%s) - start ))" > "$work/variant-seconds"
python3 - "$work" "$sandbox" <<'PY'
from pathlib import Path
import re
import sys
work = Path(sys.argv[1])
import tomllib
workspace = {
    tomllib.loads(manifest.read_text())['package']['name']
    for manifest in (work / 'fixture/crates').glob('*/Cargo.toml')
}
for kind in ('baseline', 'variant'):
    # The dependency derivation must compile dependencies once. Only the real
    # application derivation must reuse them after restoration, on both builds.
    lines = [line for line in (work / f'{kind}.log').read_text().splitlines()
             if line.startswith('mailstronaut>')]
    built = set(re.findall(r'\b(?:Compiling|Checking) ([\w-]+) v', '\n'.join(lines)))
    assert built <= workspace, f'{kind}: third-party crates rebuilt: {built - workspace}'
    if kind == 'variant':
        assert 'mailstronaut' in built, 'changed application was not compiled'
        if sys.argv[2] == 'false':
            roots = re.findall(r'Compiling mailstronaut v[^ ]+ \((.+)/crates/mailstronaut\)', '\n'.join(lines))
            assert roots and all(root != '/build/source' for root in roots), 'unsandboxed build paths were not observed'
            print('Actual unsandboxed package source:', roots[0])
        assert any('test result: ok.' in line for line in lines), 'actual tests were not observed'
print(f'Both real package builds reused third-party artifacts (requested sandbox={sys.argv[2]}).')
PY
for kind in baseline variant; do
  "$work/$kind/bin/mailstronaut" --version > "$work/$kind-version.txt"
  nix-store --query --requisites "$(readlink -f "$work/$kind")" > "$work/$kind-closure.txt"
  bash experiments/p01-nix/installed-smoke.sh "$(readlink -f "$work/$kind")" \
    > "$work/$kind-smoke.log" 2>&1
done
python3 - "$work" <<'PY_CLOSURE'
import json
from pathlib import Path
import sys
work = Path(sys.argv[1])
identities = json.loads((work / 'baseline.json').read_text())
artifact = identities['artifactOutput']
vendor = identities['vendorOutput']
for kind in ('baseline', 'variant'):
    closure = (work / f'{kind}-closure.txt').read_text().splitlines()
    assert artifact not in closure, 'build artifacts leaked into the runtime closure'
    assert vendor not in closure, 'vendor sources leaked into the runtime closure'
assert 'cache probe' not in (work / 'baseline-version.txt').read_text()
assert f'cache probe {work.name}' in (work / 'variant-version.txt').read_text()
print('Changed installed behavior is observed; both runtime closures exclude artifacts/vendor.')
PY_CLOSURE
printf 'Cache source-only acceptance passed. Artifacts: %s\n' "$work"
