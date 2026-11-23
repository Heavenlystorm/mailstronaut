# Mailbox projections and index progress

In a private native Wayland session, the fixture added 32,768 synthetic
deletions to the durable lexical index queue. It issued the misspelled query
`unrelatd` while old entries were being removed and the actual original
was still waiting for indexing. The mailbox showed a spinner, its pending
count, and an incomplete-results message. Once the queue drained, the result
appeared without further input. The fixture clicked it and obtained the exact
cached body, “Synthetic unrelated same-subject message.” Only that explicit
open set Seen.

Run `experiments/p06-mailbox/wayland.sh` with `MAILSTRONAUT_INDEX_PROGRESS_GUI=1`
in the pinned `gui-test` shell, using the standard prebuilt app and worker
fixture. [Pending query](artifacts/p03-index-progress/pending.png) and
[completed query](artifacts/p03-index-progress/complete.png) show the same input.
The existing [native rebuild and lexical search walkthrough](P06-fuzzy-search.md)
and [history coverage fixture](P04-history.md#scoped-cache-coverage) cover
removed indexes and interrupted or capped server history.

On 2026-11-23, `b98dadf` passed the native run and the 100,000-message
measurement in the pinned x86_64 Linux development shell. Two accounts each
contained 50,000 synthetic messages. Five warm measurements gave these medians.

| Operation | Milliseconds |
| --- | ---: |
| First unified inbox page | 8.3 |
| Deep unified inbox page | 9.2 |
| First folder page | 27.7 |
| Ranked page from 1,000 hits | 112.4 |
| SQLite literal search fallback | 911.0 |

Run `cargo test -p mailstronaut-core --lib scale_measurement -- --ignored --nocapture`
for the scale fixture. [Release-profile timings and randomized page comparison](P03-scale.md)
are recorded separately. The 100,000-message fixture times the backend; the
native UI fixture exercises queue progress and opening the matching original.
