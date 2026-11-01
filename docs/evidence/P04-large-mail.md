# P04 — Mail too large to keep, and attachments on demand

Status: native acceptance against a disposable Dovecot with synthetic mail. No live provider
has been contacted, and saving through a real desktop file chooser has not been run.

## The problem this fixes

Sync skipped any message over `sync.maxBodyBytes`, which defaults to 2 MiB and is documented as
an analysis cap. Skipped mail appeared nowhere and nothing said so: a message with a 3 MB PDF
did not exist in the application.

## Behavior

Messages up to 8 MiB, the MIME layer's limit, are kept whole. A larger one keeps its headers
and readable text, using BODYSTRUCTURE and part fetches with `BODY.PEEK`, so fetching still
cannot mark it read. Those are assembled into an ordinary small message, so the list, reader,
search, threads and replies need no special case, and what stayed behind is stored beside it
in the same transaction, so a message cannot appear without knowing it is incomplete.

The reader says how large the message is and lists its other parts as on the server. Saving
one asks where to save first, so a cancelled save downloads nothing, then fetches only that
part. The stored UID is only trusted if the folder's UIDVALIDITY still matches. One part may
be up to 64 MiB on the wire, on its own connection with its own receive budget and timeout.
File names come from the part's own MIME headers, so encoded names decode as usual.

## Checks

`experiments/p04-sync/large.sh`, called by `credentials.sh` with
`MAILSTRONAUT_LARGE_GUI_BINARY`, runs against a real Dovecot in a private sway session:

- A plain message, a 5 MiB one and a 30 MiB one (41 MiB as stored) all appear.
- The 5 MiB one is kept whole, and only the 30 MiB one records what it left behind.
- Fetching marks neither as read, and opening the large one does.
- The store holds 8 MiB after caching 35 MiB of attachments.
- The large message's text is searchable, and its attachment is listed with its decoded name.
- Downloading that part from the same server matches the SHA-256 of what was attached.

That last check runs `crates/mailstronaut-core/tests/large_part.rs`, which calls the same
connection and decoding functions as the Save action, because the action itself needs a
desktop portal that the headless session does not have.

`crates/mailstronaut-core/src/outline.rs` tests cover removing the original structure's
headers, encoded names, a message with no readable text, the store's size limit, and decoding
base64, quoted-printable and unencoded parts.

## Not established

Saving through a real file chooser, and a slow or interrupted download: a failed transfer simply
reports an error and leaves nothing behind, but this has not been exercised. Real providers'
BODYSTRUCTURE output, which is why Gmail and iCloud need checking. Inline images in a message
this large are listed as attachments rather than shown. A part over 64 MiB cannot be saved.
A download is held in memory while it is decoded, so 64 MiB costs about twice that briefly.
