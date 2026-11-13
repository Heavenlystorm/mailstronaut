# P06: canonical semantic ingestion frontier

2026-11-13. Backend ingestion support only. There is not yet a running semantic
consumer, model preference or meaning-search UI. The encoder/index checkpoint is
[c1f4b1b](P06-semantic-backend.md); this slice prepares its canonical input.

## Behavior

Canonical schema 15 adds `semantic_pending`, independently acknowledged from the
lexical frontier. Existing originals seed the queue at upgrade. Future lexical
source changes mirror into it transactionally; lexical acknowledgments do not
consume expensive semantic work. A newer requeue has a later sequence, so a late
acknowledgment cannot erase it. The queue retains deletion notices even when
Clear cache removes obsolete lexical notices. It contains IDs, not duplicate
bodies, credentials or user-owned state.

The bounded read returns at most 32 sources, limited to 32 KiB each for decoded
subject/body at valid UTF-8 boundaries, with explicit MIME/projection truncation.
Encrypted, headers-only, removed and unavailable projections return removals.
Original SHA-256 plus the explicit projection policy form the source revision;
the future consumer must combine that with the model policy. Unchanged cached
revisions can skip inference through the vector adapter's `contains` method.

The separate metadata-only candidate query uses current enabled accounts,
effective memberships and UIDVALIDITY, inbox/account/folder scope, UTC half-open
dates and the same literal parsed-From filter as lexical search. It returns all
allowed original hashes before a vector hit limit. It neither loads originals
nor sets Seen. Typed methods use the existing bounded worker channel/sole SQLite
owner; no additional store connection, scheduler or service.

## Verification and limits

Focused strict Clippy/core tests and the full `nix develop --command bash
ci/check.sh` passed on CI. Four synthetic tests establish independent frontiers,
late-ack replay/deletion, schema-14 upgrade seeding without lexical requeue,
account/scope/sender/date/UIDVALIDITY restriction with unchanged unread counts,
and bounded Unicode plus encrypted/outline exclusion. The legacy schema-six
attachment upgrade fixture now removes the new table/trigger when constructing
its historical database; the existing upgrade/draft-preservation tests pass.

This is not canonical-result presentation acceptance. The native connector must
fence exact source revisions again in the same canonical query that produces
ranked conversation rows, preserve ranked cursors through changes, and ignore
retired query/configuration generations. A previously materialized allowlist
alone is insufficient against a source change between retrieval and display.
Runtime ownership, bounded inference scheduling, model availability/coverage,
configuration adoption, native GUI and combined performance remain open.

The full workspace check reported **319 passed, 26 ignored**. The serialized
x86_64 Nix release gate also passed (`gate-exit=0`), including vendor-lock equality,
release Clippy/tests, package and private CLI/credential checks; aarch64 was omitted.
[Workspace excerpt](artifacts/p06-semantic-ingestion/checks.log),
[release excerpt](artifacts/p06-semantic-ingestion/release-gate.log).
The unchanged vendor pin is `0ebeac1d2d97ce07166583e4321061eb935ca4b1b19f6700edaee0b24d416009`;
prepared app sources are `6750b4a431f5c243d406f9fa9facdf844c4c91fef17bd550d276b545cf3d8042`.
New-source Flatpak SDK/runtime acceptance remains open. CI ended with 26 GiB root
and 13 GiB data available. No garbage collection or deletion was performed.
