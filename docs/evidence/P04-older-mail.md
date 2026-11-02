# P04 — Fetching older mail from the mailbox

Status: native acceptance against a disposable Dovecot with synthetic mail. No live provider
has been contacted.

## Behavior

> Superseded: automatic sync now fetches the complete history by default, and a declared
> `sync.initialDays` is only a cap. See [complete history](P04-history.md). This record
> describes the earlier 30-day default.

Automatic sync downloads `sync.initialDays` of mail, 30 by default. Older mail could only be
fetched one page at a time from an account's Settings. Now, at the end of what is cached in
the unified inbox, an account or a folder, **Fetch older mail** asks the server for more. Each
press downloads up to 50 messages, or looks through at most 64 windows of 128 UIDs, and says
what it found. Once a folder is fetched back to its beginning, the button goes away.

It walks back through the whole folder rather than starting below the oldest cached UID,
because UID order is not date order in mail that has been moved or imported. How far it has
got is kept only while the application runs, so after a restart it walks past what is
already cached, which refreshes those flags and downloads nothing twice. Fetching uses
`BODY.PEEK` like all sync, so it marks nothing read.

## Checks

`experiments/p04-sync/older.sh`, called by `credentials.sh` with
`MAILSTRONAUT_OLDER_GUI_BINARY`, runs against a real Dovecot in a private sway session:

- Automatic sync takes only the two recent messages, with two older ones on the server.
- One of those older ones has a higher UID than a recent one, and the button still finds both.
- Nothing is marked read by fetching.
- With a folder's beginning reached, the button is gone.
- Archive, which has nothing cached, fetches its own 200-day-old message.
- Asking again after a restart duplicates nothing.

## Not established

Live providers, and timing on a large folder over a slow link. Each press holds a connection
until it finishes, and there is no Cancel. Flags on older mail are only refreshed when it
is walked past again: ordinary sync cycles look at recent mail, so another client's change to
a years-old message may not show until then.
