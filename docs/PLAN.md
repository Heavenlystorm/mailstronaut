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

## Product and interface review

Build a minimal client people can choose for daily mail: fast to scan, easy to
navigate, reliable under errors and comfortable with several accounts. A lean,
well-engineered codebase and the requested useful features matter more than
matching another client's appearance or number of features.

Before adding custom UI, check the pinned GPUI Kit component catalog. Use the
existing component when it solves the problem; record a concrete API limitation
before introducing a custom replacement. [INTERFACE.md](INTERFACE.md) maps the
current components and supplied design references to mail workflows.

- [x] Download and inspect all seven supplied Apple Mail/Thunderbird/Spark/Proton/
  Gmail screenshots. Keep the images and download manifest only in ignored
  `target/design-references/`; do not commit or ship them.
- [x] Audit the pinned GPUI Kit Component/Base catalog against P01–P11 UI
  subproblems and record exact choices/limits in [INTERFACE.md](INTERFACE.md).
  This is source/API research, not implemented feature or runtime acceptance.
- [ ] At each completed UI workflow and before release, compare the actual app
  with those references for usability: navigation, information hierarchy, search,
  reader space, keyboard access and recoverable errors. Record remaining issues;
  screenshots do not establish working mail behavior or performance.
- [ ] After **each implemented feature**, capture the current actual application
  in a private native Wayland session and show the screenshot in chat. Use only
  synthetic mail; identify the feature and any acceptance still unverified. Show
  light/dark or narrow/wide variants when that feature affects them. A mockup,
  reference image or old application screenshot cannot satisfy this step.

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
- [x] Pass current-stack format, strict Clippy, tests, Nix package and installed
  CLI/config smoke. Record exact commands, revision and remaining limits.

## P01 — Native GPUI Kit and Blitz foundation

