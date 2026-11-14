#!/usr/bin/env bash
# Isolated safe API/parity probe; no application classifier adoption claim.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
probe="$root/target/p09-priority/probe"
mkdir -p "$probe/crates/nli/src"
# The existing minimal encoder graph already pins every required dependency.
sed -E '/^(lancedb|arrow-array|arrow-schema|futures-util|tokio|model2vec-rs|usearch|rand|rayon) = /d' \
  experiments/p06-search/workspace.toml.in > "$probe/Cargo.toml"
python3 - "$probe" <<'PY'
import pathlib,sys
probe=pathlib.Path(sys.argv[1])
text=pathlib.Path('Cargo.toml').read_text()
with (probe/'Cargo.toml').open('a') as output:
    output.write('\n'+text[text.index('[workspace.lints.rust]'):text.index('# Retain source locations')])
lock=pathlib.Path('experiments/p06-search/encoder-Cargo.lock').read_text()
assert lock.count('name = "mailstronaut-encoder-probe"')==1
(probe/'Cargo.lock').write_text(lock.replace('name = "mailstronaut-encoder-probe"','name = "mailstronaut-nli-probe"'))
PY
cat > "$probe/crates/nli/Cargo.toml" <<'TOML'
[package]
name = "mailstronaut-nli-probe"
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
[lints]
workspace = true
TOML
cp experiments/p09-priority/nli.rs "$probe/crates/nli/src/main.rs"
cp experiments/p09-priority/nli-model-manifest.json experiments/p09-priority/nli-pairs.json "$probe/crates/nli/src/"
export CARGO_TARGET_DIR="$root/target"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
case "${1:-check}" in
  check)
    cargo fmt --manifest-path "$probe/Cargo.toml" --all
    cargo clippy --manifest-path "$probe/Cargo.toml" --release --all-targets --locked --offline -- -D warnings
    cargo build --manifest-path "$probe/Cargo.toml" --release --locked --offline
    ;;
  run)
    mkdir -p target/p09-priority/parity
    command time -v -o target/p09-priority/parity/resources.txt \
      "$CARGO_TARGET_DIR/release/mailstronaut-nli-probe" \
      "$root/target/p09-priority/model" "$root/target/p09-priority/parity/observed.json" \
      | tee target/p09-priority/parity/metrics.log
    ;;
  *) printf 'Usage: %s [check|run]\n' "$0" >&2; exit 2 ;;
esac
