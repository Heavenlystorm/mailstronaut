#!/usr/bin/env bash
# Run with: nix develop .#servo-probe --command bash experiments/p01-servo/reproduce.sh
# Set PROBE_DIR to a dedicated previous probe workspace to reuse its target cache.
set -euo pipefail
probe_dir=${PROBE_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/mailstronaut-servo-probe.XXXXXXXX")}
mkdir -p "$probe_dir"
printf 'Probe workspace: %s\n' "$probe_dir"
cp Cargo.toml README.md "$probe_dir/"
cp experiments/p01-servo/Cargo.lock "$probe_dir/Cargo.lock"
cp -R crates "$probe_dir/"
mkdir -p "$probe_dir/crates/probe/src"
sed -i '/\[workspace.dependencies\]/a servo = { version = "=0.7.0", default-features = false, features = ["baked-in-resources"] }' "$probe_dir/Cargo.toml"
# Isolated compatibility experiment only; does not change the application graph.
sed -i 's/=0.40.0/=0.38.0/; s/=2.6.0/=2.4.1/' "$probe_dir/Cargo.toml"
cat > "$probe_dir/crates/probe/Cargo.toml" <<'MANIFEST'
[package]
name = "servo-probe"
version.workspace = true
edition.workspace = true
[dependencies]
servo.workspace = true
url.workspace = true
MANIFEST
cp experiments/p01-servo/probe.rs "$probe_dir/crates/probe/src/lib.rs"
cp experiments/p01-servo/pixels.rs "$probe_dir/crates/probe/src/main.rs"
# SpiderMonkey's xsum deliberately uses -Wno-format, which conflicts with
# the Nix compiler wrapper's format-security error flags. Keep other hardening.
NIX_HARDENING_ENABLE=" ${NIX_HARDENING_ENABLE:-} "
NIX_HARDENING_ENABLE=${NIX_HARDENING_ENABLE// format / }
export NIX_HARDENING_ENABLE AS="$CC" CARGO_BUILD_JOBS=2 CMAKE_BUILD_PARALLEL_LEVEL=2 MAKEFLAGS=-j2
for tool in "$CC" "$CXX" "$AS" clang python3 cmake make cargo rustc; do
    printf '%s: %s\n' "$tool" "$(command -v "$tool")"
done
# Building checks both the library and binary; a separate check can rebuild the
# expensive native dependencies under another Cargo fingerprint.
cargo build --manifest-path "$probe_dir/Cargo.toml" -p servo-probe --jobs 2 --locked

# Fixed synthetic data URL only. This is not a resource-policy test.
(cd "$probe_dir" && timeout 45s target/debug/servo-probe)
