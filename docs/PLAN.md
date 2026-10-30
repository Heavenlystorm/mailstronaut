# Implementation checklist

This is the **single source of truth** for the current direction and progress.
The application uses **GPUI Kit + Blitz**, with **native Wayland required**.
`[x]` records verified work at the stated scope; `[ ]` records unfinished
implementation or acceptance. Backend evidence remains useful where the code is
retained. Previous UI/browser screenshots, tests and package results do not prove
acceptance of the new stack. See [historical evidence](evidence/README.md).

Use `nix develop`, centralized workspace dependencies/lints and focused commits.
Keep mail, MIME, TLS, OAuth, Nix evaluation, threading, inference and rendering in
existing libraries. Do not implement a second toolkit, browser or search engine.

## Milestone map

| ID | Outcome | Dependencies |
| --- | --- | --- |
| P00 | Current architecture and reproducible workspace | — |
| P01 | GPUI Kit + Blitz foundation, native Wayland and HTML safety | P00 |
| P02 | Native Nix configuration, credentials and GPUI settings | P00/P01 |
| P03 | Durable store, canonical identity and MIME projections | P02 schema |
| P04 | Reliable IMAP synchronization and offline operation | P02/P03 |
| P05 | Google, iCloud and institutional onboarding | P02/P04 |
| P06 | Native mailbox workspace, conversations and strong local search | P01/P03/P04 |
| P07 | Compose, drafts, attachments, submission and mailbox actions | P04/P05/P06 |
| P08 | Remind me, notifications and optional background operation | P03/P06 |
| P09 | Local priority classification, contextual expiry and deadlines | P03/P06/P08 |
| P10 | Nix/Home Manager, Flatpak and minimal-container CI | P01–P09 |
| P11 | Release and five-provider acceptance | P01–P10 |

All milestones block a complete v0.1 release. Start with the new P01 foundation,
then connect the retained backend and restore mail workflows through GPUI Kit.
Model research must not indefinitely postpone the usable mailbox/compose UI.

## P00 — Architecture and reproducible foundation

- [x] Retain `members = ["crates/*"]`, centralized dependencies, inherited metadata
  and strict practical Clippy policy.
- [x] Retain the Nixpkgs/Rust/Crane/flake foundation and minimal Nix-container CI.
  A previous-stack build is not acceptance of the new GUI package.
- [x] Record the current GPUI Kit, GPUI, Blitz and CPU-raster APIs, exact dependency
  choices, native Wayland route and upstream limitations in DEPENDENCIES/RENDERING.
- [x] Resolve/lock the new graph and native inputs; remove obsolete dependencies
  and build hooks. Avoid duplicate GPUI versions and untracked build downloads.
- [ ] Pass current-stack format, strict Clippy, tests, Nix package and installed
  CLI/config smoke. Record exact commands, revision and remaining limits.

## P01 — Native GPUI Kit and Blitz foundation

Read RENDERING.md and DEPENDENCIES.md. Record evidence in
`evidence/P01-gpui-blitz.md`. Earlier renderer evidence is historical.

- [x] Pin matching GPUI Kit/GPUI and Blitz/AnyRender/Vello CPU releases. Use the
  published component ecosystem; carry no large forks or reference-app patches
  without a demonstrated, documented need and an upstream removal path.
- [ ] Build a normal GPUI Kit window with standard assets/components, semantic
  light/dark themes, focus, keyboard bindings and responsive layout.
- [ ] Run on native Wayland with `DISPLAY` unset and no XWayland. Verify the actual
  platform connection and packaged window, not merely a compiled backend feature.
- [x] Embed actual Blitz HTML/CSS layout through AnyRender's Vello CPU backend and
  GPUI's standard image API. No child browser window, foreign X surface, alternate
  HTML engine or unnecessary GPU-sharing layer.
- [ ] Bound HTML/DOM/resource/frame dimensions and memory; keep layout/raster work
  off the UI thread and fence message/resize/theme generations against stale frames.
- [ ] Render synthetic ordinary and nested email tables, inline CSS, malformed
  HTML and Unicode; record unsupported layout/font/image behavior honestly.
- [ ] Enforce parser-based sanitization and default-deny resource, navigation and
  shell providers. Include no JS runtime, form submission, native commands,
  credential access or automatic external links. Do not assume browser CSP support.
- [ ] Prove no remote/DNS/localhost/file/redirect/CSS/font/SVG/tracking loads on
  open, using working positive controls and independent observations.
