# Strong search: library research and adoption gate

Initial research date: 2026-10-30. Tantivy lexical/fuzzy search is now integrated;
the model comparisons below remain isolated probes. The user requires
both typo tolerance and multilingual search by meaning, and explicitly wants
pre-existing components. Do not write a fuzzy matcher, inverted index, vector
index, transformer, translation system or search service for this feature.

## Decision (2026-11-01): Tantivy for typo-tolerant search

**`mailstronaut-search` uses Tantivy 0.27.0, and that part of the gate below is closed.**
The same machine and pinned toolchain gave these release results:

| At 100,000 synthetic messages | Tantivy 0.27 | LanceDB 0.41 (earlier probe) |
| --- | --- | --- |
| Added crates | 43 | About 500 |
| Worst query p95 | 35 ms | 7 ms fuzzy, 225 ms vector |
| Peak process memory | 133 MiB | 876 MiB |
| Index size | 55 MiB | 165 MiB |

The corpora differ, so only the orders of magnitude compare. LanceDB's single advantage was
one engine for both text and vectors, but neither encoder has passed the relevance screen,
so that advantage buys nothing yet at a cost the desktop would always pay.
[Measurements and behavior](evidence/P06-fuzzy-search.md).

Search by meaning stays open. It needs an encoder that passes the screen first, and then a
vector library beside this index. LanceDB remains a candidate only if that combination
turns out worse than its resource cost.

### Next encoder candidate (2026-11-10)

The pinned multilingual MiniLM L12 sentence encoder passes the unchanged fixed
screen (10/10 first), independent Candle/PyTorch parity and the provisional
encoder-only release resource limits. It reuses Candle's existing BERT model,
with no ONNX runtime or new production dependencies. [Exact evidence and limits](evidence/P06-minilm-encoder.md).
A separate 26-document/24-query synthetic set was frozen in `98d90a9` before
evaluation. Rust/reference parity passed and retrieval scored 22/24 top-one,
24/24 top-three, above its predeclared thresholds. The two book-pickup queries
preferred a renewal distractor and are retained as limitations. Production model
lifecycle and the semantic UI/index remain open; this is not an implemented
meaning-search feature.

### Native metadata filters (2026-11-10)

The search toolbar's standard Kit popover combines a literal sender substring with
an inclusive received-date range. Sender matching covers parsed From names and
addresses, excluding To/Cc recipients. Dates use the configured time zone and the
following calendar day's midnight as the exclusive upper bound, including DST
changes. Sender matching is ASCII case-insensitive; punctuation is literal, with
no wildcard or query-language interpretation. Account/folder scope stays in the
sidebar. Applying, clearing and changing scope never mark mail Seen.

With a text query, Tantivy's existing range and regex queries restrict originals
before its bounded hit collection. SQLite rechecks restrictions on the same
original used for opening, preserves in-scope thread unread counts and deduplicates
conversations. Without query text, the store reads filtered metadata directly in
bounded batches; the text index's hit cap does not restrict those results. The
additional raw sender field rebuilds the derived index on schema mismatch through
the existing recovery path. [Behavior and checks](evidence/P06-search-filters.md).

## Original recommendation, before the measurements above

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
LanceDB's exact `lance-index =14.0.2` source was inspected as well. That initial audit
added no application dependencies; Tantivy was adopted later as described above.
A later isolated API probe
built and ran successfully; see [probe evidence](evidence/P06-search-probe.md).
Model and adoption/resource acceptance remain open.
The first E5 inference/reference probe passed numerical parity but failed the
fixed multilingual ranking screen. [Results](evidence/P06-model-probe.md).
The existing static Model2Vec library then retrieved all ten expected items in the
top three, but also missed the fixed rank-first/top-one criteria; its debug load
memory initially needed paired release profiling. LanceDB was the initial unified
backend candidate; the later lexical decision above supersedes that recommendation.
Neither encoder is adopted yet. These findings do not close P06. The subsequent
[paired release comparison](evidence/P06-encoder-comparison.md) found that static
encoding is fast, but its 1.66 GiB process peak exceeds the existing memory screen.
Both candidates still fail the fixed relevance screen.

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
provides local-only loading and static embeddings. Its multilingual Potion model
is 128M parameters, not the much smaller English-only 8M variant. The audited
artifact is about 489 MiB. A subsequent checksum-verified inspection of the
published 0.3.0 source found that its I8 loader expands values to F32 storage;
the newer repository quantization API must not be treated as a proven released
memory reduction. Peak load, steady memory and mail retrieval quality still need
measurement. A shared model manifest/lifecycle can be reused without requiring
the same tensor runtime.
[Official Rust implementation](https://github.com/MinishLab/model2vec-rs),
[model card](https://huggingface.co/minishlab/potion-multilingual-128M),
[published source](https://docs.rs/crate/model2vec-rs/0.3.0/source/src/model.rs).

FastEmbed 7.5.0 is a credible ready-made embedding API, but its small ONNX model
path adds ONNX Runtime. Its default features also download runtime binaries/models;
those defaults are unsuitable here. Do not describe its newer Candle features as
universal support for small BERT models: the inspected manifest scopes them to
other model families. Keep this as a measured alternative if it substantially
reduces glue or resource use; no second runtime is selected now.
[Feature manifest](https://github.com/Anush008/fastembed-rs/blob/main/Cargo.toml).
Its published 7.5.0 source already includes the quantized multilingual MiniLM
variant `ParaphraseMLMiniLML12V2Q`, making it a concrete next candidate if a smaller
memory envelope is required. Probe the existing local user-defined-model loader,
explicitly provisioned artifacts, ONNX Runtime/Nix/Flatpak packaging and the same
unchanged quality screen before adopting it; no runtime or model is selected here.
[Published model list](https://docs.rs/crate/fastembed/7.5.0/source/src/models/text_embedding.rs),
[published embedding tests](https://docs.rs/crate/fastembed/7.5.0/source/tests/text-embeddings.rs).


The separate opt-in experiment now has a checksum-pinned E5 manifest. Explicit
provisioning and subsequent measurements are recorded in its evidence; metadata
alone does not establish model correctness or email retrieval adequacy. The
published Candle core 0.11 manifest also pulls in tokenizers 0.22/Oniguruma on
native targets, so the probe reuses that tokenizer instead of keeping 0.23.1 too.

## Required steps before application integration

- [x] Source-level comparison with exact published versions/checksums, public Rust
  API inspection, local-only model candidates and explicit tradeoffs recorded.
- [x] Build and exercise the isolated LanceDB 0.41.0 public Rust API under the
  pinned Nix development toolchain: fuzzy/filter/vector/hybrid queries, independent
  process reopen, logical deletion and controlled network observation passed.
  [Evidence](evidence/P06-search-probe.md). This is not model/relevance/adoption proof.
- [x] Run one bounded pinned `nix develop` release probe for LanceDB. It passed the
  provisional 10k/100k latency, memory and disk screening targets; see
  [measurements and limits](evidence/P06-search-probe.md). Keep probe
  artifacts in the project and coordinate builds; do not start several model/
  search compilation graphs at once. The lean comparison remains available if
  combined model/resource costs require it. Application feature unification and
  Nix/Flatpak packaging remain unproven.
- [x] Demonstrate bounded synthetic local fuzzy queries and library-provided hybrid
  ranking on the exact Rust API, including filters/logical deletion/process restart
  and controlled network observation. [Evidence](evidence/P06-search-probe.md). No
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
