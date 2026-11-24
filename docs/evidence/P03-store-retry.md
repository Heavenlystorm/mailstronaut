# P03 — Private store startup and retry

The native Wayland fixture opens the application with a private XDG profile and
synthetic mail. Holding `profile.lock` produces a locked-store message and Retry.
After the fixture releases the lock, Retry opens the seeded mailbox in the same
window. The profile and blob directories have mode `0700`. The database, lock
and stored message blobs have mode `0600`. Opening the mailbox preserves all
unread flags. Selecting a conversation marks only that message as seen.
[Locked store](artifacts/p03-store-retry/store-in-use.png) ·
[Mailbox after Retry](artifacts/p03-store-retry/store-recovered.png)

In a second private profile the fixture substitutes an invalid SQLite file.
Retry leaves its SHA-256 digest intact and releases the profile lock after the
failed open. The fixture then moves the healthy profile back into place. Retry in
the existing window reopens it with the original message body and read state.
Recovery requires an externally restored profile in this case.
[Damaged store](artifacts/p03-store-retry/store-damaged.png) ·
[Recorded test results](artifacts/p03-store-retry/verification.log)

| Store behavior | Regression test |
| --- | --- |
| A second worker receives an in-use error until the owner shuts down. SQLite lock contention leaves the async timer responsive. | `only_one_application_worker_can_own_a_profile_until_shutdown`, `sqlite_writer_contention_does_not_block_the_async_timer` |
| Filling all 16 command slots suspends admission. Cancelling a waiting request leaves no database row and the next admitted request commits. | `a_full_store_queue_defers_admission_without_committing_cancelled_requests` |
| Reopening the database preserves WAL, foreign-key enforcement and a five-second busy timeout. | `migrations_and_foreign_keys_survive_reopen` |
| A schema upgrade backs up committed WAL contents in a private backup. An unsuccessful migration rolls back and preserves user data. | `upgrades_with_consistent_private_backup_including_committed_wal`, `failed_upgrade_rolls_back_schema_and_keeps_original_user_state` |

To repeat the database checks, run `cargo test --locked --offline -p
mailstronaut-core --lib --test worker` in the pinned development shell. In the
`gui-test` shell, build the application and the `worker` integration test, then
pass their executable paths to the fixture.

```sh
MAILSTRONAUT_STORE_RETRY_GUI=1 MAILSTRONAUT_MAILBOX_FIXTURE="$worker" \
  bash experiments/p06-mailbox/wayland.sh "$app"
```
