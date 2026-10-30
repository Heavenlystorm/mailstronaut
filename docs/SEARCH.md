# Strong search: library research and adoption gate

Research date: 2026-10-30. **Source audit, not an implementation or benchmark.**
The current application provides local literal SQLite FTS search. The user requires
both typo tolerance and multilingual search by meaning, and explicitly wants
pre-existing components. Do not write a fuzzy matcher, inverted index, vector
index, transformer, translation system or search service for this feature.

## Recommendation

**Probe LanceDB 0.41.0 first as the unified embedded search library.** It already
combines fuzzy full-text retrieval, vector retrieval, filters and hybrid ranking;
that minimizes application-owned search machinery. Its resource/build costs must
pass the gate below before adoption. **Tantivy 0.27.0 plus an existing vector library
is the smaller, more modular comparison**, not an instruction to build a hybrid
engine ourselves. Prefer whichever meets quality/resource requirements with less
maintained integration code. Do not add both production search engines.

This recommendation is based on capabilities and dependency inspection; no
performance winner has been established. Existing SQLite remains the canonical
mail/state store. A chosen search engine owns only rebuildable search data. Keep
the working FTS path until the replacement passes acceptance; avoid indefinitely
maintaining duplicate lexical indexes.

The semantic component should reuse the planned local model-loading/tokenizer/
CPU runtime wherever suitable. It needs retrieval embeddings: priority-classifier
labels are not a substitute for a multilingual retrieval model. Do not run a
query/mail NLI pair against every message on each search. Index embeddings once
per source/model revision; encode each query once, then use library retrieval.

## Engines inspected

Published crate versions/checksums and repository snapshots are recorded in
[source-audit.json](evidence/artifacts/p06-search-research/source-audit.json).
Tantivy and LanceDB source archives were checked against crates.io checksums;
LanceDB's exact `lance-index =14.0.2` source was inspected as well. No dependencies
were added to Cargo.toml/Cargo.lock and no candidate/model build was run.

