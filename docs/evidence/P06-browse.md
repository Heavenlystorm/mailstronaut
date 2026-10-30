# P06 — Account/folder navigation and device-only search

Status: implemented bounded native browsing slice; **P06 remains incomplete**.
Builds on the [conversation prototype](P06.md). The ongoing release gate for
`b607ad9` precedes this slice and must not be cited as its package acceptance.

## Implemented behavior

- Native selector: unified inbox, each enabled account's cached folders together,
  and individual known folders. Selection and search discard the prior reader and
  invalidate its async results. Disabled accounts and cross-account folder IDs are
  rejected by the store; a missing folder never falls back to syncing INBOX.
- Search uses the already-persisted SQLite FTS5 subject/address/normalized-body
  index, within the selected scope. Terms never enter an IMAP search or remote
  service. At most 1024 UTF-8 bytes/32 whitespace-separated terms are accepted;
  every term is quoted and AND-combined using the standard
  [FTS5 string rules](https://www.sqlite.org/fts5.html#fts5_strings). Quotes,
  punctuation, boolean words and column selectors remain tokenized literal data.
  This is token search, not substring, wildcard or an advanced-query language.
- Each matching conversation appears once and opens its newest matching member,
  so an older relevant reply is not hidden behind a newer nonmatching one. Read/
  star aggregates describe the full selected folder/account scope. Total cached
  member counts still include current Sent/Archive memberships. Keyset pagination
  uses the matching member's receipt time/local ID; changing scope/query resets it.
- Explicit foreground Sync first obtains a bounded complete authenticated LIST,
  persists selectable exact wire names and the configured inbox role, then fetches
  a bounded page. Removed folders retain their cache for offline navigation; this
  catalog is not a promise that a previously listed folder still exists. Modified
  UTF-7 wire names are currently displayed literally rather than decoded. Inbox
  overrides replace earlier designated inbox roles without deleting cached mail.
- Sync inboxes visits enabled inboxes; Sync account inbox visits that account's
  configured inbox; Sync folder fetches only the chosen folder. Discovery does
  not automatically download every folder. Older-page state is keyed by account
  and exact wire name. Each position also carries the public configuration hash,
  local folder ID and UIDVALIDITY. Discovery rebinding a changed account cannot
  accidentally make an old server/principal's cursor usable. Cursors are still
  in-memory only; persistent sync/IDLE/reconnect remain future work.
- Search events are redacted. No new parser, indexer, mail protocol, dependency,
  user configuration format or schema migration is introduced. The database owner
  executes all queries; native widgets perform no SQL or network work.

## Checks

Pinned `nix develop`, nightly Cranelift/Wild with the four existing LLVM exceptions:

```sh
cargo fmt --all
bash ci/check.sh
cargo build -p mailstronaut --all-features --locked --offline
shellcheck experiments/p06-conversations/gui.sh
```

Strict workspace Clippy, formatting/tool checks and **123 workspace tests plus
seven isolated renderer children passed**. The new tests cover scoped/local
matching, matching-member identity, unread counts including nonmatching siblings,
Unicode normalization through the existing tokenizer, literal syntax, disabled/
foreign scope rejection, stale UID memberships, offline index persistence,
equal-date keyset pages, catalog role replacement/rollback and retained opaque
names/cache. Native state tests cover redacted search events, stale selection
fences and folder-specific sync target/cursor routing. See the
[development log](artifacts/p06-browse/development-check.log).

The TLS fixture exercises authenticated LIST persistence before actual library UID
fetching. A regression binds a new account context through discovery while keeping
identical folder/UIDVALIDITY numbers and passing the old cursor; sync correctly
restarts at the newest UID span. Existing TLS/auth, flag, MIME, draft/outbox and
renderer checks pass. This proves the synthetic protocol behavior, not a live
provider's folder naming, delivery or authentication behavior.

## Native interaction acceptance

Prebuild application/core worker/credentials executables in the normal pinned
shell. Supply their absolute paths through `MAILSTRONAUT_THREADS_GUI_BINARY`,
`MAILSTRONAUT_THREADS_FIXTURE`, `MAILSTRONAUT_CREDENTIAL_FIXTURE`, and set
`MAILSTRONAUT_THREADS_BROWSE_GUI=1`:

```sh
nix develop .#gui-test --command bash experiments/p02-accounts/credentials.sh
```

Private X11/Xvfb 100% run passed against two synthetic invalid-network accounts
and five cached messages. Search opened the older matching first reply; the
unified inbox excluded a Sent-only match, account and Sent selections included it,
and the other account opened its own body. Literal OR terms produced an empty
view. Clearing the query restored unified grouping; normal close/light-mode offline
restart preserved the index and could open the same older match. Database receipts
confirmed all five originals and no flag intentions, drafts or outgoing records
were created by browsing. The initial failed fixture had selected text without
actually deleting it for an empty query; the corrected fixture sends Backspace
and the complete interaction now passes. [GUI log](artifacts/p06-browse/native-gui.log).

The unchanged conversation interaction mode also passed with the new heading:
selected-member read intent, ancestor reply, other-account identity and offline
recovery. [Regression log](artifacts/p06-browse/conversation-regression-gui.log).
Both private encrypted-keyring lifecycle/unavailable-service checks passed after
each GUI run. No real account, user keyring, recipient or SMTP submission was used.

- [Matching older reply](artifacts/p06-browse/search-first.png)
- [Sent folder](artifacts/p06-browse/sent-folder.png)
- [Other account](artifacts/p06-browse/other-account-search.png)
- [Literal operator/empty view](artifacts/p06-browse/literal-operator-empty.png)
- [Light offline search](artifacts/p06-browse/offline-search.png)

## Remaining acceptance

No current-source Nix release, CI-container, Flatpak, real-provider, full folder
rescan/rename, persistent cursor, worst-case query latency, large-list scrolling,
Wayland/COSMIC screen-reader or narrow/HiDPI acceptance is claimed here. Opening a
message still leaves Seen unchanged until explicit Mark read; flag controls outside
Inbox remain limited. Attachments, move/archive/trash, Sent APPEND, reminders,
background lifetime, priority/deadline UI and P01 untrusted-HTML privacy remain
open. Search covers the partial bounded index, not every server original.
