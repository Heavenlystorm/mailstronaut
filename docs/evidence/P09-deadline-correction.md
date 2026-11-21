# P09 deadline correction

The reader lets the user correct a detected or uncertain deadline on the opened
original. The GPUI Kit date picker starts with that original's evidence. A
date-only correction includes the entire local day. A timed correction preserves
the time field and rejects ambiguous or nonexistent civil times. The selected
timezone and exact expiration are stored independently from automatic analysis.

Schema 16 stores the correction for its canonical original. If priority is
active at submission, the corrected date controls when it ends. Corrections
cannot put read, dismissed, expired or ordinary originals in priority. They
leave Seen and reminders alone. Reanalysis and clearing rebuildable indexes
preserve corrections. Clearing an account's cache also retains originals with
corrections, as it does for mail with reminders or pending actions.

The pinned x86_64 development shell passed formatting, strict Clippy and
330 workspace tests with 27 ignored. The store tests check the exclusive end of
a DST day, rejection of ambiguous or nonexistent explicit times, account
isolation, reanalysis, cache clearing and priority eligibility. The migration
fixtures also pass.

The private native Wayland walkthrough used synthetic TLS mail and a disposable
keyring. It cancelled an edit, changed the ambiguous date on one original to
15 April 2030, asserted the exact Europe/Paris expiration, and reopened the app
and original to verify the saved date. It also edited 10:30 to 11:42 on a
separate timed deadline and checked the exact UTC instant. The same run passed
the existing priority
dismissal, explicit Seen, expiry, new delivery and Show all checks. Screenshots
are under the ignored `target/p02.LoTj26/priority-gui` and the restarted reader
was shown in chat.

[Workspace checks](artifacts/p09-deadline-correction/checks.log) ·
[Native walkthrough](artifacts/p09-deadline-correction/wayland.log)

Timed correction DST handling is covered by storage tests. Contextual inference quality and the on-device model
selection remain separate open tasks. The refreshed Flatpak input archives
preserve the locked vendor input. SDK and installed Flatpak acceptance are open.
