# P04 — Automatic per-account synchronization against a real IMAP server

Status: bounded native acceptance against a disposable Dovecot; **no live provider has
been contacted, and P04 remains incomplete**.

## Behavior

`mail_sync::Engine` runs one bounded polling loop per configured account while the
application runs, at `sync.pollSeconds` (default 300). Ctrl+R, returning to the window
(at most once a minute) and opening a folder start a cycle early. `sync.automatic =
false` restores explicit sync in Settings only.

After a good cycle, each account also keeps one IDLE connection on its inbox. It uses
EXAMINE, so it cannot change anything, and it reads no mail: a notification only wakes the
ordinary cycle, which stays the one path that changes the cache. So polling still covers
the other folders and any server without IDLE. IDLE is renewed every 9 minutes on its own
deadline, because server keepalives reset the library's timeout. Each reselect compares
UIDVALIDITY, UIDNEXT and EXISTS, which catches a change between two periods. A failed
connection reconnects with the same backoff and always asks for one cycle, since it may
have missed anything. Refused credentials end the watcher until a later good cycle.

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
- A new delivery appears through polling. This server is configured without IDLE, and the
  script confirms that first, so this step is the proof of the fallback.
- A reply is accepted by SMTP exactly once, and its copy reaches the Sent folder.
- Archive moves the real message, and the inbox's other messages stay.
- With the server stopped, cached mail stays readable and an account warning is shown.
- A restart with the server back loses nothing. No log contains the synthetic credential.
- A flag set by another client on the archived copy comes back by polling.

`experiments/p04-sync/idle.sh`, called the same way with `MAILSTRONAUT_IDLE_GUI_BINARY`,
sets `sync.pollSeconds = 3600`, so a change seen within seconds can only have come
through IDLE. Against the same real server, with a second IMAP client making the changes:

- A new delivery appeared within 2 seconds, and the fetch left it unread on the server.
- Seen set and a message expunged by the other client each arrived within 2 seconds.
- With the server stopped while a message arrived and then restarted, reconnecting found
  that message within 12 seconds, and IDLE then worked again on the new connection.
- The application used 0, 5 and 6 CPU ticks (at 100 per second) over 20 quiet seconds, in
  three runs.
- A second client creates `&AMk-tudes` and `Projets/&ZeVnLA-`. The sidebar shows `Études`,
  and clicking it fetches its two messages through the exact wire name. The sidebar shows
  unread counts on the Unified inbox and each folder, but not on Sent, Drafts, Trash or Junk.

Unit tests cover the folder name decoder and the unread query. A pending read counts as
read, a message being moved away stops counting, and an old UIDVALIDITY or another account
does not count. `utf7-imap` was rejected for the decoder because it panics on invalid
names, which the server controls. Invalid names, encoded ASCII, control characters and
direction overrides all display as the raw wire name.

`crates/mailstronaut-mail/tests/imap_idle.rs` runs against a scripted TLS server. The watch
sends only EXAMINE, IDLE and DONE. Keepalives do not postpone renewal or count as changes,
and IDLE can be reissued on the same session. A closed connection, cancellation and a peer
that stops answering DONE (what a suspend or a dropped NAT mapping looks like) each retire
the session within their bound.

`crates/mailstronaut-core/tests/moves.rs` covers immediate hiding, failure and retry,
stale callbacks, undo limits, copies in other folders and dropped stale contexts.

## Not established

Live providers, Gmail's label semantics, CONDSTORE/QRESYNC, servers without MOVE or
UIDPLUS, large-mailbox timing and fairness across several busy accounts. A real suspend
and resume has not been run, and a 9-minute renewal has only run at shorter intervals
against the scripted server. The CPU figure comes from one short run in a
software-rendered session, so it is not a budget. The UI does not yet retry failed moves
individually: they are requeued on the next cycle. `Projets/日本` decoded but drew as empty
boxes because the test session has no CJK font, so font fallback on a real desktop is
unverified. The unread query has not been timed at 100k messages.
