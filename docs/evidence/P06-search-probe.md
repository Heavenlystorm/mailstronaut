# P06 strong-search library feasibility

Status: isolated LanceDB API checks passed; adoption and semantic search remain open.
The application dependency graph and its literal FTS behavior are unchanged.

The reproducible [probe](../../experiments/p06-search/README.md) uses LanceDB 0.41.0,
Arrow 58.4.1 and a separate checksum-locked workspace with 524 selected packages.
The application root owns its original dependencies; the generated experiment
workspace centralizes its own versions and inherits the root lint policy. The new
`search-probe` development shell adds standard Protobuf and strace tools.

`nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh build`
passed with the pinned nightly Cranelift/Wild development settings and strict
Clippy. No dependency fork, additional LLVM exception, model, hosted service or
custom retrieval algorithm was used. A first build exposed a changed public API:
this version accepts `RecordBatch` directly; an unboxed `RecordBatchIterator` does
not implement its `Scannable` interface. The earlier exact Arrow 58.0.0 probe pin
also conflicted with DataFusion's minimum; the accepted version is 58.4.1.
[Build/lint log](artifacts/p06-search-probe/debug-build.log).

The [runtime log](artifacts/p06-search-probe/smoke.log) records successful existing-
library fuzzy queries, account/folder prefilters, flat vector retrieval and built-in
RRF hybrid queries. A second process reopened the persisted index, repeated those
checks and verified that a logically deleted original is not returned. The data
contains a French notice, a different account's similar notice, an archived notice
and an unrelated message. The vectors are hand-assigned synthetic fixtures:
**this is not evidence of multilingual semantic understanding**.

Strace followed both search processes and observed no IP socket calls. A separate
loopback control generated a connection and was observed successfully. This bounds
the claim to these operations/features, not arbitrary engine features or all
production network paths. The four-record debug runs peaked around 214 MiB RSS;
traced/unoptimized timings are not release performance. Complete runtime artifacts
remain in `target/p06-search/run.QbzSNIdG`.

Release benchmarking is pending. Its targets and limitations were recorded before
running it in the experiment README: 10k/100k synthetic records, 384-component
dense vectors, no model, library flat vector retrieval, three warmups and 30 timed
queries. A dedicated sequential runner shares the existing release-gate lock and
records a numeric terminal receipt. Do not duplicate that build.

Still required: release/resource evidence, representative multilingual model and
relevance proof, indexing/purge lifecycle, actual application graph/packaging,
native UI integration and all remaining P06 acceptance. Logical deletion alone
does not establish physical removal from historical files. No search candidate
has been adopted yet, and the client still does not provide the requested stronger
search behavior.