- [ ] Support bounded immutable message-local CID/raster resources; test ambiguous
  IDs, encoding/traversal, missing parts, wrong types, retirement and isolation.
- [x] Forward the app's selected color scheme through Blitz's viewport/media API.
  Respect fixed-light emails; do not invert images or reproduce desktop themes.
- [ ] Implement scrolling, resize, HiDPI, explicit portal links, selection/copy and
  a native accessible text reader. A raster alone is not text accessibility.
- [ ] Verify keyboard behavior and the actual accessibility tree on Wayland.
- [ ] Measure long-email responsiveness, repeated message switches, frame memory
  and hidden idle CPU; drop obsolete documents and stop hidden rendering.
- [ ] Pass current-stack Nix packaging and an early offline Flatpak SDK/runtime
  check, preserving native Wayland and no dependency on a host Nix executable.

An upstream limitation leaves its row open. Investigate practical options within
this stack; do not restore the previous architecture or fabricate a reader.

## P02 — Native Nix configuration and encrypted secrets

Reuse `mailstronaut-config`; read CONFIGURATION.md. The input remains a native
`.nix` expression. Home Manager links/copies that same source directly; no HM
adapter, hidden TOML file, imported Nixpkgs or host Nix daemon is required.

- [x] Retain typed schema/defaults/unknown-field rejection and the bounded pure
  Snix same-binary helper, resource limits, cancellation/reaping and redacted IPC.
- [x] Retain `config init/check`, explicit paths, no-overwrite/permissions checks
  and last-valid configuration state. [Backend evidence](evidence/P02-accounts.md).
- [x] Retain scoped oo7 storage, encrypted persistence, rotation, account/purpose
  isolation, lock and unavailable behavior against a private native Secret Service.
  Existing protocol evidence does not prove new UI or Flatpak portal acceptance.
- [ ] Build GPUI Kit settings/onboarding with managed/read-only source handling,
  reload/error states, masked secret fields and foreground keyring/OAuth actions.
- [ ] Reconfigure account workers atomically; stale callbacks and invalid/removed
  configuration cannot replace the last valid account state or expose secrets.
- [ ] Verify the same `.nix` source through disposable Home Manager integration,
  native package and Flatpak regular-file/document access. Never activate user dotfiles.
- [ ] Prove no password/token appears in config, Nix store, Debug, tracing or failed
  diagnostics; test disabled accounts separately from explicit data purge.
- [ ] Verify real desktop unlock and Flatpak Secret portal behavior, without a
  plaintext credential fallback or an externally executed password command.

## P03 — Local store and canonical message projections

Reuse `mailstronaut-core`, rusqlite/migrations, mail-parser and mail-threading.
Existing synthetic backend tests/evidence remain relevant; rerun them with the
new dependency graph before claiming current-package acceptance.

- [ ] Verify private XDG paths, one blocking store owner/profile lock, bounded
  commands, WAL/foreign keys/busy timeout, migration backup/rollback and corrupt DB UX.
- [ ] Verify atomic SHA-256 blob ingestion/GC, cross-handle transactions and cache
  clearing that preserves drafts/outbox/reminders; add crash-injection acceptance.
- [ ] Complete bounded MIME/non-UTF8/alternative/nested/malformed/encrypted fixtures,
  attachment/CID descriptors and source/body revisions. No generated summaries.
- [ ] Complete account/folder/UIDVALIDITY/UID identity, multi-membership/provider
  deduplication and duplicate Message-ID tests; never merge different accounts.
- [x] Retain account-local RFC5256/JWZ grouping with subject merging disabled,
  revision-fenced stable conversations and the 37-case upstream conformance corpus.
- [ ] Complete missing/late/Sent ancestry, cycles, duplicate IDs and merge behavior
  at measured mailbox scale; sequence numbers never become persistent identity.
- [ ] Verify paginated unified/search projections, derived-index purge/rebuild,
  partial-cache states and bounded 100k-message behavior. See P06 for strong search.

## P04 — IMAP synchronization and offline operation

- [x] Retain validated TLS/required STARTTLS, bounded UID pages, PEEK retrieval,
  expunge/flag reconciliation, UIDVALIDITY fencing and transactional cache backend.
  [Protocol evidence](evidence/P04.md); earlier frontend acceptance is historical.
- [ ] Connect this backend to GPUI with explicit sync, cancellable progress/errors,
  independent account state, configuration generations and useful offline reading.
- [ ] Complete native/extra-CA trust tests, missing STARTTLS/invalid cert failures,
  capability discovery, account cancellation/backoff and fair connection/work budgets.
