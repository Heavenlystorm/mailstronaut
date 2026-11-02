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

## Signature

`accounts[].signature` in the native configuration is plain text without the delimiter, up
to 4 KiB, and may span lines. New messages start with the cursor above it, and replies and
forwards place it above the quote. It is ordinary editable body text, not a separate field.
Moving a draft to another account swaps an untouched signature and leaves an edited one.

`experiments/p07-compose/gui.sh` types above a configured signature with real input, and
checks that the received mail decodes to `-- ` plus the signature. Unit tests in
`crates/mailstronaut-core/tests/reply.rs` cover placement, swapping and the size limit. The
account swap has not been driven through the GUI.

## Attaching

`experiments/p07-compose/gui.sh` clicks Attach with a protocol-accurate fake file chooser
answering on the private bus (`crates/mailstronaut/src/portal_fixture.rs`), chooses a file with
a Unicode name, and deletes the original straight away. The received mail still carries the
file with its name and contents intact, because attaching takes a private copy.

## When a send does not simply succeed

Before starting a mail transaction, submission connects, signs in and leaves. A failure
there proves nothing was sent, which a failure during submission cannot, because lettre
reports a refused connection and a lost DATA acknowledgment the same way. It costs one extra
connection per send. Without it, sending while offline warned that the mail might already
have been delivered.

`experiments/p07-compose/delivery.sh`, called by `credentials.sh` with
`MAILSTRONAUT_DELIVERY_GUI_BINARY`, takes one message through each case in a private sway
session, with real input and the SMTP fixture on a fixed port:

- No server listening: "Not sent", with Retry and no duplicate warning.
- 451 after DATA: still "Not sent", with Retry and no duplicate warning.
- Connection dropped after DATA: "Delivery unknown" with one warning and "Send again…".
- With a server ready to accept, neither waiting nor a restart sent anything.
- "Send again…" opens a confirmation, and dismissing it sends nothing.
- Confirming while the server is unreachable keeps the warning, because the earlier attempt
  may still have delivered.
- Confirming with the server back is accepted.
- All three attempts that reached DATA carried byte-identical mail, so the Message-ID is the same.

After a refused retry, the outbox shows a banner and its longer warning pushes "Send again…"
lower in the row, so the walkthrough aims the final confirmation at that position. On the
`71c5bcc` base, `delivery.sh` passes, and the full native walkthrough set (sync, IDLE, search,
priority, compose, delivery, older mail, history, actions, large mail, notifications and
navigation) exits 0 in private headless sway sessions with synthetic mail.

`experiments/p07-compose/failures.sh` covers four more ways a send can stop, each with a fresh
profile database. A server whose certificate cannot be verified receives no data and leaves the
mail "Not sent". A refused sign-in also leaves it "Not sent", and Retry in the outbox sends the
same message once the server accepts the sign-in. A send cancelled while the server stays silent
ends "cancelled" without any data. A send stopped before its submission begins stays "Waiting to
send" across a crash. Restarting neither sends it nor contacts the server, and its Cancel in the
outbox leaves it cancelled.

`experiments/p07-compose/sent-copy.sh` runs against Dovecot. The server is down when the message
is accepted, so the Sent copy stays pending. A sync while IMAP is still down neither saves that
copy nor submits the message again, and the SMTP fixture receives no second data. After IMAP
returns, the next sync saves exactly one copy. Dovecot keeps maildir files with LF line endings
and records the CRLF size in the file name (`W=`), so the walkthrough compares the saved copy
with the submitted message after normalizing line endings, and requires `W=` to equal the
submitted size.

Both pass on the CI server in private headless sway sessions with synthetic mail, through the
`MAILSTRONAUT_SEND_FAILURES_GUI_BINARY` and `MAILSTRONAUT_SENT_COPY_GUI_BINARY` switches of
`credentials.sh`.

`experiments/p07-compose/discard.sh` checks discarding a draft from the composer. The discard
button asks first, and Cancel keeps the draft open. Confirming closes it as a tombstone, and a
late autosave does not recreate its text. After a restart it still does not come back. It passes
through the `MAILSTRONAUT_DISCARD_GUI_BINARY` switch of `credentials.sh`.

`experiments/p07-compose/attachments.sh` attaches a file through the private file chooser, then
removes it from the composer. The removal drops the draft's reference, the draft stays open, a
late autosave does not restore the attachment, and it stays removed after a restart. The remove
control appears only while its card is hovered, so the walkthrough hovers the card first. It
passes through the `MAILSTRONAUT_ATTACH_GUI_BINARY` switch.

`experiments/p07-compose/identity.sh` starts a draft under one of two synthetic accounts, then
chooses the other account in the From selector. The draft moves with its subject and text, and it
stays under that account after a restart with exactly one open draft. It passes through the
`MAILSTRONAUT_IDENTITY_GUI_BINARY` switch.

`experiments/p07-compose/forward.sh` opens a synthetic message from a private Dovecot inbox and
forwards it. The local draft has a `Fwd:` subject, quotes the original as text, carries no reply
headers, and submits nothing. It passes through the `MAILSTRONAUT_FORWARD_GUI_BINARY` switch.

`crates/mailstronaut-mail/tests/smtp.rs` checks that the pre-flight starts no transaction
and reports a closed port and a refused sign-in as sending nothing.

## Not established

No live provider, live account, or Flatpak portal. Attachments come from the private portal
fixture, not a desktop file chooser. Provider-side Sent saving is not exercised. Reply is
exercised by the navigation walkthrough, and backend tests cover several other states.
A connection lost between the pre-flight and DATA is still reported as unknown, the safe
direction.
