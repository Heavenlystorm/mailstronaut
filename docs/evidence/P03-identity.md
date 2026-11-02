# P03 — Account, folder, UIDVALIDITY and UID identity

Local store tests, run in CI with `ci/check.sh`. Each clause of the PLAN row has a named test:

| Clause | Test (`crates/mailstronaut-core/tests/mailbox.rs` unless noted) |
| --- | --- |
| Account/folder identity: canonical copies stay account-local, and different bodies under one Message-ID stay distinct | `canonical_copies_remain_account_local_and_distinct_bodies_stay_distinct` |
| Multi-membership: one message in INBOX and Archive keeps one row with two memberships | `canonical_copies_remain_account_local_and_distinct_bodies_stay_distinct` |
| Provider deduplication: Gmail IDs win over header duplicates and share label copies | `gmail_ids_win_over_header_duplicates_and_share_label_copies` |
| UID identity: conflicting UIDs and stale generations never retarget a cached message | `uid_conflicts_and_stale_generations_do_not_retarget_cached_messages` |
| UIDVALIDITY: a reset retires the old namespace's memberships and reattaches the same message once | `a_uid_reset_reattaches_the_same_message_once_and_forgets_the_old_namespace` |
| Never merge different accounts | `canonical_copies_remain_account_local_and_distinct_bodies_stay_distinct` |

The UIDVALIDITY case is also exercised against a real Dovecot server: `experiments/p04-sync/history.sh`
removes the server's UID list and index, restarts it, and checks that the inbox still holds each
message once (see [complete history](P04-history.md)).

Checks: `ci/check.sh` passed on the commit that adds the reset test, and the full native regression
passed on the same tree.
