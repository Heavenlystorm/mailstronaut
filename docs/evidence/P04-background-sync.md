# P04 — Automatic per-account synchronization

Status: implemented and unit-tested; **native acceptance against a real IMAP server is
still open**. No live provider has been contacted.

`mail_sync::Engine` runs one bounded polling loop per configured account while the
application runs, at `sync.pollSeconds` (default 300). Ctrl+R, returning to the window
(at most once a minute) and opening a folder start a cycle early. `sync.automatic =
false` restores explicit sync in Settings only.

Each cycle discovers folders and SPECIAL-USE roles, sends queued flag changes, then pages
back through recent mail in the inbox, Sent, Drafts and Archive, stopping at a window
with nothing new. It reconnects after any window that downloaded, since one connection's
receive budget is smaller than several windows.

Failures stay per account and cached mail stays readable. Network and storage errors
retry with backoff from 15 seconds to 5 minutes. Refused credentials and invalid TLS or
configuration stop that account's retries until an explicit wake, so a wrong password
cannot lock an account at the server.

Folder roles come from a configuration override, then SPECIAL-USE, then conventional
English, French and Italian names. `folder_roles` has unit tests for that precedence.
