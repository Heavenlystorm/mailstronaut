# Implementation checklist

This is the authoritative handoff. `[x]` means completed **and verified**, `[ ]`
means work whose acceptance checks remain open. P00 is complete; P01–P04 have
partial implementations recorded in their milestone evidence files.
Do not mark planned features complete because an interface, mock or upstream API
exists. Update only the acceptance rows actually proven, citing a small evidence
file with revision, commands, results and limitations. Every implementation change
uses `nix develop` and inherits the root lints/dependencies. Work in small commits.

## Milestone map

| ID | Outcome | Depends on | Release blocker |
| --- | --- | --- | --- |
| P00 | Research, contracts, buildable workspace and Nix CI scaffold | — | Yes |
| P01 | libcosmic + Servo integration/privacy/theme proof | P00 | Yes |
| P02 | Native Nix config and standard credential storage | P00; packaging feedback from P01 | Yes |
| P03 | Durable local store, MIME projection and fixtures | P00/P02 schema | Yes |
| P04 | Reliable IMAP sync and offline operation | P02/P03 | Yes |
| P05 | Google/iCloud/institutional authentication | P02/P04 | Yes |
| P06 | Unified inbox and conversation UI | P01/P03/P04 | Yes |
| P07 | Draft/compose/send and safe mailbox actions | P04/P05/P06 | Yes |
| P08 | Remind me, notifications and optional background lifetime | P03/P06; P01 lifecycle | Yes |
| P09 | Local classifier, contextual expiry and deadline boxes | P03/P06/P08 | Yes |
| P10 | Complete Nix/Flatpak delivery and container integration CI | Working pins from P01/P02; P04–P09 | Yes |
| P11 | Release hardening and five-provider acceptance | P01–P10 | Yes |

P02/P03 can proceed during a documented P01 upstream blocker, but do not build a
large UI around a renderer assumption. No deadline estimate replaces acceptance.

## P00 — Planning and reproducible foundation

- [x] Inspect the repository and CFI's root workspace/dependency/lint conventions.
- [x] Research Hylki, libcosmic, Servo embedding/theme/resource APIs and the existing
  Iced/Servo wrapper; document source/version incompatibilities.
- [x] Research Google/iCloud/academic auth and local classifier options, including
  the JevBench JSON feed; distinguish provider documentation from login proof.
- [x] Specify architecture, ownership, native Nix configuration, data identity,
  rendering policy, priority/deadline/read behavior and reminder lifecycle.
- [x] Create root `members = ["crates/*"]`, centralized dependency location,
  inherited metadata/lints and locked Rust scaffold. The current strict, practical
  Clippy policy is defined in DELIVERY.md.
- [x] Create a flake with pinned Nixpkgs, `nix develop`, Nix package and checks.
- [x] Verify scaffold format/Clippy/Cargo checks inside `nix develop`, package build,
  Nix format check and installed binary smoke; record `docs/evidence/P00.md`.
- [x] Add and statically validate minimal Nix-container CI; distinguish local
  checks from the first remote workflow run.
- [x] Commit the architecture/scaffold baseline on main after final audit.

## P01 — Prove the hard rendering boundary first

Read RENDERING.md and DEPENDENCIES.md. Scope: minimal window + synthetic fixture,
not accounts or a complete inbox. Deliver `docs/evidence/P01.md` and working pins.

- [ ] Pin libcosmic and its submodules, the chosen Servo release commit/toolchain,
  actual native libraries/resources and exact Cargo/Nix dependency hashes.
- [ ] Build a normal libcosmic Application using `cosmic::iced` reexports; verify
  Wayland, live native theme, basic keyboard navigation and accessibility.
- [ ] Compile/evaluate reuse of iced_webview_v2 with Servo only; inspect duplicate
  Iced/Stylo/wgpu/links dependencies, adapter theme/security hook exposure and
  size. Record a yes/no decision with concrete compile/API evidence.
- [ ] Adopt the existing adapter with a small maintained patch, or implement the
  smallest direct offscreen Servo adapter; no browser/toolkit rewrite or alternate
  engine. Record every carried upstream patch and a path to remove it.
