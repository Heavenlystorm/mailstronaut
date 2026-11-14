# P06: explicit native related-mail search

2026-11-13–14. The native application now connects the existing CPU MiniLM encoder,
USearch adapter and independent canonical queue. This is an explicit related-mail
feature, not completion of the whole P06 quality/resource gate.

## User behavior and existing components

The normal native field still searches words with Tantivy. Enabling native
`search.semantic` with an explicitly installed `search.modelDirectory` adds a
standard Kit **Related** button beside that field. Its selected/disabled state,
related-results `Alert`, footer and tooltip distinguish suggestions from word
matches. Quotes and `from:`/`subject:` constraints remain lexical. No second search
field, custom fuzzy matcher, ranker, transformer or background service is added.
Standard Settings switch/input, ownership checks, Save/Discard and Export to Nix
apply to the same native schema. Priority mail can remain disabled. No network
model download or remote query is possible through the encoder/connector.

One cancellable owner retains the model and vector cache off the UI thread. It
processes eight originals per batch, checks cancellation between tokenizer windows,
and commits each vector before acknowledging canonical work. Existing source hashes
skip repeated inference. The source text cap is the smaller of 32 KiB and the
configured analysis cap; up to eight overlapping windows report excerpt coverage.
Queue/status notifications wake it after sync; a thirty-second idle check also
picks up independent canonical edits. Checkpoints happen at idle/minute boundaries
and shutdown. Configured account/model/cap changes retire it, and missing/failed
models are explicit, retryable with Reload, while word search remains usable.

Queries cache one encoded phrase and retrieve at most twenty original IDs using
the maintained library's cosine ordering and a candidate floor. Canonical enabled
accounts, folder, UIDVALIDITY, received dates, parsed From and source hashes fence
candidates before retrieval. The same SQL statement that projects a conversation's
matching original rechecks its source hash; a source changing after retrieval is
excluded without shifting subsequent cursor ranks. Query/config generations reject
retired UI results. Dropping a retired query cancels work waiting for the model
mutex; already-running single-window inference remains bounded. Unencodable/failed
related queries report a redacted error instead of silently relabelling word hits.
Inference, suggestions, previews and passive selection never
mark Seen; an explicit successful original open still owns that action.

## Quality boundary: keep the failed gate

The initial unfiltered native walkthrough retrieved the French doctoral notice
first but included weak unrelated tail results. A frozen 0.5 cosine trial dropped
two required positives and was rejected. A separately frozen development-only
calibration derived 0.371 before inspecting held-out/no-answer outcomes. At that
floor the original corpus scored 10/10 first/top-three and the held-out corpus
22/24 first, 23/24 top-three. However **4/12** absent-topic queries returned a
candidate, failing the unchanged maximum of one. The complete failures and rule
are preserved in the [precision screen](../../experiments/p06-search/precision-screen.md)
and its logs, including [0.371](artifacts/p06-meaning/rejected-cutoff-0371.log).
Independent Rust/PyTorch parity on the new no-answer vectors passed.

Therefore no automatic high-confidence/no-answer claim is made. Word search remains
the default, and Related is explicitly requested and labelled as suggestions that
may be unrelated. The 0.371 floor trims weak tails, not false-positive certainty.
Existing local cross-encoder reranking is a practical researched next option over
a bounded shortlist if it passes additional multilingual, CPU and memory gates.
No test criteria or corpus were rewritten to make this implementation pass.

## Verification

- The full pinned `nix develop --command bash ci/check.sh` passed on CI:
  strict workspace Clippy, format/Nix/action/ShellCheck and **322 tests, 27 ignored**.
  This was repeated on the final cancellation/error-handling implementation, with
  the same passing workspace count. Release checks are separately recorded below.
- Explicit real-weight USearch integration passed with the candidate floor: fixed
  10/10 first/top-three; held-out 22/24 first, 23/24 top-three, identical after save
  and reopen. The production worker test passed actual French inference, account
  scoping, canonical deletion/immediate exclusion, eventual cache removal,
  cancellation, an explicit redacted oversized-query error and unchanged unread
  counts. These tests are opt-in, not hidden
  successes among the 27 ignored cases.
