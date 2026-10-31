# Architecture

Current direction: **GPUI Kit + Blitz**, native Rust, native Wayland. The previous
UI/browser implementation is being replaced. PLAN.md is the current progress
contract; historical frontend evidence does not prove this architecture.

## 1. Product boundary

Build a polished, responsive local email client using maintained components.
Support Google, iCloud and manually configured INRIA, Politecnico di Torino and
Université de Pau et des Pays de l'Adour accounts; unified inboxes, account-local
threads, durable composition, strong local search, reminders and lightweight
on-device priority/deadline analysis remain the product requirements.

GPUI Kit owns the visual system: standard navigation, resizable panes, virtualized
lists, inputs/editor, menus, dialogs, themes, focus and keyboard behavior. Do not
reproduce another desktop's appearance or read desktop-specific theme/configuration.
Use native Wayland; neither the application nor embedded HTML may require an X
server, XWayland or a foreign embedded window. The selected upstream GPUI family
may compile optional X11 support; that is not a runtime fallback for Mailstronaut.

Before custom UI, check the existing GPUI Kit component. Use its Sidebar for mail
navigation, Settings for preferences/accounts and Attachment for file presentation;
keep only mail-specific state and actions in our views. [INTERFACE.md](INTERFACE.md)
defines component reuse and recurring daily-use reviews against the supplied
references. The references set usability questions, not an appearance or feature
checklist. Show the current actual app in chat after each implemented feature.

Blitz handles HTML/CSS layout; its existing CPU renderer produces bounded pixels
for GPUI's ordinary image element. This avoids a full browser and unnecessary GPU
interop. Untrusted mail gets no script/native execution, ambient resource loader,
clipboard/file dialog capability or automatic remote tracking request. Rich text
accessibility and interactive HTML need their own evidence; retain a native text
reader and document actual Blitz limitations.

Native Nix configuration and direct Home Manager use are first-class. Models,
queries, messages and analysis remain local. No hidden TOML adapter, plaintext
secret fallback, automatic model download or telemetry service is introduced.
Calendar/contacts, HTML composition, remote drafts and cryptographic mail support
are deferred unless explicitly added to the roadmap.

## 2. Small workspace, explicit ownership

The root Cargo.toml owns metadata, dependencies, profiles and lint policy with
`members = ["crates/*"]`. Add crates only when implemented.

| Component | Responsibility |
| --- | --- |
| `mailstronaut` | GPUI Kit views/actions, CLI entry points, native portals and application lifetime |
| `mailstronaut-config` | Pure native Nix evaluation/helper, schema, validation and last-valid source state |
| `mailstronaut-core` | Canonical identities, SQLite/blob ownership, MIME projections, threads, drafts/operations/reminders |
| `mailstronaut-mail` | Standard TLS/IMAP/lettre SMTP/OAuth/credential adapters |
| `mailstronaut-render` | Sanitized Blitz document, bounded message resources and CPU raster output, no window/network capabilities |

Keep reusable backend behavior while replacing frontend-specific code directly;
Git history preserves the old interface. Do not maintain two toolkits or a
compatibility abstraction around old UI messages. A separate search/model crate
is appropriate only when its verified implementation exists.

The GPUI UI thread owns entities, focus, layout and displayed immutable snapshots.
Use the framework's tasks/executors; one retained Tokio runtime supports the
existing async protocol/portal APIs. Blocking SQLite stays on its existing sole
owner thread. HTML/layout/raster and inference must not block input/scrolling.
Renderer documents and fonts remain on their owning worker; only bounded owned
pixels/metadata cross into GPUI. Avoid per-message runtimes or daemon protocols.
The HTML worker retains one document with latest-value requests and one pending
reply. GPUI's standard scroll handle supplies logical offsets and actual native
bounds; only matching-generation frames are painted. Settings, text mode and
message replacement retire the previous document and local resource table.

## 3. Commands, cancellation and resource budgets

Reuse typed backend handles and immutable paged results. Changes notify views to
query a new snapshot rather than broadcasting a whole mailbox. Account/folder,
query, original-body, document, theme and size generations fence stale callbacks.
Leaving a view cancels or retires its work; dropping a task must not interrupt
already committed submission/flag/reminder bookkeeping.

A selected row is not automatically opened. Only an explicit open action that
successfully loads the selected original may mark it Seen; search, preview, list
refresh, passive focus and restored selection never do so. Recheck the selected
original/generation before applying the result. Opening one thread member does not
mark other originals read. Model output never invokes mailbox actions.

