# Mailstronaut architecture and product contract

Status: architecture and implementation contract; **implementation in progress**.
The initial native window, configuration schema and storage primitives do not yet
form a usable email client. See the per-milestone evidence for current limits.
Research date: 2026-10-27. Follow [PLAN.md](PLAN.md) in dependency order.
Upstream evidence and version caveats are in [DEPENDENCIES.md](DEPENDENCIES.md)
and [RESEARCH.md](RESEARCH.md). Requirements here are design decisions, not claims
that upstream components already implement the complete application.

## 1. Product boundary

Build a Linux desktop email client for COSMIC, with all application logic and UI
written in Rust. Use libcosmic widgets and Servo for HTML mail. Use standard mail
protocols, an embedded local database, and a small CPU classifier. No hosted
backend, web application, JavaScript UI, Python service, Ollama server, or custom
mail protocol implementation. Build descriptions and assets can use their native
formats (Nix, YAML, XML, Fluent, SVG, CSS inside email fixtures).

“Written in Rust” applies to our code. Servo includes SpiderMonkey and other
native dependencies; SQLite and desktop/graphics libraries also contain C/C++.
The requested components cannot provide a dependency tree consisting exclusively
of Rust. Track these native dependencies in packaging; do not replace the requested
components to manufacture that claim.

The first usable release includes:

- Google OAuth2, iCloud app passwords, and arbitrary IMAP/SMTP accounts, including
  separate incoming/outgoing usernames and configurable OAuth2 for institutions.
- Offline cached reading, folder navigation, unified inbox, search, conversation
  grouping, read/unread, star, archive, move to trash, and undo before submission.
- Plain-text compose, reply, reply-all, forward, To/Cc/Bcc, attachments, persistent
  drafts and an outbox. No automatic sending or forwarding by the classifier.
- A priority section above the inbox, contextual expiration, deadline boxes under
  the two/three-line text preview, and manual “Remind me” notifications.
- Optional background synchronization/reminders after closing the window, optional
  startup at login, explicit Quit, and visible degraded/offline/account error states.
- Native Nix configuration usable directly by Home Manager; Nix and Flatpak builds.

This is an email replacement target, not Thunderbird feature parity. Calendars,
CardDAV, extensions, RSS, POP3, NNTP, Exchange EWS/Graph, JMAP, automatic generative
summaries, HTML composition, S/MIME, and OpenPGP are outside v0.1. Show unsupported
encrypted messages honestly and offer saving/opening the original. Never send or
classify ciphertext as if it were readable. If an account requires Graph/EWS
because IMAP is disabled, report that limitation; do not claim support for it.
Importing an existing Thunderbird profile is also outside v0.1; IMAP accounts can
resynchronize. Explain that local-only Thunderbird folders require a later import
tool before a user retires the old client.

## 2. Small workspace, explicit ownership

Use the root `Cargo.toml`, `members = ["crates/*"]`, workspace package metadata,
workspace dependencies and inherited lints, following the inspected CFI layout.
Start with the buildable `mailstronaut` scaffold; create these other crates only
when their first milestone starts. Do not create an empty crate per abstraction.

| Crate | Owns | Must not own |
| --- | --- | --- |
| `mailstronaut` | Binary, libcosmic application/update/view/subscriptions, compose UI, CLI, desktop activation, portal integration | SQL, IMAP commands, tensor inference, HTML security policy |
| `mailstronaut-core` | Domain IDs, SQLite store/migrations, cached MIME projection, conversation projection, priority/reminder policy, engine command/event orchestration | GUI types, Servo, authentication browser, provider SDKs |
| `mailstronaut-mail` | Tokio IMAP/SMTP sessions, provider/auth configuration, OAuth2 token refresh, secret lookup, MIME input/output | Widget state, classifier decisions, database schema |
| `mailstronaut-render` | Servo adapter, sanitized message document/resource map, native theme propagation, input/render lifecycle and privacy enforcement | Account credentials, account network sessions, message persistence |
| `mailstronaut-classify` | Bounded text preprocessing, Candle model/tokenizer, deterministic deadline candidate normalization, typed analysis results | Mail transport, networking during inference, mutable UI state |
| `mailstronaut-config` | Versioned Rust configuration, pure Nix evaluation/deserialization, validation, native Nix output for app-owned configuration | Mail cache, credential contents, Home Manager option translation |

Dependency direction: binary → core/mail/render/classify/config; core →
mail/classify/config; mail → config; render and classify do not depend on core.
Core converts renderer/classifier inputs and output IDs at its boundary. Use
owned domain data and concrete types, not a generic backend/plugin framework.
Add a tiny shared types crate only if a demonstrated dependency cycle cannot be
removed by passing values. Keep runtime state enums in their owning crate.

One installed executable, one application process per profile, one Tokio engine,
one database writer. Keep synchronization alive when the last window closes only
if background mode is enabled. A separate service executable, local HTTP API,
daemon/client RPC, notification server, or custom scheduler is unnecessary.
The native user service runs the same executable with `--background`; activation
opens a window in that existing process. Use zbus for a small well-known D-Bus
activation object and keep the name owned for the entire headless/GUI lifetime.
Do not assume a toolkit's window-level single-instance facility survives after
all windows are gone. P01/P08 must prove this lifecycle.

