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