- `experiments/p06-search/meaning.sh`, dispatched through the private credential
  fixture, passed using the current app under standalone `.#gui-test`: real TLS
  Dovecot mail, native Wayland, private encrypted Secret Service. English word
  search first returned no hit; explicit Related displayed only the French notice,
  excluding the other three native-fixture messages. Passive keyboard/list/model
  work left Seen unchanged; clicking that result opened and marked only it Seen.
  The final run also rapidly replaced pending related queries before returning to
  the doctoral phrase; only the current results were presented. Quoted exact intent
  and ordinary typo search retained word behavior. Search
  Settings saved `semantic = false` to its editable native source, stopped meaning
  results, and a restart with a missing model kept lexical search usable with an
  explicit inline warning. The accompanying credential lifecycle tests also passed.
  The private profile was `target/p02.D5w0N4/meaning-gui`; screenshots stay ignored
  under `target/remote/meaning-final-images` and were displayed in chat.
- The first GUI-shell attempt failed before application startup because nested
  `nix develop` polluted Dovecot's variable expansion. A standalone pinned GUI
  shell resolved that fixture failure. The next run reached Settings but its
  coordinate clicked above the switch after description wrapping; the actual
  screenshot located the switch and the final complete run passed. These failed
  attempts are not runtime acceptance.
- The first explicit vector test invocation supplied a relative output path and
  failed to find its vectors from the crate's test directory. Repeating with the
  absolute fixture path passed without altering model, corpus or result gates.

- The first release gate rejected a test helper's unconstrained predicate future
  (`future_not_send`) under the stable release compiler. Explicit `Send + Sync`
  predicate bounds fix it without suppressing lint checks; the helper's behavior
  and all app runtime code remain unchanged.

- The final serialized x86_64 `ci/nix-check.sh` passed, including exact vendored-lock
  equality, stable LLVM release Clippy/tests, the installed package, Home Manager
  module checks and private credential/portal/instance fixtures: **`gate-exit=0`**.
  The independent final receipt `target/meaning-integrated.status` is `0`.
  aarch64 was omitted. The package is
  `/nix/store/yi5wzbfi15hmspzhcmd8z4h171g1mibl-mailstronaut-0.1.0`.
- Reproducible Flatpak input preparation passed with app archive
  `40b67f6d2ddd1e387019f80607fe5ef3b98c8af6570842f3be6b69b2675f79ee` and
  unchanged vendor archive
  `0ebeac1d2d97ce07166583e4321061eb935ca4b1b19f6700edaee0b24d416009`.
  Cargo.lock and dependency references are unchanged in this feature; the gate's
  exact vendor-lock check passed. Full current-source SDK/runtime acceptance is
  not established by source preparation.

The final native walkthrough was also repeated against the **installed LLVM
release executable**, not just the Cranelift development binary. It passed the
same actual-model, rapid-query, explicit Seen, Settings save and unavailable-model
checks in `target/p02.mArORX/meaning-gui`. [Installed native run](artifacts/p06-meaning/installed-native.log).

The cached `nix build` also returned that same output. Its installed executable
passed `--version`, `config check` and `config export` with the new semantic fields
(`installed-smoke=0`); this did not launch a GUI or contact a provider.
[Installed CLI smoke](artifacts/p06-meaning/installed-smoke.log).

[Workspace/native excerpt](artifacts/p06-meaning/checks.log),
[release excerpt](artifacts/p06-meaning/release-gate.log). Full logs remain in the
ignored target. After the gate CI retained about 25 GiB root and 11 GiB data free;
no garbage collection or deletion was needed in this slice.

## Remaining scope

No combined 100k-mailbox GUI/model memory/latency benchmark, aarch64 build, physical
crash injection, secure SQLite free-page erasure, current-source Flatpak model
import/runtime, live provider or real Home Manager activation acceptance is claimed.
Headless background sync can queue semantic work; the model owner currently belongs
to the visible workspace lifetime. Partial server-cache coverage is separate from
semantic queue/excerpt counts. Better no-answer discrimination remains open.
