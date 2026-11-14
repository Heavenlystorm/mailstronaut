# Frozen synthetic priority-quality screen

2026-11-14, before running the classifier on these messages. The numerical API
probe is separate: it proves arithmetic/tokenizer parity, not these labels.

`priority-corpus.json` contains 180 synthetic messages, 60 each in EN/FR/IT.
There are **30 authored semantic families**, translated into three languages, each
with bare and greeting/signature variants. Thus the 180 rows are not 180 independent
semantic cases. The fixed split holds every sender, thread, family, translation
and variant together: 18 development families (108 messages) and 12 held-out
families (72 messages). Each held-out language has 24 messages, 14 positive and
10 negative; effective sample sizes and template correlation must be disclosed.

Freeze receipt/authored time at 4 November 2026 09:00 Europe/Paris. Keep expected
priority/deadline labels unchanged. Include required academic forms, signatures,
questionnaires, payments, RSVP/check-in, healthcare/account access, implied blocking
requests and list obligations; negatives include receipts, completed/expired tasks,
optional event dates, quoted requests, negation, marketing and malicious model
instructions. Longer true deadlines must not be clamped to the no-deadline TTL cap.
No real mail, external resource or private identity occurs in the corpus.

Use the unchanged small NLI model and the existing rule/date/quote pipeline as a
baseline. The prospective initial model policy is the existing CLASSIFIER.md
threshold: action entailment >=0.70 and (time-sensitive entailment >=0.65 or an
accepted future actionable deadline). Require action entailment minus neutral
>=0.15. Strong informational evidence (entailment >=0.80), unsupported inputs and
ambiguous evidence abstain; existing definitive no-action, read/dismissal and
expired-deadline guards remain authoritative. Next-day evidence may choose the
24-hour fallback, otherwise normal action uses 72 hours; accepted deadlines win.
Model probabilities are uncalibrated scores, not user-facing confidence promises.

Calibrate only on the development split, record any revised policy and freeze it
before evaluating held-out outputs. The harness must not encode held-out messages
as a side effect of development evaluation. Do not regenerate families, relabel
failures or tune on held-out results. If a defect in a label is later established,
retain the original outcome and explain it before introducing a new separately
frozen screen.

Unchanged held-out targets: priority precision >=0.85, recall >=0.65 in every
language, accepted-deadline precision >=0.95 with coverage, and zero read-cleared or
expired originals in priority. Report raw counts, per-language abstention and
rules-only comparison; disclose unsupported forms and mixed-language limitations.
A synthetic pass is not live-mail accuracy or calibration proof. Reference logits,
full per-mail CPU/p95/memory, idle unloading/cancellation, combined semantic-model
costs and actual native/runtime/packaging acceptance remain separate gates.

## Held-out semantic challenge extension, before first evaluation

The initial 180 messages remain byte-for-byte unchanged (SHA-256
`a2ff10ab4315b03c8a185301baff00f90f8f15ae4038c308ead8b59e268306ca`).
Their simple obligations deliberately exercise the existing rules, so they alone
may not establish a benefit from adding a model. Before any model **or rules**
evaluation on either quality set, freeze `semantic-challenges.json`: six new
semantic families, three translations each, **held-out only**, 18 additional rows.
They cover implied recipient authorisation/consent, another person's obligation,
inline grammar examples and a superseded request. No new sender/thread/family
crosses into calibration. These are context decisions character matching alone
cannot reliably make.

Combined scope: 198 messages, 36 families, 66 messages per language. Development
stays the original 108 rows; held-out is 90 rows, 30 per language (17 positive,
13 negative). Report original and challenge subsets separately as well as combined
metrics. Keep the original language thresholds and require them on both the
original held-out and combined set. Model adoption also needs a held-out benefit
over rules, not just a passing raw accuracy: report additions/vetoes and require
at least one model-dependent correction and improved combined macro F1 without
increasing total false positives. Small correlated synthetic counts still do not
establish live-mail accuracy.