- [ ] Complete SPECIAL-USE discovery, explicit folder overrides and verified
  UTF-7/UTF-8 names, without lossy folder identity.
- [ ] Complete bounded recent sync, older paging, partial-cache status, on-demand
  bodies/attachments and safe UIDVALIDITY/gap/reset reconciliation.
- [ ] Implement IDLE/DONE/renew/reselect with polling fallback, non-INBOX flags,
  reconnect, offline/resume recovery and changes made by other clients.
- [ ] Preserve local reminder/dismiss/analysis state across moves/reconciliation.
  Neither background fetching, previews nor analysis sets Seen.
- [ ] Exercise disposable protocol servers, drops and authentication failures in
  Nix/container CI; measure idle CPU, cancellation, paging and failure isolation.

Optional CONDSTORE/QRESYNC requires separate extension proof before adoption.

## P05 — Required account onboarding and authentication

- [x] Retain public-client OAuth S256/CSRF/code exchange, external portal browser,
  scoped encrypted refresh/rotation and shared IMAP/SMTP sessions. The mock IdP,
  keyring and callback backend tests remain valid. [Evidence](evidence/P05.md).
- [ ] Port onboarding/account status to GPUI Kit; isolate account errors, token
  refresh and cancellation. Never render a provider login form inside email HTML.
- [ ] Verify Google public desktop-client ownership, consent/scopes, OAuth IMAP/SMTP,
  revoked/expired tokens and actual login/sync/controlled send with authorization.
- [ ] Verify iCloud account/app-specific password flows and provider endpoints.
- [ ] Support manual IMAP/SMTP/TLS/CA/auth configuration for INRIA, Politecnico di
  Torino and Université de Pau et des Pays de l'Adour; verify each actual supported
  route, differing login names, aliases and authentication restrictions.
- [ ] Test locked/unavailable keyring, denied/cancelled browser, invalid callback,
  refresh races and logout/purge. Complete native Wayland and Flatpak portal flows.
- [ ] Keep live-provider outcomes separate from local mock/compile evidence.

## P06 — GPUI mailbox workspace and strong local search

- [x] Retain scoped cached-folder queries, literal FTS5 and account-local
  conversation projections. [Backend evidence](evidence/P06-browse.md).
- [ ] Deliver a coherent GPUI Kit sidebar, resizable mailbox/list/reader panes,
  virtualized rows, account badges, metadata/preview hierarchy, unread/star states,
  keyboard commands and responsive empty/offline/error/partial-cache states.
- [ ] Preserve stable selection/scroll through updates; cancel stale account,
  folder, query, body and render work. Measure large-list responsiveness.
- [ ] Display each account-local thread once, include cached Sent ancestry, open
  the matching original and allow per-message selection/actions.
- [ ] Mark Seen only after an **explicit open action loads that original's content**.
  Passive focus/selection/restoration/search/previews/refresh never mark read;
  opening one member does not mark its whole thread. Preserve explicit unread.
- [ ] Display extracted two/three-line previews, safe HTML/CID and accessible text
  modes; verify actual Wayland keyboard/copy/link behavior.
- [x] Research maintained fuzzy/hybrid libraries and local encoders, with exact
  source/API and isolated engine/resource probes. [Research](SEARCH.md).
- [ ] Finish model/library adoption gates and choose the smallest maintained
  integration meeting quality/resource budgets. Existing E5 and Model2Vec candidates
  fail their fixed relevance screen; Potion also exceeds its memory screen.
- [ ] Provide typo-tolerant fuzzy subject/address/body search, accent/partial terms,
  useful ranking, deliberate exact/filter queries and bounded expansion, using a
  maintained library rather than a custom matcher or index.
- [ ] Add multilingual retrieval by meaning: **"reminder about doctoral school
  re-enrollment" finds the French doctoral registration notice without matching
  query words**. Test EN/FR/IT paraphrases, cross-language and near-topic negatives.
- [ ] Keep queries/text/embeddings/inference local. Models are small CPU encoders,
  pinned/hashed/licensed and explicitly installed; no startup downloads or remote
  fallback. Semantic search is independent of enabling the priority classifier.
- [ ] Use one native search field with library ranking, scoped account/folder/date/
  sender filters, honest snippets and conversation deduplication. Search never sets Seen.
- [ ] Maintain a private rebuildable index keyed by canonical original/source/model
  revision; bounded incremental indexing, restart/model change, deletion/purge and
  disabled accounts must not retain stale results or touch user-owned state.
- [ ] Show partial-cache/index coverage and unavailable/disabled models. Local
  lexical/fuzzy search remains usable, but this does not complete semantic acceptance.
