#!/usr/bin/env bash
# Run with: nix develop --command bash ci/check.sh
set -euo pipefail

cargo fmt --all -- --check
nixfmt --check flake.nix examples/config.nix examples/home-manager.nix
actionlint
shellcheck ci/check.sh
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
