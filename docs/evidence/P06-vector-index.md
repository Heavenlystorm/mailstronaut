# P06: bounded USearch vector backend screen

2026-11-10. The isolated safe Rust probe selects USearch 2.26.6 alongside Tantivy.
This records library feasibility, not an implemented semantic-search UI or final
combined app/model/index acceptance. SQLite remains canonical.

## Library selection

Published archives were verified against crates.io checksums before source
inspection. [Source metadata](artifacts/p06-vector/source-audit.json) records
USearch 2.26.6 (Apache-2.0) and hnsw_rs 0.3.5 (MIT/Apache-2.0). The inspected
USearch Rust `Index` offers `filtered_search`, `remove`, `compact`, `save/load`,
and bounded-buffer serialization. Its safe methods handle the existing C++17
vector engine; no application unsafe code, graph, metric or transformer is added.
Default features are disabled: no NumKong/OpenMP or `-march=native` build flag.
This chooses the portable scalar implementation, rather than claiming CPU feature
compatibility from the host build. The inspected hnsw_rs Hnsw public methods do
not expose comparable deletion/update operations; that would require more
application-owned lifecycle glue. This is a source/API comparison, not a measured
performance comparison with hnsw_rs.

- [USearch pinned Rust API](https://docs.rs/usearch/2.26.6/usearch/struct.Index.html)
- [Published USearch source](https://docs.rs/crate/usearch/2.26.6/source/rust/lib.rs)
- [hnsw_rs API](https://docs.rs/hnsw_rs/0.3.5/hnsw_rs/hnsw/struct.Hnsw.html)

`experiments/p06-search/vector.sh` materializes an isolated workspace with the
root lint policy and a dedicated committed lock. It shares the existing CI target,
not a second target or a production LanceDB dependency. Strict release Clippy,
formatting and build passed on the pinned `nix develop .#search-probe` shell.

## Actual model vectors and lifecycle

The exact independently checked MiniLM F32 vectors from the encoder experiments
were inserted with F16 storage, dimension 384, cosine distance, connectivity 32,
build expansion 128. The original corpus retained 10/10 rank-first and 10/10
top-three; the frozen held-out set retained 22/24 rank-first and 24/24 top-three.
The query predicate excludes foreign-account and Archive copies before top-k.
Removal, reinsertion, compaction, file save/reopen, unchanged result order and an
empty allowed scope passed. These are synthetic API checks, not complete
canonical-store/source/model-revision or crash acceptance.

## Predeclared synthetic scale screen

The benchmark uses seeded independent normalized 384-dimensional random vectors,
parallel insertion, thirty independent queries and an exact oracle supplied by
USearch itself. The fixed predicate accepts one third of keys. The oracle takes
the exact global top 128 and requires at least ten eligible hits before comparing
the first ten; it refuses otherwise. No benchmark query is an inserted vector.

Before measurement the sweep was fixed at search expansion 128, 512, 2048, 8192,
selecting the first with at least 285/300 exact neighbors (95% recall), warm
filtered-query p95 <=500 ms and serialized index <=1 GiB. Three queries warm each
setting before thirty timed queries. These are screening limits, not final
whole-app budgets or real-mail relevance proof. Parallel HNSW construction can
change its graph; the reported numbers describe this run.

| Synthetic vectors | Build | Exact top-128 p95 | Passing expansion | Recall | Filtered p95 | Index |
| --- | --- | --- | --- | --- | --- | --- |
| 10,000 | 2.122 s | 6.008 ms | 128 | 297/300 | 5.674 ms | 10,447,632 B |
| 100,000 | 104.566 s | 60.753 ms | 2048 | 300/300 | 104.478 ms | 104,434,128 B |

At 100k, expansion 128 found 158/300 neighbors (15.516 ms p95), and 512 found
275/300 (43.400 ms p95). They fail the unchanged recall screen; production should
start at the passing 2048, not silently adopt the faster defaults. The index
reported 185,352,448 B library memory at 100k (16,860,416 B at 10k).
[Full command output](artifacts/p06-vector/checks.log).
External process measurements are recorded separately with GNU time. This excludes
the loaded encoder, GPUI, SQLite, lexical index, real document chunk counts,
persistence and store-filter materialization; combined runtime/packaging and
incremental throughput remain acceptance work. No CPU/vector speedup claim is
made against earlier LanceDB corpora.

The run left root/data disks with 30/22 GiB available. No cleanup or garbage
collection was performed. Benchmark indexes are project-local ignored artifacts.
