# Durable mailbox foundation (2026-10-29)

This is a focused store/worker implementation, not completed P03 or a working
mail client. The application has not yet connected this store to IMAP or its
inbox UI. Rendering still accepts synthetic fixtures only.

`Store::cache_message` commits a bounded MIME projection, original blob reference,
account-local canonical identity, UID membership, observed Seen/Flagged flags and
FTS data in one SQLite IMMEDIATE transaction. It uses nonzero UIDVALIDITY/UID,
never sequence numbers. Gmail protocol IDs take precedence over Message-ID/raw
hash identity, and different Gmail IDs remain distinct even for identical bytes.
Other exact copies require both Message-ID and raw hash. Missing Message-ID or
different bodies do not collapse; accounts never merge. An existing UID cannot
acquire another provider identity or unrelated original. A new SELECT generation
retires old memberships and invalidates pending server operations while retaining
messages, reminders, drafts, outbox and analysis for reconciliation.

Inbox queries accept enabled account IDs, return at most 100 rows and use a
(received time, immutable local ID) keyset cursor. Label/folder copies appear once.
Only participating inbox memberships determine the row's Seen/Flagged state.
Archive Seen does not mark an unread inbox copy Seen. Originals can be reopened
without a server; reads reject invalid hash paths, symlinks/nonregular files,
oversized originals and SHA-256 mismatches.

The numbered forward migration makes message IDs AUTOINCREMENT so deletion and
restart cannot retarget an old reminder to a newly reused ID. Before upgrading an
existing schema, SQLite `VACUUM INTO` creates a private consistent database backup,
including committed WAL state. This is a database backup, not a complete mail/blob
archive after future deletion. The supported table-rebuild procedure disables FK
enforcement outside the migration transaction, validates every FK before commit,
and restores enforcement. Failed upgrades roll back; future schemas are rejected.
Synchronous FULL is explicit. The low-level append-only ingestion API retains
its separate contract; transport uses the new canonical cache method.

A bounded Tokio channel serves one dedicated blocking database-owner thread.
No SQLite connection crosses into the GUI. Startup errors are joined/redacted;
shutdown waits off the async thread. Closed requests can be skipped before work
starts; an already started transaction completes atomically. The queue admits 16
operations, each original at most 8 MiB; callers must also bound concurrent
producers. Engine/network scheduling and XDG application wiring remain separate.

Retained decoded header fields are capped at 4096 bytes and lists at 256 entries.
Oversized routing addresses/ancestry identifiers are omitted, not truncated into
new identities; display text is cut at a Unicode boundary. Status reports header
truncation and originals remain intact. Reply/reply-all must surface incomplete
routing headers for review rather than silently composing from an incomplete list.

## Focused verification

Inside the pinned `nix develop` shell, Cranelift/Wild checks passed:

```sh
cargo fmt --all
cargo clippy -p mailstronaut-core --all-targets --locked --offline -- -D warnings
cargo test -p mailstronaut-core --locked --offline
```

All **29 core tests** passed: 10 store/migration, five mailbox, 12 MIME and two
worker tests. The suite includes a deliberately broken legacy FK fixture for
rollback, a committed uncheckpointed WAL backup, private backup permissions,
nonrecycled reminder identities after deletion/restart, account/copy/provider
isolation, UID conflict/generation handling, pagination with concurrent arrivals,
bounded headers and damaged/symlink originals. A current-thread async test holds
an independent SQLite IMMEDIATE writer transaction while the worker waits: a
25 ms timer still completes within one second, then releasing the lock allows
the request to finish. The worker also survives a restart and closes cloned
handles on joined shutdown.

[Development log](artifacts/p03-mailbox/development-check.log).
The new core has **not yet passed a combined Nix release/package gate**; that gate
will run with the next integrated dependency changes. No crash injection,
adversarial filesystem race, 100k-message benchmark, CI, GUI inbox, live account,
threading, full UID reconciliation or distribution acceptance is claimed.
