# P04 — Automatic per-account synchronization against a real IMAP server

Status: bounded native acceptance against a disposable Dovecot; **no live provider has
been contacted, and P04 remains incomplete**. IDLE is not implemented; polling is.

## Behavior

`mail_sync::Engine` runs one bounded polling loop per configured account while the
application runs, at `sync.pollSeconds` (default 300). Ctrl+R, returning to the window
(at most once a minute) and opening a folder start a cycle early. `sync.automatic =
false` restores explicit sync in Settings only.

Each cycle discovers folders and SPECIAL-USE roles, sends queued flag changes and moves,
copies accepted mail to Sent, then pages back through recent mail in the inbox, Sent,
Drafts and Archive, stopping at a window with nothing new. It reconnects after any window
that downloaded, since one connection's receive budget is smaller than several windows.

Failures stay per account and cached mail stays readable. Network and storage errors
retry with backoff from 15 seconds to 5 minutes. Refused credentials and invalid TLS or
configuration stop that account's retries until an explicit wake, so a wrong password
cannot lock an account at the server. Folder roles come from a configuration override,
then SPECIAL-USE, then conventional English, French and Italian names.

Archive, Trash and Move to act on the conversation's copies in the current folder and
hide it at once. The server is told after an 8-second grace period, during which the
in-app Undo works. UID MOVE is preferred; without it, UID COPY plus UIDPLUS UID EXPUNGE
removes that exact UID, and a server with neither is refused rather than risking a
global EXPUNGE. A failed move shows the message again. An interrupted move rechecks the
UID on retry, so it does not move a second time.

Accepted mail is appended to Sent during sync. The Message-ID is searched first, so a
crash after APPEND leaves no duplicate. This never changes SMTP delivery state.

## Checks

`experiments/p04-sync/gui.sh`, called by `experiments/p02-accounts/credentials.sh` with
`MAILSTRONAUT_SYNC_GUI_BINARY`, runs the debug application on the CI server in a private
sway session, with a private keyring, an unprivileged Dovecot 2.4 over TLS on loopback and
the SMTP fixture. It presses nothing to start syncing, and uses the server's own maildir
as an observer independent of the client database:

- Starting the application caches the inbox and records Sent/Trash/Archive from SPECIAL-USE.
- Fetching leaves the mail unread locally and on the server (PEEK).
- An explicit open sets Seen on the server, and Star sets Flagged, with no manual sync.
- A new delivery appears through polling.
- A reply is accepted by SMTP exactly once, and its copy reaches the Sent folder.
- Archive moves the real message, and the inbox's other messages stay.
- With the server stopped, cached mail stays readable and an account warning is shown.
- A restart with the server back loses nothing. No log contains the synthetic credential.

`crates/mailstronaut-core/tests/moves.rs` covers immediate hiding, failure and retry,
stale callbacks, undo limits, copies in other folders and dropped stale contexts.

## Not established

Live providers, Gmail's label semantics, CONDSTORE/QRESYNC, servers without MOVE or
UIDPLUS, large-mailbox timing, idle CPU and fairness across several busy accounts. The
UI does not yet retry failed moves individually: they are requeued on the next cycle.