- [x] Implement and exercise the bounded synthetic-only software viewport,
  coalesced async wakes, owned RGBA frames, native theme notification and resize.
  P01 evidence records three renderer tests; complete mail privacy acceptance
  remains in the separate unchecked rows.
- [x] Compose the fixed synthetic viewport in the native COSMIC window using
  event-driven wakes on the UI thread. Save light/dark captures on isolated X11
  and Weston/Wayland, and exercise native page navigation and bounded resize.
  See P01 evidence; this does not complete mail input, privacy or lifecycle.
- [x] Exercise bounded synthetic wheel scrolling and initial X11 scale at
  100/150/200%, with pixel-displacement tests and uncropped before/after captures.
  See P01 evidence; dynamic Wayland scale changes and other input remain open.
- [x] Recreate disposable synthetic WebViews on one retained engine/context;
  verify 100 cycles, delegate release, native theme after reopening, and idle
  measurements. The retained graphics context is explicit; full release remains open.
- [ ] Wire waker/spin/frame-ready/redraw, input, scroll, selection/copy, HiDPI,
  resizing, focus, links and webview teardown; one message viewport at a time.
- [ ] Forward effective COSMIC dark/light theme through Servo's native method.
  Save light/dark screenshots including dark-aware and fixed-light emails.
- [x] Exercise the synthetic resource interceptor with isolated allow controls:
  loopback images, backgrounds, stylesheets/imports and font fetches, plus file
  and data rasters. See P01 evidence for the seven cancellations and proof limits.
- [x] Bind a bounded synthetic document/known PNG table to a fresh generation and
  concrete WebView; exercise initial request association, visible local raster,
  A-to-B retirement and unknown resource denial under the production CSP.
  Wrong-view/destination checks are registry tests; generic CID/origin privacy
  and concurrent in-flight retirement remain open. See P01 evidence.
- [ ] Enforce sanitizer + CSP + scheme/navigation/resource/permission policy;
  prove **zero remote requests** on load, including CSS/font/SVG/import/redirect
  and non-HTTP attempts. Test CID isolation and no local-file access.
- [x] Exercise synthetic inline-script and image-load-handler denial with separate
  script-permissive positive and sanitizer-only controls. Observe the permitted
  local image trigger, distinct script pixels and loopback execution markers.
  This does not close external-script, forms/frames or comprehensive privacy tests.
- [ ] Verify external browser links and blocked scripts/forms/frames; ensure email
  content cannot reach credentials or invoke native commands.
- [ ] Measure idle CPU, viewport memory, long-email scrolling and 100 message swaps;
  stop redraw work when hidden and release browser/graphics resources headless.
- [ ] Verify HTML accessibility bridge or document/test the native accessible text
  reader mode; a texture alone is not an accessibility tree.
- [x] Add a fixed synthetic native Text mode using COSMIC selection/copy and
  standard focus/scroll operations; verify keyboard/mouse copy, theme, resize and
  initial X11 100/150/200% scale. Actual AT-SPI publication is blocked and remains
  in the separate unchecked accessibility row. See `evidence/P01-text-reader.md`.
- [x] Inventory a project-local x86_64 Flatpak SDK with pinned Rust/LLVM extensions;
  exercise frozen offline workspace fetch and all five nested mozjs metadata
  calls in the SDK and metadata-only builder. This does not establish compilation.
- [x] Build the x86_64 Nix package with strict release Clippy and all workspace
  tests; exercise the installed CLI/native Nix helper with an empty environment.
  See P01 evidence; installed graphical and cross-architecture acceptance stay open.
- [x] Compile the workspace offline in the pinned x86_64 Flatpak SDK and exercise
  its installed probe binary/native config helper in the Platform runtime. See P01
  evidence; production Flatpak delivery and graphical acceptance stay open.

Stop condition: if composition, theme/resource hooks, build closure or required
lifecycle cannot be implemented narrowly, keep P01 open and record the upstream
blocker. Do not “solve” it with an insecure renderer or hidden engine substitution.

## P02 — Native configuration and secrets

Read CONFIGURATION.md. Create `mailstronaut-config`; no HM module or TOML parser.

