# Continuous mailbox and settings evidence

On 2026-11-02, the checkout based on `cfdccef` passed the coordinated
`ci/nix-check.sh` gate on `ci-py`. The earlier `ce3a25e` base passed the pinned
workspace `ci/check.sh`. The Nix gate passed release Clippy, workspace tests and the disposable
Home Manager module check; aarch64-linux was omitted.

On the earlier `8c37fea` base, private synthetic Wayland walkthroughs passed continuous mailbox browsing across
100 rows, passive keyboard navigation, refresh anchoring, search and folder changes,
and explicit-open-only Seen. The account settings walkthrough saved separate incoming
and SMTP logins in an app-owned native Nix file. A walkthrough on the `ce3a25e` base saved a
30-day history limit and cleared it back to unlimited, preserving Seen state.
The native navigation walkthrough passed folder, thread, offline, action and reminder
checks. A synthetic Dovecot history run showed actual folder progress and passed
resume, age-cap, UIDVALIDITY and Seen checks on the earlier `8c37fea` base.

On the `5f71a6c` base, the thread reader's pinned workspace checks passed. A
private Wayland fixture with 108 originals crossed the 100-original boundary by
keyboard. Its store showed that passive selection left all but the explicitly
opened newest original unread; confirming an older original changed only that
original's Seen state. A worker-backed synthetic regression checked that a new
arrival does not remove the already loaded boundary on refresh. These checks do
not yet cover mouse-wheel loading, very large threads, or every stale-query race.
The x86_64 Nix package gate passed on that base, and passed again after rebasing
onto `ec181c8`, reusing the application derivation while rerunning changed flake
checks. Aarch64, hosted CI, and installed-package GUI acceptance were not tested.
After a further rebase onto `9d8773b`, the full x86_64 Nix package gate passed
again, including release tests and the Home Manager module check.

These checks do not establish live-provider authentication or sending, full Home
Manager activation, Flatpak acceptance, or multi-account progress coverage. The
history cap limits fetching; it does not evict
cached mail.
