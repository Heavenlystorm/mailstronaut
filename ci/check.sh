#!/usr/bin/env bash
# Run with: nix develop --command bash ci/check.sh
set -euo pipefail

cargo fmt --all -- --check
nixfmt --check flake.nix examples/config.nix examples/home-manager.nix
actionlint
shellcheck ci/check.sh experiments/p02-accounts/credentials.sh experiments/p02-accounts/gui.sh experiments/p04-inbox/gui.sh experiments/p07-compose/gui.sh
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