- [ ] Pin snix-serde/eval and Serde, audit default features, GPL notices, build
  closure and memory; use the existing pure strict evaluator.
- [ ] Implement schemaVersion=1 structs/tagged enums, camelCase, unknown-field
  rejection, defaults and full cross-field validation. Account IDs are stable and
  unique; endpoint/port/TLS/auth/CA fields and optional distinct SMTP auth work.
- [x] Implement bounded same-binary evaluation helper, limits, cancellation/reap,
  typed result/error IPC; no I/O builtins, imported Nixpkgs, shell or host Nix.
- [ ] Test native Nix let/references/lists/conditionals, invalid types/unknown keys,
  unsupported schema, attempted IO and nontermination; verify limits empirically.
- [x] Implement `config init/check`, explicit config path, missing/invalid startup
  state and atomic last-valid reload; no partially applied account changes.
  See `evidence/P02-accounts.md`; account worker reconfiguration remains P04.
- [ ] Detect managed/store-backed/read-only files; preserve source expressions,
  do not rewrite HM files; app-owned unmanaged writes use native Nix serializer
  and atomic validation. No hidden SQLite preference overrides.
- [ ] Implement oo7 credential store, attributes, foreground unlock/redacted errors,
  no plaintext fallback; test native Secret Service and Flatpak Secret portal.
- [x] Implement and test the scoped oo7 adapter against a disposable native Secret
  Service: persistence, rotation, account/purpose isolation, ambiguity, lock and
  unavailable behavior; exercise installed GUI masked save/readback with managed
  source preservation. Flatpak portal and real desktop unlock acceptance remain
  in the separate unchecked credential row. See `evidence/P02-accounts.md`.
- [ ] Link the same .nix source via a disposable Home Manager test configuration;
  test Flatpak regular-file copy/direct-link caveat. Do not activate user dotfiles.
- [ ] Confirm no password/token can enter config/store paths, Debug output, tracing
  or a failed validation diagnostic. Test account disable vs explicit data purge.

Deliver schema docs/examples, evaluator/secret tests, HM link proof, exact pins and
`docs/evidence/P02.md`. App and Flatpak must work without a host Nix executable.

## P03 — Store and canonical message projections

Create `mailstronaut-core`. Use rusqlite, numbered migrations, mail-parser and
mail-threading; no ORM/custom MIME/threading engine. Add only current dependencies.

- [ ] Implement XDG data/cache/config paths, permissions, dedicated blocking store
  owner, bounded commands, WAL/foreign keys/busy timeout and transaction boundaries.
- [ ] Add tables/constraints in ARCHITECTURE.md and migration backup/upgrade policy.
  Test fresh DB, old schema upgrade, failed migration rollback and corrupt DB UX.
- [ ] Add atomic SHA-256 blob writes, dedup/reference counting and orphan cleanup;
  drafts/reminders/outbox must survive “clear cache” and interrupted processes.
- [ ] Parse synthetic RFC5322/MIME fixtures with current 1.0 parser; non-UTF8,
  multipart alternative, nested messages, malformed parts and bounded attachments.
- [ ] Produce original body, deterministic preview with source/body revision,
  address/header fields, attachments/CID map; no model summary or remote fetching.
- [ ] Implement account/folder/UIDVALIDITY/UID identity and multiple memberships;
  canonicalize Gmail provider IDs, preserve duplicate Message-ID distinct bodies,
  never merge accounts or persist sequence numbers as IDs.
- [ ] Use existing RFC5256/JWZ grouping with subject merging off; test missing
  ancestors/cycles/references/late parents/duplicate IDs/Sent ancestors and stable
  conversation IDs through merges. Inspect the upstream conformance corpus.
- [ ] Implement paginated unified-inbox/search projections and FTS5 indexing/purge;
  verify empty/partial-cache/offline behavior and bounded 100k-message fixture cost.

Deliver migrations and synthetic fixtures with no real email; record crash/recovery
tests and measured scale limits in `docs/evidence/P03.md`.

## P04 — IMAP synchronization

