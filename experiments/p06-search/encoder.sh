#!/usr/bin/env bash
# Lean BERT probe: existing dependency pins/lints, shared app target, no LanceDB build.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
probe="$root/target/p06-search/encoder"
mkdir -p "$probe/crates/encoder/src"
# Keep the shared pins, omitting the original probe's unused engine dependencies.
sed -E '/^(lancedb|arrow-array|arrow-schema|futures-util|tokio|model2vec-rs|usearch|rand|rayon) = /d' \
  experiments/p06-search/workspace.toml.in > "$probe/Cargo.toml"
python3 - "$probe/Cargo.toml" <<'PY'
import pathlib,sys
text=pathlib.Path('Cargo.toml').read_text()
with pathlib.Path(sys.argv[1]).open('a') as output:
    output.write('\n'+text[text.index('[workspace.lints.rust]'):text.index('# Retain source locations')])
PY
cat > "$probe/crates/encoder/Cargo.toml" <<'TOML'
[package]
name = "mailstronaut-encoder-probe"
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
cp experiments/p06-search/encoder.rs "$probe/crates/encoder/src/main.rs"
cp experiments/p06-search/minilm-model-manifest.json experiments/p06-search/relevance.json "$probe/crates/encoder/src/"
export CARGO_TARGET_DIR="$root/target"
export CARGO_CACHE_AUTO_CLEAN_FREQUENCY=never
lock="$root/experiments/p06-search/encoder-Cargo.lock"
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
    run=$(mktemp -d "$root/target/p06-search/minilm.XXXXXX")
    command time -v -o "$run/resources.txt" "$CARGO_TARGET_DIR/release/mailstronaut-encoder-probe" \
      "$root/target/p06-search/model-minilm" "$run/embeddings.json" | tee "$run/metrics.txt"
    printf 'Encoder artifacts: %s\n' "$run"
    ;;
  privacy)
    run=$(mktemp -d "$root/target/p06-search/minilm-privacy.XXXXXX")
    strace -f -qq -e trace=network -o "$run/positive.trace" python3 - <<'PY'
import socket
with socket.socket() as server:
    server.bind(('127.0.0.1',0)); server.listen(1)
    with socket.create_connection(server.getsockname()) as client:
        connection,_=server.accept()
        with connection: connection.sendall(b'positive-control')
        assert client.recv(32)==b'positive-control'
PY
    strace -f -qq -e trace=network -o "$run/encoder.trace" \
      "$CARGO_TARGET_DIR/release/mailstronaut-encoder-probe" \
      "$root/target/p06-search/model-minilm" "$run/embeddings.json" > "$run/metrics.txt"
    python3 - "$run" <<'PY'
import pathlib,sys
root=pathlib.Path(sys.argv[1])
assert 'AF_INET' in (root/'positive.trace').read_text(), 'observer did not see TCP control'
assert 'AF_INET' not in (root/'encoder.trace').read_text(), 'encoder attempted IP networking'
print('Actual TCP positive control observed; bounded Rust encoder made no IP socket calls.')
PY
    mkdir -p "$run/tampered/1_Pooling"
    cp "$root/target/p06-search/model-minilm/1_Pooling/config.json" "$run/tampered/1_Pooling/"
    printf 'not-the-pinned-config\n' > "$run/tampered/config.json"
    # Match the expected size so the checksum guard, rather than the size guard, fails.
    truncate -s "$(stat -c %s "$root/target/p06-search/model-minilm/config.json")" "$run/tampered/config.json"
    if "$CARGO_TARGET_DIR/release/mailstronaut-encoder-probe" \
      "$run/tampered" "$run/unexpected.json" > "$run/tampered.log" 2>&1; then
      printf '%s\n' 'Tampered model was accepted' >&2; exit 1
    fi
    grep -q 'artifact checksum mismatch' "$run/tampered.log"
    [[ ! -e "$run/unexpected.json" ]]
    printf 'Tampered config rejected before inference; artifacts: %s\n' "$run"
    ;;
  *) printf 'Expected lock, fetch, check, run or privacy\n' >&2; exit 2 ;;
esac
