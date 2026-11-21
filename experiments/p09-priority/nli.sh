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
python3 - "$probe" <<'PYTHON'
import pathlib,sys,tomllib
probe=pathlib.Path(sys.argv[1])
text=pathlib.Path('Cargo.toml').read_text()
pins=tomllib.loads(text)['workspace']['dependencies']
base=(probe/'Cargo.toml').read_text()
extra='\nchrono = { version = "'+pins['chrono']+'", features = ["serde"] }\nchrono-tz = "'+pins['chrono-tz']+'"\nregex = "'+pins['regex']+'"\n'
base=base.replace('\n[profile.dev]',extra+'\n[profile.dev]',1)
(probe/'Cargo.toml').write_text(base+'\n'+text[text.index('[workspace.lints.rust]'):text.index('# Retain source locations')])
frozen=pathlib.Path('experiments/p09-priority/Cargo.lock')
if frozen.exists():
    lock=frozen.read_text()
else:
    lock=pathlib.Path('Cargo.lock').read_text()
(probe/'Cargo.lock').write_text(lock)
PYTHON
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
chrono.workspace = true
chrono-tz.workspace = true
regex.workspace = true
[lints]
workspace = true
TOML
cp experiments/p09-priority/nli.rs "$probe/crates/nli/src/main.rs"
cp experiments/p09-priority/nli-quality.rs "$probe/crates/nli/src/quality.rs"
mkdir -p "$probe/crates/nli/src/rules"
# The registered p1/p2 scores used policy 2 from this exact application commit.
rules_revision=783e1fa459524fa4169273f59f54b78cacce3fbb
git show "$rules_revision:crates/mailstronaut-priority/src/lib.rs" > "$probe/crates/nli/src/rules/mod.rs"
git show "$rules_revision:crates/mailstronaut-priority/src/deadline.rs" > "$probe/crates/nli/src/rules/deadline.rs"
cp experiments/p09-priority/priority-corpus.json experiments/p09-priority/semantic-challenges.json "$probe/crates/nli/src/"
cp experiments/p09-priority/nli-model-manifest.json experiments/p09-priority/nli-pairs.json "$probe/crates/nli/src/"
export CARGO_TARGET_DIR="$root/target"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
case "${1:-check}" in
  quality-lock)
    cargo generate-lockfile --manifest-path "$probe/Cargo.toml" --offline
    cp "$probe/Cargo.lock" experiments/p09-priority/Cargo.lock
    ;;
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
  quality-development|quality-heldout)
    split=${1#quality-}
    run="$root/target/p09-priority/$split"
    mkdir -p "$run"
    command time -v -o "$run/resources.txt" \
      "$CARGO_TARGET_DIR/release/mailstronaut-nli-probe" \
      "$root/target/p09-priority/model" "$run/observed.json" "$split" \
      | tee "$run/metrics.log"
    ;;
  *) printf 'Usage: %s [quality-lock|check|run|quality-development|quality-heldout]\n' "$0" >&2; exit 2 ;;
esac
