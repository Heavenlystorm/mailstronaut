#!/usr/bin/env bash
# Run with: nix develop --command bash ci/check.sh
set -euo pipefail

cargo fmt --all -- --check
nixfmt --check flake.nix examples/config.nix examples/home-manager.nix nix/home-manager.nix nix/home-manager-check.nix
actionlint
shellcheck ci/check.sh ci/nix-check.sh ci/remote.sh experiments/p01-blitz/privacy.sh experiments/p01-blitz/reader.sh experiments/p01-blitz/portal.sh experiments/p01-gpui/wayland.sh experiments/p02-accounts/credentials.sh experiments/p02-accounts/gui.sh experiments/p02-config/export-gui.sh experiments/p02-config/preferences-gui.sh experiments/p05-onboarding/gui.sh experiments/p05-onboarding/auth-gui.sh experiments/p06-mailbox/wayland.sh experiments/p06-mailbox/navigation.sh experiments/wayland-session.sh experiments/p07-compose/gui.sh experiments/p04-sync/gui.sh experiments/p04-sync/dovecot.sh experiments/p08-reminders/portal.sh experiments/p08-reminders/gui.sh experiments/p08-reminders/instance.sh experiments/p08-reminders/instance-gui.sh experiments/p06-search/reproduce.sh experiments/p06-search/run-release.sh experiments/p04-sync/idle.sh experiments/p04-sync/large.sh experiments/p04-sync/older.sh experiments/p06-search/gui.sh experiments/p07-compose/delivery.sh experiments/p07-compose/actions.sh experiments/p09-priority/gui.sh
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