The first rendering spike must establish whether Servo must run on the UI thread.
Do not send its `Rc`/thread-affine objects through Tokio. Accept only bounded work
and event-driven redraws in the GUI. If crash isolation later proves necessary,
that requires its own design review; it is not a reason to invent three services now.

## 3. Engine interface and resource limits

Use bounded Tokio `mpsc` commands and `watch`/bounded broadcast events. An initial
concrete interface (refine names, preserve semantics):

```text
Command:
  Sync(account_id), OpenMessage(message_id), SetSeen(message_id, bool),
  SetStarred(message_id, bool), Move(message_id, folder_id),
  SaveDraft(draft), SendDraft(draft_id),
  SetReminder(message_id, due_at), CancelReminder(reminder_id),
  DismissPriority(message_id), Reanalyze(message_id), ReloadConfig,
  SetOnline(bool), Shutdown

Event:
  AccountStatus(account_id, Connecting|Online|Offline|NeedsLogin|Error),
  InboxChanged(revision), MessageReady(message_id, body_revision),
  DraftSaved(draft_id), OutboxChanged, ReminderDue(reminder_id),
  ConfigRejected(field_errors), BackgroundUnavailable(reason)
```

Views query paginated immutable snapshots after `InboxChanged`; do not broadcast
whole mailboxes. Revision numbers allow discarding stale view/analysis responses.
Invalidate selected-message render work on navigation. A slow account must not
block another account, scrolling, configuration errors, or reminders.

Initial policy limits (measurable targets, tune from recorded evidence): at most
two IMAP connections/account (one selected INBOX IDLE, one work connection), four
active account workers, one classifier job, 256 queued engine commands, 100-row
inbox pages. Additional accounts queue fairly. Bounded exponential reconnect backoff
with jitter, cap five minutes; reset after a healthy session. On network restoration
resync immediately. All sessions, DNS/TLS, and operations have cancellation and
timeouts. Do not authenticate repeatedly on invalid credentials.

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
so reminders and notification links remain resolvable. Thread grouping must include
Sent and Archive ancestors even when the inbox shows only inbox conversations.

## 5. Synchronization and sending

Use library IMAP UID methods and capabilities; wrap them with a small account worker.
Initial sync: list subscribed/available folders, identify INBOX and SPECIAL-USE
Sent/Drafts/Trash/Archive, fetch envelopes/flags in bounded UID batches, then body
previews/new inbox bodies. Start with recent 30 days and page older mail on demand;
display that the cache is partial. Fetch all newly delivered inbox messages for
local analysis within the configured body-size budget, even if no row is opened.
Never use an opening/read API that sets `\\Seen` during preview/classification;
fetch with `BODY.PEEK`. Fetch attachments on explicit request.

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
Show “Priority” as a separate highlighted grouping using COSMIC theme surfaces,
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

When headless, release Servo, its surfaces/windows, and classifier weights after
an idle timeout; synchronization/reminders do not require a browser. Do not keep
a GPU context merely to poll mail. Disable-background means no hidden work after
last-window-close. No notification is possible while the machine is off, the user
session is absent, or background execution has been refused; show that operational
limit next to the setting. Do not enable user lingering or wake the machine.

## 8. Native rendering and dark mode

libcosmic owns the application theme. Forward the effective light/dark theme to
Servo using `WebView::notify_theme_change(servo::Theme::Dark/Light)` at creation and
on every live theme change. Use Servo's native color-scheme behavior. No color
inversion, dark CSS rewrite, Dark Reader, injected JavaScript, or duplicate HTML
darkening implementation. A sender's explicitly fixed light colors may remain
light; do not claim the native theme API automatically recolors every email.

The main rendering gate is described in [RENDERING.md](RENDERING.md). First attempt
reuse of `iced_webview_v2` with its defaults off and Servo only, aligned with the
libcosmic Iced fork. It is a candidate, not a compatible component already proven.
The upstream wrapper targets another Iced source/version and does not expose all
the required security/theme hooks. Prefer small upstreamable adapter changes;
otherwise a thin direct Servo adapter confined to this crate. Never switch to
WebKit/Chromium or implement a browser engine without changing the architecture.

Mail rendering is untrusted input. Sanitize with Ammonia and enforce an email
resource/navigation policy in Servo, covering CSS imports, fonts, SVG and redirects,
not just `<img>`. Script elements/handlers, forms, frames, active embeds, base tags,
refresh and service workers are forbidden. CSP is defense in depth. Loading an
email must cause zero remote resource requests by default. Local CID data is
provided only through an opaque per-message resource map; no `file://` or access
to mailbox/database/keyring paths. External links go to the default browser via
the portal after explicit activation. OAuth always uses that external browser.

## 9. Configuration and delivery

See [CONFIGURATION.md](CONFIGURATION.md) for the actual native Nix schema and
Home Manager examples. Use the existing `snix-serde` evaluator; do not implement a
Nix parser/evaluator or require host `nix` inside Flatpak. Static configuration,
runtime mailbox state and credentials have distinct storage and lifetimes.

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
Pin and compile risky libraries before building the rest around them. If Servo
cannot meet the privacy/accessibility/composition gate, or a required institutional
account cannot authorize standard IMAP/SMTP, record the blocker honestly and keep
dependent checklist items open. Do independent work; do not hide the failure behind
a mock widget, plaintext-only “complete” renderer, or a silently changed backend.
