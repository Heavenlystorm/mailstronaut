# P06 — Native sender and received-date filters

Date: 2026-11-10. Scope: local metadata filtering alongside existing typo-tolerant
search. This does not complete P06 or implement semantic search.

## Behavior and components

The toolbar uses GPUI Kit's `Popover`, `Input`, range `DatePicker`, date presets
and Apply/Clear `Button`s. The existing sidebar chooses account/folder scope.
Sender text is a literal, ASCII case-insensitive substring of parsed From names
and addresses, excluding recipient fields;
punctuation is not interpreted as a wildcard. Both calendar endpoints are inclusive.
They become a half-open UTC interval using local midnights in the configured time
zone, so a day spanning a DST transition need not be 24 hours. Incomplete/reversed
ranges and invalid/oversized sender values are rejected without replacing the
applied filters. Staged edits are applied only by the Apply button.

With query text, maintained Tantivy `RangeQuery` and escaped `RegexQuery` restrict
originals before bounded hit collection. The derived index adds a raw sender field;
the existing schema-recovery path recreates/requeues an older index. Without query
text, metadata-only search uses the canonical store directly, with ordinary cursor
batches and no text-index result cap. SQLite rechecks filters on the selected
original before grouping, preserving unread counts for other in-scope members.
Opening a match selects that original. Applying, clearing, searching and navigating
folders do not set Seen.

## Verification

The following behavior tests cover the filter contract:

- `store::browse::tests::metadata_filters_select_matching_originals_without_clearing_unread_siblings`
  exercises ranked and fallback reads, exclusive upper bounds, invalid ranges,
  case-insensitive named-sender/address matching, recipient exclusion, literal
  punctuation and full thread unread counts.
- `store::browse::tests::sender_only_filters_page_and_scope_without_hiding_thread_members`
  exercises metadata-only cursor continuation, account scope and requiring text/date
  to match the same original rather than different members of a thread.
- `metadata_restrictions_precede_the_hit_limit_and_treat_sender_metacharacters_literally`
  exercises Tantivy filtering before a one-hit limit, filter-only queries, invalid
  bounds and literal regex punctuation.
- `search_filters::tests::inclusive_calendar_ranges_respect_dst_and_reject_incomplete_values`
  exercises a 23-hour Europe/Paris day, clearing and invalid calendar ranges.

`experiments/p06-search/filters.sh`, dispatched by the private credential fixture
with `MAILSTRONAUT_FILTERS_GUI_BINARY`, fetches three synthetic messages from Dovecot
in native Wayland. It combines a misspelled `budegt` query with Finance sender and
Last 30 days, exercises metadata-only search and Sent/INBOX navigation, then opens
the matching original by its From address behind a display name. A To-address
filter returns no row and sets no Seen flag. SQLite assertions require exactly
the matching message to become
Seen. Clearing filters exposes the newer Library message; it remains unread until
explicitly opened. Captures are kept under the private fixture's `search-filters/`.

On x86_64 `ci-py`, through `ci/remote.sh`, the pinned GUI-test Nix shell passed:

```sh
bash ci/check.sh
cargo build -p mailstronaut
MAILSTRONAUT_FILTERS_GUI_BINARY="$PWD/target/debug/mailstronaut" bash experiments/p02-accounts/credentials.sh
MAILSTRONAUT_SEARCH_GUI_BINARY="$PWD/target/debug/mailstronaut" bash experiments/p02-accounts/credentials.sh
```

Formatting, Nix formatting, Actionlint, ShellCheck and strict workspace Clippy
passed. The workspace reported 309 passing tests and 24 ignored opt-in tests. Both
private native walkthroughs passed, including the existing French/Italian fuzzy
search, new delivery and index-rebuild scenarios. Both runs also passed the private
Secret Service checks. The complete local receipt is
`target/remote/search-filters-complete-check.log`; the final filter captures are in
`target/p02.uh6Vyg/search-filters/`, with the lexical regression under
`target/p02.TIhB6o/search-gui/`. These paths are ignored test artifacts, not a hosted
CI claim. A compact persistent receipt is in
[checks.log](artifacts/p06-search-filters/checks.log).

Fresh vendor staging with a placeholder hash produced
`sha256-6SKj1Do4dyHiMejkv5pj3KhjCfVVpnm5s03oVScO6NU=`. The final shared gate passed
with exit 0 and `all checks passed!`:

```sh
bash ci/remote.sh flock /builds/.mailstronaut-nix-gate.lock bash ci/nix-check.sh
```

It checked workspace/vendor lock equality, strict release Clippy, release workspace
tests, private package fixtures and the Home Manager module checks. The package is
`/nix/store/7s42fkhw3bpsfzh1bblih227lqpkx7x6-mailstronaut-0.1.0`.
[Release receipt](artifacts/p06-search-filters/release-gate.log); complete ignored
log: `target/remote/search-filters-final-package-gate.log`. Only x86_64-linux was
built; the gate explicitly omitted aarch64-linux. Root/data free space remained
32/25 GiB during the final build, without deleting any artifacts.

## Remaining acceptance

Match snippets/highlights and multilingual semantic retrieval remain open. Text
search still has the existing 1,000-original cap before folder scope, so a common
term in one folder can omit older matches. Sender filtering deliberately does not
promise Unicode case folding or accent normalization. No new 100k-filter latency,
screen-reader, live-provider, aarch64 or current-source Flatpak acceptance is claimed.