Create `mailstronaut-mail`; the engine owns scheduling/persistence. Use local
protocol fixtures, not real accounts, for deterministic integration tests.

- [x] Implement and test a bounded read-only UID page from validated TLS through
  the blocking store worker to offline native reading, with expunge/flag updates,
  UIDVALIDITY-bound paging and private dark/light GUI fixtures. This is the
  receiving prototype in [P04 evidence](evidence/P04.md), not complete sync.

- [ ] Implement validated TLS/required STARTTLS stream setup with native roots and
  optional extra CAs; test wrong host/expired cert/missing STARTTLS/revoked trust.
- [ ] Add account runtime/cancellation/backoff states and capability discovery.
  Connection/work budgets and failure isolation match ARCHITECTURE.md.
- [ ] Discover folders/SPECIAL-USE and allow explicit overrides; handle UTF-7/UTF-8
  mailbox names through verified library/server capabilities without lossy IDs.
- [ ] Implement batched initial recent sync, partial-cache indication, older paging,
  body preview/analysis PEEK fetch, attachment-on-demand and transactional cursors.
- [ ] Implement UIDVALIDITY rescan/reconciliation, UID gaps/expunges/flags and
  pending-operation invalidation. No blind cursor reuse or sequence-ID mutation.
- [ ] Implement IDLE when advertised, DONE/renew/reselect correctly and five-minute
  polling fallback; periodic non-INBOX flags and online/offline/resume recovery.
- [ ] Preserve read/dismiss/reminder state while messages move/duplicate/reconcile.
  Process changes from another client, including read flags, without re-highlighting.
- [ ] Integrate disposable Dovecot and mock SMTP fixtures started from Nix (minimal
  container-compatible harness), network drops and authentication failures; assert
  no preview/classification fetch sets Seen and no secrets appear in logs.
- [ ] Measure cancellation, fair account scheduling, large-folder paging, offline
  startup and idle CPU; cap bodies/connections/queues as documented.

Do not enable optional CONDSTORE/QRESYNC until a separate verified extension test
shows it reduces work without changing correctness. Deliver P04 evidence.

## P05 — Required account onboarding/authentication

Standard OAuth2 library, external browser and oo7; no login UI inside Servo.

- [x] Deliver the bounded public-client prototype: native portal authorization,
  library S256/CSRF/code exchange, bound encrypted refresh storage, serialized
  rotation and IMAP/SMTP credential sharing. Local protocol/keyring/native mock-
  portal evidence: [P05](evidence/P05.md). Live providers and full auth acceptance
  below remain pending.

- [ ] Implement shared authorization-code + S256 PKCE + CSRF state/loopback flow,
  secure callback lifecycle and redacted errors; timeout/duplicate/cancel/denial tests.
- [ ] Implement serialized refresh/rotation, expiry and invalid_grant behavior,
  token persistence and SASL XOAUTH2/OAUTHBEARER only as supported by the server.
- [ ] Add Google preset, correct scope, desktop client registration/test-user
  instructions, Gmail folder/label/provider-ID behavior and quota-aware backoff.
- [ ] Add iCloud preset, app-password guidance, independent incoming/outgoing login
  fields and strict TLS; never collect the Apple web account password.
- [ ] Add generic manual IMAP/SMTP/password/custom OAuth/CA/folder configuration,
  optional distinct SMTP credentials and a test-connection action that sends no mail.
- [ ] Add Microsoft public-client OAuth preset sufficient for enabled Exchange
  IMAP/SMTP; document tenant admin consent/SMTP AUTH/IMAP restrictions and no Graph.
- [ ] Validate Google and iCloud using authorized accounts; reconnect after restart,
  revoked credentials, keyring locked and offline; test both Nix and Flatpak callbacks.
- [ ] Obtain current Inria settings and validate actual account login/sync; keep
  guessed endpoints out of presets and public fixtures.
- [ ] Validate the user's PoliTO account path (student IMAP or EOL+MFA); distinguish
  blocked tenant policy from an application bug and from a passed login.
- [ ] Validate UPPA Partage with its actual login/address distinction and current
  security policy. No institutional account support claim before this test.
