# P04 — Local state across moves and reconciliation, and Seen

## Behavior

A reminder, a priority dismissal and an analysis result belong to the message, not to its
folder location. A move changes where the message is held on the server, and the cache keeps one
message row for its copies. Reconciling the source folder removes only that folder's membership.

Background fetching, previews and analysis never set `\Seen`.

## Checks

- Store test `a_reminder_a_dismissal_and_the_analysis_follow_the_message_through_a_move`
  (`crates/mailstronaut-core/tests/moves.rs`): a message gets an analysis result, a reminder and a
  dismissal. It is then moved, its copy in Archive is cached and matched to the same message, and the
  source folder is reconciled. The reminder is still held, the analysis is still stored, and the
  dismissal is not undone.
- Native, fetching: `experiments/p04-sync/history.sh` checks that fetching the complete history
  changed no Seen flag, here or on the server.
- Native, analysis: `experiments/p09-priority/gui.sh` checks that analysis set no Seen flag.
- Native, previews and opening: `experiments/p06-mailbox/navigation.sh` checks that the Seen state
  matches the messages the user explicitly opened.

## Not established

- A move made by another client while the application runs, on a real server, followed by
  reconciliation. The store test covers the sequence the application performs, not the server
  producing it.
