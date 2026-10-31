# Dependency decisions

Research snapshot: 2026-10-30. The current UI is **GPUI Kit + Blitz**. The
previous toolkit/browser graph is retired. Versions below describe the selected
integration, not completed runtime, privacy or distribution acceptance. Add actual
dependencies centrally and commit the lockfile and Nix vendor hash together.

## Selected building blocks

| Component / initial version | Intended use and feature choices | Primary evidence |
| --- | --- | --- |
| Nix build: Crane `0.24.0`, git `692f7e9ef2ece8125b466f66f2af532b3edaed0d` | Standard dependency artifact cache for stable LLVM release builds; reuse `buildDepsOnly` output in `buildPackage`, with identical immutable vendor paths and strict Clippy/tests. No application runtime dependency. | [Pinned builder](https://github.com/ipetkov/crane/blob/692f7e9ef2ece8125b466f66f2af532b3edaed0d/lib/buildDepsOnly.nix), [artifact hook](https://github.com/ipetkov/crane/blob/692f7e9ef2ece8125b466f66f2af532b3edaed0d/lib/setupHooks/installCargoArtifactsHook.sh) |
| `gpui-kit` `=0.8.1` | Application facade, standard components and embedded assets; matching `gpui-pre-*` `0.3.10`. Import through `gpui_kit` reexports rather than independently selecting GPUI. Default component/assets only; no JavaScript shell. | [Kit documentation](https://gpui-kit.com/docs/getting-started/), [published API](https://docs.rs/gpui-kit/0.8.1/gpui_kit/) |
| `blitz-dom`, `blitz-html`, `blitz-paint`, `blitz-traits` `=0.3.0-beta.2` | Parse HTML, resolve CSS/layout and paint. Disable defaults throughout; enable DOM system-fonts, floats and complex-scripts. No network provider, SVG renderer, file-input or custom-widget capability. See RENDERING.md for the passive resource boundary and remaining proof. | [Blitz](https://github.com/DioxusLabs/blitz), [DOM configuration](https://docs.rs/blitz-dom/0.3.0-beta.2/blitz_dom/struct.DocumentConfig.html) |
| `anyrender` `=0.13.0`, `anyrender_vello_cpu` `=0.17.0` | Existing CPU raster backend with multithreading; `HtmlDocument::resolve`, `blitz_paint::paint_scene`, `ImageRenderer::render_to_vec`. Owned bounded pixels cross to GPUI `RenderImage`; convert RGBA to GPUI's BGRA. No custom GPU interop or foreign window. | [AnyRender](https://docs.rs/anyrender/0.13.0/anyrender/), [CPU backend](https://docs.rs/anyrender_vello_cpu/0.17.0/anyrender_vello_cpu/) |
| `image` `0.25`, defaults off | Standard RGBA frame container; PNG/JPEG for bounded raster attachments once local-resource support is accepted. No general SVG email capability. | [Image](https://docs.rs/image/0.25.10/image/) |
| `tokio` `1`, `tokio-util` `0.7`, `futures-util` `0.3` | One engine runtime; bounded channels, timers, cancellation tokens, async IMAP streams. Enable only rt-multi-thread/macros/net/io-util/sync/time/fs/process/signal as used. Test-util in dev only. | [Tokio](https://docs.rs/tokio/latest/tokio/), [cancellation](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) |
| `async-imap` `0.12.0` | Defaults off, `runtime-tokio`; UID fetch/store/search/move/copy, CAPABILITY, IDLE. It accepts an already-secured stream; no TLS implementation inside this library. Check extension methods before promising QRESYNC/CONDSTORE. | [API and STARTTLS](https://docs.rs/async-imap/0.12.0/async_imap/), [IDLE](https://docs.rs/async-imap/0.12.0/async_imap/extensions/idle/) |
| `tokio-rustls` `0.26`, `rustls` `0.23`, `rustls-native-certs` `0.8` | TLS for IMAP and custom institutional CAs; choose one crypto provider consistently after checking the unified graph. Authenticate only after certificate/hostname validation and required STARTTLS. Host roots/native and runtime roots/Flatpak; explicit extra CA files supported, no accept-invalid-certificate option. | [Rustls](https://docs.rs/rustls/latest/rustls/), [Tokio adapter](https://docs.rs/tokio-rustls/latest/tokio_rustls/), [roots](https://docs.rs/rustls-native-certs/latest/rustls_native_certs/) |
| `lettre` `0.11.23` | Defaults off; `builder`, `smtp-transport`, `tokio1-rustls`, `ring`, `rustls-native-certs`. This selects Tokio/Rustls and host roots without the convenience `rustls-tls` feature's WebPKI roots; test the final unified provider/root graph. Builds MIME and submits using `AsyncSmtpTransport<Tokio1Executor>`, Credentials and XOAUTH2 mechanism. Use mandatory STARTTLS/implicit TLS builders. | [Features](https://docs.rs/crate/lettre/0.11.23/features), [SMTP](https://docs.rs/lettre/0.11.23/lettre/transport/smtp/) |
| `mail-parser` `1.0.0` | RFC headers/MIME/charsets, text and HTML alternatives, attachments, deterministic preview helpers. Enable `full_encoding` for real mail. The focused P03 projection uses bounded parsing and checked text decoding; inline OpenPGP detection is a bounded marker check, not cryptographic parsing. Test the 1.0 API rather than copying old 0.9 Hylki examples. | [API](https://docs.rs/mail-parser/1.0.0/mail_parser/) |
| `mail-threading` `0.1.3` | Implemented account-local RFC5256/JWZ grouping with `thread_messages_with`, subject merging and phantom pruning disabled. Recursive ancestry runs in a bounded same-executable metadata child. The 37-case upstream corpus and application identity/revision tests pass; native provider thread IDs remain future work. | [API and conformance scope](https://docs.rs/mail-threading/0.1.3/mail_threading/) |
| `oauth2` `=5.0.0`, `reqwest` `=0.12.28` | Implemented public-client S256/CSRF/code exchange and refresh. OAuth2 defaults off, timing-resistant secret comparisons; Reqwest defaults off, Rustls/native roots. A bounded adapter implements the standard async HTTP interface with redirects disabled and at most 64 KiB retained response body. No hand-built token response parsing. | [OAuth2](https://docs.rs/oauth2/5.0.0/oauth2/), [Google native apps](https://developers.google.com/identity/protocols/oauth2/native-app) |
| `oo7` `=0.6.0`, `secrecy` `=0.10.3` | Implemented scoped credential adapter: async `oo7::Keyring::new`, native Secret Service and Flatpak portal-backed encrypted storage selected by library. Defaults off, `async-std`, `native_crypto`, matching GPUI's oo7 runtime selection. Await credential futures from the existing Tokio mail runtime; revalidate the private Secret Service fixtures with this unified graph. Construction/unlock are foreground-only; reads/writes report locked state. Account/reference/purpose identity, zeroizing redacted values, bounded operations, no plaintext fallback or shell password commands. Native fixture acceptance is in P02 evidence; Flatpak portal remains unverified. | [Keyring](https://docs.rs/oo7/0.6.0/oo7/enum.Keyring.html), [backend guidance](https://docs.rs/oo7/0.6.0/oo7/) |
| `rusqlite` `=0.38.0`, `rusqlite_migration` `=2.4.1` | Retain the tested canonical store/migration versions, bundled SQLite/FTS5 and one bounded blocking store owner. The old browser's SQLite link constraint is gone; upgrade only for a concrete benefit and migration acceptance. | [SQLite binding](https://docs.rs/rusqlite/0.38.0/rusqlite/), [migrations](https://docs.rs/rusqlite_migration/2.4.1/rusqlite_migration/) |
| `ammonia` `4.2.4` | Maintained HTML allowlist sanitizer. Defaults alone are not a full email CSS/resource policy. Keep narrowly allowed email layout/style only with engine-level network enforcement and CSS fixtures. | [Sanitizer](https://docs.rs/ammonia/4.2.4/ammonia/) |
| `snix-serde` + `snix-eval` git `f393b22aa9b8e0e74c9e8d67f15454409299e510` | Existing pure Nix evaluator and Serde bridge, repo `https://git.snix.dev/snix/snix.git`, packages under `snix/`. `from_str::<Config>` uses a pure strict evaluator; no external Nix executable, store/build/glue daemon. API unstable; isolate and prove P02. Audit default evaluator features/closure size. | [Serde](https://snix.dev/rustdoc/snix_serde/), [exact bridge source](https://git.snix.dev/snix/snix/src/commit/f393b22aa9b8e0e74c9e8d67f15454409299e510/snix/serde/src/de.rs), [scope and instability](https://snix.dev/about/) |
| `serde` `1`, `serde_json` `1`, `thiserror` `2` | Typed configuration/domain DTOs and errors; JSON only for model config, fixture data and upstream tools, never the user's hidden config. Reject unknown config fields. | [Serde](https://serde.rs/), [Thiserror](https://docs.rs/thiserror/latest/thiserror/) |
| `candle-core`, `candle-nn`, `candle-transformers` `0.11.0`; `tokenizers` `0.22.2` | Defaults off for accelerators; CPU safetensors inference through existing model classes. The actual Candle 0.11 source unconditionally requires tokenizers 0.22 with `onig` on native targets. Reuse that version/backend rather than the originally proposed 0.23.1 plus a second tokenizer/regex stack; defaults-off does not remove Candle's Oniguruma dependency. No CUDA/ONNX/Python/model server in the application baseline. BERT embeddings and XLM-R classification remain distinct heads/models. | [Existing classifier model](https://docs.rs/candle-transformers/0.11.0/candle_transformers/models/xlm_roberta/struct.XLMRobertaForSequenceClassification.html), [Candle core manifest](https://github.com/huggingface/candle/blob/0.11.0/candle-core/Cargo.toml), [tokenizers](https://docs.rs/tokenizers/0.22.2/tokenizers/) |
| `chrono` `0.4`, `chrono-tz` `0.10`, `interim` `0.2.1`, `regex` `1` | Retain Chrono TZ reminder persistence/civil-time behavior. Prefer GPUI Kit date components when restoring the native picker. Deadline matching/parsing remains a separate P09 gate; bounded FR/IT token adapters may normalize to the existing English parser, with explicit evidence and ambiguity handling. | [Interim](https://docs.rs/interim/0.2.1/interim/), [Chrono TZ](https://docs.rs/chrono-tz/latest/chrono_tz/) |
| `hyper` `=1.12.0`, `hyper-util` `=0.1.21`, `http-body-util` `=0.1.5` | Existing Hyper graph reused for a bounded HTTP/1 loopback callback; no custom HTTP parser. Tokio adapter, short-lived listener, fixed static responses. | [HTTP/1 server](https://docs.rs/hyper/1.12.0/hyper/server/conn/http1/struct.Builder.html) |
| `ashpd` `0.13.13`, `zbus` `5.19.0` | Use the existing GPUI async-I/O desktop graph for future portal adapters; do not enable both ASHPD async-io and Tokio. The existing zbus Tokio flag currently serves test fixtures. Mail transport stays Tokio. Portals cover links/files/notifications/background; validate actual runtime permissions and activation separately. | [ASHPD](https://docs.rs/ashpd/0.13.13/ashpd/), [Background protocol](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html) |
| `directories` `6`, `uuid` `1`, `sha2` `0.10`, `url` `2`, `tracing` `0.1`, `tracing-subscriber` `0.3` | XDG locations, stable local IDs, hashes, URL validation, redacted diagnostics. Prefer transitive-compatible versions, narrow features. Select localization when implemented; keep it separate from toolkit types. | [Directories](https://docs.rs/directories/latest/directories/), [Tracing](https://docs.rs/tracing/latest/tracing/) |
| `rustix` `1`, feature `process` | Safe resource-limit calls for the bounded same-binary Nix evaluation and conversation grouping helpers; no unsafe pre-exec hook. Put limits inside the freshly spawned helper before evaluation. | [Process API](https://docs.rs/rustix/latest/rustix/process/) |
| Dev only: `tempfile` `3`, `proptest` `1` | Disposable stores and meaningful property tests for UID/identity, deadlines/timezones, hostile MIME/resource policy. Add a fuzz workspace only if justified by the parsing boundary. | [Proptest](https://docs.rs/proptest/latest/proptest/) |

Semver bands in this table are deliberate where a combined lock is not yet proven.
P01/P02 record exact working Cargo revisions/features and native hashes. A failed
gate is not permission to keep upgrading unrelated libraries. Rust 1.98 is the
declared lower bound required by mail-parser 1.0.0. The full x86_64 SDK release
graph was checked with Rust 1.98.0 before the stack change. The new GPUI/Blitz
graph has not been accepted at that minimum or in the Flatpak SDK. The development compiler is
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
- **Search engines**: the original literal-search FTS5-only decision is superseded
  by the user's fuzzy/multilingual semantic requirements. [SEARCH.md](SEARCH.md)
  compares existing engines and model APIs; LanceDB is the first unified probe,
  with Tantivy plus a vector library as the lean comparison. No candidate has been adopted. Paired isolated probes are recorded in SEARCH.md; none passes every quality/resource requirement yet. SQLite remains canonical; do not implement a search
  engine or add a separate search daemon.
- **SQLx, SeaORM and external mail indexers**: the blocking SQLite owner and single
  application process remain sufficient for canonical mail/state persistence.

## License and supply-chain rules

Project-authored code starts GPL-3.0-only, compatible in intent with the selected
GPLv3 Snix evaluator; audit every exact dependency/license before distribution.
GPUI Kit and the exact GPUI/Blitz dependency graph, embedded icons/fonts and
model data all require license/notices review before distribution. An upstream
code license does not cover arbitrary test corpora or model weights. Preserve
applicable notices and source obligations; the project LICENSE covers our code.

Pin Git dependencies by full revision. Inspect duplicate GPUI families, native
`links` conflicts and runtime feature unification before upgrading. Do not carry
Aviary's vendored patches wholesale. Prefer published compatible releases or a
small demonstrated upstream fix with a documented removal condition.

## Compatibility findings

Pulp 0.22.3, reached through raster dependencies, emits assembly declarations
with an LLVM `\x01` symbol marker. Pinned Cranelift debug linking left
`libpulp_v0_22_3_st_b32s_0000000000000001` undefined. `.cargo/config.toml` selects
LLVM only for this package in dev/test; release already uses LLVM. This is a
build-backend exception, not a dependency fork or relaxed application safety rule.
A real raster test also identified `fearless_simd::Avx2::cvt_u32_f32x8` using
unsupported `llvm.x86.avx.cvtt.ps2dq.256`; the same scoped LLVM override is applied
to fearless_simd 0.4.1 and its Vello Common/CPU kernel instantiations. A fearless-only
override still aborts inside `vello_common::strip::render_impl`. Other packages
remain on Cranelift in debug/test.


GPUI Kit 0.8.1 selects its matching GPUI 0.3.10 family. Its published facade
compiles both Wayland and X11 support; removing transitive defaults would require
an upstream feature change. The application explicitly selects
`platform::linux(WindowingModes::WAYLAND)` and a `WindowingRequest::Windowed`
with no X11 display, and refuses missing Wayland sessions. Compiled X11 code is
not a runtime dependency on XWayland. Actual native-session acceptance remains a
P01 check, including `DISPLAY` unset and no XWayland server.

GPUI's Linux platform selects oo7's async-std backend. Enabling oo7's Tokio
backend simultaneously enables incompatible ashpd async-io/Tokio features. Select
one oo7 backend (async-std) rather than patching GPUI or weakening secret storage.
The private credential/OAuth fixtures must verify this boundary from our Tokio
mail runtime. This does not change mail transport or configuration to async-std.

[Aviary at b451c2f](https://github.com/Catvert/aviary/tree/b451c2fd5f045cfcf5a3f5a9bd3842a63774f68b) is a useful integration reference: it
uses Blitz/AnyRender CPU painting with GPUI images. Its local table/paint/font and
Stylo patches demonstrate limitations, not patches automatically needed here.
Test ordinary/nested email tables, malformed HTML, Unicode and dark media queries
against our exact pins. Published beta2 is not a complete browser: script APIs,
full web compatibility and interactive forms are outside this passive reader.

Blitz uses Stylo 0.20/Taffy 0.14/Parley 0.11; GPUI's separate layout/font stack
need not share their Rust types. No DOM/font objects cross the rendering task
boundary. Do not assert origin isolation or CSP enforcement from browser APIs
that Blitz does not implement. The explicit provider and sanitizer policy is
specified in RENDERING.md.
