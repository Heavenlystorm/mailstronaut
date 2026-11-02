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

A second account points at a port with nothing listening. Its failures run beside the first account's
sync, and the walkthrough checks that the first account still completes.

```
OK: the history reached its beginning despite the resets
OK: 6 connections were reset mid-sync, and each message was kept exactly once
OK: an unreachable account did not hold back the other one
OK: a wrong password was tried 1 time(s) and then left alone
OK: the idle application used 0.00% of a processor over ten seconds
OK: the first sync began a history pass
OK: closing took 109 ms with 1 unfinished history passes
Native fault acceptance: .../faults-gui
faults=0
```

The closing check starts a fresh profile, waits until its first history pass has begun, then closes the
window. It measures the time from the close request until the process exits.

The proxy's reset log records six resets, after 20122 to 20452 bytes each. The walkthrough checks
the history count, the membership count (one per message, so no duplicates), and the processor time of
the running application, measured from `/proc` over ten seconds after the history completed.

The first run of this walkthrough, before the retry change, did not finish: six resets failed six
cycles and the backoff ran past the wait. That run is the reason the retry exists.

Unit test: `account_actions::tests::only_a_lost_connection_is_repeated_and_cancellation_never_is`.

## Not established

- A wrong password is measured by connection count: a third account with a wrong keyring secret is
  tried once, and the account then waits for a change instead of retrying. Other authentication
  failures, such as an expired OAuth token or a locked keyring during sync, are not exercised.
- Cancellation is measured for one close, mid-pass, on a small mailbox. Closing under sustained
  backfill of a large mailbox is not measured.
- The walkthrough runs in the native harness, not in the Nix or container CI, which still runs only
  the package gate.
- Live providers, and a real suspend.