- [ ] Pass fixed and held-out relevance, filter/isolation/restart/deletion tests,
  the user's French example, and measured 10k/100k CPU/RAM/index/build budgets.

## P07 — Composition, submission and mailbox actions

- [x] Retain revisioned transactional drafts, lettre MIME/Unicode/Bcc handling,
  immutable originals/Message-ID, fenced SMTP attempts and delivery uncertainty.
  [Backend evidence](evidence/P07.md); previous UI evidence is historical.
- [x] Retain reply/reply-all/text-forward account/alias/ancestry preparation and
  self-recipient exclusion. [Evidence](evidence/P07-reply.md).
- [x] Retain bounded owned attachment snapshots, independent account copies,
  rejection recovery and freeze/hash integrity. [Evidence](evidence/P07-attachments.md).
- [x] Retain read/unread/star/unstar intentions, unsent-only undo, TLS application
  and exact UID/configuration fences. [Evidence](evidence/P07-actions.md).
- [ ] Build the GPUI Kit composer from standard inputs/editor/identity controls,
  To/Cc/Bcc, portal Attach/Remove, signature, reply/forward and validation.
- [ ] Verify autosave, draft switching, source-file access loss, close/restart and
  failure recovery on Wayland. A selected file becomes a private owned snapshot.
- [ ] Integrate explicit send/offline queue/cancel/retry and redacted TLS/auth errors.
  Preserve fixed frozen bytes and Message-ID; no automatic ambiguous-delivery retry.
- [ ] Complete accepted-DATA/drop reconciliation, provider Sent auto-save and
  separately retriable Sent APPEND. Sent-copy failure must never trigger SMTP resend.
- [ ] Port native flag/undo controls and implement safe archive/trash/move with
  pending operations, UID MOVE/UIDPLUS fallbacks and no unrelated global EXPUNGE.
- [ ] Recover interrupted operations and reconcile real server state, retaining
  local reminder/analysis identity and visible permanent failure/undo behavior.
- [ ] Verify native composition/attachments/actions on Wayland and Flatpak, plus
  authorized controlled send/reply flows on all required providers.

HTML composition, remote draft sync, signing and permanent purge remain deferred.

## P08 — Remind me and background lifetime

- [x] Retain durable reminder generations/claims/receipts and store transitions.
  [Backend/historical evidence](evidence/P08.md). The old UI/scheduler walkthrough
  does not establish GPUI acceptance.
- [ ] Add GPUI Kit presets/date/time controls, original timezone and UTC instants,
  reschedule/cancel and per-message routing independent of priority/read labels.
- [ ] Reconnect notification delivery and action handling; generation fences,
  earliest-due wake, overdue coalescing and restart/resume/clock changes must be safe.
- [ ] Verify stable replacement, both crash windows, Open/Remind again/Dismiss,
  denied/disabled notifications, DST/ambiguous times and deleted-message cases.
- [ ] Implement single-instance D-Bus activation/window reopening and `--background`
  without a second DB writer. Explicit Quit must stop work.
- [ ] Implement independent background/autostart/notification settings and zero
  hidden work when disabled; unload renderer/models while headless when enabled.
- [ ] Use the standard Background portal for Flatpak permission/revocation and
  Home Manager-managed native service semantics, without host-unit installation
  from the sandbox. Test close, suspend, login/restart and missing display.

No delivery guarantee is made while powered off or logged out.

## P09 — On-device priority and contextual deadlines

Read CLASSIFIER.md. Use existing small CPU inference/date libraries, not a custom
transformer, hosted model or summarization service.

- [ ] Implement versioned evidence/result types, pure eligibility/expiry and
  user-dismissed/read-cleared state that late analysis cannot override.
- [ ] Run bounded local analysis for newly received readable inbox mail independently
  of opening; explicit opt-in backlog, CPU/memory budgets and unload/cancellation.
- [ ] Select a pinned/hashed/licensed lightweight EN/FR/IT model after reference
  logits, held-out quality, confidence and resource measurements. Abstain on
  unreadable/encrypted/unsupported/ambiguous input; treat model text as untrusted.
- [ ] Extract actionable date evidence using existing date parsing and exact spans;
  distinguish deadlines from quoted dates, meetings, negation and marketing.
- [ ] Expire date-only deadlines at exclusive next **local calendar midnight**;
  explicit times expire at their instant. "Due tomorrow" remains today/tomorrow,
  never the following day. Past obligations are never newly prioritized.
