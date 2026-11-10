#!/usr/bin/env bash
# Probe USearch through its safe Rust API; no new application dependencies.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
probe="$root/target/p06-search/vector"
mkdir -p "$probe/crates/vector/src"
sed -E '/^(lancedb|arrow-array|arrow-schema|futures-util|tokio|model2vec-rs|candle-core|candle-nn|candle-transformers|tokenizers|sha2) = /d' \
  experiments/p06-search/workspace.toml.in > "$probe/Cargo.toml"
python3 - "$probe/Cargo.toml" <<'PY'
import pathlib,sys
text=pathlib.Path('Cargo.toml').read_text()
with pathlib.Path(sys.argv[1]).open('a') as output:
    output.write('\n'+text[text.index('[workspace.lints.rust]'):text.index('# Retain source locations')])
PY
cat > "$probe/crates/vector/Cargo.toml" <<'TOML'
[package]
name = "mailstronaut-vector-probe"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
[dependencies]
usearch.workspace = true
rand.workspace = true
rayon.workspace = true
serde.workspace = true
serde_json.workspace = true
[lints]
workspace = true
TOML
cp experiments/p06-search/vector.rs "$probe/crates/vector/src/main.rs"
export CARGO_TARGET_DIR="$root/target"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
lock="$root/experiments/p06-search/vector-Cargo.lock"
case "${1:-check}" in
  lock)
    cargo generate-lockfile --manifest-path "$probe/Cargo.toml"
    cp "$probe/Cargo.lock" "$lock"
    ;;
  fetch)
    cp "$lock" "$probe/Cargo.lock"
    cargo fetch --manifest-path "$probe/Cargo.toml" --locked
    ;;
  check)
    cp "$lock" "$probe/Cargo.lock"
    cargo fmt --manifest-path "$probe/Cargo.toml" --all -- --check
    cargo clippy --manifest-path "$probe/Cargo.toml" --release --all-targets --locked --offline -- -D warnings
    cargo build --manifest-path "$probe/Cargo.toml" --release --locked --offline
    ;;
  run)
    : "${2:?actual model vectors}" "${3:?matching frozen fixture}"
    run=$(mktemp -d "$root/target/p06-search/vector.XXXXXX")
    command time -v -o "$run/resources.txt" "$CARGO_TARGET_DIR/release/mailstronaut-vector-probe" "$2" "$3" "$run/data" | tee "$run/results.txt"
    printf 'Vector artifacts: %s\n' "$run"
    ;;
  bench)
    : "${2:?10000 or 100000}"
    run=$(mktemp -d "$root/target/p06-search/vector-bench-$2.XXXXXX")
    command time -v -o "$run/resources.txt" "$CARGO_TARGET_DIR/release/mailstronaut-vector-probe" --bench "$2" "$run/data" | tee "$run/results.txt"
    printf 'Vector benchmark artifacts: %s\n' "$run"
    ;;
  *) printf 'Expected lock, fetch, check, run or bench\n' >&2; exit 2 ;;
esac
