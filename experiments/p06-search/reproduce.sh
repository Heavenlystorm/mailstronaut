#!/usr/bin/env bash
# Run through nix develop .#search-probe; all generated artifacts stay in project.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
probe="$root/target/p06-search/probe"
mkdir -p "$probe/crates/search-probe/src" "$root/target/p06-search/runtime"
chmod 700 "$root/target/p06-search/runtime"
cp experiments/p06-search/workspace.toml.in "$probe/Cargo.toml"
python3 - "$root" "$probe" <<'PY'
import pathlib,sys
root,probe=map(pathlib.Path,sys.argv[1:])
text=(root/'Cargo.toml').read_text()
lints=text[text.index('[workspace.lints.rust]'):text.index('# Retain source locations')]
with (probe/'Cargo.toml').open('a') as output: output.write('\n'+lints)
PY
cat > "$probe/crates/search-probe/Cargo.toml" <<'TOML'
[package]
name = "mailstronaut-search-probe"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
[dependencies]
lancedb.workspace = true
arrow-array.workspace = true
arrow-schema.workspace = true
futures-util.workspace = true
tokio.workspace = true
[lints]
workspace = true
TOML
cp experiments/p06-search/probe.rs "$probe/crates/search-probe/src/main.rs"
export CARGO_TARGET_DIR="$root/target/p06-search/target"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
export XDG_CACHE_HOME="$root/target/nix-cache"
export XDG_RUNTIME_DIR="$root/target/p06-search/runtime"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(nproc)}"
export CMAKE_BUILD_PARALLEL_LEVEL="$CARGO_BUILD_JOBS" MAKEFLAGS="-j$CARGO_BUILD_JOBS"
cd "$probe"
case ${1:-smoke} in
  lock)
    cargo generate-lockfile
    cp Cargo.lock "$root/experiments/p06-search/Cargo.lock"
    ;;
  fetch)
    cp "$root/experiments/p06-search/Cargo.lock" Cargo.lock
    cargo fetch --locked
    ;;
  build)
    cp "$root/experiments/p06-search/Cargo.lock" Cargo.lock
    cargo fmt --all -- --check
    cargo build --workspace --all-features --locked --offline --jobs "$CARGO_BUILD_JOBS"
    cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
    ;;
  release)
    cp "$root/experiments/p06-search/Cargo.lock" Cargo.lock
    command time -v -o "$root/target/p06-search/release-build-resources.txt" cargo build --release --workspace --all-features --locked --offline --jobs "$CARGO_BUILD_JOBS"
    ;;
  bench)
    binary="$CARGO_TARGET_DIR/release/mailstronaut-search-probe"
    for rows in 10000 100000; do
      run=$(mktemp -d "$root/target/p06-search/bench-$rows.XXXXXXXX")
      command time -v -o "$run/resources.txt" "$binary" "$run/data" "--bench=$rows" | tee "$run/metrics.txt"
      python3 - "$run/data" <<'PYSIZE'
import os,pathlib,stat,sys
root=pathlib.Path(sys.argv[1]);total=0
for directory,_,files in os.walk(root,followlinks=False):
 for name in files:
  entry=(pathlib.Path(directory)/name).lstat()
  if stat.S_ISREG(entry.st_mode):total+=entry.st_size
print(f'index_bytes={total}')
PYSIZE
      printf 'Benchmark artifacts: %s\n' "$run"
    done
    ;;
  smoke)
    run=$(mktemp -d "$root/target/p06-search/run.XXXXXXXX")
    binary="$CARGO_TARGET_DIR/debug/mailstronaut-search-probe"
    strace -f -qq -e trace=network -o "$run/control.trace" "$binary" --network-observer-control
    command time -v -o "$run/create-resources.txt" strace -f -qq -e trace=network -o "$run/create.trace" "$binary" "$run/data"
    command time -v -o "$run/reopen-resources.txt" strace -f -qq -e trace=network -o "$run/reopen.trace" "$binary" "$run/data" --reopen
    python3 - "$run" <<'PYTRACE'
import pathlib,sys
root=pathlib.Path(sys.argv[1])
control=(root/'control.trace').read_text()
assert 'connect(' in control and 'AF_INET' in control, 'network observer positive control failed'
for name in ['create','reopen']:
 trace=(root/f'{name}.trace').read_text()
 assert 'AF_INET' not in trace, f'{name} attempted IP networking'
print('Both bounded search runs made no IP socket calls; the observer detected the loopback control.')
PYTRACE
    printf 'Probe runtime data: %s\n' "$run"
    ;;
  *) printf 'Usage: %s {lock|fetch|build|release|bench|smoke}\n' "$0" >&2; exit 2 ;;
esac