Clarify the prospective combined decision before evaluation: existing definitive
no-action/expired guards win. A confident model-positive uses the initial action,
neutral-margin, urgency/deadline criteria above. A strong non-obligation result
(action contradiction >=0.70 with >=0.15 margin over neutral, or informational
entailment >=0.80 with action entailment <=0.30) can veto a rule positive. Otherwise
the model abstains and preserves the existing rules-only decision, explicitly
recorded as such. This fallback does not count as model coverage or a model
correction. Report model-only scores/abstention alongside the combined policy.
Accepted deadline precision must use the same context veto, retaining rejected
text as uncertain evidence rather than inventing a deadline. These rules may be
calibrated only on development data and frozen again before held-out scoring;
none of the held-out messages may be encoded during development evaluation.


## Development harness, frozen before first scoring

`nli-policy.json` is the initial numeric policy above, with explicit hypotheses,
projection identity and the exact policy-2 rule-source hash. Conflicting strong
action and informational votes abstain to rules. `nli-quality.rs` reuses the exact
application rule/date/quote source as a module materialized in the ignored probe;
it does not maintain a second matcher. Synthetic sender/group/thread identifiers
are used only for split validation and reports, never as model input. Input is the
subject and the library's authored-text ranges. Gold labels are used only for
metrics, not projection or decisions.

The isolated quality lock adds the already pinned application Chrono/timezone/regex
graph, with no application Cargo.lock change. Check/build first; encode **only the
development split** and then evaluate the raw observations with the offline test
runtime:

```sh
nix develop --command bash experiments/p09-priority/nli.sh check
nix develop --command bash experiments/p09-priority/nli.sh quality-development
nix develop .#search-reference --command bash -c \
  '"$MAILSTRONAUT_REFERENCE_PYTHON" experiments/p09-priority/quality-evaluate.py development'
```

Run these commands through `ci/remote.sh`. Compilation copies the frozen corpora,
but selection happens before inference or per-message output: development must
contain exactly 108 rows and no challenge/held-out observations. The explicit
`quality-heldout` and evaluator `heldout` commands are reserved until development
calibration is finished and its final policy is committed. Successful export or
metric calculation alone is **not** an acceptance marker.


## Initial development result and predeclared wording diagnostic

Policy p1 produced no combined corrections over the rules on the 108 development
messages. Its high three-way entailment and entailment-minus-neutral requirements
mostly abstained: the model-only action/urgency policy found zero English/French
positives and one Italian positive. The combined fallback preserved the rules,
so high combined precision/recall must **not** be attributed to the classifier.
The full candidate-quality/adoption gate is not passed. Held-out data remains
unencoded. [Initial results](../../docs/evidence/artifacts/p09-quality/p1-development.log),
[development-only diagnostics](../../docs/evidence/artifacts/p09-quality/p1-diagnostics.log).

Before further calibration, freeze `hypothesis-candidates.json`: two simpler
hypothesis families and three elementary NLI label-order sanity controls. Compare
only bare development examples from enrollment, the implied blocking request and
receipts in all three languages; no held-out or challenge example is used. Keep
all scores and the initial outcome. This diagnoses whether metatextual wording,
input framing or scoring is at fault before selecting any new policy.
The maintained Transformers [zero-shot pipeline](https://github.com/huggingface/transformers/blob/main/src/transformers/pipelines/zero_shot_classification.py)
uses templated hypotheses and, in independent-label mode, entailment-versus-
contradiction normalization. Any use of that standard score must retain the
three-way neutral value for abstention, be calibrated only on development and
be frozen before held-out evaluation. It is not a trustworthy confidence number.

The diagnostic's split assertion initially stopped after the three elementary
controls: marketing belongs to the held-out split, so it was correctly excluded
and the expected nine development examples were not available. No variant or
held-out example was encoded. Replace that diagnostic group with the already
frozen development receipt family before running the wording comparison; never
move or alter the original corpus split. The three basic controls confirm the
published entailment/neutral/contradiction label interpretation.
