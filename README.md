# Mailstronaut

A local-first Rust email client built with **GPUI Kit + Blitz**, for **native
Wayland**. The project is under development; it is not ready to replace a daily
email client. The current roadmap is [docs/PLAN.md](docs/PLAN.md).

The UI is being rebuilt with GPUI Kit's components, assets and light/dark themes.
Blitz lays out passive HTML and AnyRender/Vello CPU paints bounded images for the
native window. HTML resource/privacy and accessibility acceptance are explicit
P01 gates; previous toolkit screenshots are historical evidence.

The retained backend provides native Nix configuration, encrypted credentials,
TLS IMAP/SMTP, offline SQLite/MIME storage, account-local conversations, durable
drafts/attachments, submission state and reminders. Their new GPUI workflows are
tracked separately in the plan. Strong fuzzy and multilingual semantic search,
local priority/deadline inference and optional background operation remain goals.

```sh
nix develop
cargo run -- --demo           # synthetic UI/Blitz preview, no accounts or network
cargo run                    # configured local mail cache
cargo run -- config init
cargo run -- config check
bash ci/check.sh
nix flake check --no-update-lock-file --print-build-logs
nix build --no-update-lock-file
```

A Wayland session is required for the window; CLI configuration commands also work
without a display. `--demo` never opens the real account store. Cached message
content becomes read only through an explicit successful open, never list/search
refresh or focus. Currently the replacement UI is a foundation: account setup,
network sync, compose and notification workflows are being restored.

Development uses pinned nightly Rust, Cranelift and Wild; release uses stable
LLVM, thin LTO and one codegen unit. See [delivery](docs/DELIVERY.md) for native
inputs, checks and distribution limitations. All application code is Rust.

Configuration is a directly evaluated `.nix` file, without a TOML conversion or
Home Manager module. See [configuration](docs/CONFIGURATION.md),
[example](examples/config.nix) and [Home Manager composition](examples/home-manager.nix).
Secrets belong in encrypted storage and must never be put in the Nix store.

Architecture and decisions: [architecture](docs/ARCHITECTURE.md),
[dependencies](docs/DEPENDENCIES.md), [rendering](docs/RENDERING.md),
[search](docs/SEARCH.md), [classifier](docs/CLASSIFIER.md).

Nix and Flatpak are release targets. A new-stack Nix package and actual Flatpak
runtime/portal/Wayland acceptance must pass before distribution is claimed.
Historical evidence is indexed in [docs/evidence](docs/evidence/README.md).

Project-authored code is GPL-3.0-only; dependencies and bundled assets retain
their respective licenses. See [LICENSE](LICENSE).
