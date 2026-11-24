# P07 — Message row context actions

Right-clicking a mailbox or priority row offers Open, Mark read or unread, Star
or Unstar, Archive and Trash. Opening the menu, dismissing it and changing a
star leave the message's read state alone. On a grouped row, Open and the flag
commands target the original represented by the row. Archive and Trash use
the conversation move queue and its existing Undo.

The menu captures the account and message identity of the clicked row. Before
acting, it requires the same configuration revision and folder scope, and finds
the message in the current list. Flag and move actions share the reader
toolbar's storage and synchronization paths.

The full-capability variant of `experiments/p07-compose/actions.sh` opens
“Synthetic delta” and uses a context menu on unread “Synthetic charlie”. It
checks the affected message in SQLite, reads the still-open original through
the native clipboard, undoes the move and checks the server's flags.

This workflow passed against private Dovecot and native Wayland on 2026-11-23
at `9af3619`, including changing Charlie to read and back to unread.
[Context menu](artifacts/p07-row-actions/menu.png) · [Fixture results](artifacts/p07-row-actions/native.log).
