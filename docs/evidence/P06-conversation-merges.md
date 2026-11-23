# Mailbox selection after a conversation merge

A late reply can join two previously separate conversations. The store retains the
older conversation ID and records an alias for the other. During a mailbox refresh,
the application resolves disappeared row IDs in batches of at most 100 and within
the enabled accounts. When it applies the page, it uses these aliases to restore
the current keyboard selection and the top visible row's pixel offset. This works
when a new original represents the group or the group moves into a priority section.

## Native walkthrough

Run `experiments/p02-accounts/credentials.sh` in the pinned GUI shell with
`MAILSTRONAUT_SYNC_GUI_BINARY` pointing to the application,
`MAILSTRONAUT_SMTP_FIXTURE` pointing to its compiled SMTP test executable and
`MAILSTRONAUT_MERGE_SELECTION_GUI=1`. The fixture uses a private GNOME keyring,
headless Sway and a loopback Dovecot account.

1. Four unrelated messages appeared in the unified inbox. Arrow keys selected the
   conversation whose ID would become an alias.
2. Dovecot received a fifth message referencing both original roots. Refresh merged
   their conversations and changed the representative to the new reply. All five
   originals remained unread in the local mailbox.
3. Pressing Enter opened the new reply and marked it Seen on Dovecot. The other
   four originals remained unread. The fixture captured the native mailbox before
   the merge, after refresh and after the explicit open.

The upstream `b5078a9` binary ran the same fixture. It reached the merged, unread
state, then timed out waiting for Enter to open the new representative.
The fixed binary at `88b6970` completed the walkthrough.

The nine mailbox tests include the exact intra-row offset after a merge across
sections and rejection of a different account's row. Six conversation-store tests
include flattened aliases after successive merges and restart, and a read flag
change limited to one original. Pinned Rustfmt, strict Clippy for the application
and core targets, and the repository's ShellCheck list passed.

Mailbox removal and large-list responsiveness remain separate parts of the P06
acceptance row.
