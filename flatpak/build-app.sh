#!/usr/bin/env bash
# Runs inside the Flatpak module. Builds the application offline from the pinned vendor tree and
# installs only its binary; nothing is fetched from the network during the build.
set -euo pipefail
build_jobs=$(nproc)
export CARGO_BUILD_JOBS="$build_jobs"
mkdir -p "$CARGO_HOME"
vendor_root=/run/build/mailstronaut/vendor
cp "$vendor_root/.cargo/config.toml" "$CARGO_HOME/config.toml"
cargo fetch --frozen --offline
cargo build --frozen --offline --release --package mailstronaut --jobs "$build_jobs"
install -Dm755 target/release/mailstronaut "$FLATPAK_DEST/bin/mailstronaut"
# The desktop entry and metainfo are shipped beside the application; the icon comes from the
# application's own assets.
install -Dm644 io.github.eduardz1.Mailstronaut.desktop \
  "$FLATPAK_DEST/share/applications/io.github.eduardz1.Mailstronaut.desktop"
install -Dm644 io.github.eduardz1.Mailstronaut.metainfo.xml \
  "$FLATPAK_DEST/share/metainfo/io.github.eduardz1.Mailstronaut.metainfo.xml"
install -Dm644 assets/icon.svg \
  "$FLATPAK_DEST/share/icons/hicolor/scalable/apps/io.github.eduardz1.Mailstronaut.svg"
