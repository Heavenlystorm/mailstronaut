# P03 — Mailbox queries at 100,000 messages

Status: measured in a release build on the 16-CPU CI server, over synthetic rows. Each figure
is the median of five warm runs, with two accounts of 50,000 messages each, three messages
per conversation, one in ten unread and one in five archived. Real mail, cold storage and a
laptop's disk have not been measured.

`scale_measurement` in `crates/mailstronaut-core/src/store/scale_tests.rs` inserts rows
directly, because parsing and fsyncing 100,000 messages one at a time would measure something
else. To repeat it: `cargo test --release -p mailstronaut-core --lib scale_measurement --
--ignored --nocapture`.

| Query | Before | After |
| --- | --- | --- |
| First page of the unified inbox | 508 ms | 4.9 ms |
| A deep page | 443 ms | 7.0 ms |
| First page of one folder | 282 ms | 19.9 ms |
| Page of 1,000 search hits | 2,562 ms | 72.8 ms |
| Literal search (now only the fallback) | 534 ms | 520 ms |
| Unread counts | 38 ms | 36 ms |
| Priority group | 27 ms | 27 ms |
| Analysis queue | 27 ms | 27 ms |

Grouping one 50,000-message account takes about 50 ms to read, 260 ms to group and 200 ms to
commit. It runs only when mail has arrived.

## What changed

Every page used to rank a whole mailbox with window functions, which was the cost above and
would have made searching as you type unusable. A page with no search now walks messages
newest first and stops once it is full. A page of hits starts from the hits and their
conversations. `page_without_a_search_agrees_with_the_complete_query` compares the new browse
query with the old one, field by field and page by page, across randomized mailboxes with
conversations, duplicate dates, copies in several folders, pending flags, pending moves and
stale projections, in every scope.

This also found that an account with more than 50,000 messages made the list fail entirely,
because grouping refused it and that error reached the list. It now lists and searches that
account's mail ungrouped.

## Not established

Grouping above 50,000 messages per account: past that, conversations appear as separate rows.
A small folder in a huge mailbox walks past everything newer than its mail, so its cost
depends on how old that mail is. Unread counts run on every list refresh, so 36 ms is paid
each time. Worst cases for pending operations have not been measured.
