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

Later application rules advance separately. The probe materializes policy 2
from Git revision `783e1fa459524fa4169273f59f54b78cacce3fbb` so reproducing
the rejected candidate measures the original rule source and projections.

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


## Second development wording pass, before full development encoding

The nine-message wording diagnostic confirms the three logical label controls and
shows stronger action entailment from the frozen second-person hypotheses than
from metatextual genre descriptions. Receipt negatives still do not reliably
entail the informational hypothesis. Preserve the complete diagnostic scores,
not just the favourable cases, in `wording-diagnostic.log` under the quality
evidence artifacts. Choose the second-person family for a full **development-only**
export. `nli-policy-p1.json` preserves the first policy. The current p2-development
policy changes only the four hypotheses before this export; its inherited
three-way thresholds are not a final calibrated policy. Any later threshold
selection may use only these 108 development rows. The 90 held-out rows remain
unencoded and cannot be used to select hypotheses or thresholds.


## Development calibration of the second-person observations

Before recalculating combined metrics, register a single small revised policy:
use the maintained independent-label entailment-versus-contradiction action score
>=0.70, while retaining three-way action entailment >=0.25 and neutral <=0.75.
Require an existing future actionable deadline or three-way urgency entailment
>=0.15, neutral <=0.75 and urgency entailment-versus-contradiction >=0.40. The
urgency vote is deliberately weaker than the action vote; it cannot independently
create priority. Strong contradiction/information vetoes and definitive rule
guards are unchanged. Do not interpret these scores as calibrated probabilities.
This policy is chosen from the development-only wording observations, including
their false/neutral receipt examples, before computing its aggregate metrics.
Preserve the inherited-threshold p2 result as another non-improving outcome.
No held-out row has been encoded; final adoption still requires its unchanged
quality and benefit gates.


## Final p2 freeze before held-out inference

The single registered calibration corrects seven development false negatives
without adding a false positive or regressing a previously correct decision.
Combined EN/FR/IT recall is 20/20, 19/20 and 18/20 at 100% development precision;
the model alone recalls 11/20, 16/20 and 10/20. Retain the complete counts, including
abstentions, in `p2-calibrated-development.log`. These are small correlated training
examples, not accuracy claims. Freeze `nli-policy.json` as **nli-0a71-p2**, unchanged
hypotheses, thresholds, source projection and guards. This is the final policy
before encoding any held-out row. No further tuning on its results is allowed.

The held-out evaluator now explicitly fails unless both original and combined
per-language precision/recall and accepted-deadline precision meet the unchanged
limits, and combined macro F1 improves with at least one correction and no increase
in total false positives. It writes all outcomes before reporting failure. Its
result still cannot prove application lifecycle, combined memory, native UI or
live-provider acceptance.


## Held-out decision: rejected, no application adoption

The frozen p2 evaluation encoded exactly 90 held-out rows and **failed** its
unchanged quality and benefit gate (exit 1). Combined EN/FR/IT counts are
TP/FP/FN = 14/2/3, 15/4/2 and 17/7/0. Rules alone are 14/2/3, 14/2/3 and 15/4/2.
Thus there are three model-dependent corrections but five regressions, total
false positives grow from eight to thirteen, and combined macro F1 falls from
0.8434 to 0.8370. Original-subset Italian precision also fails (14/18 = 0.7778).
Accepted deadline precision is 12/14 = 0.8571 in every language on the combined
set: the rule parser misattributes dates in two semantic challenge families,
and the context model does not veto them. The original subset alone achieves
12/12 accepted dates, demonstrating why the semantic extension was necessary.
Warm p95 is 247.317 ms; isolated peak is 859264 KiB.

Keep `p2-heldout.log` with all subsets/counts. Do not retune this policy on those
results, relabel the failures, remove the challenge families or call the rule
fallback model success. This candidate is not selected for application use.
Production remains rules-only with its documented contextual limitations. A new
candidate requires a separately frozen held-out extension and its own resource
and quality evidence. Numeric/reference and isolated loading success do not
override the failed task-quality gate.
