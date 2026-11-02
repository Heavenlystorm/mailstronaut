# P04 — Complete history by default, with durable backfill

## Behavior

Each folder that sync covers (INBOX, the Sent, Drafts, Archive and Trash roles, and any
folder the user opened) is fetched to its beginning, newest UID window first, without an
age limit. The configuration's `sync.initialDays` decides only how far back a fetch goes:

- absent or `null`: the complete history of every synchronized folder;
- a number from 1 to 3650: the earlier cap, kept exactly for configurations that declared it.

Declaring or changing a cap never deletes cached mail. Nothing is evicted, and drafts,
outbox records, pending operations and locally owned attachments are never touched.

Every cycle, for each folder in turn:

1. Arrivals and flag changes first: the newest two UID windows are always examined, and
   older ones only while they still bring new mail (at most eight).
2. Then history: at most sixteen windows per folder. A folder with more history continues in
   the next cycle, which starts after 30 seconds rather than the poll interval.

A window that downloads nothing does not end a pass: cached messages and UID gaps are
normal. A pass finishes only at UID 1.

Originals are retrieved with `BODY.PEEK`, and the server's flags are recorded, never set.

## Schema and fencing

`SCHEMA_VERSION` is 14. The new `backfill` table has one row per account and folder:

| Column | Meaning |
| --- | --- |
| `uidvalidity` | UIDVALIDITY of the namespace the cursor belongs to |
| `context` | SHA-256 of the authenticated mailbox context (32 bytes, checked) |
| `cap_days` | the age cap the pass started under, NULL for the complete history |
| `next_before` | exclusive UID bound of the next window; NULL once UID 1 is reviewed |
| `total` | messages present when the pass began (EXISTS), NULL if unreported |
| `reviewed` | messages reviewed by the pass so far, downloaded or not |
| `finished_at` | Unix seconds when the pass reached UID 1 |

A stored cursor is used only when its UIDVALIDITY, context and cap all match the current
selection. Otherwise the pass starts again from UIDNEXT, so a UID reset, a different account
or a changed limit never trusts old UIDs. A cursor is saved after its window's originals
have committed, so an interrupted window repeats safely.

A finished pass rests for six hours (`REWALK_SECONDS`) and then starts again, so flags on
old mail and gaps are reconciled without rescanning the history on every poll.

## Progress contract

After each window, the cycle sends
`mail_sync::Update::Progress(account_id, Fetch { folder, completed, total })`, where:

- `account_id` is the configured account ID, as in `Update::Started`;
- `folder` is the server's wire name, as stored;
- `completed` is the number of messages reviewed by the current pass;
- `total` is the EXISTS count at the start of the pass, or `None` when SELECT did not say.

The send is `try_send`: a full channel drops that report, and the next window reports again.
`Cycle.backfill_pending` is true while any folder still has history to fetch. The status bar is
not wired to these updates yet; `Workspace::sync_update` ignores them for now.

## Checks

Unit and store tests:

- `mailstronaut-config`: an absent or null limit is the complete history, an explicit 30 is
  kept, and only a declared limit is written out.
- `mailstronaut-core` `sync::history_tests`: a partial pass resumes exactly at its cursor; a
  new folder starts at UIDNEXT with a total; a UID reset never resumes the old namespace's
  cursor; a changed context or cap starts again; a finished pass rests until the rewalk
  interval has passed.
- `mailstronaut-core` `store::backfill`: a pass survives reopening and each save replaces
  the last; a pass belongs to one account and folder, and needs a real folder.
- `mailstronaut` `native`: `sync.initialDays = null` evaluates through the Nix evaluator
  to the complete history, `= 30` to a 30-day cap, and `= 0` is rejected.
- `mailstronaut` `mail_sync`: history continues at 30 seconds, or the poll if that is shorter,
  and everything else waits for the poll.
- `configuration_reconcile` now expects the optional `Some(7)` for a declared limit.
- The migration fixture that rewinds to schema six drops `backfill` as well.

Native, on a real Dovecot server (`experiments/p04-sync/history.sh`, run through
`credentials.sh` with `MAILSTRONAUT_HISTORY_GUI_BINARY`). The walkthrough seeds mail older
than any cap (400 and 200 days, one already read, and 90 days), two days' mail, and 2300
recent messages, which span far more than one cycle's windows:

```
OK: the first windows of history were reviewed and their cursor saved
OK: the application was killed while history was still being fetched
OK: the interrupted pass finished
OK: the complete history is cached, mail from over a year ago included
OK: mail older than 30 days was fetched by default
OK: resuming reviewed each message once, neither repeating nor skipping any
OK: fetching changed no Seen flag, here or on the server
OK: the cap fetched only the recent mail
OK: the capped pass finished
OK: an explicit age cap limits fetching and is recorded with the pass
OK: the capped pass stays finished after a restart
OK: a restart under the same cap changed nothing
OK: a UIDVALIDITY reset re-attached every message once, none duplicated
Native history acceptance: .../history-gui
history=0
```

The last check removes the server's UID list and index files and restarts Dovecot, which gives the
mailbox a new UIDVALIDITY. The pass restarts under the new namespace and finishes, and the inbox
still holds each message once. The core test `a_uid_reset_reattaches_the_same_message_once_and_forgets_the_old_namespace`
covers the same rule at the store level.

The walkthrough kills the application with SIGKILL mid-pass, so the durable cursor is all that
survives. The resumed pass ends with `reviewed` equal to the message count, which means no
window was reviewed twice and none was skipped.

Screenshots (ignored target directory): `p04-history-complete.png` shows the cached inbox
after the complete history, with the status bar reading "100 cached conversations · page 1";
`p04-history-capped.png` shows the capped profile.

Existing walkthroughs: `older.sh` pins `sync.initialDays = 30` and therefore exercises the
compatibility path. See the regression record below.

## Not established

- The Settings and Home Manager controls for the age limit, and the status-bar `Progress`
  that consumes `Update::Progress`. The backend contract exists; the UI is not wired.
- Retention eviction. No cached mail is ever removed by this change, and the PLAN row stays open.
- An explicit Cancel control for a running pass. Cancellation is the cycle's token, and
  the native walkthrough tests an abrupt kill instead.
- Cost of the six-hour rewalk on very large mailboxes, and any live provider.
- Flags on old mail outside the newest windows refresh only while a pass walks past them; a
  finished pass refreshes them at the next rewalk.
