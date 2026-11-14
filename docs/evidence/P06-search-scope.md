# P06: search scope before collection and ranked refresh

2026-11-14. Canonical account/folder membership now restricts text collection
before the existing 1,000-original hit cap. A large Sent or Archive folder can no
longer consume every result slot before the inbox filter runs.

The existing store owner reads only canonical original IDs into a shared ordered
set. It validates enabled accounts and current folder UIDVALIDITY, including
headers-only mail. Tantivy 0.27's maintained `FilterCollector` on the existing ID
fast field applies that set before `TopDocs`. No new index, folder schema,
matching algorithm, thread, package or network path is introduced. Each fuzzy
match tier sees the restriction, so an exact hit outside the scope cannot suppress
an in-scope typo match. Sender/date restrictions retain their existing query
implementation. Empty scopes permit no results. SQLite rechecks all scope/metadata
restrictions when projecting conversations, including membership changes during
search. This does not promise a source-stable lexical snapshot while queued index
updates are still pending.

A separate regression reproduces ranked prefix loss: 120 older exact `Budget`
hits precede 120 newer `Budgetary` prefix hits. Refreshing a previously loaded
200-row prefix incorrectly stopped at 100 because it compared arrival timestamps
across tiers. The failing actual-store/index test recorded `left: 100, right: 200`.
The page adapter now reports whether it actually used rank. Ranked refresh seeks
the loaded original ID rather than its arrival date; literal/metadata browsing
keeps its chronological cursor. The existing hit cap bounds a vanished ranked
boundary. Repeated original IDs across batches fail with an explicit retry state
instead of producing duplicate rows. Every batch remains at most 100 metadata
rows and cancellation stays effective.

## Verified behavior so far

- The scope-only implementation passed pinned strict workspace Clippy, formatting,
  Nix/action/ShellCheck and **326 tests, 27 ignored** on x86_64 CI.
- The new collector regression excludes a target behind 1,005 newer unscoped hits,
  then finds it when scoped before collection; an exact match in another scope
  cannot suppress an in-scope typo. Canonical store coverage includes inbox,
  account, folder, disabled/foreign accounts, empty scopes, UIDVALIDITY retirement
  and unchanged Seen flags.
- `MAILSTRONAUT_SEARCH_SCOPE=1` extends the existing private native search
  walkthrough with 1,005 actual TLS Dovecot Sent messages plus an older inbox
  target. It passed in standalone pinned `.#gui-test`, with a prebuilt app and
  credential fixture: exact/typo search, inbox/Sent switching, explicitly clicked
  original Seen changes, new delivery, and complete derived-index rebuild.
  The profile is `target/p02.TJlv03/search-gui`; its current actual app screenshots
  were shown in chat. The user's desktop, accounts and keyring were not used.
- The ranked-prefix test deliberately failed on the old timestamp comparison;
  that failure is a reproduction, not a passing repository check.

The final combined implementation passed the pinned full workspace check:
**327 tests, 27 ignored**, strict Clippy and format/Nix/action/ShellCheck. The new
actual-store/index regression passes across both tiers and after another exact
match arrives. [Workspace excerpt](artifacts/p06-search-scope/workspace-check.log),
[original failure](artifacts/p06-search-scope/ranked-reproduction.log),
[scope native walkthrough](artifacts/p06-search-scope/native-debug.log).
Installed-release and package evidence is recorded below only after those checks
complete. No 100k combined GUI/ML, live provider,
current-source Flatpak SDK, or model-quality acceptance follows from these fixes.
