#!/usr/bin/env bash
# Run inside nix develop, from the repository root. The temporary workspace is
# printed for inspection; nothing in the real lockfile is changed.
set -euo pipefail
probe_dir=$(mktemp -d "${TMPDIR:-/tmp}/mailstronaut-servo-probe.XXXXXXXX")
printf 'Probe workspace: %s\n' "$probe_dir"
cp Cargo.toml Cargo.lock README.md "$probe_dir/"
cp -R crates "$probe_dir/"
mkdir -p "$probe_dir/crates/servo-probe/src"
sed -i '/\[workspace.dependencies\]/a servo = { version = "=0.7.0", default-features = false, features = ["baked-in-resources"] }' "$probe_dir/Cargo.toml"
if [[ ${1:-} == --compatible-sqlite ]]; then
    sed -i 's/=0.40.0/=0.38.0/; s/=2.6.0/=2.4.1/' "$probe_dir/Cargo.toml"
fi
cat > "$probe_dir/crates/servo-probe/Cargo.toml" <<'MANIFEST'
[package]
name = "servo-probe"
version.workspace = true
edition.workspace = true
[dependencies]
servo.workspace = true
MANIFEST
cp experiments/p01-servo/probe.rs "$probe_dir/crates/servo-probe/src/lib.rs"
cargo check --manifest-path "$probe_dir/Cargo.toml" -p servo-probe
