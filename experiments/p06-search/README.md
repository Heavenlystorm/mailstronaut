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
DataFusion 54's Arrow minimum conflicts with an exact 58.0.0 pin. The initial
standalone lock had 524 entries. The expanded model experiment locks 583 entries
including its two local crates. Dependency fetching is separate from the explicit
model artifact provisioning below.

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

## Explicit multilingual model experiment

`model-fetch` explicitly provisions only the committed E5 model manifest to
`target/p06-search/model`; it verifies sizes and SHA-256 hashes before publication.
No application startup or inference code downloads anything. This is about 465 MiB
including the tokenizer. `embedding-build` checks the existing Candle 0.11 CPU
BERT and tokenizers 0.22.2 APIs with default accelerator/network features off.
This reuses Candle core's existing tokenizer version and Oniguruma backend;
the preliminary 0.23.1/fancy-regex choice otherwise duplicates that stack.
`embedding-release` builds the release executable; `embedding-smoke` runs the
synthetic relevance corpus, library vector/hybrid retrieval and network tracing.
All commands use the same `nix develop .#search-probe` prefix as above.

Before running the model, the fixed ten-query English/French/Italian screen in
`relevance.json` requires semantic-only top-1 >= 8/10 and top-3 = 10/10. The user's
English doctoral re-enrollment example must rank the French notice first despite
the English near-topic distractor. Other-account and Archive duplicates must
never appear. These are small synthetic feasibility fixtures, not a representative
held-out mailbox evaluation or numerical reference-parity proof.

Preliminary CPU-model screening budgets: hash-verified load <= 5 s, short-query
warm p95 <= 300 ms, model/search process peak RSS <= 1.5 GiB. Record failures
honestly rather than changing these thresholds after observing results. The
combined 100k-message model/GUI footprint is a separate acceptance requirement.
The safe owned-buffer safetensors loader avoids workspace unsafe code but may
temporarily duplicate weight storage. E5 task prefixes, attention-mask mean
pooling and L2 normalization follow the publisher; no transformer is implemented
here. Output embeddings are retained for a later independent local reference.
Their JSON stores exact IEEE-754 bit patterns as integers: the pinned Cranelift
backend aborts in the JSON dependency's SIMD float formatter. Integer interchange
avoids that diagnostic-only path without a compiler override or precision loss.

Run `reference.py OUTPUT_EMBEDDINGS_JSON` through
`nix develop .#search-reference --command bash -c
'"$MAILSTRONAUT_REFERENCE_PYTHON" experiments/p06-search/reference.py "$1"' -- PATH`.
This test-only CPU PyTorch/Transformers environment uses the same local safetensors
and tokenizer, with offline mode, no remote code and no GPU. Its fixed acceptance
is maximum absolute component error <= 1e-4 and minimum paired cosine >= 0.99999.
It is not a Python application dependency or an inference service.

`potion-fetch` explicitly provisions the second committed manifest to
`target/p06-search/model-potion`. After `embedding-build`, `potion-smoke` uses the
**debug/Cranelift** executable and the published Model2Vec 0.3 local-only byte
loader and `encode_single`, with no E5 task prefixes. It reuses the identical ten
queries, filters, LanceDB ranking and fixed relevance thresholds. Those timings
are debug/traced screening only. The released model loader keeps F32 storage;
no unverified main-branch quantization API or custom static encoder is used.

For comparable untraced release measurements, first run `embedding-release` once,
then run `embedding-bench` and `potion-bench` sequentially through the same pinned
search-probe shell. Both use the same release executable and corpus; only the
verified model changes. These modes preserve the relevance assertion and exit
nonzero if it fails. Record completed timing/resource output separately from the
quality result. Query timing measures the encoder only, not end-to-end retrieval;
GNU time reports process peak RSS, not idle memory. OS page caches are uncontrolled.
[Paired results](../../docs/evidence/P06-encoder-comparison.md).

## Quality-first MiniLM comparison

The next candidate is the publisher's
[multilingual MiniLM L12 sentence encoder](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/tree/e8f8c211226b894fcb81acc59f3b34ba3efd5f42),
pinned in `minilm-model-manifest.json` (Apache-2.0). Its standard BERT architecture
matches the existing Candle API family; compatibility and numerical parity for
these weights still need actual Rust proof. It uses the published 128-token limit,
attention-mask mean pooling and no E5 task prefixes. Both the sentence and pooling
configuration files are pinned and validated rather than inferred from the name.

Before rebuilding an encoder/search graph, run the unchanged ten-query relevance
screen through the independent, test-only CPU reference environment:

```sh
nix develop --command python3 experiments/p06-search/provision-model.py --minilm
nix develop .#search-reference --command bash -c \
  '"$MAILSTRONAUT_REFERENCE_PYTHON" experiments/p06-search/screen.py minilm'
```

Provisioning is explicit and hash-verifies every artifact. `screen.py` runs entirely
from the verified local files with remote code, telemetry, network model fallback
and GPU use disabled. It computes the small fixture's pairwise cosine scores with
NumPy, not a new production retrieval engine. Account/folder duplicates are excluded
before ranking. The original thresholds remain top-1 >=8/10, top-3 =10/10 and the
user example first; a failure returns nonzero. `screen.py e5` can repeat the older
encoder's reference-only control if its verified files are present.

This is an early quality screen, not model adoption, held-out relevance, Rust
numerical parity, a Rust resource benchmark or an implemented semantic-search UI.
No application Cargo dependencies change for this screen. A passing candidate must
still meet those later gates before integration.

`encoder.sh lock` deliberately generates the separate encoder-only lockfile;
normal reproduction uses `fetch`, `check`, `run` and `privacy` through the pinned
search-probe shell. This reuses `workspace.toml.in` and the root lint policy but
materializes only a BERT encoder member, so the 160-package lock contains no
LanceDB/Arrow/search engine. It reuses the main Cargo target directory. Release
uses LLVM/thin LTO/one codegen unit. `privacy` includes an actual TCP observer
control and rejection of a same-size tampered configuration.

Pass the exported vector file to `reference.py OUTPUT --minilm` through the
search-reference shell to verify numerical parity and score the actual Rust
vectors. Resource measurements from `run` are untraced and separate from `privacy`.

`relevance-heldout.json` freezes 26 new synthetic documents and 24 cross-language
queries, including same-topic negatives and foreign-account/archive copies,
before the candidate is evaluated against them. Its predeclared screen is at
least 20/24 top-one and 23/24 top-three. This is additional synthetic coverage,
not a claim about a representative real mailbox. Run `screen.py minilm --held-out`.
For Rust parity use the same release encoder with a third argument naming this
fixture, then `reference.py OUTPUT --minilm --held-out`. Preserve every miss;
do not edit the frozen corpus or thresholds after observing results.
