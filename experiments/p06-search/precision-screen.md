# Frozen relevance-cutoff screen

2026-11-13. The first native GUI walkthrough retrieved the correct French notice
first, but displayed unrelated tail results. No similarity scores have yet been
examined to select a cutoff. Before evaluating this additional screen, fix the
candidate at **cosine similarity >=0.5** (USearch cosine distance <=0.5).

Adopt it only if the unchanged original relevance gates still pass (>=8/10 first,
10/10 top-three, the doctoral example first), the unchanged held-out gates still
pass (>=20/24 first and >=23/24 top-three), and at most one of the twelve frozen
no-answer queries returns any result against either original or held-out scoped
corpus. All twelve concern absent cinema, guitar, recipe or chess correspondence.
Also require the native four-message doctoral walkthrough to retain the French
notice while excluding its library/department/budget distractors.

Keep failed outcomes and these datasets unchanged. These are synthetic screening
criteria, not proof of perfect no-answer handling or a universal cosine cutoff.
This proposes a filter over the existing library metric, not a new ranker/model.

## First candidate and separate calibration rule

The fixed 0.5 candidate failed the unchanged original screen: 8/10 first and
8/10 top-three, dropping two required cross-language positives. It is rejected.
The run stopped there; neither the held-out cutoff outcomes nor no-answer
similarities were examined.

Before the next evaluation, fix a development-only calibration rule: take the
lowest expected-positive cosine among the original ten development queries,
subtract 0.02, round down to three decimals, and clamp to [0, 0.5]. Freeze that
number before testing the held-out set or the no-answer queries. Keep all original,
held-out and no-answer acceptance requirements above unchanged. This uses the
original set for calibration; it does not relabel it as held-out acceptance.

The frozen development rule produced **0.371** (minimum expected-positive cosine
0.39173101 minus 0.02, rounded down). See the committed development-calibration
log. That value is now frozen before held-out/no-answer cutoff evaluation.


## Rejected no-answer behavior and product boundary

The frozen 0.371 run preserved the original gate (10/10 first, 10/10 top-three)
and held-out gate (22/24 first, 23/24 top-three), but returned suggestions for
**4/12** absent-topic queries, exceeding the unchanged maximum of one. All three
cinema translations and English guitar were false positives. It is **rejected
for automatic relevance/no-answer acceptance**. The complete log is retained.
Neither datasets nor acceptance criteria have been relaxed after inspecting them.

The native feature instead exposes an explicit **Related** button with a selected
state, related-mail heading/status, and a warning in its tooltip/Settings that
suggestions may be unrelated. Ordinary word search stays the default; quotes and
field constraints stay lexical. The development-derived 0.371 floor only trims
weak tail candidates; it is not a calibrated relevance probability or an accepted
no-answer detector. A future quality improvement must retain these frozen cases
and add independently frozen validation data, rather than tune on this test set.

The maintained Sentence Transformers [retrieve-and-rerank guidance](https://www.sbert.net/examples/sentence_transformer/applications/retrieve_rerank/README.html)
distinguishes cheap bi-encoder retrieval from more accurate cross-encoder ranking.
If the additional quality gate requires a reranker, use an existing local
multilingual model/runtime over the bounded shortlist; do not build a ranker or
compare every mailbox original to every query. Its extra model, memory and latency
costs require a separate explicit screening decision. No reranker is added here.


## Reproduction

Use the explicitly provisioned model and production fixed/held-out vector exports
from the backend evidence. Generate the twelve new query vectors with the existing
independently parity-checked encoder probe (its output uses F32 bit patterns):

```sh
nix develop --command bash experiments/p06-search/encoder.sh check
mkdir -p target/p06-search/precision
python3 - <<'PYTHON'
import json,pathlib
root=pathlib.Path('target/p06-search/precision')
negative=json.loads(pathlib.Path('experiments/p06-search/no-answer.json').read_text())
(root/'input.json').write_text(json.dumps({'documents':[], 'queries':negative['queries']}))
for name in ('fixed','heldout'):
    (root/f'{name}.json').write_bytes((pathlib.Path('target/p06-search/production-model-debug')/f'{name}.json').read_bytes())
PYTHON
nix develop --command target/release/mailstronaut-encoder-probe \
  target/p06-search/model-minilm target/p06-search/precision/negative.json \
  target/p06-search/precision/input.json
nix develop .#search-reference --command bash -c \
  '"$MAILSTRONAUT_REFERENCE_PYTHON" experiments/p06-search/precision.py target/p06-search/precision 0.371'
```

The final command is **expected to fail** the no-answer criterion; do not change
its exit status into an acceptance marker. Run builds through `ci/remote.sh` per
AGENTS.md; model setup and fixed exports are explicit prerequisites, not downloads
performed by this script.
