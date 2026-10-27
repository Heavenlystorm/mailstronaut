# Research record and unresolved assumptions

Researched 2026-10-27 against primary upstream sources and the local checkout.
URLs on moving branches are research links; implementation pins must be full
revisions plus Cargo.lock/Nix source hashes. No live mailbox login, Servo build,
model inference or Flatpak build was performed in the planning phase.

## Local conventions inspected

`/home/eduard/Projects/cluster-flock-inator/main/Cargo.toml` uses `crates/*`, root
package/dependency tables and inherited lints; this project follows that structure.
It extends the lint policy to every active opt-in Clippy group as requested. The
Mailstronaut checkout initially contained README.md and assets/icon.svg at commit
`b7b9322`, with no existing implementation or AGENTS.md. Initial worktree was clean.
No CFI source or Hylki application code was copied.

## Primary references and decisions

| Source inspected | Evidence / implication |
| --- | --- |
| [Hylki repository](https://github.com/hyprlab/hylki) and [Cargo manifest](https://github.com/hyprlab/hylki/blob/main/Cargo.toml) | Product reference; manifest 1.45.3 uses GTK/libadwaita/WebKit and familiar Rust mail libraries. A COSMIC/Servo application is a separate design. |
| [libcosmic](https://github.com/pop-os/libcosmic), [exact manifest](https://github.com/pop-os/libcosmic/blob/8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec/Cargo.toml) | Inspected head `8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec`; Iced submodule `229186b5180b6217d93866c912e6373ac378b231`. GUI types must come from the same source. |
| [Servo repository](https://github.com/servo/servo), [release 0.7.0](https://github.com/servo/servo/releases/tag/v0.7.0), [embedding book](https://book.servo.org/embedding/overview.html) | Embedding is supported but its documentation/build setup still requires integration work. Main's rust-toolchain.toml was 1.97.1; prove the selected release's actual requirement rather than assume main == release. |
| [WebView API](https://doc.servo.org/servo/webview/struct.WebView.html), [theme announcement](https://servo.org/blog/2025/06/18/this-month-in-servo/) | Native `notify_theme_change` propagation exists. It is not evidence of automatic forced recoloring of sender-authored fixed colors. |
| [ServoDelegate](https://doc.servo.org/servo/trait.ServoDelegate.html), [resource loading](https://doc.servo.org/servo/struct.WebResourceLoad.html), [builder](https://doc.servo.org/servo/struct.ServoBuilder.html) | Resource/protocol/event APIs are available; unhandled resource requests continue normally. Both view-associated and other loads need an explicit policy. |
| [iced_webview_v2 source](https://github.com/franzos/iced_webview_v2/tree/3ed0ad922e130faf2f5be3a0ff602bfa292a0272) | Existing adapter merits reuse research. Its manifest and README differ on some Servo availability/version details; code/manifest win. Its Iced source and required security/theme hooks need a gate, not an assumed direct drop-in. |
| [async-imap 0.12](https://docs.rs/async-imap/0.12.0/async_imap/), [lettre 0.11.23](https://docs.rs/lettre/0.11.23/lettre/), [feature list](https://docs.rs/crate/lettre/0.11.23/features) | Existing protocol/TLS-stream/SMTP/MIME-building APIs. Mail identity/recovery/business rules remain application responsibilities. |
| [mail-parser 1.0](https://docs.rs/mail-parser/1.0.0/mail_parser/), [mail-threading 0.1.3](https://docs.rs/mail-threading/0.1.3/mail_threading/), [Ammonia 4.2.4](https://docs.rs/ammonia/4.2.4/ammonia/) | Reuse MIME parsing, header ancestry grouping and HTML sanitization. They do not independently enforce all application-level identity/privacy rules. |
| [Snix component overview](https://snix.dev/docs/components/overview/), [Serde bridge](https://snix.dev/rustdoc/snix_serde/), [actual source](https://git.snix.dev/snix/snix/src/commit/f393b22aa9b8e0e74c9e8d67f15454409299e510/snix/serde/src/de.rs), [project status](https://snix.dev/about/) | Native Nix-as-configuration is already a Rust library use case. Source constructs pure strict evaluation; APIs remain unstable. Pin behind one boundary and measure dependencies/limits. No need for a host Nix binary or config translation module. |
| [oo7 backend guidance](https://docs.rs/oo7/0.6.0/oo7/), [Keyring](https://docs.rs/oo7/0.6.0/oo7/enum.Keyring.html) | Existing async standard keyring with native/Flatpak backend selection; no custom password vault needed. |
| [Google native OAuth](https://developers.google.com/identity/protocols/oauth2/native-app), [XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol), [loopback desktop support](https://developers.google.com/identity/protocols/oauth2/resources/loopback-migration) | Registered desktop app, external browser/PKCE/loopback and proper full mail scope; production verification remains separate from developer success. |
| [Apple iCloud settings](https://support.apple.com/en-ie/102525) | IMAP/SMTP with an app-specific password and possible distinct incoming/outgoing username form. |
| [PoliTO student settings](https://it.polito.it/studenti/il-tuo-account/guida-alla-configurazione-della-posta-elettronica/parametri-per-la-configurazione-della-posta-elettronica), [PoliTO EOL/MFA](https://it.polito.it/ambiti/it-for-research/altri-strumenti-it-per-la-ricerca), [Microsoft OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/) | Student and EOL documentation coexist. Need account-specific manual settings and standard OAuth2; do not universalize a password preset. |
| [UPPA Partage FAQ](https://formation.univ-pau.fr/fr/vie-etudiante/outils-numeriques-adresse-mail-etudiante/messagerie-partage.html) | Official incoming/outgoing host/port/login examples; current actual account login still needs acceptance. No verified public Inria configuration was found. |
| [MiniLM NLI card](https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli), [pinned config](https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli/blob/0a71e92a985b6e1ad1828cf67ce9c459639c1dca/config.json), [Candle model API](https://docs.rs/candle-transformers/0.11.0/candle_transformers/models/xlm_roberta/struct.XLMRobertaForSequenceClassification.html) | A small multilingual NLI model matches an existing Rust classifier implementation. Metadata/architecture are verified; semantic quality, reference-logit parity and CPU cost are not. |
| [Interim](https://docs.rs/interim/0.2.1/interim/), [Chrono TZ](https://docs.rs/chrono-tz/latest/chrono_tz/) | Existing English relative-date parsing and timezone arithmetic, with bounded language adapters. No evidence of arbitrary multilingual deadline semantics from these libraries alone. |
| [Nix develop manual](https://nix.dev/manual/nix/2.34/command-ref/new-cli/nix3-develop.html), [minimal image manual](https://nix.dev/manual/nix/2.34/installation/installing-docker.html) | Standard flake shell and minimal official Nix container are sufficient foundations. |
| [Flatpak build introduction](https://docs.flatpak.org/en/latest/building-introduction.html), [permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html), [Rust SDK extension manifest](https://github.com/flathub/org.freedesktop.Sdk.Extension.rust-stable/blob/branch/26.08/org.freedesktop.Sdk.Extension.rust-stable.json), [Background portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html) | Use controlled SDK/vendor sources and explicit runtime portals. Inspected extension manifest on its default branch specifies 26.08 and Rust 1.98.0; installation/build was not attempted. Container namespace support and COSMIC portal implementation need runtime proof. |
| [Clippy guidance](https://doc.rust-lang.org/clippy/usage.html) | `all` excludes opt-in groups; restriction includes contradictory rules. Enable requested groups and keep narrow reasoned exceptions. |

Crate feature tables were additionally inspected from published crate archives:
oo7 0.6 (`tokio`, `native_crypto`), tokenizers 0.23.1 (`fancy-regex` optional Rust
regex backend; avoid default onig/esaxx_fast for this inference-only use),
rusqlite_migration 2.6 (depends on rusqlite 0.40), ASHPD 0.12.3 (Tokio/Wayland
features). This is manifest inspection, not a successful combined graph build.

## Benchmark feed evidence

The requested [live JevBench board](https://benchmarkheaven.com/jev-models) returned
more than the web fetcher's content limit. Collaborative-browser reading was tried
with two calls; raw errors were `Preview automation snapshot timed out after
15000ms.` and `Preview automation evaluate timed out after 15000ms.` No successful
visual inspection is claimed. Versioned method pages were readable; the public
[JSON API](https://benchmarkheaven.com/api/jevbench/latest) was fetched successfully
with curl and verified TLS, without interacting with an account.

The API reported revision v1.7.11, 300 listed systems. Its top-level source fields
still referenced some original v1.6.1 artifacts, and individual model measurements
retain their own release/hardware labels. Preserve that provenance rather than
labeling every number a new v1.7.11 experiment. Selected factual rows, raw-response
SHA-256 and reported measurement conditions are saved in
[research/jevbench-selection.json](research/jevbench-selection.json). No rank is
used as the email classifier acceptance criterion. The live feed can change.

The old NixOS wiki URL supplied in the request returned HTTP 403 to the web reader;
the maintained Nix manual was used for development-shell behavior instead. Several
moving source/doc URLs were unavailable or stale; exact primary source manifests
were used when available. No failed page read was treated as supporting evidence.

## Open questions with assigned resolution gates

| Question | Owner / next proof |
| --- | --- |
| Can current Servo and libcosmic compose with little adapter maintenance? | P01: exact source-compatible compile, active theme/input/resource/lifecycle demo |
| Is offscreen readback light enough, with accessible text available? | P01/P06: CPU/PSS, scrolling/HiDPI/selection, screen-reader test |
| Does Servo expose sufficient default-deny coverage without broad engine patches? | P01: trapped HTTP/CSS/font/SVG/non-HTTP fixtures, no filesystem/network leaks |
| Is pure Snix config acceptable in startup time/closure size and available offline in Flatpak? | P02: helper benchmarks, pure-evaluation/error/limits tests and sandbox run |
| Which current Inria/PoliTO route does the user's account actually use? | P05: user/IT settings, authorized sign-in; no current login assumed |
| Can the intended desktop/tenant OAuth registrations authorize the required scopes? | P05/P11: development vs production verification/consent evidence |
| Does the small NLI model improve mail priority/deadlines in EN/FR/IT on CPU? | P09: reference logits, held-out local fixture benchmark, CPU memory/latency |
| Can COSMIC supply all required Secret/Background/notification actions in each package? | P02/P08/P10: installed runtime permission/activation tests |
| Can Flatpak build/run in minimal Nix container CI with acceptable capabilities? | P01 spike/P10: SDK source build and real runner namespace proof |

These are bounded feasibility/acceptance tasks, not a license to replace a requested
component silently. Record a blocker and continue independent work where possible.
