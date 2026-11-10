# P06 — Multilingual MiniLM encoder feasibility

Date: 2026-11-10. Candidate screening only: the application has no semantic-search
integration yet. No application Cargo dependencies were added, and the semantic
search/model-adoption plan rows remain open.

## Pinned existing implementation

Candidate: `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` at
`e8f8c211226b894fcb81acc59f3b34ba3efd5f42`, Apache-2.0. The
[publisher's pinned model card](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/blob/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/README.md)
specifies standard BERT, mean pooling, 384 dimensions and a 128-token sentence
limit. Seven artifact sizes/SHA-256 hashes, including sentence/pooling settings,
are committed in the [manifest](../../experiments/p06-search/minilm-model-manifest.json).
Provisioning is explicit; the app downloads nothing. Local files total 458 MiB.

The Rust probe uses Candle 0.11's existing `BertModel`, its safe owned-buffer
safetensors loader and tokenizers 0.22.2, with CPU/F32 and accelerator defaults
disabled. It reuses the previous experiment's dependency pins and the root lint
policy, with a separate 160-package lock containing no LanceDB/Arrow. It shares
the main Cargo target instead of creating another target cache. No transformer,
vector engine, model runtime, unsafe code or compiler exception was introduced.

## Fixed relevance and independent parity

The unchanged original ten-query EN/FR/IT screen passed **10/10 top-one and
10/10 top-three**, with the user's English doctoral re-enrollment query retrieving
the French notice first. The original requirements remain >=8/10 top-one,
10/10 top-three and that example first. Same-topic negatives and account/folder
duplicates are unchanged. The model uses no E5 prefixes.

An independent, local CPU PyTorch 2.14.1 / Transformers 5.17.0 process loaded the
same hash-verified files, with offline mode, no remote code and no GPU. Comparison
of all 12 document and 10 query vectors passed the original parity limits:

| Output | Maximum absolute component error | Minimum paired cosine |
| --- | ---: | ---: |
| Documents | 1.3411045e-7 | 0.99999976 |
| Queries | 1.1175871e-7 | 0.99999982 |

The limits are <=1e-4 and >=0.99999. NumPy independently ranked the actual Rust
vectors and reproduced all ten correct first results. This is bounded fixture
correctness, not a proof for all possible model inputs.

## Release resources and observation

The encoder-only release build completed in 1m10s on `ci-py` through pinned
`nix develop .#search-probe`, with LLVM/thin LTO/one codegen unit. Formatting and
strict inherited Clippy passed. One subsequent untraced run measured:

| Measurement | Observed |
| --- | ---: |
| Hash-verified model load | 1,391.444 ms |
| Warm short-query encoding p95, 30 samples | 58.124 ms |
| Process peak RSS | 1,210,672 KiB |
| Steady RSS after encoding | 759,384 KiB |

These meet the predeclared encoder screening limits of 5s load, 300ms query p95
and 1.5 GiB process peak. Page caches were warm; the run follows earlier reference
loading, so this is not cold-disk startup or whole-app/query/index performance.
It excludes a production search engine and cannot establish a speedup against
earlier probes that linked LanceDB. The Python oracle's memory is not treated as
a Rust performance result.

A separate traced run observed no IPv4/IPv6 socket calls from the Rust encoder,
with an actual successful TCP connection as the observer's positive control.
A same-size modified configuration was rejected by its checksum before inference
and produced no vectors. These are bounded operations, not general application
privacy acceptance. No production, GUI, live-account or distribution acceptance
is claimed for this model.

Complete ignored artifacts: `target/remote/minilm-quality.log`,
`target/remote/minilm-rust-probe.log`, `target/remote/minilm-rust-reference.log`,
`target/remote/minilm-privacy.log`, and the
`target/p06-search/minilm.6v6wd8/` / `minilm-privacy.ZHdt6U/` directories on CI.
The final release binary SHA-256 is
`1821351595548870d2ec080d8c8661d607ae24cd749ba693e0f743db8193a8e8`.
Compact persistent receipts are under
[`artifacts/p06-minilm`](artifacts/p06-minilm/checks.log).

## Frozen additional screen

Before observing additional results, `relevance-heldout.json` freezes 26 new
synthetic messages (12 targets, 12 near-topic negatives, two out-of-scope copies)
and 24 balanced EN/FR/IT cross-language queries. The predeclared thresholds are
>=20/24 top-one and >=23/24 top-three. The probe commit freezes the corpus and its
thresholds before evaluation. They are not representative real-mailbox acceptance.
The freeze commit is `98d90a9`. Only after that commit, the Rust encoder exported
the held-out vectors and the independent reference evaluated them. Both numerical
parity limits passed again: document maximum absolute error 1.7136335e-7 / minimum
cosine 0.99999964; query error 1.1920929e-7 / cosine 0.99999976.

Held-out retrieval passed **22/24 top-one and 24/24 top-three**. The two misses,
`library-en` and `library-fr`, ranked the book-renewal distractor (117) before the
correct reserved-book pickup notice (105), which remained second. The corpus,
labels and thresholds were not edited after observing these misses. This exposes
a negation/context limitation rather than establishing general email accuracy.
The original ten-query user-example screen remains separately passed.

Complete receipt: `target/remote/minilm-heldout-reference.log`; exported vectors
and run metrics: `target/p06-search/minilm-heldout.Vvf27c/` on CI. A persistent
[held-out receipt](artifacts/p06-minilm/heldout.log) preserves every ranking.

Production model manifest/lifecycle, chunking/coverage,
incremental vector indexing with deletion/scope/model revision fences, 10k/100k
budgets, configuration and the native search connector remain required.