Preserve bounded queues, 100-row pages and the existing blocking store owner
(capacity 16). Initial account targets are four active accounts and at most two
IMAP connections per account, one classification job and bounded reconnect backoff
up to five minutes. Fairness, cancellation and idle costs need measurements.
Render input/DOM/resource/frame budgets are specified in RENDERING.md. Never
allocate a whole unbounded mailbox, HTML document image or analysis backlog in a
UI event handler. Use standard components/libraries rather than new infrastructure.

## 4. Local persistence and mail identity

Use `rusqlite` with SQLite FTS5, foreign keys, WAL, `busy_timeout`, transactions,
and numbered forward-only migrations through `rusqlite_migration`. One dedicated
blocking writer owns the connection; never hold a mutex/transaction across a network
await. GUI reads go through the engine; do not add an async ORM. Use prepared,
parameterized SQL. Configure durable commits appropriate to drafts and reminders;
benchmark before changing SQLite durability defaults.

Locations: platform XDG data/cache/config paths via `directories`. The mailbox
database, original-message blobs, drafts, outbox and reminders live under data;
rebuildable thumbnails and model downloads live under cache. Directories `0700`,
files `0600` where meaningful. Raw messages are content-addressed by SHA-256,
written to a temporary sibling then atomically renamed before a DB reference is
committed. Unreferenced crash leftovers can be collected. Do not erase drafts,
reminders, or outgoing-mail state in a “clear cache” operation. Mail is not
encrypted at rest in v0.1; explain reliance on the user's encrypted home/disk.

Minimum logical tables and invariants:

| Table / projection | Required fields / rules |
| --- | --- |
| `accounts` | Stable user-chosen config ID and runtime sync state; account addresses are not primary keys |
| `folders` | Account ID, opaque server mailbox identity/name, delimiter, special-use attributes, UIDVALIDITY, UIDNEXT, optional HIGHESTMODSEQ |
| `messages` | Local immutable ID, account, parsed Message-ID/References/In-Reply-To, received/sent instants, sender/recipients, subject, preview, blob hash, parse status |
| `memberships` | Unique `(account, folder, uidvalidity, uid)`; flags, modseq; never persist IMAP sequence numbers as identity |
| `provider_ids` | Optional Gmail X-GM-MSGID/X-GM-THRID; scoped to account, not universal IDs |
| `conversations` / membership | Stable local conversation ID, account-scoped ancestry projection, last activity, unread count |
| `analysis` | Message/body hash, model/tokenizer/policy revisions, scores/reasons, priority start/expiration, deadline evidence, status, dismissal/read-cleared state |
| `reminders` | ID, message ID, scheduled UTC instant, display timezone, generation, pending/delivered/cancelled state, notification ID |
| `drafts` / `outbox` | Draft content/attachments, chosen identity, fixed Message-ID, submission phase, durable errors and reconciliation state |
| `pending_operations` | ID, target identity, intended flag/move state, submitted/confirmed/conflict state; replay is explicit and idempotent where possible |
| FTS5 | Local subject/address/normalized body search with bounded indexing; delete corresponding entries on purge |

One message may have multiple folder/label memberships. Gmail label copies should
share a canonical account-local message using its provider message ID. Else use
Message-ID plus raw hash only for exact duplicates; never collapse distinct bodies
solely because Message-ID repeats. Never deduplicate between different accounts in
v0.1. Unified inbox is a query across memberships in designated inbox folders, not
a special server folder. Show account badges; conversation rows belong to one
account even when addresses/subjects match another account.

Use `mail-threading::thread_messages_with`, initially `subject_merge = false`, on
metadata decoded by `mail-parser`. Prefer verified provider thread IDs within the
same account. For other mail use References, then In-Reply-To; missing ancestors,
cycles, invalid/duplicate IDs must remain safe. Subject-only grouping can merge
unrelated mail and is disabled. Recompute the affected account's graph off the UI
thread initially; measure before introducing incremental graph infrastructure.
Preserve the oldest surviving local conversation ID through merges and aliases
so reminders and notification links remain resolvable. The implemented first
projection runs the library in a short-lived same-executable resource-limited
child because deep ancestry can recurse. Supply metadata only; enforce byte/node,
CPU/address-space and parent wall-time limits, cancellation and reaping. Validate
exact identity coverage and the unchanged account revision before committing.
This is computation isolation, not a second persistent service or a capability
sandbox. Unavailable/stale projections fall back to individual cached messages. Thread grouping must include
Sent and Archive ancestors even when the inbox shows only inbox conversations.

### 4.1. Strong local search