| Candidate | Existing functionality and fit | Decision for the probe |
| --- | --- | --- |
| **LanceDB 0.41.0** | In-process Rust API; fuzzy `MatchQuery`, vector retrieval, prefilters and built-in reciprocal-rank fusion. One derived search store can cover both requirements. Arrow 58/DataFusion 54 are unconditional dependencies, even though default features are empty. | First unified candidate. Measure its closure/build/RAM/index costs; do not infer that a single crate is lightweight. [Rust query API](https://docs.rs/lancedb/0.41.0/lancedb/query/trait.QueryBase.html), [RRF implementation](https://github.com/lancedb/lancedb/blob/f74a7d38cae4ee92b00e80f720f3e90e388931f4/rust/lancedb/src/rerankers/rrf.rs), [manifest](https://github.com/lancedb/lancedb/blob/f74a7d38cae4ee92b00e80f720f3e90e388931f4/rust/lancedb/Cargo.toml) |
| **Tantivy 0.27.0** | Embedded Rust full-text engine with fuzzy term/prefix queries, query composition, fields, document storage and incremental updates. Provides the lexical/fuzzy part; the inspected API is not an integrated multilingual embedding search pipeline. | Lean lexical candidate if the unified option is too costly. Reuse its tokenizers/query/scoring APIs, not hand-written edit distance or postings. [Fuzzy API](https://docs.rs/tantivy/0.27.0/tantivy/query/struct.FuzzyTermQuery.html), [engine features](https://github.com/quickwit-oss/tantivy) |
| **USearch 2.26.6** | Existing vector index with exact/approximate search, filtering, deletion, persistence and quantized storage; safe Rust-facing methods wrap a C++ implementation. Generates no text embeddings itself. | Vector component to compare with Tantivy if needed. Measure native packaging and filters; do not build our own ANN structure. [Rust Index API](https://docs.rs/usearch/2.26.6/usearch/struct.Index.html) |
| **Nucleo** | Convenient fzf/skim-style Rust matcher. Does not supply the complete persistent mail/semantic retrieval stack. | Suitable for small pickers, not the primary mail-search engine. [Upstream scope](https://github.com/helix-editor/nucleo) |
| **Meilisearch / milli** | Meilisearch supplies a search server; its internal `milli` crate is `publish = false` in the current workspace. | Avoid adding a server or coupling the desktop to unpublished engine internals. [Current manifest](https://github.com/meilisearch/meilisearch/blob/main/crates/milli/Cargo.toml) |
| **SQLite FTS5 / sqlite-vec** | Existing FTS5 remains useful baseline infrastructure. The published sqlite-vec Rust setup uses unsafe extension registration. | Do not write fuzzy/semantic behavior around FTS from scratch or add unsafe initialization to our workspace. Reconsider only with a demonstrated maintained safe integration compatible with Servo's SQLite pin. [Upstream Rust setup](https://github.com/asg017/sqlite-vec/blob/main/site/using/rust.md) |

For LanceDB's pinned probe, use `default-features = false` and keep cloud/remote,
OpenAI/Bedrock, metrics-export and automatic model download features off. Its
built-in `sentence-transformers` feature currently selects Candle 0.9.1 and
tokenizers 0.19.1 in the published source, differing from our planned versions.
Do not enable that feature just to get convenient downloads or duplicate runtimes.
Supply locally computed vectors, or use the standard `EmbeddingFunction` interface
to call our shared verified model loader. No LanceDB fork is needed for that API.
[Embedding interface](https://docs.rs/lancedb/0.41.0/lancedb/embeddings/trait.EmbeddingFunction.html).

Concrete public APIs confirmed in the published sources: `FullTextSearchQuery`
and `MatchQuery` are reexported through `lancedb::index::scalar`;
`FullTextSearchQuery::new_fuzzy` and `MatchQuery::with_fuzziness` /
`with_max_expansions` cover typo expansion. `full_text_search`, `nearest_to`,
`only_if_expr` and the built-in `RRFReranker` cover the hybrid path. Prove that
exact combination compiles/runs at the selected release; do not copy a differently
versioned Python example. Keep prefiltering enabled for account/folder restrictions.
Tantivy already provides `FuzzyTermQuery::new/new_prefix` and
`QueryParser::set_field_fuzzy`; use its standard query machinery if selected.

## Local semantic model options

**Candle 0.11 + multilingual E5-small is the compatibility baseline**, because
Candle is already selected for priority inference. E5's publisher config specifies
`BertModel` with hidden size 384 and 12 layers, despite using an XLM-R tokenizer.
Use Candle's existing BERT implementation, not an assumed XLM-R classifier path.
The model card requires task prefixes, masked mean pooling and normalized vectors.
Its metadata reports 117,653,760 F32 parameters (about 449 MiB of float weights),
so “small” does not establish an acceptable idle footprint. Revision:
`614241f622f53c4eeff9890bdc4f31cfecc418b3`. Validate numerical parity, multilingual
relevance, cold load and CPU/RAM costs before adoption.
[Publisher card](https://huggingface.co/intfloat/multilingual-e5-small),
[config](https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/config.json),
[existing Candle BERT](https://github.com/huggingface/candle/blob/0.11.0/candle-transformers/src/models/bert.rs).

**Model2Vec 0.3.0 is the lower-compute challenger.** The official Rust implementation
provides local-only loading and library-owned int8 quantization. Its multilingual
Potion model is 128M parameters, not the much smaller English-only 8M variant.
The audited multilingual artifact is about 489 MiB before quantization; peak load,
steady memory and mail retrieval quality still need measurement. A shared model
manifest/lifecycle can be reused without requiring the same tensor runtime.
[Official Rust implementation](https://github.com/MinishLab/model2vec-rs),
[model card](https://huggingface.co/minishlab/potion-multilingual-128M),
[quantization API](https://github.com/MinishLab/model2vec-rs/blob/1e56ee247d9c693b3f78c76e3ee8cd0b2484cb93/src/model.rs).

FastEmbed 7.5.0 is a credible ready-made embedding API, but its small ONNX model
path adds ONNX Runtime. Its default features also download runtime binaries/models;
those defaults are unsuitable here. Do not describe its newer Candle features as
universal support for small BERT models: the inspected manifest scopes them to
other model families. Keep this as a measured alternative if it substantially
reduces glue or resource use; no second runtime is selected now.
[Feature manifest](https://github.com/Anush008/fastembed-rs/blob/main/Cargo.toml).

No model weights were downloaded. Configuration/metadata alone does not establish
model correctness or adequacy for English/French/Italian email retrieval.

## Required steps before application integration

- [x] Source-level comparison with exact published versions/checksums, public Rust
  API inspection, local-only model candidates and explicit tradeoffs recorded.
- [ ] After the active coordinated release gate finishes, run one bounded pinned
  `nix develop` probe for LanceDB, then the lean comparison if needed. Keep probe
  artifacts in the project and coordinate builds; do not start several model/
  search compilation graphs at once. Pin all native/transitive inputs and inspect
  feature unification, compiler compatibility and Nix/Flatpak packaging implications.
- [ ] Demonstrate local fuzzy queries and library-provided hybrid ranking on the
  exact Rust API, including filters/deletion/restart and no network access. No
  custom ANN, spelling engine, neural architecture, rank-fusion engine or daemon.
- [ ] Benchmark actual cold/warm latency, indexing throughput, idle/peak RSS, index
  size and incremental build/closure cost at 10k and 100k synthetic messages.
  Fix acceptance budgets before tuning. Choose one production backend based on
  these measurements and integration size; record why any heavier choice is needed.
- [ ] Explicitly provision a hashed model, validate reference embeddings and
  compare held-out English/French/Italian retrieval. Include the user's English
  doctoral re-enrollment query against French notices, paraphrases without lexical
  overlap, and near-topic negative controls. Never equate fuzzy success with
  semantic success, or publisher benchmark scores with our email acceptance.
- [ ] Implement only application glue: bounded source projection/chunk provenance,
  stable identities, local model/index ownership, cancellation and native result
  presentation. Keep SQLite canonical; preserve drafts/reminders and read status.
  Record the final library choice before creating the production search component.
