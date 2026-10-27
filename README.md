<img src="assets/icon.svg" width="64" height="64" alt="">

# Mailstronaut

A planned local-first email client for the COSMIC desktop, written in Rust with
libcosmic and Servo. This repository currently contains the architecture and a
buildable scaffold. **It is not yet a usable email client.**

Start with the [implementation checklist](docs/PLAN.md). It defines dependency-
ordered milestones, precise acceptance checks and the evidence required to mark
each step complete. The first feature milestone proves Servo/libcosmic integration.

| Document | Purpose |
| --- | --- |
| [Architecture](docs/ARCHITECTURE.md) | Product scope, crate ownership, mail identity, sync, threads and feature contracts |
| [Dependencies](docs/DEPENDENCIES.md) | Selected Rust libraries, versions, usage and compatibility gates |
| [Rendering](docs/RENDERING.md) | Servo integration, native dark mode, HTML privacy and input/accessibility proof |
| [Configuration](docs/CONFIGURATION.md) | Native config.nix, direct Home Manager integration, secrets and five-provider auth |
| [Classifier](docs/CLASSIFIER.md) | CPU-only model candidate, priority expiry, deadline extraction and local evaluation |
| [Delivery](docs/DELIVERY.md) | Nix development, minimal Nix-container CI, Nix/Flatpak packaging and release gates |
| [Research](docs/RESEARCH.md) | Primary sources, inspected pins and unresolved assumptions |

Planned essentials: unified inbox, grouped conversations, offline mail, draft/send,
Remind me notifications, optional background operation and a highlighted priority
section that clears on read or contextual expiry. Previews use extracted text;
deadline evidence appears below them. Classification stays on device, with no
automatic model download. Home Manager installs the client's native `.nix` file
directly; there is no separate TOML configuration or conversion module.

```sh
nix develop
bash ci/check.sh
nix flake check --no-update-lock-file --print-build-logs
nix build
nix run
```

`nix run` currently reports that this is a planning scaffold. The flake exports
x86_64-linux and aarch64-linux outputs; runtime verification and release support
for each architecture are separate checklist gates. Flatpak production packaging
is planned in P10 after Servo's build dependencies are established.

Root `Cargo.toml` uses `members = ["crates/*"]`, centralized dependencies and all
active Clippy groups, including opt-in restriction/nursery/pedantic/cargo. See
[AGENTS.md](AGENTS.md) before implementing a milestone. GPL-3.0-only; third-party
code/model licenses and notices remain separate.

Created in [T3 Code](https://t3.codes).