Literal FTS5 is a baseline, not the complete search feature. Provide typo-tolerant
fuzzy matching over subject, addresses and cached readable text, plus semantic
retrieval by meaning across languages. In particular, the English query
"reminder about doctoral school re-enrollment" must find a relevant French notice
about renewing doctoral registration even when it contains none of those query
words. English/French/Italian paraphrase and cross-language retrieval are acceptance
cases; fuzzy character matching alone cannot satisfy this requirement.

Use one native search field with lexical/fuzzy and semantic candidate ranking,
useful matching snippets and unchanged account/folder/date/sender filter semantics.
Conversation grouping must retain which original matched and open that original.
Searching or loading result previews does not mark a message Seen. Show partial
mail-cache/semantic-index coverage and an explicit unavailable-model state; local
lexical/fuzzy search stays usable when semantic inference is disabled or absent.
That degraded mode does not constitute completion of semantic search acceptance.

Queries, readable email text, embeddings, indexing and retrieval remain on device.
Choose a lightweight CPU multilingual encoder, an existing Rust inference/runtime
path and maintained fuzzy/vector components after pinned-build and resource/quality
proof. Search is independent of the priority classifier toggle or its classification
head. Models are explicit, hashed installations; never upload a query/message or
silently fetch weights. Semantic indexes are private, derived data keyed by account,
original message and source/model revision. Incremental bounded indexing, invalidation,
purge and rebuild must preserve user-owned drafts/reminders and exclude unavailable
or encrypted text. Record held-out retrieval relevance and CPU/memory/index costs;
implementation and acceptance steps are in P06 of [PLAN.md](PLAN.md).
Use the library comparison and pre-implementation adoption gate in [SEARCH.md](SEARCH.md);
research and measure existing components before writing search integration.

## 5. Synchronization and sending

Use library IMAP UID methods and capabilities; wrap them with a small account worker.
Initial sync: list subscribed/available folders, identify INBOX and SPECIAL-USE
Sent/Drafts/Trash/Archive, fetch envelopes/flags in bounded UID batches, then body
previews/new inbox bodies. Start with recent 30 days and page older mail on demand;
display that the cache is partial. Fetch all newly delivered inbox messages for
local analysis within the configured body-size budget, even if no row is opened.
Never use an opening/read API that sets `\\Seen` during preview/classification;
fetch with `BODY.PEEK`. Fetch attachments on explicit request.

Mark a message Seen locally only after an explicit user activation (click or
keyboard activation) opens that message and its content is available. Listing,
preview loading, restored selection, page entry and focus changes do not count as
opening it. Preserve explicit Mark unread until another explicit open. An open
marks only the selected message, never every member of its conversation.

Record a cursor only in the transaction that commits the fetched batch. On
UIDVALIDITY change invalidate that folder's UID mapping and pending UID operations,
rescan, and retain user-owned state until messages are reconciled. Flags seen on
another device must clear priority locally. IDLE is capability-dependent; renew
before server timeout (initial target 25 minutes). Otherwise poll every five
minutes. Synchronize other folders periodically and on activation; no connection
per folder. Optional CONDSTORE/QRESYNC is a later optimization after library/API
proof. The baseline must handle expunges and flag changes without those extensions.

State changes update local projections immediately and enter a durable operation
queue. Server success confirms; failures surface retry/undo. Use UID MOVE if
available. Otherwise perform COPY plus targeted deletion safely; never issue a
global EXPUNGE that might remove another client's deleted messages. If safe
UIDPLUS behavior is absent, show the limitation rather than risk unrelated mail.
Archive removes an inbox label/membership or moves to the account's configured
Archive; it does not hard-delete messages. Trash is reversible; permanent purge
is a separate explicit action and outside the first implementation slice.

Use lettre both for MIME message building and asynchronous SMTP submission; do not
add a second MIME builder. Reply uses In-Reply-To and References correctly, chooses
the original receiving account, avoids duplicate/self reply-all recipients, keeps
Bcc out of the transmitted visible headers, and validates header injection through
the library. Account aliases are explicit. Preserve non-ASCII names and charsets.

Compose attachments are bounded owned snapshots, selected through the standard
file portal and copied off the UI/store-owner threads. Keep at most 16 files and
5 MiB decoded bytes per draft, with validated basenames used only as MIME metadata.
Store content-addressed bytes and revisioned draft-local references; never persist
source paths as a requirement for later sending. A missing or corrupt owned blob
must stop freezing, leaving the draft editable. Account changes and rejected-send
recovery preserve references. Lettre owns multipart and transfer encoding; the
complete frozen MIME remains bounded to 8 MiB. Received-attachment export is a
separate operation requiring its own safe destination handling.

