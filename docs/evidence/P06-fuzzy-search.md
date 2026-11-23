# P06 — Typo-tolerant local search

Status: native acceptance against a real server with synthetic mail. **Search by meaning
is not implemented**, and this does not complete P06.

## Behavior

Typing in the one search field searches as you type, after a 200 ms pause, and Enter
searches at once. Words are lowercased and accent-folded with one analyzer for every
language, so `reinscription` finds `Réinscription`. Results come in tiers, newest first in
each: whole words, then word beginnings. Only when both find nothing are typos corrected,
one edit for words of three to five letters and two beyond that, so a correctly spelled word
does not bring back its lookalikes. Quotes mean exactly that phrase, without expansion,
and `from:` and `subject:` restrict a part. Every word must match. Queries are plain text:
no character in them is an operator to the engine.

Results are conversations, deduplicated by the same query as the list, with account, folder
and inbox scope enforced there rather than trusted from the index. Paging continues from
the cursor's place in the ranked order. Searching never sets Seen.

## Design

`crates/mailstronaut-search` wraps Tantivy and stores terms and identifiers, not text that
could be read back. It sits in the application's private data directory beside the mail
store, which remains the only source of what is in it.

`search_pending` (schema 10) is written in the same transaction as the cached message, and
a trigger queues deleted messages, so the index can lag but cannot miss anything. A batch of
64 is acknowledged only after Tantivy commits, so an interruption repeats a batch harmlessly,
and a message queued again meanwhile keeps its place. An index that is damaged or missing is
recreated and refilled from the store; one in use by another process is left alone. Until
the index opens, or if it never does, the store's literal FTS5 search answers. That table
stays for now, as this fallback and as the source of indexed text.

## Checks

`experiments/p06-search/gui.sh`, called by `credentials.sh` with
`MAILSTRONAUT_SEARCH_GUI_BINARY`, types into the real field in a private sway session, against
mail fetched from a real Dovecot. Opening the first row is what proves which row came
first, since that sets Seen on exactly that message:

- `reinscripton`, misspelled and without its accent, finds the French notice. Nothing was
  marked read by searching.
- `riunone giovdi`, two misspelled Italian words, finds the Italian message.
- `from:finance subject:budg` finds the English message by named fields and a word beginning.
- `zzzzzzzzzz` finds nothing.
- `xylofone` finds a message delivered while the application was running.
- With the index directory deleted, it refills itself, and `bugdet` finds its message.

`crates/mailstronaut-search/tests/search.rs` covers folding, typo limits, tiers, phrases,
named fields, account isolation, replacement and removal, hostile queries, persistence, a
damaged index and a busy one. `store::search` covers the queue, including a message queued
again while being indexed. `store::browse` covers order, grouping, scope and paging.

## Restart during index creation

A process can stop after Tantivy creates its directory but before SQLite queues
the existing mail for indexing. On the next launch, an index with zero committed
documents requests a refill. A nonempty index continues from the durable queue.

On 2026-11-23 the pinned native build at `0db848c` used a disposable Dovecot
and a private Wayland profile. With an empty SQLite indexing queue, the fixture
substituted a valid empty index created by a second application launch and
restarted. Searching `subject:returm` found the cached “Book return” message.
It stayed unread through the query and became read on explicit open. A separate
restart covered a missing index. The index persistence, corruption and busy
writer regression also passed.

## Measurements

`scale_measurement` in a release build on the 16-CPU CI server. The corpus is synthetic: 300
words per message from a small multilingual vocabulary, with one word in seven made unique
to grow the term dictionary. Real mail will index larger, since this vocabulary compresses
unusually well. Times are for 30 warm queries with a limit of 1,000.

| | 10,000 messages | 100,000 messages |
| --- | --- | --- |
| Whole word `budget`, p95 | 0.5 ms | 3.0 ms |
| Typo `budgt`, p95 | 2.7 ms | 10.4 ms |
| Typo `reunoin`, p95 | 5.0 ms | 17.3 ms |
| Two words, p95 | 0.4 ms | 3.7 ms |
| Phrase, p95 | 1.8 ms | 17.9 ms |
| Short beginning `per`, p95 | 3.6 ms | 35.2 ms |
| Time to index everything | 4.4 s | 47.8 s |
| Index size | 6 MiB | 55 MiB |
| Peak process memory | 48 MiB | 133 MiB |

These measure the index alone. Turning 1,000 hits into a page of rows takes another 73 ms
at this scale; [the store's measurements](P03-scale.md) are recorded separately.

## Not established

Search by meaning, including the French doctoral example phrased in English. Date bounds, the
filter popover, snippets and match highlights. CJK text has no word segmentation, so it
only matches from the start of a run of characters. At most 1,000 hits are considered before
folder scope applies, so a very common word searched within one folder can miss older
matches there; account scope is applied in the index and has no such limit. Expunged mail
stays indexed, though it is never shown, because the store itself keeps those messages.
## Literal body excerpts

Unqualified word results use SQLite FTS5 to select a bounded excerpt from the
canonical original the row opens. Tantivy's existing analyzer identifies actual
literal words and supplies UTF-8 ranges for the native GPUI styling. The same
metadata, rank, and open action stay in place. A row without a literal body match
keeps its ordinary preview. Related mode, quoted phrases and field-restricted
queries keep their existing presentation.

The pinned local workspace checks passed, including tests for the canonical
original, body text outside the ordinary preview, accent-folded UTF-8 offsets,
and excluded query syntax. The native Wayland walkthrough searched for
`eligibilite` and showed the actual accented word in the body excerpt.
The result list kept Seen unchanged. The same run passed scoped typo search
with 1,005 newer Sent distractors, late delivery and index rebuild. See
`target/p02.GQLD8s/search-gui/literal-body.png` and
`target/local-checks/literal-excerpt-native.log`.
