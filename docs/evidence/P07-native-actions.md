# P07 — Undo, and moving mail on servers that lack MOVE or UIDPLUS

Status: native acceptance against a disposable Dovecot with synthetic mail, with real input in
a private sway session. No live provider has been contacted.

## What this found

Testing these paths through the real interface for the first time found three bugs that no
test had caught:

- **Undo often failed.** The 8-second window only delayed the wake-up that told the account to
  apply the move, and any sync pass during it took the move anyway. With IDLE, that was the
  usual case: opening a message sets Seen, the server reports it, and a pass starts within a
  second. The hold is now stored with the move, so no pass can claim it early.
- **One refused move stopped an account's mail.** A failed move aborted the whole pass, and the
  next pass retried it. On a server that cannot move safely, one Archive stopped new mail from
  arriving at all. A failed move now lets the pass continue, and one that can never work
  is given up on and reported once.
- **The message stayed hidden after a failure.** A failed pass did not refresh the list, so a
  message that had come back stayed invisible until something else refreshed it.

## Checks

`experiments/p07-compose/actions.sh`, called by `credentials.sh` with
`MAILSTRONAUT_ACTIONS_GUI_BINARY`, runs once per server. Each run first confirms what that
server really advertises, using Dovecot's own `imap_capability` setting.

With **MOVE and UIDPLUS**:

- Archive hides the message at once, right after opening it, which is the case that used to fail.
- Undo brings it back, and past the grace period the server still has it in the inbox.
- Without Undo, UID MOVE reaches the server and the other messages stay where they were.
- Trash reaches its own folder.

With **UIDPLUS only**:

- COPY and UID EXPUNGE move the message.
- Another client's message, already marked for deletion, is left in place with its flag. A
  plain EXPUNGE would have destroyed it, which is why this path exists.

With **neither**:

- The server's mail is not touched, the message comes back to the list, and a warning explains why.
- The move is not tried again.
- New mail still arrives afterwards.

`crates/mailstronaut-core/tests/moves.rs` covers the stored hold and giving up on a move.

## Not established

Live providers. A move that fails for a passing reason keeps its warning and is retried on each
pass, which has only been exercised through its unit paths, not on a real server refusing and
then recovering. Move to from the menu has not been driven through the GUI.