Draft autosave is local and transactional. Freeze rendered MIME and Message-ID
before entering the outbox. SMTP is not exactly-once: a connection can drop after
the server accepted DATA. Mark this `delivery_unknown`; do not blindly retry and
duplicate an email. Search Sent by Message-ID where useful and ask the user to
decide when ambiguous. APPEND to Sent is separate and retriable; do not append a
duplicate on Gmail if the provider already saves Sent. A failed Sent copy must
never trigger a second SMTP send. Test attachments, SMTPUTF8/8BITMIME capabilities,
size errors, and off-network outbox recovery.

## 6. Priority and deadline semantics

The model supplies evidence, never authority over the mailbox. It cannot mark read,
archive, delete, send, fetch a URL, or invoke tools. Analysis is local on CPU using
Candle and a small multilingual NLI encoder; [CLASSIFIER.md](CLASSIFIER.md) defines
the exact candidate, labels, bounded input, deadline parsing and validation gates.
Show “Priority” as a separate highlighted grouping using GPUI Kit semantic theme surfaces,
an icon and text; the supplied iOS image is a behavior reference, not an asset or
layout to copy. Retain normal inbox access and a control to dismiss priority.

Eligibility is evaluated against current state at every query:

```text
message is in an inbox membership
AND message is currently unread
AND analysis says priority with sufficient validated confidence
AND priority_started_at <= now < priority_expires_at
AND read_cleared_at IS NULL AND dismissed_at IS NULL
```

Set `read_cleared_at` on the first observed read after analysis (including read on
another device or immediately before an in-flight analysis result commits). Marking
unread later does not revive that old priority decision. A genuinely new unread
message in the conversation can make the conversation priority again. Commit model
results only if body/config revisions still match, then recheck read/dismissed
state. Disabling classification immediately hides priority and cancels inference.
Create a pending analysis record when new message metadata arrives and immediately
record an already-Seen message there. Read events clear that record even before
inference starts, so a later unread toggle cannot defeat this rule.

Conversations are priority if any member is eligible. A row appears once: in the
priority group while eligible, otherwise in the chronological inbox, at its normal
date. Reading one member must not silently mark every unread member read. Sort
priority conversations by nearest accepted deadline, then score, then receipt time;
use stable local ID as a final tie breaker. At most five expanded priority rows,
with “Show all (N)”; do not discard overflow.

An accepted action deadline determines expiration. A date-only deadline is inclusive
through that local calendar day; store **exclusive next-local-midnight** converted
to UTC (calendar arithmetic, not `+86400`). An explicit time expires at that instant.
“Questionnaire due tomorrow” remains today and tomorrow, disappears at the start
of the following day even if unread. An already passed deadline is never newly
priority. Earliest accepted actionable deadline wins if several occur; do not
extend automatically past it. If there is no accepted deadline, use a fixed TTL
selected from contextual classes (initial 24 hours / 3 days / 7 days, calibrated
in P09), anchored to receipt, never sliding on re-sync/restart. Confidence alone
is not a deadline. Persist the chosen time and its reason.

The inbox preview is deterministic text: prefer the decoded plain alternative;
else `mail-parser` HTML-to-text/preview utilities. Collapse whitespace, omit obvious
quote/signature tail when safely identifiable, preserve original text elsewhere,
cap stored preview at 320 Unicode scalar values and display up to three lines.
Do not add a summarization model. Define `Preview { text, source: Extracted |
Generated { model_revision }, body_revision }` so a later local summary can be
introduced without conflating it with original text or changing analysis records.

Under each preview show a small deadline box only when a date/action association
is supported by an exact source span: “Due tomorrow · 28 Oct” or “Due 28 Oct,
11:00 Europe/Paris”. The reader also shows the source quote, timezone assumption,
and edit/dismiss controls. Never infer a deadline from an arbitrary date alone.
An expired box may remain in the message reader as “Deadline passed”; remove it
from active priority. Corrected/dismissed deadline evidence is local user state
and survives resync/model upgrades. No automatic reminder is implied by a box.

## 7. Remind me and background lifecycle

“Remind me” is a local durable reminder for an email, independent of the read flag,
priority TTL and server labels. Provide one hour, this evening, tomorrow morning,
and custom date/time using native date/time widgets. Resolve civil presets in the
configured timezone and persist a UTC instant. Keep one active reminder per message;
rescheduling increments its generation, invalidating old wakeups. A reminder does
not hide/move the email, mark unread, or re-add it to priority. It remains scheduled
after reading unless the user cancels it. Thread UI can target its latest message
but must display which message is scheduled.

