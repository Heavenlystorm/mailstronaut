# P07 — Permanent IMAP move warnings

If a server offers neither UID MOVE nor UIDPLUS, Mailstronaut restores the
message to its original folder. A warning in the mailbox identifies the account
and stays visible across successful syncs and application restarts until the
user dismisses it. Temporary failures continue through the normal retry path.

Schema 17 adds a default-off warning flag to each account. Existing accounts
upgrade with their data intact. The store records a server refusal in the same
SQLite transaction that removes the claimed move, matching the attempt token
and authenticated mailbox context. Dismissal uses that same account and
context. Binding a different authenticated mailbox clears the old warning.

The core `moves` integration tests exercise restart, stale dismissal, context
replacement and a transient refusal. The migration test
`upgrading_schema_sixteen_starts_existing_accounts_without_move_warnings` opens
an existing schema 16 account and verifies its initial state after upgrading.

The `neither` Dovecot variant of `experiments/p07-compose/actions.sh` refuses a
move, delivers more mail, restarts the native application, dismisses the
warning and restarts again. It checks that the queued move is gone, all five
inbox messages remain and the server retains the original message.

The pinned Linux run passed all eight move integration tests, the schema 16
migration test and the three Dovecot capability variants. The screenshot
shows the warning for Synthetic reader after restarting, with the original
message and the newly delivered message both visible in the inbox.
[Restart screenshot](artifacts/p07-move-warnings/restarted.png) ·
[Native assertions](artifacts/p07-move-warnings/native.log)
