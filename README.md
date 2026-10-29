<img src="assets/icon.svg" width="64" height="64" alt="">

# Mailstronaut

A planned local-first email client for the COSMIC desktop, written in Rust with
libcosmic and Servo. This repository contains the architecture, a native COSMIC
preview window, native Nix account setup, secured IMAP fetching and a native
offline unified inbox, and a native plain-text draft/outbox prototype.
**It is not yet a usable email client.** The preview displays a fixed synthetic
Servo document with native theme forwarding. Untrusted mail rendering, input,
accessibility and background lifecycle remain feasibility gates.

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
nix run -- --version # display-independent package smoke check
```

`nix develop` supplies a pinned nightly compiler with Cranelift and Wild for
debug/test builds. Release builds use LLVM with thin LTO and one codegen unit;
the Nix release package keeps the pinned stable compiler.
Four SpiderMonkey FFI crates use LLVM in debug/test because the pinned Cranelift
cannot handle their symbol-name marker. Wild still links those builds; see the
narrow overrides and removal condition in [.cargo/config.toml](.cargo/config.toml).

`nix run` opens native account setup; Preview and Text retain the synthetic
rendering probes. `nix run -- --config /path/to/config.nix` selects an explicit
native source. Password accounts can save credentials through an explicitly
unlocked encrypted keyring. Select Inbox and Sync inboxes to fetch a bounded page
from each configured account; cached messages open in the native text reader.
Fetch older page continues the partial cache. Opening currently leaves read flags
unchanged; Mark read/unread and Star/Unstar save local intentions. Apply flags
submits a bounded batch after keyring unlock; Undo unsent flags works before any
network attempt. Changed server identities require a new sync. Compose supports local autosaved drafts, account/alias identities,
To/Cc/Bcc and explicit password/XOAUTH2 SMTP with required TLS.
OAuth accounts can Connect in browser after keyring unlock; refresh tokens stay in
the encrypted keyring and access tokens stay in memory. Google needs your own
registered public desktop client ID; live sign-in acceptance remains open. Closing the
window saves the final edit. Delivery uncertainty remains visible and requires an
explicit possible-duplicate retry; accepted mail cannot be resent to repair a Sent
copy. The cached reader can prepare reply/reply-all drafts and explicit text-only
forwards; routing uses Reply-To, the receiving account/alias and bounded ancestry.
Sent APPEND, attachments and automatic/background sync remain unimplemented. The Servo preview still accepts only its fixed synthetic document.
Current implementation evidence
and unverified acceptance checks are recorded in [P01](docs/evidence/P01.md),
[P02](docs/evidence/P02.md), [P03](docs/evidence/P03.md), and
[P04 receiving/inbox](docs/evidence/P04.md),
[P05 OAuth](docs/evidence/P05.md), and
[P07 compose/outbox](docs/evidence/P07.md). The flake exports
x86_64-linux and aarch64-linux outputs; runtime verification and release support
for each architecture are separate checklist gates. Flatpak production packaging
is planned in P10 after Servo's build dependencies are established.

Root `Cargo.toml` uses `members = ["crates/*"]`, centralized dependencies and all
Clippy policy: all, pedantic and nursery, plus selected unfinished-code and
unguarded-unwrap checks. See
[AGENTS.md](AGENTS.md) before implementing a milestone. GPL-3.0-only; third-party
code/model licenses and notices remain separate.

Created in [T3 Code](https://t3.codes).