Core queries pending reminders at startup, after resume, after clock/timezone
changes and when a reminder changes. Sleep until the earliest due reminder with
Tokio; persist no in-memory-only timer as the source of truth. After suspend/log
out deliver each overdue reminder once on the next allowed run; coalesce large
backlogs into a summary. Notifications have stable replaceable IDs and buttons
Open, Remind again, Dismiss. Notification action resolves a stable message ID and
opens the right account/thread without parsing arbitrary command strings.
If the message was removed, offer cancellation and indicate it is unavailable;
never silently retarget a reminder to a same-subject message.

Durably transition the reminder and use a stable notification identifier so a
crash around delivery can replace a notification instead of multiplying it.
There is no atomic transaction with the notification server: explicitly test both
crash windows, prefer possible replacement over silent loss, and never promise
exactly-once desktop delivery. Muted/unavailable desktop notifications retain a
visible overdue entry in the app.

Three separate controls: `background.enabled` (continue after window closes),
`background.autostart` (start at login), and `notifications.enabled`. Native Nix
uses an ordinary Home Manager `systemd.user.services` definition running the same
binary. Other native users can use the Background portal if available. Flatpak
uses ASHPD's Background portal and honors permission denial/revocation; no host
systemd installation or `flatpak-spawn --host`. Explicit Quit terminates this
process and suspends work until a subsequent activation/login. A user service
uses `Restart=on-failure`, not `always`, so intentional Quit stays quit.

When headless, release Blitz, its surfaces/windows, and classifier weights after
an idle timeout; synchronization/reminders do not require a browser. Do not keep
a GPU context merely to poll mail. Disable-background means no hidden work after
last-window-close. No notification is possible while the machine is off, the user
session is absent, or background execution has been refused; show that operational
limit next to the setting. Do not enable user lingering or wake the machine.

## 8. Native rendering and light/dark mode

Use the published GPUI Kit facade and matching GPUI family, its styled components,
assets and semantic theme tokens. Native Wayland is mandatory. Verify both actual
window creation with DISPLAY unset and ordinary HTML displayed inside that window;
a compiled Wayland feature or headless component test alone does not establish it.

Blitz supplies layout and its viewport color-scheme/media handling. AnyRender's
Vello CPU backend supplies rasterization; GPUI displays the owned image. Start
with that public integration, not custom GPU resource sharing. RENDERING.md defines
resource denial, bounded message-local images, lifetime, input, accessibility and
measurement requirements. System fonts are an explicit rendering input; email URLs
are not filesystem authority. Include no JS runtime or built-in networking loader.

The application theme is a GPUI Kit theme, selected from validated appearance
configuration/native window appearance. Pass light/dark to Blitz without global
image inversion or desktop-specific configuration. Fixed-light email styles may
remain light. Verify themed controls and message content separately.

## 9. Configuration and delivery

See [CONFIGURATION.md](CONFIGURATION.md) for the actual native Nix schema and
Home Manager examples. Use the existing `snix-serde` evaluator; do not implement a
Nix parser/evaluator or require host `nix` inside Flatpak. Static configuration,
runtime mailbox state and credentials have distinct storage and lifetimes.

Home Manager installs native Nix directly. An optional thin module may add
`settings`/`mutableSettings` file lifecycle without duplicating the Rust schema.
Default managed files are read-only in Settings; opt-in mutable mode keeps one
app-owned native Nix file, with declared values reapplied at HM activation and
undeclared UI preferences retained. Export to Nix provides a validated, secret-free
snapshot for the user's declarative source. Do not add a hidden format, preference
overlay database or automatic dotfile rewriting; CONFIGURATION.md defines the
ownership and round-trip acceptance contract.

See [DELIVERY.md](DELIVERY.md) for reproducible Nix development, minimal Nix
container CI, Flatpak permissions, build inputs, version pins and release checks.
No telemetry, cloud classification, remote avatar service, URL preview fetch,
crash upload, or automatic model download. Remote assets can be loaded only per
message by explicit request, with tracking implications stated. Model artifacts
are optional, hashed, pinned and installed/downloaded with informed user action.

## 10. Exit criteria and changes

No feature is done because an API exists or compilation succeeds. A milestone is
done only with its listed tests/artifacts and checked acceptance rows in PLAN.md.
Keep small evidence notes with command, revision, environment, result and limits.
Pin and compile risky libraries before building the rest around them. If Blitz
cannot meet the privacy/accessibility/composition gate, or a required institutional
account cannot authorize standard IMAP/SMTP, record the blocker honestly and keep
dependent checklist items open. Do independent work; do not hide the failure behind
a mock widget, plaintext-only “complete” renderer, or a silently changed backend.