- [ ] Track production Google/Entra client ownership/verification separately from
  developer/test-user success; no borrowed client IDs or shipped confidential secret.

Live sign-in is a user-operated acceptance step. No agent sends/deletes mail or
submits app-provider verification forms without authorization. Record only
redacted status/versions, never auth codes, tokens, passwords or mail contents.

## P06 — Unified inbox and conversations

Extend the working libcosmic app with native widgets and synthetic/offline fixtures
first. Core provides pagination and stable IDs; UI performs no SQL/network work.

- [ ] Add account/folder navigation, unified inbox, conversation list, reader pane,
  account badges, unread/star states and visible account/sync/partial-cache status.
- [ ] Load immutable paged view models; preserve selection and scroll across updates,
  discard stale body/render results and provide large-list measurement evidence.
- [ ] Display each account-local conversation once; opening expands message members,
  sent ancestors included. Reading a visible message does not mark the whole thread.
- [ ] Add extracted two/three-line previews and safe attachments/CID handling;
  native accessible text mode, keyboard navigation, copy and error/empty/offline UX.
- [ ] Add local search/folder controls and identity-safe actions through engine
  commands; search never uploads text. Integrate sender URLs through portals only.
- [ ] Establish the priority group/deadline-box view model with synthetic data,
  COSMIC themed highlight/text/icon and overflow control; model integration is P09.
- [ ] Verify live theme, narrow/wide window behavior, 100/150/200% scale, keyboard
  and screen-reader use on COSMIC; no custom application theme/darkening framework.

Deliver screenshots/interaction checklist and measurements in P06 evidence.

## P07 — Compose, drafts, submission and mailbox actions

- [x] Deliver the bounded plain-text/password-account prototype: native To/Cc/Bcc,
  configured identity selection, local transactional autosave and close/restart
  recovery, frozen originals, fenced SMTP attempts and durable delivery uncertainty.
  Synthetic localhost TLS and private Xvfb/keyring evidence: [P07](evidence/P07.md).
  This covers neither full composer/action acceptance nor provider/distribution gates.

- [ ] Implement native plain-text composer, account/alias identity selection,
  To/Cc/Bcc, attachments via portals, signature, reply/reply-all/forward and validation.
- [ ] Use lettre's typed MIME builder for headers, References/In-Reply-To, UTF8,
  quoted text and attachments; avoid Bcc leakage/header injection/self reply loops.
- [ ] Persist drafts transactionally with autosave/restart recovery; original
  attachment source access loss must give a recoverable error, not silent omission.
- [ ] Implement frozen-MIME outbox phases, cancellation before submission, explicit
  send command, offline queue, fixed Message-ID and SMTP/TLS/error reporting.
- [ ] Test accepted-DATA/drop ambiguity, delivery_unknown, manual reconciliation,
  provider Sent auto-save and separately retriable Sent APPEND. Never resend merely
  because the Sent copy failed; no exactly-once SMTP claim.
- [ ] Implement read/unread/star/archive/trash with pending-ops/undo UX, safe UID
  MOVE/UIDPLUS fallbacks, Gmail labels and no global EXPUNGE of unrelated mail.
- [ ] Recover interrupted operations/moves and reconcile actual server state;
  preserve local reminders/analysis and show permanent failures.
- [ ] Exercise controlled send/reply/attachment flows with explicit authorization
  on required providers; protocol fixtures run on every relevant change.

HTML composition, remote draft sync, crypto signing and permanent purge remain
outside this slice. Deliver P07 evidence and explicit known limitations.

## P08 — Remind me and background behavior

- [ ] Persist reminder creation/reschedule/cancel generations; native presets/custom
  picker, UTC instant plus original timezone; independent of read/priority labels.
- [ ] Wake for the earliest due reminder and requery after restart/resume/clock
  changes; coalesce overdue backlog without losing per-message reminder records.
- [ ] Deliver stable replaceable notifications via portal/native fallback, with
  Open/Remind again/Dismiss; resolve stable IDs and deleted-message cases safely.
- [ ] Test both crash windows around DB transition/notification, permission denial,
  disabled notifications, clock jumps and DST presets; document delivery guarantees.
