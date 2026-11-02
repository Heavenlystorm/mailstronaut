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

These checks do not establish live-provider authentication or sending, full Home
Manager activation, Flatpak acceptance, multi-account progress coverage, or a
continuous thread-original list. The history cap limits fetching; it does not evict
cached mail.
