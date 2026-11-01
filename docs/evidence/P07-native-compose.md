# P07 — Native GPUI composer, autosave and explicit submission

Status: bounded current-stack implementation; **P07 and the full client remain
incomplete**. Everything below uses synthetic accounts, recipients and credentials.

## Behavior

Compose (Ctrl+N) and Drafts & outbox (Ctrl+2) open a Kit `Form`/`Field` editor: identity
`Select`, To/Subject `Input`s, Cc/Bcc shown on request (Ctrl+Shift+B), a `Textarea` body
and an `Attachment` row. Plan deviation: Cc/Bcc use `Field::visible` rather than
`Collapsible`, because a collapsible cannot sit between fields in a `Form` layout.

- Edits commit to the existing revisioned drafts after 500 ms. Switching drafts or
  leaving the view saves first. Closing the window waits briefly for the store owner,
  so the last keystroke survives. A failed save leaves the editor and its text intact.
- Choosing an identity on another account moves the content and files to a new draft
  under that account and discards the source; aliases stay on the same draft.
- Discard asks through `AlertDialog`, then closes the draft and drops its attachment
  references. The tombstone remains, so a late autosave cannot recreate the text.
- Send (or Ctrl+Enter) unlocks the keyring first, so a locked keyring leaves the draft
  editable. It then saves, freezes MIME and submits through the retained lettre path.
  Accepted mail returns to the mailbox with an in-app notification. Any other outcome
  stays in Drafts & outbox with a persistent `Alert`.
- The outbox offers Send, Cancel, Retry and Edit as new draft according to durable
  state. Nothing resubmits by itself. A possibly delivered message asks through
  `AlertDialog` before sending again.
- Reply, Reply all and Forward use the retained preparation code on the open original.
- Attach uses GPUI's portal file chooser and the existing bounded `read_files` snapshot.

## Checks

Run on the CI server through `ci/remote.sh`. `bash ci/check.sh` passes formatting, tool
lints, strict Clippy and the normal workspace tests.

`experiments/p07-compose/gui.sh`, called by `experiments/p02-accounts/credentials.sh`
with `MAILSTRONAUT_COMPOSE_GUI_BINARY` and `MAILSTRONAUT_SMTP_FIXTURE`, passes in a
private headless sway session, private bus and private unlocked keyring, against the
loopback TLS SMTP fixture. It uses real keyboard input and asserts durable state:

- Typed To/Cc/Bcc/Subject/Unicode/body fields reach SQLite through autosave.
- Closing the window right after a keystroke keeps that edit and exits normally.
- A restart lists the draft; reopening it submits nothing.
- Ctrl+Enter produces exactly one SMTP DATA, phase `accepted`, no open draft.
- The received original has the visible recipients, no Bcc header and no blind address.
- The synthetic credential appears in no application or server log.
- A further restart still shows the accepted entry and offers no resend.

## Not established

No live provider or real desktop portal. The GUI path has not yet exercised: attachments
through a portal, failed or unknown delivery with its resend dialog, discard, identity
switching, or Reply/Forward. Backend tests cover several of those states. There is no
signature field yet, and Sent APPEND is not implemented, so the Sent-copy state stays
pending for non-Google accounts.