- [ ] Implement lifetime D-Bus single-instance/activation across window closes and
  --background starts; app commands open/reopen correct windows without a second
  DB writer. Test native service plus desktop activation races.
- [ ] Implement independent background/autostart/notification settings; explicit
  Quit, zero hidden work when disabled, idle renderer/model unload when enabled.
- [ ] Implement Flatpak Background request/denial/revocation through ASHPD and
  HM-managed native service semantics; never install host units from Flatpak.
- [ ] Verify after last-window-close, suspend/resume, restart/login, missing display
  environment and manual Quit. No claims of delivery while logged out/powered off.

Deliver P08 evidence with real COSMIC portal and notification-action tests.

## P09 — Lightweight local priority and deadlines

Read CLASSIFIER.md before coding. Preserve rules-only/no-model operation and the
extracted preview boundary. This milestone includes measurement, not just inference.

- [ ] Create versioned analysis/result/evidence types and pure eligibility/expiry
  policy; read-cleared/dismissed is authoritative, stored and never overwritten.
- [ ] Implement the rules-only baseline and fixed/contextual TTLs anchored to mail
  receipt; query-time expiry, priority thread aggregation and ordinary inbox fallback.
- [ ] Implement bounded candidate matching/normalization and existing date parser;
  EN/FR/IT supported forms, action association, original spans and visible assumptions.
- [ ] Test tomorrow/next-midnight behavior, explicit times/zones, delayed delivery,
  Date vs INTERNALDATE, no-year dates, negation, quote tails, multiple dates and
  expired obligations. DST/ambiguous/unsupported cases abstain rather than guess.
- [ ] Pin/hash/license the approved MiniLM safetensors/config/tokenizer artifact set;
  optional Nix model package and explicit Flatpak import/download; no automatic
  startup download, pickle, remote code or network dependency inside classifier.
- [ ] Load Candle's existing XLM-R classifier, correct pair tokenization/masks/label
  ordering; compare reference logits before trusting scores. Retain CPU F32 baseline.
- [ ] Add bounded local analysis worker, hash/revision cache, single-job scheduling,
  lazy load/60s idle unload and prompt disable/shutdown. Record actual CPU threading.
- [ ] Analyze all newly fetched inbox messages within configured limits; skip/status
  encrypted/oversized/malformed input honestly and handle body-fetch later safely.
- [ ] Connect typed results to highlighted priority rows and deadline boxes under
  deterministic previews, plus evidence/edit/dismiss controls; no generated summaries.
- [ ] Prove local/remote read immediately clears, stale inference cannot resurrect,
  mark-unread does not revive old priority, new thread member can qualify, expired
  priority never survives restart/time changes, and reminders remain independent.
- [ ] Build the >=180-message EN/FR/IT split fixture benchmark, compare rules/model,
  freeze thresholds and record precision/recall/coverage/deadline false positives.
- [ ] Measure cold/warm CPU latency, total peak memory, bundle size, background idle
  and no network/GPU use on the user's laptop; meet documented budgets or keep the
  failing feature/language unclaimed with concrete follow-up. Do not swap in a huge LLM.

Deliver `docs/evidence/P09.md`, model manifest and reproducible benchmark artifacts;
never commit personal mail. General JevBench scores are not this gate's evidence.

## P10 — Complete packaging and CI

Read DELIVERY.md. Both Nix and Flatpak must install/run the same completed feature
set; distinct data/secret namespaces and supported architectures are documented.

- [ ] Finalize Nix native library/resource wrappers, package icons/desktop/metainfo/
  translations/activation and model optional output; offline reproducible builds.
- [ ] Select/verify a supported Freedesktop SDK/runtime/Rust extension, pin sources,
  generate Cargo source manifest and explicitly hash all Servo/SpiderMonkey inputs.
- [ ] Add production Flatpak manifest and bundle build, least permissions, Secret/
  Background/file/browser/notification portals and application ID consistency.
- [ ] Prove regular-file native Nix config in Flatpak and documented HM linking/copy
  route without a format converter or host Nix executable.
