# Upstream threading conformance fixtures

Unmodified fixtures and coverage matrix from `mail-threading` 0.1.3, as distributed
in its checksummed crates.io archive (Cargo.lock). Upstream:
https://github.com/planetaryescape/mail-threading . The original MIT and Apache-2.0
licenses are preserved alongside the corpus.

These synthetic fixtures test the pinned component with each case's declared
options. Some exercise subject merging or phantom pruning which Mailstronaut
intentionally disables. Separate application-policy tests cover those choices,
account isolation, stable conversation IDs and bounded subprocess behavior. Passing
this corpus does not establish MIME, provider, protocol or GUI conformance.
