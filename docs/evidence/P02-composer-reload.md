# P02 — Composer account changes

The composer retires Send and outbox Retry when a valid replacement declaration
is adopted. The transport records its outcome against the original outbox entry.
A retired completion can refresh the current account's lists. It cannot change
the new editor, close a replacement draft or start another submission.

The editor also fences pending draft loads, recovered drafts, attachment choices
and draft/outbox list reads. An explicitly requested local write can finish for
its owning account. An invalid configuration leaves the active composer and
its submission in place.

The private Wayland fixture uses an encrypted test keyring and a loopback TLS
SMTP server. It exercises the following cases in disposable XDG profiles.

| Transition | Observable result |
| --- | --- |
| Invalid reload during Send, then replace the account while SMTP is waiting for a greeting | The original row stays pending across the invalid edit. Replacement cancels it, and a new draft saves under the replacement account. |
| Retry a refused login and replace the account before SMTP submission | The old attempt is cancelled. The replacement account can save a separate draft. |
| Hold a SQLite write lock, request recovery of a rejected message, replace its account and release the lock | The original account gets its recovered draft. The replacement editor has none of its contents. |

All three cases passed on 2026-11-23 at `e133a5f` in the pinned x86_64 Linux
development environment. The same run delivered the positive-control message,
preserved a pending submission across restart and checked the isolated keyring.
The SMTP cancellation cases paused before DATA. [Native results](artifacts/p02-composer-reload/native.log)
· [Replacement editor after recovery](artifacts/p02-composer-reload/replacement.png).

To repeat, build the application, SMTP integration test and credentials
integration test in the pinned `gui-test` shell and run the private session.

```sh
MAILSTRONAUT_SEND_FAILURES_GUI_BINARY="$app" \
  MAILSTRONAUT_SMTP_FIXTURE="$smtp_test" \
  MAILSTRONAUT_CREDENTIAL_FIXTURE="$credentials_test" \
  bash experiments/p02-accounts/credentials.sh
```
