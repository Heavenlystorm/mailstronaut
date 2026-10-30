# P06 local multilingual model feasibility

Status: E5 inference/reference checks passed, but the fixed retrieval screen failed.
No semantic-search model or search backend has been integrated into the application.
The existing client still uses its literal SQLite FTS path.

The experiment reuses Candle 0.11's existing BERT implementation, tokenizers 0.22.2,
and LanceDB 0.41's vector/filter/hybrid APIs. E5-small is explicitly provisioned
from the committed [manifest](../../experiments/p06-search/model-manifest.json):
all five artifact sizes and SHA-256 hashes passed. The application downloads
nothing. See [provisioning](artifacts/p06-model-probe/provision.log) and the
[reproducible commands](../../experiments/p06-search/README.md).

## Independent correctness and relevance

The Rust adapter uses the publisher's query/passage prefixes, attention-mask mean
pooling and L2 normalization. A separate CPU PyTorch 2.14.1 / Transformers 5.17.0
process loaded the same verified local files with offline mode, no remote code,
no GPU, and eager attention. Across all 12 synthetic documents and ten queries:

| Comparison | Maximum absolute component error | Minimum paired cosine |
| --- | --- | --- |
| Documents | 1.27e-7 | 0.99999982 |
| Queries | 2.04e-7 | 0.99999976 |

The [reference log](artifacts/p06-model-probe/reference.log) passed the criteria
fixed before observation (1e-4 and 0.99999 respectively).
Standard NumPy cosine calculations independently reproduced every LanceDB
semantic top-three ordering; see [independent rankings](artifacts/p06-model-probe/independent-ranking.log).
This bounds the conclusion to the actual fixed
fixtures; it is not numerical proof for every tensor shape/model/input.

The untouched ten-query English/French/Italian relevance screen scored **6/10
semantic top-one and 8/10 top-three**. The user's English doctoral re-enrollment
query retrieved the French notice second, behind an English near-topic distractor.
Both cross-language conference-refund queries missed the correct Italian notice
in the top three. Library RRF hybrid ranking did not repair those misses.
The predeclared screen required at least 8/10 top-one, 10/10 top-three and the
user example first. It therefore **failed**. Neither a correct implementation
nor a publisher benchmark is sufficient reason to adopt this model for the app.

Account/folder duplicates were excluded. Strace observed no IP socket calls in
either the Rust process or the independent reference. The earlier library probe
supplies the successful loopback-observer positive control. This is bounded
synthetic observation, not a general privacy or full mailbox acceptance claim.
The full [relevance log](artifacts/p06-model-probe/relevance.log) preserves the
misses and failed exit condition.

## Build, runtime and diagnostic limits

At `3dbe4d1`, the coordinated LLVM/thin-LTO/one-codegen-unit release build completed
in 13m53s. Its invocation was `a60e60e18c4545d5a0f5758c22217801`; the runner's numeric
receipt was **101**, because the rank-first assertion failed at runtime. The
service was later collected/inactive. That is a failed relevance gate, not a
successful package acceptance run. [Initial failure](artifacts/p06-model-probe/first-release-failure.log).

An untraced invocation of that same existing release executable measured a
hash-verified process-start model load of 1.15 s and short-query warm p95 of
54.7 ms over 30 repetitions, with process peak RSS 1.20 GiB. The
[untraced log](artifacts/p06-model-probe/release-untraced.log),
[resources](artifacts/p06-model-probe/release-untraced-resources.txt) and
[binary checksum](artifacts/p06-model-probe/release-binary.sha256) identify this
baseline. The OS file cache was warm; this is not cold-storage
latency. Later diagnostic changes did not alter model math, but have not been
rebuilt/rebenchmarked in release. Do not label these measurements as a release
check of the latest application or add separate library/model timings to claim
end-to-end UI performance. Whole-app idle/peak memory and 100k-message model/index
cost remain unmeasured.

Pinned development compilation and strict Clippy passed. The first diagnostic
export aborted in `zmij`'s SSE2 JSON F32 formatter under Cranelift; the
[backtrace](artifacts/p06-model-probe/diagnostic-float-backtrace.log) identified it
after inference and table creation. Exporting lossless IEEE-754 bit patterns as
JSON integers avoids this diagnostic path. No transformer, float formatter,
compiler override or dependency fork was added. The corrected Cranelift run
completed every query and the independent parity check, retaining its failing
quality status. No app package, CI, live-provider, GUI or Flatpak acceptance was
run for these isolated experiments.

The already researched Model2Vec static multilingual alternative is the next
bounded comparison against the same fixtures and thresholds. Its published Rust
release expands even loaded I8 values to F32; do not assume a quantized memory
benefit from a different repository API. No additional model is adopted here.