- [ ] Run full protocol/store/classifier checks in minimal Nix container CI; add
  GUI network-privacy/headless rendering checks and artifact retention.
- [ ] Run Flatpak builds in the Nix container on a runner with verified user
  namespaces/bubblewrap capability; no unrecorded host package/privilege dependency.
- [ ] Add sandbox-enabled Nix build proof, exact-lock drift checks, dependency/license
  audit and offline-source verification. Separate live provider tests from CI.
- [ ] Test clean Nix and Flatpak installs on COSMIC, portal denial, headless reopening,
  software GL and actual GPU; collect logs/screenshots with secrets/mail redacted.
- [ ] Verify aarch64 on real matching CI/runtime before publishing that architecture;
  flake evaluation/export is not an architecture support test.

Deliver exact-head CI links, Nix derivation paths, Flatpak install/run artifacts and
P10 evidence. Do not call manifest validation or local compilation distribution proof.

## P11 — v0.1 release acceptance

- [ ] Every mandatory P01–P10 gate has evidence; review the exact final commit.
- [ ] Five requested accounts work through their actual supported auth routes;
  login/sync/flags/reconnect/controlled send differences documented separately.
- [ ] Unified inbox/threading/offline/search/draft/outbox behavior survives restarts,
  account failure, invalid config reload and server UIDVALIDITY resets.
- [ ] Read/expiry/deadline/reminder acceptance cases pass; model is optional, local,
  bounded, and reports supported-language/accuracy limitations honestly.
- [ ] Default email open is network-silent; no exposed scripts/local files, unprompted
  downloads/telemetry, credentials in Nix/DB/logs, or permission bypasses.
- [ ] Backup/upgrade/downgrade policy, cache/data distinction, local-only Thunderbird
  migration caveat, encrypted-mail limitations and background operational limits
  appear in user docs and release notes.
- [ ] CI green on release head; both install channels and desktop integration pass;
  license/notices/source artifacts, app-client ownership and privacy policy ready.
- [ ] Obtain explicit authorization before publishing/signing a release, submitting
  provider verification/Flathub forms or editing/activating the user's Home Manager.

## How the next implementer should start

Read AGENTS.md → ARCHITECTURE.md → DEPENDENCIES.md → RENDERING.md, then implement
**P01 only** in the first feature change. Use synthetic mail. Record pins/results
and blockers before moving on. Do not implement every unchecked phase in one large
patch. Ask the architect/user only for an actual product tradeoff or missing live
account detail after completing independent work; routine coding choices use these
contracts. Main may receive commits; no push/publication is implied by that permission.

## Requirement traceability

| User requirement | Contract and completion gate |
| --- | --- |
| Rust throughout application, simple component reuse, CFI-style workspace | ARCHITECTURE sections 1–3; DEPENDENCIES; P00/P01/P03 |
| COSMIC/libcosmic, Servo, native dark mode | RENDERING; P01/P06 |
| Google, iCloud, Inria, PoliTO and UPPA | CONFIGURATION provider matrix; individual live gates in P05/P11 |
| First-class Home Manager without TOML conversion | CONFIGURATION pure native Nix input/direct link; P02/P10 |
| nix develop, flake, minimal Nix-container CI | DELIVERY; flake/workflow scaffold P00, integration checks P10 |
| Nix and Flatpak distribution | Early native/SDK spike P01, complete packages and installed acceptance P10/P11 |
| Unified inbox and grouped threads | ARCHITECTURE sections 4–5; P03/P06 |
| Remind me notifications and optional work after close | ARCHITECTURE section 7; P08 |
| Lightweight on-device priority model | CLASSIFIER; P09 CPU/quality/privacy acceptance |
| Priority always clears on read, context controls lifetime | ARCHITECTURE section 6; P04 external flags/P09 expiry and read-race tests |
| Deadline box below extracted two/three-line preview | ARCHITECTURE section 6; CLASSIFIER; P06/P09 |
| Future generated summaries without implementing them now | Preview source/body-revision contract in ARCHITECTURE section 6; P03 |
| Local behavior/privacy wherever possible | Renderer default-deny P01, credentials P02, local FTS P03, offline P04, classifier P09 |
