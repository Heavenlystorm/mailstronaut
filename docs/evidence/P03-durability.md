# P03 — Blob ingestion, garbage collection, cross-handle writes, cache clearing and crash injection

Each clause of the PLAN row has a named check. The local store checks run in CI with `ci/check.sh`.
The crash checks run natively against a real Dovecot server with `experiments/p04-sync/history.sh`.

| Clause | Evidence |
| --- | --- |
| Blob ingestion stays consistent when an insert fails: the original is written inside the transaction, and an unreferenced original is collected | Store tests `failed_insert_leaves_collectible_orphan` (`crates/mailstronaut-core/src/store.rs`) and the blob-collection tests in `crates/mailstronaut-core/tests/attachments.rs` |
| A shared blob stays while anything refers to it | `shared blob protected` case in `crates/mailstronaut-core/tests/attachments.rs` |
| Cross-handle transactions: a write behind another handle's lock is refused, leaves no message behind, and succeeds once the lock is released | `a_write_behind_another_handles_lock_is_refused_and_leaves_nothing_behind` (`crates/mailstronaut-core/tests/mailbox.rs`) |
| Cache clearing keeps drafts, the outbox, reminders and queued moves; removes other messages, their memberships and the history passes; and removes their unreferenced originals | `clearing_the_cache_keeps_what_the_user_owns_and_restarts_every_history_pass` (`crates/mailstronaut-core/tests/mailbox.rs`) |
| Crash injection: real kills, not simulated ones, at three points of one history pass. After each kill SQLite's integrity check passes and every cached message's original is on disk, and the resumed pass reviews each message once | `experiments/p04-sync/history.sh`, recorded in [complete history](P04-history.md) |

Not established: crashes injected inside a store transaction at chosen statements, and crashes on a
real provider rather than the synthetic Dovecot server.
