# Isolated search-engine feasibility probe

This is an experiment, not a production search crate or a semantic-search feature.
The script materializes a small workspace under `target/p06-search/probe`, with
centralized pinned dependencies and the application workspace's lint policy.
The application Cargo.toml/Cargo.lock remain unchanged. All generated runtime
fixtures, targets, logs and traces stay in the current project.

Commands, run one build at a time:

```sh
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh fetch
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh build
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh smoke
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh release
nix develop .#search-probe --command bash experiments/p06-search/reproduce.sh bench
```

`lock` deliberately refreshes the tracked experiment lock and is for dependency
changes, not normal reproduction. The first version required Arrow 58.4.1:
DataFusion 54's Arrow minimum conflicts with an exact 58.0.0 pin. The lock fixes all
524 selected packages; no candidate model weights or embedding runtime are fetched.

`build` uses the default pinned nightly Cranelift/Wild setup. `release` uses LLVM,
thin LTO and one codegen unit, as required for the application. Native protocol
code generation uses Nix's Protobuf executable. No dependency fork or new LLVM
backend exception is used.

`smoke` tests a typo query, account/folder prefilters, library vector and hybrid
retrieval, reopening in a separate process, and logical deletion. The network
observer must first see a loopback connection from its positive control; neither
search process may issue an IP socket call. This only establishes the bounded
synthetic operations observed. It is not a privacy proof for arbitrary features,
physical erasure proof, mailbox acceptance or multilingual-model quality evidence.

The performance cases contain 10k/100k synthetic records with 384-component dense
vectors, no model. They build a text index and use library flat vector retrieval;
ANN index construction/recall is not measured. Three warmups precede 30 timed runs
for fuzzy/vector/hybrid queries. The initial query follows insertion in the same
process and the OS file cache stays warm: it is not a cold-storage measurement.
GNU time records process peak RSS and elapsed time; file sizes are logical bytes.
Do not interpret debug-profile or traced timings as release performance.

Before running performance cases, the provisional release screening targets are:
fuzzy p95 <= 200 ms; vector/hybrid p95 <= 500 ms; process peak RSS <= 1 GiB and
derived index bytes <= 1 GiB for 100k records. These are screening targets, not
claims about real-message indexing, semantic-model memory, recall or UI latency.
Actual source/model/chunk coverage and held-out retrieval remain required by
`docs/SEARCH.md`; do not adopt the library from this smoke test alone.