Read RENDERING.md, DEPENDENCIES.md and [reader component choices](INTERFACE.md#p01-reader-and-shell). Record evidence in
`evidence/P01-gpui-blitz.md`. Earlier renderer evidence is historical.

- [x] Pin matching GPUI Kit/GPUI and Blitz/AnyRender/Vello CPU releases. Use the
  published component ecosystem; carry no large forks or reference-app patches
  without a demonstrated, documented need and an upstream removal path.
- [x] Build a normal GPUI Kit window with `TitleBar`/`Root`, standard assets/components, semantic
  light/dark themes, focus and keyboard bindings. Responsive resize/HiDPI acceptance
  is tracked below; the first native-window fixture is 1280×840.
- [x] Run on native Wayland with `DISPLAY` unset and no XWayland. Verify the actual
  platform connection and packaged window, not merely a compiled backend feature.
- [x] Embed actual Blitz HTML/CSS layout through AnyRender's Vello CPU backend and
  GPUI's standard image API. No child browser window, foreign X surface, alternate
  HTML engine or unnecessary GPU-sharing layer.
- [x] Bound HTML/DOM/resource/frame allocations; keep layout/raster work
  off the UI thread and fence message/resize/theme generations against stale frames.
  [Native owner/viewport evidence](evidence/P01-cached-html.md). Process-memory
  and latency/idle measurements remain open below.
- [x] Render synthetic ordinary and nested email tables, inline CSS, malformed
  HTML and Unicode; record unsupported layout/font/image behavior honestly.
  [Bounded native fixtures and limits](evidence/P01-cached-html.md); this is not
  acceptance of a comprehensive real-world layout corpus.
- [x] Enforce parser-based sanitization and default-deny resource, navigation and
  shell providers. Include no JS runtime, form submission, native commands,
  credential access or automatic external links. Do not assume browser CSP support.
  Remove active subtrees as well as attributes. [Evidence](evidence/P01-passive-viewport.md).
- [x] Verify the passive rendering API against remote/DNS/localhost/file/redirect/
  CSS/font/SVG/tracking fixtures with actual resolver, TCP and readable-file
  positive controls and independent syscall traces. The isolated network namespace
  is test containment, not the application's resource policy.
  [Bounded observer evidence](evidence/P01-passive-viewport.md).
- [x] Repeat bounded resource-policy and retirement acceptance through actual cached-message
  opening, switching, resizing and hiding after the MIME/HTML connector is enabled.
  [Independent native HTTP/file observations and current-frame pixels](evidence/P01-cached-html.md).
- [x] Implement immutable per-render PNG/JPEG CID tables through Blitz's decoded
  raster API; verify painting/alpha, ambiguous IDs, encoding/traversal, missing
  resources, wrong types, budgets and isolation between renders.
  [Renderer and preview evidence](evidence/P01-inline-images.md).
- [x] Connect bounded original-message MIME/CID descriptors to the HTML reader;
  respect truncated/ambiguous MIME parts and verify resource retirement during
  asynchronous message switching. CSS CID backgrounds remain unsupported.
  [MIME/worker/native isolation evidence](evidence/P01-cached-html.md).
- [x] Extract transfer-decoded local PNG/JPEG parts through the existing MIME
  library, preserving opaque IDs and validating original revision, ambiguity,
  encoding, descriptor ranges and compressed-byte budgets. Renderer validation
  remains separate. [Evidence](evidence/P01-passive-viewport.md).
- [x] Forward the app's selected color scheme through Blitz's viewport/media API.
  Respect fixed-light emails; do not invert images or reproduce desktop themes.
- [x] Provide a native read-only text reader using `TextareaState`/`Textarea::readonly(true)`;
  verify Unicode selection/copy, rejected edits, document-end keyboard scrolling
  and selection preservation across theme changes in a private Wayland session.
  [Bounded evidence](evidence/P01-native-reader.md).
- [x] Use `TabBar::segmented`/`Tab` for HTML/text mode and `Toolbar`/`ToolbarGroup`
  for reader actions. Reuse `ScrollableElement`/`ScrollHandle` for scrollbars;
  implement actual viewport rendering, resize, HiDPI and native keyboard scrolling.
  [Installed Wayland evidence](evidence/P01-cached-html.md).
- [ ] Establish an appropriate public-API HTML selection/copy route if practical;
  keep native Text/Select text usable for every readable message. A raster alone
  is not text accessibility. Verify actual assistive-technology behavior below.
- [x] Provide a retained passive document API that reuses DOM/font/raster state,
  repaints only bounded physical viewport pixels, forwards logical scale/theme,
  clamps scroll and rejects invalid geometry/excessive CSS extents. Verify pixel
  results across scroll, scale, resize and theme changes. Native UI integration,
  scrollbars and switching are verified above; performance acceptance remains below.
  [Renderer evidence](evidence/P01-passive-viewport.md).
- [x] Implement explicit validated portal links with a standard `Button` (or
  `Link` without `href`). In this pin `Link::on_click` does not override `href` and
  `Link::disabled` does not gate navigation; never pass untrusted URLs to defaults.
  Actual current-frame clicks and a Kit Button/DropdownMenu web-link picker use
  canonical HTTP(S) targets and the ASHPD portal. Source/viewport, keyboard,
  cancellation/refusal, 2x hit and no-fetch observations pass in private fixtures.
  [Bounded installed-package evidence](evidence/P01-web-links.md). Real desktop/
  Flatpak portal activation remains P10/P11 acceptance.
- [ ] Verify keyboard behavior and the actual accessibility tree on Wayland.
- [ ] Measure long-email responsiveness, repeated message switches, frame memory
  and hidden idle CPU; drop obsolete documents and stop hidden rendering.
- [ ] Pass current-stack Nix packaging and an early offline Flatpak SDK/runtime
  check, preserving native Wayland and no dependency on a host Nix executable.

An upstream limitation leaves its row open. Investigate practical options within
this stack; do not restore the previous architecture or fabricate a reader.

## P02 — Native Nix configuration and encrypted secrets

Reuse `mailstronaut-config`; read CONFIGURATION.md. The input remains a native
`.nix` expression. Home Manager directly installs that source or manages an
explicitly mutable native Nix file. An optional small HM module may manage file
ownership and activation, but must not mirror the application's schema or convert
to another configuration format. No hidden TOML, imported Nixpkgs or host Nix daemon
is required by the application.
Use the [settings component mapping](INTERFACE.md#p02-settings-and-p05-onboarding).

- [x] Retain typed schema/defaults/unknown-field rejection and the bounded pure
  Snix same-binary helper, resource limits, cancellation/reaping and redacted IPC.
- [x] Retain `config init/check`, explicit paths, no-overwrite/permissions checks
  and last-valid configuration state. [Backend evidence](evidence/P02-accounts.md).
- [x] Retain scoped oo7 storage, encrypted persistence, rotation, account/purpose
  isolation, lock and unavailable behavior against a private native Secret Service.
  Existing protocol evidence does not prove new UI or Flatpak portal acceptance.
- [x] Connect declared password accounts to native GPUI Kit Settings pages,
  source-path copy, masked encrypted password save, explicit bounded TLS/PEEK sync
  and offline reading. Verify invalid reload retention, explicit-open-only Seen,
  restart and disable-without-purge in the private Wayland fixture.
  [Bounded current-stack evidence](evidence/P02-gpui-accounts.md).
  This does not complete editable onboarding, provider login or all P02/P04 gates.
- [x] Build settings/onboarding with GPUI Kit `Settings`, `SettingPage`,
  `SettingGroup`, `SettingItem` and existing `SettingField` controls; reuse built-in
  page navigation/search and disabled-state behavior. Use masked `Input` for secret
  entry and explicit foreground keyring/OAuth actions, never a generic stored field.
  Basic preferences, encrypted credential pages and native first-run/manual/
  Google/iCloud declaration forms passed private Wayland workflows.
  [Scoped onboarding evidence](evidence/P05-onboarding.md); live-provider and
  installed-package acceptance are separate requirements.
- [x] Show the native source path/ownership/last reload with `DescriptionList`,
  a `Clipboard` path-copy action and persistent `Alert` reload errors.
  Protect managed, store-backed, read-only and unadopted user-authored expressions;
  credentials remain editable through the keyring even with managed configuration.
  [Preferences and ownership evidence](evidence/P02-preferences.md).
- [x] Provide the native `config export/reconcile` CLI and optional HM module:
  raw declared keys precede defaults, standard Snix serialization preserves data,
  app-owned sources use cooperative locking and checked atomic replacement, and
  invalid/unadopted/read-only/foreign-link changes are rejected. Verify raw helper
  bounds/redaction, native round trips, precedence/omissions/lists and conflicts.
  [Bounded native Nix evidence](evidence/P02-native-nix.md).
- [x] Connect editable UI preferences to opt-in HM `mutableSettings` with one writable native `config.nix`:
  declared attributes win on activation, undeclared UI values survive, lists replace
  as whole values, and removing a declaration does not implicitly delete data.
  Preserve explicitly declared keys before applying schema defaults. Explain that
  UI edits to declared values are reapplied from HM on the next switch.
  Appearance, automatic sync/polling, notification/privacy, timezone and reminder
  time fields stage Save/Discard; the shared writer preserves raw omissions and
  rejects stale sources. [Bounded native acceptance](evidence/P02-preferences.md).
  Full Home Manager generation activation remains a separate P10 gate.
- [x] Add **Export to Nix** preview/copy from Settings with a validated secret-free native
  attrset snapshot that can be imported as HM `settings` or installed directly.
  Include supported account declarations and secret references, never credentials
  or mailbox/runtime state. Do not rewrite the user's original expression or dotfiles.
  Use `Dialog` + read-only `Textarea`, `Clipboard` and a Save `Button` invoking
  the portal; use an in-app `Notification` for completed copy/save feedback.
  Native preview/Clipboard/read-only/unchanged-source/Seen and light/dark checks
  pass; no-clobber private save publication is unit-tested and missing-portal
  feedback is exercised. [Evidence](evidence/P02-native-nix.md).
- [ ] Accept successful/cancelled/denied FileChooser export through the native
  desktop and Flatpak portals. The save hook and unavailable-portal path above
  are implemented; they do not establish successful portal saving.
- [ ] Verify edit/save/restart/export/reimport equivalence, native string escaping,
  default/omitted fields, activation precedence, concurrent-write conflicts, invalid
  reload retention and mutable/immutable transitions in disposable profiles.
  Basic preference round trips, conflicts and invalid reloads pass the private
  native workflow above; full HM generation ownership transitions remain open.
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
  commands, WAL/foreign keys/busy timeout and migration backup/rollback. Present
  unavailable/corrupt stores with `Empty`/`Alert` and a real Retry action; use
  `AlertDialog` only for explicitly destructive purge/reset decisions.
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
  [Queries are measured at 100k and an over-limit account still lists its
  mail](evidence/P03-scale.md); partial-cache states remain open.
  Rebuild progress uses `Spinner`, or `Progress` only with a known bounded total;
  [components present store state](INTERFACE.md#p03-store-and-p04-synchronization), not persistence logic.

## P04 — IMAP synchronization and offline operation

- [x] Retain validated TLS/required STARTTLS, bounded UID pages, PEEK retrieval,
  expunge/flag reconciliation, UIDVALIDITY fencing and transactional cache backend.
  [Protocol evidence](evidence/P04.md); earlier frontend acceptance is historical.
- [ ] Connect this backend to GPUI with explicit sync, cancellable progress/errors,
  independent account state, configuration generations and useful offline reading.
  Reuse `Button::loading`/`Spinner`, Cancel/Retry `Button`s and persistent per-account
  `Alert`s; show offline/partial coverage in `StatusBar`, retaining cached rows.
  [Automatic sync passes against a real Dovecot](evidence/P04-background-sync.md);
  live providers remain open.
- [ ] Complete native/extra-CA trust tests, missing STARTTLS/invalid cert failures,
  capability discovery, account cancellation/backoff and fair connection/work budgets.
- [x] Complete SPECIAL-USE discovery, explicit folder overrides and verified
  UTF-7/UTF-8 names, without lossy folder identity.
  [Evidence](evidence/P04-background-sync.md). Raw UTF-8 wire names stay refused, since
  UTF8=ACCEPT is not enabled; invalid names display as sent.
- [ ] Complete bounded recent sync, older paging, partial-cache status, on-demand
  bodies/attachments and safe UIDVALIDITY/gap/reset reconciliation.
  [On-demand attachments](evidence/P04-large-mail.md) and [older mail from the
  mailbox](evidence/P04-older-mail.md) pass on a real server; partial-cache status
  and gap reconciliation remain open.
  Use `ListDelegate::has_more/load_more` or an explicit Load older `Button` for
  backend paging; never report an invented progress percentage.
- [x] Implement IDLE/DONE/renew/reselect with polling fallback, non-INBOX flags,
  reconnect, offline/resume recovery and changes made by other clients.
  [Real Dovecot and scripted-server evidence](evidence/P04-background-sync.md); a real
  suspend and live providers remain unrun.
- [ ] Preserve local reminder/dismiss/analysis state across moves/reconciliation.
  Neither background fetching, previews nor analysis sets Seen.
- [ ] Exercise disposable protocol servers, drops and authentication failures in
  Nix/container CI; measure idle CPU, cancellation, paging and failure isolation.

Optional CONDSTORE/QRESYNC requires separate extension proof before adoption.

## P05 — Required account onboarding and authentication

- [x] Retain public-client OAuth S256/CSRF/code exchange, external portal browser,
  scoped encrypted refresh/rotation and shared IMAP/SMTP sessions. The mock IdP,
  keyring and callback backend tests remain valid. [Evidence](evidence/P05.md).
- [x] Build first-run/account declarations and status with GPUI Kit. Never render
  a provider login form inside email HTML. Shared checked Nix persistence appends
  accounts, rejects duplicates/stale sources and generates fresh credential refs.
  Use `Form`/`Field`, `Select`/`SelectState` for provider/auth/TLS, `Input` for
  address/login/host and `NumberInput` for ports. Typed schema validation remains
  authoritative; `Field::required` alone does not validate a connection.
  Private native tests cover advanced manual values, Google/iCloud declarations,
  keyboard submission, credential routing, export/restart and unchanged Seen.
  A private encrypted-keyring/Dovecot test verifies that saving a password wakes
  a stopped automatic-sync worker. [Evidence](evidence/P05-onboarding.md).
- [ ] Verify native browser authorization, per-account refresh errors/cancellation
  and reauthentication across provider changes and pending configuration reloads.
  Existing backend mock tests remain valid; declarations/controls alone do not
  establish successful native browser consent.
- [ ] Verify Google public desktop-client ownership, consent/scopes, OAuth IMAP/SMTP,
  revoked/expired tokens and actual login/sync/controlled send with authorization.
- [ ] Verify iCloud account/app-specific password flows and provider endpoints.
- [ ] Support manual IMAP/SMTP/TLS/CA/auth configuration for INRIA, Politecnico di
  Torino and Université de Pau et des Pays de l'Adour; verify each actual supported
  route, differing login names, aliases and authentication restrictions.
  Put optional SMTP/folder/CA controls in `Collapsible`; use `InputGroup` with a
  Browse/Import `InputGroupButton` for standard portal file selection. Use ordinary
  Sign in/Unlock/Reauthenticate/Cancel buttons with `Spinner`/`Alert`, not a new wizard.
- [ ] Test locked/unavailable keyring, denied/cancelled browser, invalid callback,
  refresh races and logout/purge. Complete native Wayland and Flatpak portal flows.
- [ ] Keep live-provider outcomes separate from local mock/compile evidence.

## P06 — GPUI mailbox workspace and strong local search

Use the [mailbox/search component mapping](INTERFACE.md#p06-mailbox-threads-and-search).

- [x] Retain scoped cached-folder queries, literal FTS5 and account-local
  conversation projections. [Backend evidence](evidence/P06-browse.md).
- [x] Replace the row scaffold with GPUI Kit List/ListState/ListDelegate/ListItem,
  one external query field, account/thread/unread indicators and passive arrow
  selection. Confirm once through ListEvent::Confirm; reload canonical unread
  counts after a committed explicit open while retaining the newer body and
  passive group selection. Verify the overlapping write/query/body case and
  matching-original search in the private Wayland fixture.
  [Bounded current-stack evidence](evidence/P06-gpui-mailbox.md).
- [x] Use GPUI Kit `Sidebar`, `SidebarHeader`, `SidebarGroup`, `SidebarMenu`,
  `SidebarMenuItem` and `SidebarFooter` for unified/per-account folders and Settings.
  Reuse active/collapsed states, icons and `Badge::count`; show only implemented
  mail routes. Prefer Sidebar nesting; evaluate `Tree`/`TreeState` only if actual
  folder scale requires it, without maintaining two navigation models.
  [Routes, active states and nesting](evidence/P06-navigation.md); [unread counts on a
  real server](evidence/P04-background-sync.md).
- [ ] Deliver `h_resizable`/`resizable_panel` mailbox/list/reader panes and
  `List`/`ListState`/`ListDelegate`/`ListItem` virtualized rows. Disable the list's
  extra query field with `ListState::searchable(false)`. Use `v_virtual_list` only
  when measured variable-height rows require it; do not implement virtualization.
  Compose `Label`, `Icon`, `Badge::count/dot` and explicit row accessibility labels
  for account, sender/subject/date/two-or-three-line preview hierarchy,
  distinct focus/selection/unread/priority states and keyboard commands. Keep search,
  compose and essential actions discoverable without crowding the reading area.
- [ ] Verify useful empty/loading/offline/error/partial-cache states, long subjects,
  Unicode names, narrow/wide windows and light/dark appearance using synthetic data.
  Reuse list empty/loading hooks, `Empty`, `Skeleton`, `Alert` and `StatusBar`;
  distinguish no results, missing models and partial indexes from an empty mailbox.
- [ ] Preserve stable selection/scroll through updates; cancel stale account,
  folder, query, body and render work. Measure large-list responsiveness.
  Separate query and body/render generations: repaint rows from the canonical
  projection after a local flag commit without cancelling a newer explicit body
  open. The bounded same-scope path is implemented above; complete account/folder,
  conversation-merge, paging, removal and large-list acceptance before this row
  is complete. Preserve unread siblings through every state change.
- [ ] Display each account-local thread once, include cached Sent ancestry, open
  the matching original and allow per-message selection/actions.
  Reuse `Collapsible` originals (or `Accordion::multiple(true)` if appropriate)
  and `DescriptionList` for From/To/Cc/date; do not adopt chat/tail-following behavior.
  [Selection and opening pass](evidence/P06-navigation.md); per-message actions
  arrive with P07.
- [ ] Mark Seen only after an **explicit open action loads that original's content**.
  Passive focus/selection/restoration/search/previews/refresh never mark read;
  opening one member does not mark its whole thread. Preserve explicit unread.
  Keep `ListDelegate::set_selected_index`/`ListEvent::Select` passive;
  `ListDelegate::confirm`/`ListEvent::Confirm` invokes the existing content-load gate.
- [ ] Display extracted two/three-line previews, safe HTML/CID and accessible text
  modes; verify actual Wayland keyboard/copy/link behavior.
- [x] Research maintained fuzzy/hybrid libraries and local encoders, with exact
  source/API and isolated engine/resource probes. [Research](SEARCH.md).
- [ ] Finish model/library adoption gates and choose the smallest maintained
  integration meeting quality/resource budgets. [Tantivy is adopted for lexical
  search](SEARCH.md). Existing E5 and Model2Vec candidates fail their fixed relevance
  screen; Potion also exceeds its memory screen.
- [x] Provide typo-tolerant fuzzy subject/address/body search, accent/partial terms,
  useful ranking, deliberate exact/filter queries and bounded expansion, using a
  maintained library rather than a custom matcher or index.
  [Tantivy, with native acceptance and measurements](evidence/P06-fuzzy-search.md).
- [ ] Add multilingual retrieval by meaning: **"reminder about doctoral school
  re-enrollment" finds the French doctoral registration notice without matching
  query words**. Test EN/FR/IT paraphrases, cross-language and near-topic negatives.
- [ ] Keep queries/text/embeddings/inference local. Models are small CPU encoders,
  pinned/hashed/licensed and explicitly installed; no startup downloads or remote
  fallback. Semantic search is independent of enabling the priority classifier.
- [ ] Use one native search field with library ranking, scoped account/folder/date/
  sender filters, honest snippets and conversation deduplication. Search never sets Seen.
  Use `InputGroup`/`Input` with clear/filter buttons, a filter `Popover`,
  `Select`/`Combobox` for account/folder and `DatePickerState::range` for date bounds.
  Use `Label::highlights` for actual literal matches or GPUI `StyledText::with_highlights`
  for backend-provided UTF-8 ranges; do not fabricate highlights for semantic matches.
  `SearchableVec`/default list substring matching is suitable for small option menus,
  not the required fuzzy or multilingual search engine.
  [The one field, `from:`/`subject:`, scope and Seen pass](evidence/P06-fuzzy-search.md);
  the filter popover, date bounds, snippets and highlights remain open.
- [ ] Maintain a private rebuildable index keyed by canonical original/source/model
  revision; bounded incremental indexing, restart/model change, deletion/purge and
  disabled accounts must not retain stale results or touch user-owned state.
  [The lexical index passes](evidence/P06-fuzzy-search.md); a model revision does not
  exist yet, and neither does purging one account's data.
- [ ] Show partial-cache/index coverage and unavailable/disabled models. Local
  lexical/fuzzy search remains usable, but this does not complete semantic acceptance.
  [Messages not yet indexed are counted in the status bar](evidence/P06-fuzzy-search.md);
  partial-cache and model states remain open.
- [ ] Pass fixed and held-out relevance, filter/isolation/restart/deletion tests,
  the user's French example, and measured 10k/100k CPU/RAM/index/build budgets.

## P07 — Composition, submission and mailbox actions

Use the [composer/action component mapping](INTERFACE.md#p07-composition-and-actions).

- [x] Retain revisioned transactional drafts, lettre MIME/Unicode/Bcc handling,
  immutable originals/Message-ID, fenced SMTP attempts and delivery uncertainty.
  [Backend evidence](evidence/P07.md); previous UI evidence is historical.
- [x] Retain reply/reply-all/text-forward account/alias/ancestry preparation and
  self-recipient exclusion. [Evidence](evidence/P07-reply.md).
- [x] Retain bounded owned attachment snapshots, independent account copies,
  rejection recovery and freeze/hash integrity. [Evidence](evidence/P07-attachments.md).
- [x] Retain read/unread/star/unstar intentions, unsent-only undo, TLS application
  and exact UID/configuration fences. [Evidence](evidence/P07-actions.md).
- [x] Build the composer with `Form`/`Field`, an identity `Select`, recipient/subject
  `Input`s and body/signature `Textarea`; no source-code editor or custom text editor.
  Support To/Cc/Bcc, signature, reply/forward and validation. Keep identity and save/send
  state unambiguous. Use `Collapsible` for Cc/Bcc. Start with plain address input;
  if recipient chips are needed, reuse `InlineToken`/`InputToken` and existing
  replacement APIs rather than custom token editing or a contacts subsystem.
  [Editor, identity, Cc/Bcc, reply/forward, validation and signature](evidence/P07-native-compose.md).
- [ ] Reuse `Attachment`, `AttachmentGroup`, `AttachmentMedia`, `AttachmentContent`,
  `AttachmentTitle`, `AttachmentDescription` and `AttachmentActions` in composer and
  reader. Show filename/type/size and real pending/processing/failed/ready states;
  use built-in remove/retry affordances only where the operation supports them.
- [ ] Connect portal Attach/Remove to owned draft snapshots, and explicit received
  attachment Save/Open to bounded extraction and safe portal destinations.
  [Attach and Save pass through a fake portal on a private
  bus](evidence/P04-large-mail.md); Remove, Open and real desktop portals remain open. The
  component is presentation, not a downloader or upload service. Never pass an
  email-supplied URL/path to its image source; use generic icons or validated owned
  raster previews. Verify keyboard actions, long names, failures and narrow layouts.
- [ ] Verify autosave, draft switching, source-file access loss, close/restart and
  failure recovery on Wayland. A selected file becomes a private owned snapshot.
  Expose draft/save/outbox state in `StatusBar`, with persistent `Alert` failures.
- [ ] Integrate explicit send/offline queue/cancel/retry and redacted TLS/auth errors.
  Preserve fixed frozen bytes and Message-ID; no automatic ambiguous-delivery retry.
  [Accepted, unreachable, rejected and unknown delivery pass natively](evidence/P07-native-compose.md);
  Cancel and TLS or sign-in errors still need GUI acceptance.
- [ ] Complete accepted-DATA/drop reconciliation, provider Sent auto-save and
  separately retriable Sent APPEND. Sent-copy failure must never trigger SMTP resend.
  [Sent APPEND passes against a real server](evidence/P04-background-sync.md), and a lost
  acknowledgment [waits for an explicit decision](evidence/P07-native-compose.md); a failed
  Sent copy has not been exercised.
- [ ] Port native flag/undo controls and implement safe archive/trash/move with
  pending operations, UID MOVE/UIDPLUS fallbacks and no unrelated global EXPUNGE.
  Reuse `Toolbar`/`ToolbarGroup`, `Button`, `DropdownButton`/`PopupMenu` and
  `ContextMenuExt`; use searchable `Combobox` for move destinations and explicit
  in-app `Notification::action` for Undo. Keep durable errors beyond toast lifetime.
  [Read, star, archive, trash and move with Undo](evidence/P04-background-sync.md);
  [Undo and servers without MOVE or UIDPLUS pass natively](evidence/P07-native-actions.md).
- [ ] Recover interrupted operations and reconcile real server state, retaining
  local reminder/analysis identity and visible permanent failure/undo behavior.
  Use `AlertDialog`/`Dialog` for discard, destructive purge or ambiguous-delivery
  decisions; no routine confirmations for already reversible actions.
- [ ] Verify native composition/attachments/actions on Wayland and Flatpak, plus
  authorized controlled send/reply flows on all required providers.

HTML composition, remote draft sync, signing and permanent purge remain deferred.

## P08 — Remind me and background lifetime

Use the [reminder component mapping](INTERFACE.md#p08-reminders-and-background).

- [x] Retain durable reminder generations/claims/receipts and store transitions.
  [Backend/historical evidence](evidence/P08.md). The old UI/scheduler walkthrough
  does not establish GPUI acceptance.
- [x] Use `PopupMenu` for reminder presets and `DatePicker` with
  `DatePickerState::time_precision(TimePrecision::Minute)` for a custom instant. Retain
  original timezone/UTC, reschedule/cancel and per-message routing. Chrono TZ resolves
  the component's civil time; the widget does not implement DST policy or scheduling.
  [Presets, custom picker, list and cancel](evidence/P08-native-reminders.md);
  native elapsed-hour and custom date/time, reschedule, per-original defaults and
  restart/cancel checks pass. The picker uses its default 24-hour clock and
  `set_date_time` for the current reminder or tomorrow morning; avoid duplicating
  the external preset menu inside it.
- [ ] Complete native GUI coverage for evening/tomorrow presets, invalid civil
  times and timezone changes with a reminder already scheduled. DST policy has
  backend tests; these user-facing cases remain separate acceptance.
- [x] Use `TimeField` for configured morning/evening defaults and searchable
  `Combobox` for editable timezone; verify save/discard/restart through the native
  Settings workflow. [Evidence](evidence/P02-preferences.md).
- [x] Use `DescriptionList` for scheduled-time details, including the original
  timezone and UTC instant. Native light/dark and ordinary/narrow layouts pass
  [focused acceptance](evidence/P08-native-reminders.md#time-details-and-rescheduling).
- [x] Announce new inbox mail through the notification portal while no window is in
  front: one replaced notification per account, revealing only a count unless
  `notifications.showPreview` is set. [Evidence](evidence/P08-new-mail.md); real desktop
  servers remain open.
- [ ] Reconnect notification delivery and action handling; generation fences,
  earliest-due wake, overdue coalescing and restart/resume/clock changes must be safe.
  Use GPUI `Notification` in explicit `InApp` mode for foreground feedback only.
  Its pinned system path submits no actions and is window-bound; retain stable
  backend IDs, portal delivery and Open/Remind again/Dismiss routing for reminders.
  [Portal delivery and actions pass on a private bus](evidence/P08-native-reminders.md);
  real desktop servers remain open.
- [ ] Verify stable replacement, both crash windows, Open/Remind again/Dismiss,
  denied/disabled notifications, DST/ambiguous times and deleted-message cases.
- [x] Claim single-instance session ownership before UI/backend startup and route
  passive `org.freedesktop.Application.Activate` requests to the existing window.
  Test concurrent forwarding/coalescing, source/demo conflicts, timeout, normal
  close/restart, bus loss and unchanged Seen with the independent store profile
  lock retained. [Bounded native and bus evidence](evidence/P08-instance.md).
- [ ] Complete foreground activation with a fresh caller token on real Wayland
  desktops. The current public GPUI API requests attention from its own seat;
  private Sway retains the other workspace. Do not equate routed Activate with
  successful focus or mark the desktop entry D-Bus activatable yet.
- [ ] Move backend ownership outside the window and implement `--background` and
  window reopening without a second DB writer. Prefer GPUI's existing
  `WindowingRequest::Headless`/`App::request_windowing` lifecycle after a native
  attach/detach proof; no hidden GPU window or custom daemon. Explicit Quit stops work.
- [ ] Implement independent background/autostart/notification settings and zero
  hidden work when disabled; unload renderer/models while headless when enabled.
  Reuse `SettingField::switch` and ownership/disabled-state text for each setting;
  a toggle cannot grant portal permission or mutate HM-managed systemd services.
- [ ] Use the standard Background portal for Flatpak permission/revocation and
  Home Manager-managed native service semantics, without host-unit installation
  from the sandbox. Test close, suspend, login/restart and missing display.

No delivery guarantee is made while powered off or logged out.

## P09 — On-device priority and contextual deadlines

Read CLASSIFIER.md. Use existing small CPU inference/date libraries, not a custom
transformer, hosted model or summarization service.
Use the [priority/model component mapping](INTERFACE.md#p09-priority-deadlines-and-models).

- [x] Implement versioned evidence/result types, pure eligibility/expiry and
  user-dismissed/read-cleared state that late analysis cannot override.
  [Rules, store and native acceptance](evidence/P09-priority-rules.md).
- [ ] Run bounded local analysis for newly received readable inbox mail independently
  of opening; explicit opt-in backlog, CPU/memory budgets and unload/cancellation.
  [The rules run this way](evidence/P09-priority-rules.md); no model exists to budget,
  and unread mail up to 30 days old is analyzed without asking, because rules are cheap.
- [ ] Select a pinned/hashed/licensed lightweight EN/FR/IT model after reference
  logits, held-out quality, confidence and resource measurements. Abstain on
  unreadable/encrypted/unsupported/ambiguous input; treat model text as untrusted.
- [x] Extract actionable date evidence using existing date parsing and exact spans;
  distinguish deadlines from quoted dates, meetings, negation and marketing.
  [Evidence](evidence/P09-priority-rules.md).
- [x] Expire date-only deadlines at exclusive next **local calendar midnight**;
  explicit times expire at their instant. "Due tomorrow" remains today/tomorrow,
  never the following day. Past obligations are never newly prioritized.
  [Evidence](evidence/P09-priority-rules.md).
- [x] Choose bounded contextual 24h/3d/7d TTLs without accepted deadlines, anchored
  to receipt and never sliding on restart/resync; persist reason/time/version.
  [24 and 72 hours](evidence/P09-priority-rules.md); seven days is deliberately not
  selected, as CLASSIFIER.md requires without a reliable long-lived context detector.
- [x] Remove a priority original immediately on read/dismiss, and do not resurrect
  it via unread toggling or stale inference. A newly eligible thread member may
  reprioritize that conversation; reading one member does not read all members.
  [Evidence](evidence/P09-priority-rules.md).
- [ ] Present priority with `List` sections/headers, `Label`, `Icon`, `Badge::count`
  and semantic theme surfaces, stable ordering and a Show all button. Keep normal
  inbox access without a second custom list engine or independently scrolling inbox.
  [List sections and ordering pass](evidence/P09-priority-rules.md); the group shows at
  most 20 rows, and Show all remains open.
- [ ] Use a small `GroupBox` + `Label`/`Icon` for deadline boxes beneath extracted
  previews; show evidence/timezone/assumptions through a clicked `Popover` or reader
  `DescriptionList`. Use `Dialog`/`Form`/`DatePicker` and Dismiss `Button` for corrections,
  retaining backend persistence/expiry; evidence must not require hover alone.
  [A `Tag` below the preview, Dismiss, and evidence in the reader
  pass](evidence/P09-priority-rules.md), since rows must share one height; correcting a
  date remains open.
- [ ] Use Settings `SettingField::switch`, model metadata `DescriptionList`, an
  explicit Import `Button`/portal and `Alert` for absent/unsupported local models.
  Show `Progress` only for measured work; add no model marketplace or dashboards.
- [x] Test INTERNALDATE vs Date, delayed delivery, DST, multiple dates, quote tails,
  unsupported languages, stale read races, malicious prompts and missing models.
  [Evidence](evidence/P09-priority-rules.md). No model exists yet, so these establish
  the rules alone, which is also what a missing model falls back to.
- [ ] Keep previews extracted with source/body revision; support a future generated
  summary type without implementing or implying summarization now.

## P10 — Nix/Home Manager, Flatpak and CI

- [x] Keep one maintainable flake, shared native inputs and standard Crane caching;
  prove dependency reuse after a source-only edit. Debug/test use Cranelift/Wild;
  release uses LLVM/thin LTO/one codegen unit. Retire obsolete native-browser hooks.
  [Local source-only reuse and hosted build evidence](evidence/P01-ci-cache.md).
- [ ] Build/install through `nix develop`, `nix flake check`, `nix build`, `nix run`
  and direct Home Manager package/config composition. No custom HM format adapter.
- [x] Provide the direct native-file HM recipe and an optional thin
  `programs.mailstronaut` module with `enable`, `package`, native `settings` and
  `mutableSettings = false` by default. Share Rust evaluation/validation/serialization
  for mutable reconciliation/export; no duplicate per-field HM schema or hidden
  preference database. Test generated activation against the installed package
  with disposable, explicitly stubbed HM plumbing. [Evidence](evidence/P02-native-nix.md).
- [ ] Verify full Home Manager evaluation/generation activation (without stubbed
  plumbing), immutable links, mutable first/repeated activation, declared-value
  precedence, retained UI edits, explicit empty account lists, invalid/concurrent
  sources and changing ownership without silent overwrite. Document export/import
  and why removing a declaration is different from deleting an account's local data.
- [ ] Verify actual native Wayland launch, assets/fonts/icons, TLS roots, portals,
  keyring, notifications and GPU loader behavior on NixOS.
- [ ] Build a pinned offline Flatpak with matching SDK/Rust inputs and native
  Wayland access. No X11/XWayland requirement, broad home access, host Nix daemon,
  plaintext secrets or uncontrolled model downloads.
- [ ] Verify Flatpak config-file/model import, keyring/file/notification/Background
  portals, read-only managed configuration and packaged HTML resources.
  Reuse the P02 Settings/source/export and P05 import components; do not create a
  second sandbox-specific settings UI. Packaging and portal policy remain backend work.
- [x] Run format/strict release Clippy, backend/MIME/renderer tests, private
  credential fixtures and installed CLI/config checks in the official minimal
  Nix container on x86_64. [Hosted evidence](evidence/P01-ci-cache.md).
- [ ] Add full privacy-matrix and headless native Wayland current-stack acceptance
  to hosted CI. Local private-window evidence does not complete this row.
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
  Use `Kbd::binding_for_action` and Tooltip for real bound shortcuts, standard
  Toolbar/Menu/Dialog/Select/List focus behavior, and `FocusTrapElement::focus_trap`
  only for a genuinely custom overlay. Preserve real keyboard/AT-SPI/screen-reader
  acceptance; component availability/aria source hooks do not check off that proof.
- [ ] Review an actual daily-mail walkthrough against the supplied references:
  find an account/folder, scan unread/priority/deadline rows, search/open the matching
  original, read a thread, reply/attach/save/send, set a reminder and recover from
  offline/auth errors. Compare usability and code complexity, not visual similarity;
  add no unrelated features from the references. Show current app screenshots in chat.
- [ ] Verify the HM-first settings round trip: declare, activate, change an allowed
  mutable preference, restart, export to Nix, import into HM and activate again.
  Managed configuration and secrets must retain their stated ownership.
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
| GPUI Kit + Blitz, native Wayland, reusable components, daily usability | INTERFACE/RENDERING; P01/P06/P07/P11 |
| Native Nix config, HM mutable settings and export to Nix | CONFIGURATION; P02/P10/P11 |
| Five providers | P04/P05/P11 |
| Unified inbox and threads | P03/P06 |
| Explicit-open-only Seen | P06/P07/P09 |
| Fuzzy and multilingual meaning search | SEARCH; P06/P11 |
| Durable compose/attachments/outbox | P03/P07 |
| Remind me and optional work after close | P08 |
| Local priority, contextual expiry, deadline boxes | CLASSIFIER; P09 |
| Nix/Flatpak, minimal Nix-container CI | DELIVERY; P10/P11 |
