# Dependency decisions

Research snapshot: 2026-10-27. Cargo.lock now includes the initial libcosmic,
Snix/Serde configuration and SQLite storage components. Their milestone evidence
records what has been built and tested. The table remains an implementation
prescription for the full application, **not** proof that all selected components
compose or that Servo is integrated. Add each dependency to the
root `[workspace.dependencies]` in its milestone; crates use `workspace = true`.
Commit Cargo.lock and exact Nix fetch hashes. Do not fetch every planned dependency
merely to populate a manifest. Disable unused default features where indicated.

## Selected building blocks

| Component / initial version | Intended use and feature choices | Primary evidence |
| --- | --- | --- |
| `libcosmic` git `8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec` | Package `libcosmic`, Rust library `cosmic`; native Application/update/view/subscription, navigation, dialogs and theme. Start with upstream default Wayland/X11/a11y/Tokio configuration; add `wgpu` for the Servo texture route and `xdg-portal` as needed. Use `cosmic::iced` reexports, never independently select upstream Iced. | [Manifest](https://github.com/pop-os/libcosmic/blob/8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec/Cargo.toml), [examples](https://github.com/pop-os/libcosmic/tree/8990a058e1deb8f9f7f4c0a42f6b1362fa1437ec/examples) |
| `servo` `=0.7.0`, defaults off / `baked-in-resources` | Implemented synthetic-only software viewport using ServoBuilder, WebViewBuilder, rendering context, event-loop waker and delegates. It does not accept untrusted mail yet. Exact graph/checks and four scoped FFI backend exceptions are recorded in P01 evidence. | [Release](https://github.com/servo/servo/releases/tag/v0.7.0), [embedding book](https://book.servo.org/embedding/overview.html), [API](https://doc.servo.org/servo/) |
| `iced_webview_v2` `0.1.13`, candidate only | Library name `iced_webview`, `default-features = false`, `servo` feature only. Existing offscreen Servo→Iced texture integration to evaluate before writing adapter glue. Main targets Iced 0.14/Servo 0.1; next tracks other versions. Neither is established compatible with libcosmic. Do not enable Blitz/CEF/litehtml. | [Manifest](https://github.com/franzos/iced_webview_v2/blob/3ed0ad922e130faf2f5be3a0ff602bfa292a0272/Cargo.toml), [Servo implementation](https://github.com/franzos/iced_webview_v2/blob/3ed0ad922e130faf2f5be3a0ff602bfa292a0272/src/engines/servo.rs) |
| `tokio` `1`, `tokio-util` `0.7`, `futures-util` `0.3` | One engine runtime; bounded channels, timers, cancellation tokens, async IMAP streams. Enable only rt-multi-thread/macros/net/io-util/sync/time/fs/process/signal as used. Test-util in dev only. | [Tokio](https://docs.rs/tokio/latest/tokio/), [cancellation](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) |
| `async-imap` `0.12.0` | Defaults off, `runtime-tokio`; UID fetch/store/search/move/copy, CAPABILITY, IDLE. It accepts an already-secured stream; no TLS implementation inside this library. Check extension methods before promising QRESYNC/CONDSTORE. | [API and STARTTLS](https://docs.rs/async-imap/0.12.0/async_imap/), [IDLE](https://docs.rs/async-imap/0.12.0/async_imap/extensions/idle/) |
| `tokio-rustls` `0.26`, `rustls` `0.23`, `rustls-native-certs` `0.8` | TLS for IMAP and custom institutional CAs; choose one crypto provider consistently after checking the unified graph. Authenticate only after certificate/hostname validation and required STARTTLS. Host roots/native and runtime roots/Flatpak; explicit extra CA files supported, no accept-invalid-certificate option. | [Rustls](https://docs.rs/rustls/latest/rustls/), [Tokio adapter](https://docs.rs/tokio-rustls/latest/tokio_rustls/), [roots](https://docs.rs/rustls-native-certs/latest/rustls_native_certs/) |
| `lettre` `0.11.23` | Defaults off; `builder`, `smtp-transport`, `tokio1-rustls`, `ring`, `rustls-native-certs`. This selects Tokio/Rustls and host roots without the convenience `rustls-tls` feature's WebPKI roots; test the final unified provider/root graph. Builds MIME and submits using `AsyncSmtpTransport<Tokio1Executor>`, Credentials and XOAUTH2 mechanism. Use mandatory STARTTLS/implicit TLS builders. | [Features](https://docs.rs/crate/lettre/0.11.23/features), [SMTP](https://docs.rs/lettre/0.11.23/lettre/transport/smtp/) |
| `mail-parser` `1.0.0` | RFC headers/MIME/charsets, text and HTML alternatives, attachments, deterministic preview helpers. Enable `full_encoding` for real mail. The focused P03 projection uses bounded parsing and checked text decoding; inline OpenPGP detection is a bounded marker check, not cryptographic parsing. Test the 1.0 API rather than copying old 0.9 Hylki examples. | [API](https://docs.rs/mail-parser/1.0.0/mail_parser/) |
| `mail-threading` `0.1.3` | Existing RFC5256/JWZ grouping with `thread_messages_with`; disable subject merging. Use native provider thread IDs where established. Reuse its conformance corpus; application tests cover account isolation/stable IDs. | [API and conformance scope](https://docs.rs/mail-threading/0.1.3/mail_threading/) |
| `oauth2` `5`, `reqwest` `0.12` | Standard authorization-code/PKCE/state/refresh implementation. oauth2's reqwest async client, redirect policy `none`, TLS certificate validation. Pin a mutually supported reqwest version; do not add an independent second HTTP stack. No client implementation from hand-built token JSON. | [OAuth2](https://docs.rs/oauth2/5.0.0/oauth2/), [Google native apps](https://developers.google.com/identity/protocols/oauth2/native-app) |
| `oo7` `0.6.0`, `secrecy` `0.10` | Async `oo7::Keyring::new`, native Secret Service and Flatpak portal-backed encrypted storage selected by library. Defaults off, `tokio`, `native_crypto` (verified feature names). Credentials wrapped/redacted; no plaintext fallback or shell password commands. | [Keyring](https://docs.rs/oo7/0.6.0/oo7/enum.Keyring.html), [backend guidance](https://docs.rs/oo7/0.6.0/oo7/) |
| `rusqlite` `=0.38.0`, `rusqlite_migration` `=2.4.1` | Match Servo 0.7's native SQLite link dependency; six store behavior tests and eleven MIME tests pass with these pins. SQLite/FTS5 and numbered migrations; currently `bundled`. A dedicated blocking store worker remains planned. Do not restore 0.40/2.6 while Servo requires the incompatible native link version. | [SQLite binding](https://docs.rs/rusqlite/0.38.0/rusqlite/), [migrations](https://docs.rs/rusqlite_migration/2.4.1/rusqlite_migration/) |
| `ammonia` `4.2.4` | Maintained HTML allowlist sanitizer. Defaults alone are not a full email CSS/resource policy. Keep narrowly allowed email layout/style only with engine-level network enforcement and CSS fixtures. | [Sanitizer](https://docs.rs/ammonia/4.2.4/ammonia/) |
| `uuid` `=1.29.0`, `v4`; `content-security-policy` `=0.9.0` | Synthetic renderer generation namespaces use the existing UUID generator; request destination matching uses Servo's exact CSP enum type. Both versions were already in the locked Servo graph. No custom token generator, CSS parser or browser-origin implementation is introduced. | Locked Cargo graph and pinned Servo request types |
| `snix-serde` + `snix-eval` git `f393b22aa9b8e0e74c9e8d67f15454409299e510` | Existing pure Nix evaluator and Serde bridge, repo `https://git.snix.dev/snix/snix.git`, packages under `snix/`. `from_str::<Config>` uses a pure strict evaluator; no external Nix executable, store/build/glue daemon. API unstable; isolate and prove P02. Audit default evaluator features/closure size. | [Serde](https://snix.dev/rustdoc/snix_serde/), [exact bridge source](https://git.snix.dev/snix/snix/src/commit/f393b22aa9b8e0e74c9e8d67f15454409299e510/snix/serde/src/de.rs), [scope and instability](https://snix.dev/about/) |
| `serde` `1`, `serde_json` `1`, `thiserror` `2` | Typed configuration/domain DTOs and errors; JSON only for model config, fixture data and upstream tools, never the user's hidden config. Reject unknown config fields. | [Serde](https://serde.rs/), [Thiserror](https://docs.rs/thiserror/latest/thiserror/) |
| `candle-core`, `candle-nn`, `candle-transformers` `0.11.0`; `tokenizers` `0.23.1` | Defaults off for accelerator features; CPU safetensors inference, `XLMRobertaForSequenceClassification::new(3, ...)` and `forward(input_ids, mask, token_types)`. Tokenizers defaults off with `fancy-regex`, avoiding onig/esaxx_fast/http features for inference. No CUDA/ONNX/Python/model server in baseline. | [Existing classifier model](https://docs.rs/candle-transformers/0.11.0/candle_transformers/models/xlm_roberta/struct.XLMRobertaForSequenceClassification.html), [Candle](https://github.com/huggingface/candle), [tokenizers](https://docs.rs/tokenizers/latest/tokenizers/) |
| `chrono` `0.4`, `chrono-tz` `0.10`, `interim` `0.2.1`, `regex` `1` | Absolute timestamps/civil timezone transitions, date candidate matching, existing English relative-date parser via `chrono_0_4`. Interim's documented Jiff support is for 0.1, so do not assume it accepts libcosmic's Jiff 0.2. Small reviewed FR/IT date-token maps normalize to this parser; no new general NLP parser. | [Interim](https://docs.rs/interim/0.2.1/interim/), [Chrono TZ](https://docs.rs/chrono-tz/latest/chrono_tz/) |
| `ashpd` `0.12`, `zbus` `5` | Portals for links, file save/open, notifications and Background; Tokio feature alignment with libcosmic. Use native freedesktop notifications through zbus if the desktop lacks the portal route and verify activation actions; no simultaneous duplicate transports. | [ASHPD](https://docs.rs/ashpd/latest/ashpd/), [Background protocol](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html) |
| `directories` `6`, `uuid` `1`, `sha2` `0.10`, `url` `2`, `tracing` `0.1`, `tracing-subscriber` `0.3` | XDG locations, stable local IDs, hashes, URL validation, redacted diagnostics. Prefer transitive-compatible versions, narrow features. Reuse libcosmic Fluent/i18n support rather than adding another UI localization framework. | [Directories](https://docs.rs/directories/latest/directories/), [Tracing](https://docs.rs/tracing/latest/tracing/) |
| `rustix` `1`, feature `process` | Safe resource-limit calls for the bounded same-binary Nix evaluation helper; no unsafe pre-exec hook. Put limits inside the freshly spawned helper before evaluation. | [Process API](https://docs.rs/rustix/latest/rustix/process/) |
| Dev only: `tempfile` `3`, `proptest` `1` | Disposable stores and meaningful property tests for UID/identity, deadlines/timezones, hostile MIME/resource policy. Add a fuzz workspace only if justified by the parsing boundary. | [Proptest](https://docs.rs/proptest/latest/proptest/) |

Semver bands in this table are deliberate where a combined lock is not yet proven.
P01/P02 record exact working Cargo revisions/features and native hashes. A failed
gate is not permission to keep upgrading unrelated libraries. Rust 1.98 is the
declared lower bound required by mail-parser 1.0.0. The full x86_64 SDK release
graph now compiles with Rust 1.98.0; its tests/Clippy and other architectures have
not been checked at that minimum. The development compiler is
pinned through rust-overlay in flake.lock; Nix release builds use Nixpkgs' stable
compiler. Revisit the lower bound when the complete graph is known.

## Alternatives considered

- **Hylki**: useful product reference and evidence of the expected IMAP/SMTP/MIME
  building blocks. Its GTK/libadwaita/WebKit UI and application code are not the
  requested architecture. Do not copy its code, branding, private workarounds or
  AGPL source into this project. Current inspected manifest was 1.45.3.
- **Pimalaya/Himalaya email libraries**: valuable higher-level alternative, but v2
  has a substantial sans-I/O refactor and external auth/session components. This
  design chooses async-imap+lettre with a narrow engine instead of integrating a
  CLI, broker and two different async/config abstractions. Reconsider only with a
  concrete reduction in maintained code. [Upstream migration](https://docs.rs/crate/himalaya/latest/source/MIGRATION.md).
- **ONNX Runtime/ort, rust-bert/libtorch**: mature inference options, but bring a
  separate native ML runtime and packaging surface. Candle already exposes this
  exact architecture in Rust. No reason to ship an LLM runner for an NLI task.
- **GLiNER and Jev-style decoder models**: interesting evaluation candidates, not
  automatically better priority/deadline models. Do not use a hosted endpoint or
  import Python custom model code. See CLASSIFIER.md for the research comparison.
- **Calling `nix eval`, parsing with rnix alone, nix-rs-serde preview**: first needs
  host Nix/does not solve Flatpak; second parses syntax without Nix evaluation;
  third is another explicitly incomplete preview. Prefer the existing Snix bridge
  behind one boundary, subject to its size/compatibility gate.
- **Tantivy, SQLx, SeaORM, external mail indexers, separate daemons**: SQLite FTS5,
  a blocking connection owner and a single process meet the current requirements.

## License and supply-chain rules

Project-authored code starts GPL-3.0-only, compatible in intent with the selected
GPLv3 Snix evaluator; audit every exact dependency/license before distribution.
libcosmic and Servo are MPL-2.0 upstream; preserve their notices and any applicable
source obligations. The initial model card declares MIT. Model data and test email
corpora have separate licenses; do not assume code's license covers them. GPL text
is in the repository LICENSE. No copied Hylki code is included.

Pin git dependencies by full revision, never a floating branch in the released
manifest. Inspect cargo duplicates, `links` conflicts and feature unification when
pinning Iced/Servo/Stylo/rustls. Use `cosmic::iced` types throughout; patching only
the `iced` root while leaving incompatible `iced_core`/`iced_widget` sources is not
a fix. Record patches in a single documented patch section. Upstream small fixes
where possible; do not maintain a broad fork of libcosmic or Servo.
