# P06 strong-search library feasibility

Status: isolated LanceDB API checks passed; adoption and semantic search remain open.
The application dependency graph and its literal FTS behavior are unchanged.

The reproducible [probe](../../experiments/p06-search/README.md) uses LanceDB 0.41.0,
Arrow 58.4.1 and a separate checksum-locked workspace. Its initial standalone lock
had 524 entries; the later two-model comparison extends it to 583 entries including
two local probe crates. The original release screen below used the initial graph.
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

The coordinated release probe at `2a1a044` completed with numeric receipt `0`;
`mailstronaut-p06-search-release-probe.service` was verified inactive/dead with
`ExecMainStatus=0`. Its invocation was `70fb4b5171a44e6cbef7ea59bcd6e4b2`.
The pinned development shell built with LLVM, thin LTO and one codegen unit in
13m20s, using all 16 CPUs. The executable is 176 MiB. This is an experimental
release build, not the application's Nix package gate or installed acceptance.

| Synthetic records | Fuzzy p95 | Vector p95 | Hybrid p95 | Process peak RSS | Index bytes |
| --- | --- | --- | --- | --- | --- |
| 10,000 | 3.2 ms | 22.8 ms | 26.0 ms | 505 MiB | 16,345,735 |
| 100,000 | 7.1 ms | 224.9 ms | 208.5 ms | 876 MiB | 164,823,462 |

[Metrics](artifacts/p06-search-probe/release-benchmark.log),
[10k resources](artifacts/p06-search-probe/release-10000-resources.txt),
[100k resources](artifacts/p06-search-probe/release-100000-resources.txt) and
[build resources](artifacts/p06-search-probe/release-build-resources.txt) preserve
the measurements. The provisional screening budgets were fixed in the README
before running: fuzzy p95 <= 200 ms, vector/hybrid <= 500 ms, peak RSS and derived
index <= 1 GiB. All passed, but memory leaves limited headroom for a model and GUI.

These are 384-component dense synthetic vectors, flat retrieval without an ANN
index or model, three warmups and 30 timed queries per operation. Account/folder
prefiltering selects about one third of the corpus. Queries run sequentially with
warm OS caches after insertion; neither cold-storage latency nor an advantage of
hybrid over vector retrieval is established. Repeated short synthetic bodies also
do not establish representative mailbox indexing throughput or relevance.

The next bounded experiment uses the explicitly pinned
[E5 artifact manifest](../../experiments/p06-search/model-manifest.json).
Weights have not been downloaded as part of the search-library probe. Measure
actual multilingual retrieval and model load/query memory before adoption.

Still required: representative multilingual model and
relevance proof, indexing/purge lifecycle, actual application graph/packaging,
native UI integration and all remaining P06 acceptance. Logical deletion alone
does not establish physical removal from historical files. No search candidate
has been adopted yet, and the client still does not provide the requested stronger
search behavior.
