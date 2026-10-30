# P06 — Seen requires an explicit message-open action

This policy supersedes the earlier focus/dwell prototype in [P06-read](P06-read.md),
as requested by the user. No dwell timer remains.

Only the message row/member selector's explicit open action authorizes Seen. Its
exact configuration/reading generation must still match when readable content and
member metadata arrive. An authorization is consumed at most once, marks only the
selected message's Inbox copies, and is cancelled on hiding/leaving the reader.
Listing/preview loading, restored selection, page entry, focus return and refresh
never grant an open authorization. Explicit unread/undo cancels any authorization;
another explicit open is required to mark that message read again. Keyboard
activation of the native open control is equivalent to clicking it. Server-reported
flags continue to synchronize normally; local flag application remains explicit.

Pinned `nix develop` formatting, strict workspace Clippy and **126 workspace tests
plus seven isolated renderer children passed**, followed by the native build.
[Development log](artifacts/p06-explicit-open/development-check.log). Gate tests
verify that readiness/visibility alone cannot consume Seen, stale results cannot
consume another open, and page/focus return or explicit unread require another click.

The private native X11/Xvfb fixture passed: initial listing and page return queued
no flags; clicking one message loaded/copied its content and queued Seen only for
it. Mark unread remained unread across leaving/returning to Inbox and transferring
focus to/from another private window. Clicking it again queued Seen. Opening a
conversation member left its sibling unchanged; passive viewing and offline restart
preserved explicit unread. Sent-only ancestors stayed read-only and both receiving
accounts prepared correct local replies. No mail was sent. The browsing/search
regression and private keyring checks passed as well.
[Native log](artifacts/p06-explicit-open/native-gui.log),
[browsing regression](artifacts/p06-explicit-open/browse-regression.log).

- [Explicit click](artifacts/p06-explicit-open/explicit-click-read.png)
- [Unread after page/focus return](artifacts/p06-explicit-open/passive-return-unread.png)
- [Explicit unread](artifacts/p06-explicit-open/explicit-unread.png)
- [Offline restart](artifacts/p06-explicit-open/offline-inbox.png)

This is local development/native evidence, not a new release, CI, Flatpak, live
provider or COSMIC/Wayland/screen-reader acceptance. The ongoing reminder work is
separate. Other P06, privacy and distribution requirements remain open.