- [ ] Choose bounded contextual 24h/3d/7d TTLs without accepted deadlines, anchored
  to receipt and never sliding on restart/resync; persist reason/time/version.
- [ ] Remove a priority original immediately on read/dismiss, and do not resurrect
  it via unread toggling or stale inference. A newly eligible thread member may
  reprioritize that conversation; reading one member does not read all members.
- [ ] Present a GPUI Kit themed priority group, icon/text/highlight and overflow,
  stable ordering and normal inbox access. Show a deadline box beneath extracted
  previews, source quote/timezone/assumptions in the reader and user correction.
- [ ] Test INTERNALDATE vs Date, delayed delivery, DST, multiple dates, quote tails,
  unsupported languages, stale read races, malicious prompts and missing models.
- [ ] Keep previews extracted with source/body revision; support a future generated
  summary type without implementing or implying summarization now.

## P10 — Nix/Home Manager, Flatpak and CI

- [ ] Keep one maintainable flake, shared native inputs and standard Crane caching;
  prove dependency reuse after a source-only edit. Debug/test use Cranelift/Wild;
  release uses LLVM/thin LTO/one codegen unit. Retire obsolete native-browser hooks.
- [ ] Build/install through `nix develop`, `nix flake check`, `nix build`, `nix run`
  and direct Home Manager package/config composition. No custom HM format adapter.
- [ ] Verify actual native Wayland launch, assets/fonts/icons, TLS roots, portals,
  keyring, notifications and GPU loader behavior on NixOS.
- [ ] Build a pinned offline Flatpak with matching SDK/Rust inputs and native
  Wayland access. No X11/XWayland requirement, broad home access, host Nix daemon,
  plaintext secrets or uncontrolled model downloads.
- [ ] Verify Flatpak config-file/model import, keyring/file/notification/Background
  portals, read-only managed configuration and packaged HTML resources.
- [ ] Run format/strict Clippy/backend/MIME/privacy/fixture tests in the official
  minimal Nix container; add headless native Wayland current-stack acceptance.
- [ ] Confirm hosted CI on the actual release source and distinguish local package,
  GUI, security, accessibility, live-provider and distribution evidence.
- [ ] Include licenses/notices/reproducible sources and explicit optional model
  provisioning. Do not publish/sign or submit provider/Flathub forms without consent.

## P11 — v0.1 completion audit

- [ ] Every mandatory P00–P10 row has current, scope-matched evidence.
- [ ] All five account configurations work through their supported auth routes,
  actual login/sync/reconnect/flags and authorized controlled sends.
- [ ] Unified inbox, threads, offline browsing, strong search, drafts, attachments,
  outbox, reminders and background behavior survive expected errors/restarts.
- [ ] The GPUI Kit interface is visually consistent, keyboard/accessibility tested,
  responsive at measured mailbox scale and genuinely native Wayland.
- [ ] Ordinary HTML emails render reliably within documented Blitz limitations,
  remain network-silent by default and cannot execute active/native content or
  access local resources/other messages/credentials.
- [ ] Local priority/read/deadline/expiry and contextual reminder cases pass;
  model limits, accuracy/languages, cache coverage and unsupported crypto are clear.
- [ ] Nix/Home Manager and Flatpak installation/runtime pass; hosted CI is green.
- [ ] Document backup/upgrade/downgrade, data/cache purge, local Thunderbird import
  caveats, privacy and operational limits. Obtain publishing/signing authorization.

## Implementation sequence and traceability

Finish P01 first, then reconnect P02/P06 to the retained backend, restore P07/P08
interfaces and complete sync/auth/search/classifier/delivery acceptance. Read this
file before choosing the next slice; do not infer progress from old screenshots.

| Requirement | Current contract/gate |
| --- | --- |
| Rust, simple existing components, workspace | ARCHITECTURE/DEPENDENCIES; P00/P01/P03 |
| GPUI Kit + Blitz, native Wayland, polished UI | RENDERING; P01/P06/P07 |
| Native Nix config and direct Home Manager | CONFIGURATION; P02/P10 |
| Five providers | P04/P05/P11 |
| Unified inbox and threads | P03/P06 |
| Explicit-open-only Seen | P06/P07/P09 |
| Fuzzy and multilingual meaning search | SEARCH; P06/P11 |
| Durable compose/attachments/outbox | P03/P07 |
| Remind me and optional work after close | P08 |
| Local priority, contextual expiry, deadline boxes | CLASSIFIER; P09 |
| Nix/Flatpak, minimal Nix-container CI | DELIVERY; P10/P11 |
