# P04 — Dropped connections, retries and idle cost

## Behavior

A connection dropped in the middle of a sync window is no longer a failed cycle. The folder's
step is repeated on a new connection, up to four times per folder per cycle. Each window's cursor
is saved only after that window has committed, so repeating a step loses and duplicates nothing.

Only a lost connection is repeated: an error that `SyncFailure::from_imap` classes as retryable
(`Disconnected`, `Network`, protocol errors and the like). Authentication, TLS, configuration and
STARTTLS failures keep their stop/retry classes, and cancellation is never retried. A cycle that
still fails after its retries uses the existing backoff.

Before this change, a single reset failed the whole cycle, and the account then waited 15 seconds,
then 30, 60 and so on up to five minutes. A walkthrough with six resets therefore did not finish in
ten minutes. That is the reason for the per-folder retry.

## Checks

`experiments/p04-sync/faults.sh`, run through `credentials.sh` with `MAILSTRONAUT_FAULTS_GUI_BINARY`.
It seeds 600 synthetic messages, all within three days, so that history spans several windows. The
application talks to `experiments/p04-sync/drop-proxy.py`, a loopback proxy that resets the first
six connections once the server has sent each one 20 KB, which is past the TLS handshake and inside
a window. A reset is an abrupt TCP reset (RST), so the client sees it as a network drop.

```
OK: the history reached its beginning despite the resets
OK: 6 connections were reset mid-sync, and each message was kept exactly once
OK: the idle application used 0.00% of a processor over ten seconds
Native fault acceptance: .../faults-gui
faults=0
```

The proxy's reset log records six resets, after 20122 to 20452 bytes each. The walkthrough checks
the history count, the membership count (one per message, so no duplicates), and the processor time of
the running application, measured from `/proc` over ten seconds after the history completed.

The first run of this walkthrough, before the retry change, did not finish: six resets failed six
cycles and the backoff ran past the wait. That run is the reason the retry exists.

Unit test: `account_actions::tests::only_a_lost_connection_is_repeated_and_cancellation_never_is`.

## Not established

- Authentication failures during sync, and failure isolation between accounts, are not exercised by
  this walkthrough.
- Cancellation timing under load is not measured. The walkthrough closes an idle application only.
- The walkthrough runs in the native harness, not in the Nix or container CI, which still runs only
  the package gate.
- Live providers, and a real suspend.
