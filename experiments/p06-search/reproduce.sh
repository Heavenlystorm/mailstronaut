#!/usr/bin/env bash
# Run through nix develop .#search-probe; all generated artifacts stay in project.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
probe="$root/target/p06-search/probe"
mkdir -p "$probe/crates/search-probe/src" "$root/target/p06-search/runtime"
mkdir -p "$probe/crates/embedding-probe/src"
chmod 700 "$root/target/p06-search/runtime"
sed -E '/^(usearch|rand|rayon) = /d' experiments/p06-search/workspace.toml.in > "$probe/Cargo.toml"
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
cat > "$probe/crates/embedding-probe/Cargo.toml" <<'TOML'
[package]
name = "mailstronaut-embedding-probe"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
[dependencies]
candle-core.workspace = true
candle-nn.workspace = true
candle-transformers.workspace = true
tokenizers.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
model2vec-rs.workspace = true
lancedb.workspace = true
arrow-array.workspace = true
arrow-schema.workspace = true
futures-util.workspace = true
tokio.workspace = true
[lints]
workspace = true
TOML
cp experiments/p06-search/embedding.rs "$probe/crates/embedding-probe/src/main.rs"
cp experiments/p06-search/model-manifest.json experiments/p06-search/potion-model-manifest.json experiments/p06-search/relevance.json "$probe/crates/embedding-probe/src/"
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
  model-fetch)
    python3 "$root/experiments/p06-search/provision-model.py"
    ;;
  potion-fetch)
    python3 "$root/experiments/p06-search/provision-model.py" --potion
    ;;
  embedding-build)
    cp "$root/experiments/p06-search/Cargo.lock" Cargo.lock
    cargo fmt --all -- --check
    cargo build -p mailstronaut-embedding-probe --locked --offline
    cargo clippy -p mailstronaut-embedding-probe --all-targets --locked --offline -- -D warnings
    ;;
  embedding-release)
    cp "$root/experiments/p06-search/Cargo.lock" Cargo.lock
    command time -v -o "$root/target/p06-search/embedding-build-resources.txt" cargo build --release -p mailstronaut-embedding-probe --locked --offline
    ;;
  embedding-smoke|embedding-bench|potion-smoke|potion-bench)
    run=$(mktemp -d "$root/target/p06-search/embedding.XXXXXXXX")
    binary="$CARGO_TARGET_DIR/release/mailstronaut-embedding-probe"
    model="$root/target/p06-search/model"
    arguments=()
    if [[ $1 == potion-smoke || $1 == potion-bench ]]; then
      if [[ $1 == potion-smoke ]]; then binary="$CARGO_TARGET_DIR/debug/mailstronaut-embedding-probe"; fi
      model="$root/target/p06-search/model-potion"
      arguments=(--potion)
    fi
    result=0
    if [[ $1 != embedding-bench && $1 != potion-bench ]]; then
      command time -v -o "$run/resources.txt" strace -f -qq -e trace=network -o "$run/network.trace" "$binary" "$model" "$run/data" "$run/embeddings.json" "${arguments[@]}" | tee "$run/metrics.txt" || result=$?
      python3 - "$run/network.trace" <<'PYNETWORK'
import pathlib,sys
assert 'AF_INET' not in pathlib.Path(sys.argv[1]).read_text(), 'inference attempted IP networking'
print('The bounded local model/search process made no IP socket calls.')
PYNETWORK
    else
      command time -v -o "$run/resources.txt" "$binary" "$model" "$run/data" "$run/embeddings.json" "${arguments[@]}" | tee "$run/metrics.txt" || result=$?
    fi
    printf 'Embedding artifacts: %s\n' "$run"
    exit "$result"
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
  *) printf 'Usage: %s {lock|fetch|build|release|bench|smoke|model-fetch|embedding-build|embedding-release|embedding-smoke|embedding-bench|potion-fetch|potion-smoke|potion-bench}\n' "$0" >&2; exit 2 ;;
esac
