# P06 — Paired release encoder screening

Neither encoder is adopted. This closes the previous debug/traced versus
release/untraced comparison gap, without closing model quality or P06.

The existing isolated probe was built once with pinned `nix develop .#search-probe`,
LLVM release, thin LTO and one codegen unit. The retained build gate's service was
`mailstronaut-search-encoder-release.service`, invocation
`db2d068259ae4b0a9ad7febaa5678233`. Compilation passed in 5m56s; its subsequent
traced E5 relevance screen returned **101**, and the service is inactive. This is
an expected, retained quality failure, not an overall passing probe gate.
No application dependencies or model downloads were added.

After compilation, `embedding-bench` and the new `potion-bench` ran sequentially
with the same release executable, host, local verified manifests, synthetic
corpus, filters and existing LanceDB query/ranking code. Neither process was
traced, and no local compilation ran concurrently.

| Measurement | Candle E5-small | Model2Vec multilingual Potion |
| --- | ---: | ---: |
| Hash-verified model load | 1,016.575 ms | 3,413.726 ms |
| Warm query **encoding** p95, 30 samples | 51.965 ms | 0.017 ms |
| Process peak RSS | 1,263,240 KiB (1.20 GiB) | 1,735,900 KiB (1.66 GiB) |
| Semantic top-1 | 6/10 | 6/10 |
| Semantic top-3 | 8/10 | 10/10 |
| User's English query / French notice rank | 2 | 2 |
| Relevance assertion exit | 101 | 101 |

[Raw E5 output](artifacts/p06-encoder-comparison/e5.log),
[E5 resources](artifacts/p06-encoder-comparison/e5-resources.txt),
[Potion output](artifacts/p06-encoder-comparison/potion.log),
[Potion resources](artifacts/p06-encoder-comparison/potion-resources.txt),
[machine-readable receipt](artifacts/p06-encoder-comparison/summary.json).
Full runs: `target/p06-search/embedding.pY6YekTc` and
`target/p06-search/embedding.WLEGvYOe`.

The predeclared screen still requires the user example first, top-1 >=8/10 and
all expected results in the top three. Both miss it. Potion also exceeds the
predeclared 1.5 GiB process-peak limit, despite low encoding CPU time; static
embeddings have not demonstrated a smaller memory footprint here. The thresholds
were not changed after observing these results.

These are single-process synthetic screening samples. Encoding timings exclude
LanceDB retrieval and the GUI, the first timed query follows document encoding,
and filesystem page caches were not reset. Peak RSS is not idle RSS or a combined
100k-message application measurement. The unchanged small corpus is not held-out
mailbox acceptance, and this untraced pair adds no network-observation proof.
Existing E5 numerical-reference and separate traced observations remain recorded
in [the model probe](P06-model-probe.md).

Reproduce with the common `nix develop .#search-probe --command bash
experiments/p06-search/reproduce.sh MODE` prefix: `embedding-release`, then
`embedding-bench`, then `potion-bench`. Preserve each nonzero relevance result.
ShellCheck passed through the pinned search-probe shell for the changed runner.
