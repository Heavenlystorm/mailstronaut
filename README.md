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
refresh or focus. Settings now exposes declared accounts, encrypted password
storage, browser sign-in actions and explicit bounded inbox sync. Offline reading
and last-valid configuration reload have synthetic native Wayland acceptance;
live provider login, compose and notification workflows remain unfinished.

Declare accounts in the native Nix source, open Settings with Ctrl+, and choose
an account to save its password or start sign-in. Sync newest mail fetches one
bounded page; Load older continues it. Ctrl+1 returns to the unified inbox and
Ctrl+Shift+R reloads the declaration. Saving credentials never rewrites the Nix
source. Browser-provider and real desktop portal acceptance are still pending.

The sidebar opens each account's cached mail and individual known folders without
starting a sync. Newer/Older controls page through 100 conversations at a time.
Expand a thread's message count to choose a cached original, including Sent
ancestry; arrows select passively and Enter or a click opens only that original.
Details shows parsed recipients and dates in the declared timezone (UTC when
unset). Folder changes, thread expansion and paging never mark mail Seen.

The native text reader uses GPUI Kit's read-only multiline component. Use
Ctrl+Shift+T to focus message text, Ctrl+A/C to select/copy, and the usual
Home/End/Page keys to navigate. Ctrl+K focuses search; Ctrl+Shift+D toggles
appearance. Focusing, copying and changing appearance do not open a message or
mark additional mail Seen. Cached HTML opens in a bounded, passive Blitz viewport
with native scrollbars, resize, HiDPI and theme handling. Text/HTML tabs and Select
text keep the native text alternative available for selection and copying. Remote
resources stay blocked; only validated message-local PNG/JPEG images are supplied.
Click a web link to open it through the desktop browser portal, or use Links in
the reader toolbar (Ctrl+Shift+L, Enter, arrow keys, Enter). The picker displays
canonical web addresses; file, script and other non-HTTP(S) targets are excluded.
Invalid local images are omitted, and unsupported HTML falls
back to cached text. Full layout, accessibility, performance and Flatpak acceptance
remain open P01 requirements.

Development uses pinned nightly Rust, Cranelift and Wild; release uses stable
LLVM, thin LTO and one codegen unit. See [delivery](docs/DELIVERY.md) for native
inputs, checks and distribution limitations. All application code is Rust.

Configuration is a directly evaluated `.nix` file, without a TOML conversion or
requirement for a Home Manager adapter. Direct-file composition works today; an
optional native `mutableSettings` module and export-to-Nix workflow are planned.
See [configuration](docs/CONFIGURATION.md),
[example](examples/config.nix) and [Home Manager composition](examples/home-manager.nix).
Secrets belong in encrypted storage and must never be put in the Nix store.

Architecture and decisions: [architecture](docs/ARCHITECTURE.md),
[dependencies](docs/DEPENDENCIES.md), [rendering](docs/RENDERING.md),
[interface/component reuse](docs/INTERFACE.md), [search](docs/SEARCH.md),
[classifier](docs/CLASSIFIER.md).

Nix and Flatpak are release targets. A new-stack Nix package and actual Flatpak
runtime/portal/Wayland acceptance must pass before distribution is claimed.
Historical evidence is indexed in [docs/evidence](docs/evidence/README.md).

Project-authored code is GPL-3.0-only; dependencies and bundled assets retain
their respective licenses. See [LICENSE](LICENSE).
